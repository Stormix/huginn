use actix_web::{App, HttpResponse, HttpServer, web};
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
use serde_json::json;
use tracing::{error, info};

#[derive(Clone)]
struct WriterService {
    db: DatabaseConnection,
    rabbit_channel: Channel,
}

impl WriterService {
    async fn new() -> Result<Self, ServiceError> {
        let config = ServiceConfig::new().expect("Failed to load configuration");
        let db = Database::connect(
            sea_orm::ConnectOptions::new(&config.database_url)
                .sqlx_logging(false)
                .to_owned(),
        )
        .await?;

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
                RabbitMQConfig::CHAT_MESSAGES_ROUTING_KEY,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await?;

        // Set QoS (prefetch) for better load management
        rabbit_channel
            .basic_qos(100, BasicQosOptions::default())
            .await?;

        Ok(Self { db, rabbit_channel })
    }

    async fn start(&self) -> Result<(), ServiceError> {
        let config = ServiceConfig::new().expect("Failed to load configuration");
        let mut handles = Vec::new();

        for i in 0..config.num_consumers {
            // Create 4 parallel consumers
            let channel = self.rabbit_channel.clone();
            let db = self.db.clone();

            let handle = tokio::spawn(async move {
                // Set QoS for this specific consumer channel
                channel.basic_qos(25, BasicQosOptions::default()).await?;

                let consumer = channel
                    .basic_consume(
                        RabbitMQConfig::CHAT_MESSAGES_QUEUE,
                        &format!("writer_consumer_{}", i),
                        BasicConsumeOptions {
                            no_local: false,
                            no_ack: false,
                            exclusive: false,
                            nowait: false,
                        },
                        FieldTable::default(),
                    )
                    .await?;

                process_messages(consumer, db).await
            });

            handles.push(handle);
        }

        // Wait for all consumers to complete (or error)
        for handle in handles {
            match handle.await {
                Ok(Ok(())) => (),
                Ok(Err(e)) => error!("Consumer error: {:?}", e),
                Err(e) => error!("Join error: {:?}", e),
            }
        }

        Ok(())
    }
}

// Move process_messages out of impl to avoid self reference
async fn process_messages(
    mut consumer: Consumer,
    db: DatabaseConnection,
) -> Result<(), ServiceError> {
    // Use a batch size for processing multiple messages
    let mut batch = Vec::with_capacity(100);

    while let Some(delivery) = consumer.next().await {
        match delivery {
            Ok(delivery) => {
                match handle_message(&delivery.data, &db).await {
                    Ok(_) => {
                        batch.push(delivery);

                        // Process batch acknowledgments
                        if batch.len() >= 100 {
                            for del in batch.drain(..) {
                                del.ack(BasicAckOptions::default())
                                    .await
                                    .map_err(ServiceError::RabbitMQ)?;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error processing message: {}", e);
                        delivery
                            .nack(BasicNackOptions::default())
                            .await
                            .map_err(ServiceError::RabbitMQ)?;
                    }
                }
            }
            Err(e) => error!("Error receiving message: {}", e),
        }
    }

    // Acknowledge any remaining messages in the batch
    for del in batch {
        del.ack(BasicAckOptions::default())
            .await
            .map_err(ServiceError::RabbitMQ)?;
    }

    Ok(())
}

// Move handle_message out of impl and modify to take DatabaseConnection
async fn handle_message(data: &[u8], db: &DatabaseConnection) -> Result<(), ServiceError> {
    let chat_message: ChatMessage = from_slice(data).map_err(ServiceError::Serialization)?;

    info!(
        "Processing message from {} in #{} at {}",
        chat_message.username, chat_message.streamer, chat_message.timestamp
    );

    let chat_message_model = ChatMessageModel {
        streamer: Set(chat_message.streamer.clone()),
        username: Set(chat_message.username.clone()),
        message: Set(chat_message.message),
        timestamp: Set(chat_message.timestamp),
        metadata: Set(Some(serde_json::to_value(chat_message.metadata)?)),
        created_at: Set(Utc::now()),
        ..Default::default()
    };

    let result = chat_message_model
        .insert(db)
        .await
        .map_err(ServiceError::Database)?;

    info!(
        "Successfully stored message (id: {}) from {} in #{} at {}",
        result.id, chat_message.username, chat_message.streamer, chat_message.timestamp
    );

    Ok(())
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
    let health_server =
        HttpServer::new(|| App::new().route("/health", web::get().to(health_check)))
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
