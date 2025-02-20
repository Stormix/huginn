mod kick;

use common::config::ServiceConfig;
use common::flaresolverr::FlareSolverrClient;
use common::recovery::RecoveryManager;
use common::{
    ChatMessage, ChatMessageMetadata, PartitionConfig, RabbitMQConfig, ServiceError, ServiceStatus,
    StreamerStatus,
};
use futures::{SinkExt, StreamExt};
use lapin::{
    BasicProperties, Connection, ConnectionProperties, Consumer, options::*, types::FieldTable,
};
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info};
use actix_web::{web, App, HttpResponse, HttpServer};
use serde_json::json;
use chrono::Utc;

struct ChatCollector {
    partition_config: PartitionConfig,
    rabbit_consumer: Consumer,
    redis_client: redis::Client,
    active_connections: Arc<RwLock<HashMap<String, WebSocketConnection>>>,
    service_status: Arc<RwLock<ServiceStatus>>,
    flaresolverr_client: FlareSolverrClient,
    recovery_manager: RecoveryManager,
    rabbit_channel: lapin::Channel,
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
    ) -> Result<Self, ServiceError> {
        info!("Initializing ChatCollector service");

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

        let flaresolverr_client = FlareSolverrClient::new(config.flaresolverr_url, config.flaresolverr_max_timeout)
            .map_err(|e| ServiceError::ApiProxyError(e.to_string()))?;

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
            flaresolverr_client,
            recovery_manager,
            rabbit_channel: channel,
        })
    }

    async fn get_chatroom_id(&self, streamer: &str) -> Result<String, ServiceError> {
        self.recovery_manager
            .execute(|| async {
                let url = format!("https://kick.com/api/v2/channels/{}", streamer);
                let response = self.flaresolverr_client.get(&url).await?;
                let json: serde_json::Value = serde_json::from_str(&response)
                    .map_err(|e| ServiceError::ParseError(e.to_string()))?;
                let chatroom_id =
                    json["chatroom"]["id"]
                        .as_u64()
                        .ok_or(ServiceError::ParseError(
                            "Chatroom ID not found".to_string(),
                        ))?;
                Ok(chatroom_id.to_string())
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

            info!("Published chat message to queue from {} in {} chat", chat_message.username, streamer);
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

    let partition_id = config.partition.to_string()
        .split('-')
        .last()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    info!("Starting collector service with partition ID: {}", partition_id);

    let partition_config = PartitionConfig {
        total_partitions: config.total_partitions,
        partition_id,
    };

    // Create and run service
    let mut service = ChatCollector::new(config, partition_config).await?;
    let service_handle = service.run();
    
    let health_server = HttpServer::new(|| {
        App::new().route("/health", web::get().to(health_check))
    })
    .bind("0.0.0.0:8080")?
    .run();

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
    }

    Ok(())
}
