mod kick;

use actix_web::{App, HttpResponse, HttpServer, web};
use chrono::Utc;
use common::config::ServiceConfig;
use common::error::MonitorError;
use common::kick::KickClient;
use common::recovery::RecoveryManager;
use common::{
    ChatMessage, ChatMessageMetadata, PartitionConfig, Partitionable, RabbitMQConfig, ServiceError,
    ServiceStatus, StreamerStatus,
};
use entity::streamer::Entity;
use futures::{SinkExt, StreamExt};
use lapin::{
    BasicProperties, Connection, ConnectionProperties, Consumer, options::*, types::FieldTable,
};
use sea_orm::Database;
use sea_orm::EntityTrait;
use serde_json::json;
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info};

#[derive(Clone)]
struct ChatCollector {
    partition_config: PartitionConfig,
    rabbit_consumer: Consumer,
    redis_client: redis::Client,
    active_connections: Arc<RwLock<HashMap<String, WebSocketConnection>>>,
    service_status: Arc<RwLock<ServiceStatus>>,
    kick_client: KickClient,
    recovery_manager: RecoveryManager,
    rabbit_channel: lapin::Channel,
    db: &'static sea_orm::DatabaseConnection,
}

#[allow(dead_code)]
struct WebSocketConnection {
    streamer: String,
    sender: mpsc::UnboundedSender<Message>,
    task_handle: tokio::task::JoinHandle<()>,
}

impl ChatCollector {
    async fn new(
        config: ServiceConfig,
        partition_config: PartitionConfig,
        db: &'static sea_orm::DatabaseConnection,
    ) -> Result<Self, ServiceError> {
        info!("Initializing ChatCollector service");

        // Initialize kick client
        let kick_client = KickClient::new(config.kick_url)?;
        info!("Kick client initialized");

        // Initialize RabbitMQ connection
        info!("Connecting to RabbitMQ...");
        let conn =
            Connection::connect(&config.rabbitmq_url, ConnectionProperties::default()).await?;
        let channel = conn.create_channel().await?;

        // Declare exchanges and queues
        info!("Declaring RabbitMQ exchanges and queues...");
        channel
            .exchange_declare(
                RabbitMQConfig::STREAMER_STATUS_EXCHANGE,
                lapin::ExchangeKind::Topic,
                ExchangeDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Add queue declaration
        channel
            .queue_declare(
                RabbitMQConfig::STREAMER_STATUS_QUEUE,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Bind queue to exchange
        channel
            .queue_bind(
                RabbitMQConfig::STREAMER_STATUS_QUEUE,
                RabbitMQConfig::STREAMER_STATUS_EXCHANGE,
                "#", // Routing key - catches all messages
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Declare exchanges and queues
        channel
            .exchange_declare(
                RabbitMQConfig::CHAT_MESSAGES_EXCHANGE,
                lapin::ExchangeKind::Direct,
                ExchangeDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Create and bind queue
        channel
            .queue_declare(
                RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        channel
            .queue_bind(
                RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                RabbitMQConfig::CHAT_MESSAGES_EXCHANGE,
                RabbitMQConfig::CHAT_MESSAGES_ROUTING_KEY,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Set up consumer
        info!("Setting up RabbitMQ consumer...");
        let consumer = channel
            .basic_consume(
                RabbitMQConfig::STREAMER_STATUS_QUEUE,
                "chat_collector",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Initialize Redis client
        info!("Connecting to Redis...");
        let redis_client = redis::Client::open(config.redis_url)?;

        let recovery_manager = RecoveryManager::new(
            3,                       // max attempts
            Duration::from_secs(1),  // base delay
            Duration::from_secs(30), // max delay
        );

        info!("ChatCollector service initialization complete");

        Ok(Self {
            partition_config,
            rabbit_consumer: consumer,
            redis_client,
            active_connections: Arc::new(RwLock::new(HashMap::new())),
            service_status: Arc::new(RwLock::new(ServiceStatus::Starting)),
            kick_client,
            recovery_manager,
            rabbit_channel: channel,
            db,
        })
    }

    async fn get_chatroom_id(&self, streamer: &str) -> Result<String, ServiceError> {
        self.recovery_manager
            .execute(|| async {
                let response = self
                    .kick_client
                    .check_streamer(streamer)
                    .await
                    .map_err(|e| MonitorError::ApiError(e.to_string()))?;

                if !response.success {
                    return Err(MonitorError::ApiError("Failed to get streamer info".into()));
                }

                Ok(response.chatroom_id.to_string())
            })
            .await
            .map_err(|e| ServiceError::ApiError(e.to_string()))
    }

    async fn connect_to_chat(&self, streamer: &str) -> Result<(), ServiceError> {
        let base_url = "wss://ws-us2.pusher.com/app/32cbd69e4b950bf97679";
        let url_params = [
            ("protocol", "7"),
            ("client", "js"),
            ("version", "7.4.0"),
            ("flash", "false"),
        ];
        let params = url_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join("&");
        let ws_url = format!("{}?{}", base_url, params);

        info!("Connecting to websocket: {}", ws_url);

        let (ws_stream, _) = connect_async(ws_url)
            .await
            .map_err(|e| ServiceError::WebSocket(e.to_string()))?;

        // Get chatroom ID before setting up the connection
        let chatroom_id = self.get_chatroom_id(streamer).await?;

        let (write, read) = ws_stream.split();
        let (tx, rx) = mpsc::unbounded_channel();

        // Send subscription message
        let subscribe_message = serde_json::json!({
            "event": "pusher:subscribe",
            "data": {
                "auth": "",
                "channel": format!("chatrooms.{}.v2", chatroom_id)
            }
        });

        // Send the subscription message through the sender
        tx.send(Message::Text(subscribe_message.to_string().into()))
            .map_err(|e| ServiceError::WebSocket(e.to_string()))?;

        // Handle incoming messages
        let streamer_name = streamer.to_string();
        let rabbit_channel = self.rabbit_channel.clone();
        let read_task = tokio::spawn(async move {
            let mut read_stream = read;

            while let Some(message) = read_stream.next().await {
                match message {
                    Ok(msg) => {
                        if let Err(e) =
                            Self::process_chat_message(&streamer_name, msg, &rabbit_channel).await
                        {
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
        streamer: &str,
        message: Message,
        rabbit_channel: &lapin::Channel,
    ) -> Result<(), ServiceError> {
        let msg_text = message
            .to_text()
            .map_err(|e| ServiceError::WebSocket(e.to_string()))?;

        if let Ok(chat_message) = kick::parse_chat_message(streamer, msg_text) {
            // Serialize the chat message
            let message_payload =
                serde_json::to_vec(&chat_message).map_err(ServiceError::Serialization)?;

            // Publish to RabbitMQ
            rabbit_channel
                .basic_publish(
                    RabbitMQConfig::CHAT_MESSAGES_EXCHANGE,    // exchange name
                    RabbitMQConfig::CHAT_MESSAGES_ROUTING_KEY, // routing key
                    BasicPublishOptions::default(),
                    &message_payload,
                    BasicProperties::default(),
                )
                .await
                .map_err(ServiceError::RabbitMQ)?;

            info!(
                "Published chat message to queue from {} in {} chat",
                chat_message.username, streamer
            );
        }

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

    async fn get_assigned_streamers(&self) -> Result<Vec<String>, ServiceError> {
        let all_streamers = Entity::find()
            .all(self.db)
            .await?
            .into_iter()
            .map(|s| s.username)
            .collect::<Vec<String>>();

        Ok(all_streamers
            .into_iter()
            .filter(|streamer| {
                streamer.get_partition(self.partition_config.total_partitions)
                    == self.partition_config.partition_id
            })
            .collect())
    }

    async fn run(&mut self) -> Result<(), ServiceError> {
        info!("Starting chat collector service");
        *self.service_status.write().await = ServiceStatus::Healthy;

        // Get initial list of assigned streamers
        let assigned_streamers = self.get_assigned_streamers().await?;
        info!("Found {} assigned streamers", assigned_streamers.len());

        // Spawn a task for each assigned streamer
        let mut streamer_tasks = Vec::new();
        for streamer in assigned_streamers {
            let collector_clone = self.clone();
            let task = tokio::spawn(async move {
                loop {
                    match collector_clone.connect_to_chat(&streamer).await {
                        Ok(()) => {
                            info!("Connected to {}'s chat", streamer);
                            // Wait for disconnect or error
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        Err(e) => {
                            error!("Error connecting to {}'s chat: {:?}", streamer, e);
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            });
            streamer_tasks.push(task);
        }

        // Monitor RabbitMQ for streamer status updates
        while let Some(delivery) = self.rabbit_consumer.next().await {
            match delivery {
                Ok(delivery) => {
                    let status: StreamerStatus = serde_json::from_slice(&delivery.data)?;

                    if status
                        .username
                        .get_partition(self.partition_config.total_partitions)
                        == self.partition_config.partition_id
                    {
                        if let Err(e) = self.handle_streamer_status(status).await {
                            error!("Error handling streamer status: {:?}", e);
                        }
                        delivery.ack(BasicAckOptions::default()).await?;
                    } else {
                        // Reject messages not meant for this partition
                        delivery.nack(BasicNackOptions::default()).await?;
                    }
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
        let streamers: Vec<String> = self
            .active_connections
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

        // Clean up Redis keys
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        redis::cmd("DEL")
            .arg(format!(
                "collector:status:{}",
                self.partition_config.partition_id
            ))
            .exec_async(&mut conn)
            .await?;

        info!("Cleanup completed, shutdown successful");
        Ok(())
    }
}

async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "healthy",
        "timestamp": Utc::now()
    }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    // Load configuration
    let config = ServiceConfig::new().expect("Failed to load configuration");

    let partition_id = config
        .partition
        .split('-')
        .next_back()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    info!(
        "Starting collector service with partition ID: {}",
        partition_id
    );

    let partition_config = PartitionConfig {
        total_partitions: config.total_partitions,
        partition_id,
    };

    let db = Database::connect(&config.database_url).await?;
    let db = Box::leak(Box::new(db)); // Convert to static reference
    let service = ChatCollector::new(config, partition_config, db).await?;
    let mut service_clone = service.clone();
    let service_handle = tokio::spawn(async move { service_clone.run().await });

    let health_server =
        HttpServer::new(|| App::new().route("/health", web::get().to(health_check)))
            .bind("0.0.0.0:8080")?
            .run();

    // Set up signal handlers for both ctrl-c and termination
    let ctrl_c = tokio::signal::ctrl_c();

    // Run both the main service and health check server
    tokio::select! {
        result = health_server => {
            if let Err(e) = result {
                error!("Health server error: {:?}", e);
            }
        }
        result = service_handle => {
            if let Err(e) = result {
                error!("Service error: {:?}", e);
            }
        }
        _ = ctrl_c => {
            info!("Received SIGINT (Ctrl+C) signal");
        }
    }

    // Perform graceful shutdown
    if let Err(e) = service.shutdown().await {
        error!("Error during shutdown: {:?}", e);
    }

    Ok(())
}
