use chrono::Utc;
use common::RabbitMQConfig;
use common::{ChatMessage, ServiceError, config::ServiceConfig};
use entity::chatmessage::ActiveModel as ChatMessageModel;
use futures::StreamExt;
use lapin::ExchangeKind;
use lapin::{Channel, Connection, ConnectionProperties, Consumer, options::*, types::FieldTable};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
use serde_json::from_slice;
use tracing::{error, info};
use actix_web::{web, App, HttpResponse, HttpServer};
use serde_json::json;

#[derive(Clone)]
struct WriterService {
    db: DatabaseConnection,
    rabbit_channel: Channel,
}

impl WriterService {
    async fn new() -> Result<Self, ServiceError> {
        let config = ServiceConfig::new().expect("Failed to load configuration");
        let db = Database::connect(&config.database_url).await?;

        // Run migrations
        info!("Running database migrations...");
        Migrator::up(&db, None).await?;
        info!("Migrations completed successfully");

        let rabbit_conn =
            Connection::connect(&config.rabbitmq_url, ConnectionProperties::default()).await?;
        let rabbit_channel = rabbit_conn.create_channel().await?;

        // Declare exchanges and queues
        info!("Declaring RabbitMQ exchanges and queues...");
        rabbit_channel
            .exchange_declare(
                RabbitMQConfig::CHAT_MESSAGES_EXCHANGE,
                ExchangeKind::Direct,
                ExchangeDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        rabbit_channel
            .queue_declare(
                RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;

        rabbit_channel
            .queue_bind(
                RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                RabbitMQConfig::CHAT_MESSAGES_EXCHANGE,
                "chat.message",
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Set up consumer
        rabbit_channel
            .basic_consume(
                RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                "chat_writer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;

        Ok(Self { db, rabbit_channel })
    }

    async fn start(&self) -> Result<(), ServiceError> {
        let consumer = self
            .rabbit_channel
            .basic_consume(
                RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                "writer_consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;

        self.process_messages(consumer).await
    }

    async fn process_messages(&self, mut consumer: Consumer) -> Result<(), ServiceError> {
        while let Some(delivery) = consumer.next().await {
            match delivery {
                Ok(delivery) => match self.handle_message(&delivery.data).await {
                    Ok(_) => {
                        delivery
                            .ack(BasicAckOptions::default())
                            .await
                            .map_err(ServiceError::RabbitMQ)?;
                    }
                    Err(e) => {
                        error!("Error processing message: {}", e);
                        delivery
                            .nack(BasicNackOptions::default())
                            .await
                            .map_err(ServiceError::RabbitMQ)?;
                    }
                },
                Err(e) => error!("Error receiving message: {}", e),
            }
        }
        Ok(())
    }

    async fn handle_message(&self, data: &[u8]) -> Result<(), ServiceError> {
        let chat_message: ChatMessage = from_slice(data).map_err(ServiceError::Serialization)?;

        let chat_message_model = ChatMessageModel {
            streamer: Set(chat_message.streamer.clone()),
            username: Set(chat_message.username.clone()),
            message: Set(chat_message.message),
            timestamp: Set(chat_message.timestamp),
            metadata: Set(Some(serde_json::to_value(chat_message.metadata)?)),
            created_at: Set(Utc::now()),
            ..Default::default()
        };

        chat_message_model
            .insert(&self.db)
            .await
            .map_err(ServiceError::Database)?;

        info!(
            "Stored chat message from {} in #{} in database",
            chat_message.username, chat_message.streamer
        );
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
async fn main() -> Result<(), ServiceError> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Log APP_PARTITION environment variable
    if let Ok(partition) = std::env::var("APP_PARTITION") {
        info!("Starting service with APP_PARTITION: {}", partition);
    } else {
        info!("APP_PARTITION environment variable not set");
    }

    let service = WriterService::new().await?;
    let service_handle = service.start();
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
