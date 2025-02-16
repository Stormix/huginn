use common::{
  ChatMessage, ChatMessageType, PartitionConfig, ServiceError,
  ServiceStatus, StreamerStatus, RabbitMQConfig,
};
use common::config::ServiceConfig;
use tokio::sync::{mpsc, RwLock};
use futures::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use lapin::{
  Connection, ConnectionProperties, Channel, Consumer,
  options::*, types::FieldTable,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::HashMap, sync::Arc};
use tracing::{info, error};

struct ChatCollector {
  partition_config: PartitionConfig,
  rabbit_channel: Channel,
  rabbit_consumer: Consumer,
  redis_client: redis::Client,
  db_pool: PgPool,
  active_connections: Arc<RwLock<HashMap<String, WebSocketConnection>>>,
  service_status: Arc<RwLock<ServiceStatus>>,
}

struct WebSocketConnection {
  streamer: String,
  sender: mpsc::UnboundedSender<Message>,
  task_handle: tokio::task::JoinHandle<()>,
}

impl ChatCollector {
  async fn new(
      config: ServiceConfig,
      partition_config: PartitionConfig,
  ) -> Result<Self, ServiceError> {
      // Initialize database connection pool
      let db_pool = PgPoolOptions::new()
          .max_connections(5)
          .connect(&config.database_url)
          .await?;

      // Initialize RabbitMQ connection
      let conn = Connection::connect(
          &config.rabbitmq_url,
          ConnectionProperties::default(),
      ).await?;
      
      let channel = conn.create_channel().await?;

      // Declare exchanges and queues
      channel
          .exchange_declare(
              RabbitMQConfig::STREAMER_STATUS_EXCHANGE,
              lapin::ExchangeKind::Topic,
              ExchangeDeclareOptions::default(),
              FieldTable::default(),
          )
          .await?;

      // Set up consumer
      let consumer = channel
          .basic_consume(
              RabbitMQConfig::STREAMER_STATUS_QUEUE,
              "chat_collector",
              BasicConsumeOptions::default(),
              FieldTable::default(),
          )
          .await?;

      // Initialize Redis client
      let redis_client = redis::Client::open(config.redis_url)?;

      Ok(Self {
          partition_config,
          rabbit_channel: channel,
          rabbit_consumer: consumer,
          redis_client,
          db_pool,
          active_connections: Arc::new(RwLock::new(HashMap::new())),
          service_status: Arc::new(RwLock::new(ServiceStatus::Starting)),
      })
  }

  async fn connect_to_chat(&self, streamer: &str) -> Result<(), ServiceError> {
      let ws_url = format!("wss://kick.com/chat/{}", streamer); // Replace with actual WebSocket URL
      
      let (ws_stream, _) = connect_async(ws_url).await
          .map_err(|e| ServiceError::WebSocket(e.to_string()))?;
      
      let (write, read) = ws_stream.split();
      let (tx, rx) = mpsc::unbounded_channel();

      // Set up message handling
      let db_pool = self.db_pool.clone();
      let streamer_name = streamer.to_string();
      
      // Handle incoming messages
      let read_task = tokio::spawn(async move {
          let mut read_stream = read;
          
          while let Some(message) = read_stream.next().await {
              match message {
                  Ok(msg) => {
                      if let Err(e) = Self::process_chat_message(
                          &db_pool,
                          &streamer_name,
                          msg,
                      ).await {
                          error!("Error processing message: {:?}", e);
                      }
                  }
                  Err(e) => {
                      error!("WebSocket error: {:?}", e);
                      break;
                  }
              }
          }
      });

      // Handle outgoing messages
      let write_task = tokio::spawn(async move {
          let mut write_stream = write;
          let mut rx_stream = rx;
          
          while let Some(message) = rx_stream.recv().await {
              if let Err(e) = write_stream.send(message).await {
                  error!("Error sending message: {:?}", e);
                  break;
              }
          }
      });

      // Combine tasks
      let task_handle = tokio::spawn(async move {
          tokio::select! {
              _ = read_task => info!("Read task completed"),
              _ = write_task => info!("Write task completed"),
          }
      });

      // Store connection
      self.active_connections.write().await.insert(
          streamer.to_string(),
          WebSocketConnection {
              streamer: streamer.to_string(),
              sender: tx,
              task_handle,
          },
      );

      Ok(())
  }

  async fn process_chat_message(
      db_pool: &PgPool,
      streamer: &str,
      message: Message,
  ) -> Result<(), ServiceError> {
      let msg_text = message.to_text()
          .map_err(|e| ServiceError::WebSocket(e.to_string()))?;
      
      // Parse message (implement actual parsing logic based on Kick's format)
      let chat_message = parse_chat_message(streamer, msg_text)?;

      // Todo: store in database

      Ok(())
  }

  async fn disconnect_from_chat(&self, streamer: &str) -> Result<(), ServiceError> {
      if let Some(conn) = self.active_connections.write().await.remove(streamer) {
          conn.task_handle.abort();
          info!("Disconnected from {}'s chat", streamer);
      }
      Ok(())
  }

  async fn handle_streamer_status(&self, status: StreamerStatus) -> Result<(), ServiceError> {
      let streamer = status.username;
      
      if status.is_live {
          // Connect if not already connected
          if !self.active_connections.read().await.contains_key(&streamer) {
              info!("Connecting to {}'s chat", streamer);
              self.connect_to_chat(&streamer).await?;
          }
      } else {
          // Disconnect if connected
          if self.active_connections.read().await.contains_key(&streamer) {
              info!("Disconnecting from {}'s chat", streamer);
              self.disconnect_from_chat(&streamer).await?;
          }
      }

      Ok(())
  }

  async fn run(&mut self) -> Result<(), ServiceError> {
      info!("Starting chat collector service");
      *self.service_status.write().await = ServiceStatus::Healthy;

      while let Some(delivery) = self.rabbit_consumer.next().await {
          match delivery {
              Ok(delivery) => {
                  let status: StreamerStatus = serde_json::from_slice(&delivery.data)?;
                  
                  if let Err(e) = self.handle_streamer_status(status).await {
                      error!("Error handling streamer status: {:?}", e);
                  }

                  delivery.ack(BasicAckOptions::default()).await?;
              }
              Err(e) => {
                  error!("Error receiving message: {:?}", e);
                  *self.service_status.write().await = ServiceStatus::Degraded;
              }
          }
      }

      Ok(())
  }

  async fn shutdown(&self) -> Result<(), ServiceError> {
      info!("Initiating graceful shutdown");
      *self.service_status.write().await = ServiceStatus::ShuttingDown;

      // Disconnect from all chats
      let streamers: Vec<String> = self.active_connections
          .read()
          .await
          .keys()
          .cloned()
          .collect();

      for streamer in streamers {
          if let Err(e) = self.disconnect_from_chat(&streamer).await {
              error!("Error disconnecting from {}'s chat: {:?}", streamer, e);
          }
      }

      Ok(())
  }
}

fn parse_chat_message(streamer: &str, message_text: &str) -> Result<ChatMessage, ServiceError> {
  // Implement actual parsing logic based on Kick's message format
  // This is a placeholder implementation
  Ok(ChatMessage {
      streamer: streamer.to_string(),
      username: "user".to_string(),
      message: message_text.to_string(),
      timestamp: chrono::Utc::now(),
      message_type: ChatMessageType::Regular,
      metadata: None,
  })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  // Initialize tracing
  tracing_subscriber::fmt::init();

  // Load configuration
  let config = ServiceConfig::new().expect("Failed to load configuration");

  let partition_config = PartitionConfig {
      total_partitions: config.total_partitions,
      partition_id: config.partition_id,
  };

  // Create and run service
  let service = ChatCollector::new(config, partition_config).await?;
  let service = Arc::new(RwLock::new(service));
  let shutdown_service = service.clone();

  // Set up shutdown signal handler
  tokio::spawn(async move {
      if let Ok(()) = tokio::signal::ctrl_c().await {
          info!("Received shutdown signal");
          if let Err(e) = shutdown_service.write().await.shutdown().await {
              error!("Error during shutdown: {:?}", e);
          }
          std::process::exit(0);
      }
  });

  // Run the service
  service.write().await.run().await?;

  Ok(())
}