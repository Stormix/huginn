use common::error::MonitorError;
use common::kick::KickClient;
use common::recovery::RecoveryManager;
use common::{
    AlertType, HealthStatus, PartitionConfig, Partitionable, RabbitMQConfig, RecoveryState,
    RedisKeys, ServiceError, ServiceStatus, StreamerStatus, SystemAlert,
};

use actix_web::{App, HttpResponse, HttpServer, web};
use chrono::Utc;
use common::config::ServiceConfig;
use lapin::{
    BasicProperties, Channel, Connection, ConnectionProperties, options::*, types::FieldTable,
};
use redis::Client as RedisClient;
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error as tracing_error, info as tracing_info, warn as tracing_warn};

/// Main service structure
struct MonitorService {
    partition_config: PartitionConfig,
    rabbit_channel: Channel,
    redis_client: RedisClient,
    db: DatabaseConnection,
    health_check_interval: Duration,
    heartbeat_interval: Duration,
    service_status: Arc<tokio::sync::RwLock<ServiceStatus>>,
    recovery_manager: RecoveryManager,
    error_counter: Arc<tokio::sync::Mutex<HashMap<String, u32>>>,
    kick_client: KickClient,
}

impl MonitorService {
    async fn new(
        config: ServiceConfig,
        partition_config: PartitionConfig,
    ) -> Result<Self, ServiceError> {
        tracing_info!("Initializing monitor service");

        // Initialize kick client
        let kick_client = KickClient::new(config.kick_url)?;
        tracing_info!("Kick client initialized");

        // Initialize RabbitMQ connection
        let conn =
            Connection::connect(&config.rabbitmq_url, ConnectionProperties::default()).await?;
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

        // Initialize Redis client
        let redis_client = RedisClient::open(config.redis_url)?;

        // Initialize database connection
        let db = Database::connect(&config.database_url).await?;

        let recovery_manager = RecoveryManager::new(
            3,                       // max attempts
            Duration::from_secs(1),  // base delay
            Duration::from_secs(30), // max delay
        );

        Ok(Self {
            partition_config,
            rabbit_channel: channel,
            redis_client,
            db,
            health_check_interval: Duration::from_secs(config.health_check_interval),
            heartbeat_interval: Duration::from_secs(config.heartbeat_interval),
            service_status: Arc::new(tokio::sync::RwLock::new(ServiceStatus::Starting)),
            recovery_manager,
            error_counter: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            kick_client,
        })
    }

    /// Get streamers assigned to this partition
    async fn get_assigned_streamers(&self) -> Result<Vec<String>, ServiceError> {
        let all_streamers = entity::streamer::Entity::find()
            .all(&self.db)
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

    /// Check streamer status
    async fn make_api_call(&self, username: &str) -> Result<StreamerStatus, MonitorError> {
        let response = self.kick_client.check_streamer(username).await?;

        Ok(StreamerStatus {
            username: username.to_string(),
            is_live: response.is_live,
            timestamp: Utc::now(),
            viewers: Some(response.viewers),
            stream_id: None, // Not provided by the new API
            title: Some(response.title),
        })
    }

    /// Register service heartbeat
    async fn register_heartbeat(&self) -> Result<(), ServiceError> {
        let health_status = HealthStatus {
            partition_id: self.partition_config.partition_id,
            total_partitions: self.partition_config.total_partitions,
            assigned_streamers: self.get_assigned_streamers().await?,
            last_heartbeat: Utc::now(),
            status: self.service_status.read().await.clone(),
        };

        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;

        redis::cmd("SET")
            .arg(RedisKeys::heartbeat_key(self.partition_config.partition_id))
            .arg(serde_json::to_string(&health_status)?)
            .exec_async(&mut conn)
            .await?;

        Ok(())
    }

    async fn check_streamer(&self, username: &str) -> Result<StreamerStatus, MonitorError> {
        let result = self
            .recovery_manager
            .execute(|| async {
                // Actual API call implementation
                let status = self.make_api_call(username).await?;

                // Reset error counter on success
                let mut counter = self.error_counter.lock().await;
                counter.remove(username);

                Ok(status)
            })
            .await;

        match result {
            Ok(status) => Ok(status),
            Err(e) => {
                // Increment error counter
                let mut counter = self.error_counter.lock().await;
                let count = counter.entry(username.to_string()).or_insert(0);
                *count += 1;

                // If too many errors, mark service as degraded
                if *count > 5 {
                    *self.service_status.write().await = ServiceStatus::Degraded;
                    self.alert_degraded_service(username, *count).await?;
                }

                Err(e)
            }
        }
    }

    async fn alert_degraded_service(
        &self,
        username: &str,
        error_count: u32,
    ) -> Result<(), MonitorError> {
        let alert = SystemAlert {
            service: "monitor".to_string(),
            partition_id: self.partition_config.partition_id,
            alert_type: AlertType::ServiceDegraded,
            message: format!(
                "Service degraded for streamer {} after {} errors",
                username, error_count
            ),
            timestamp: Utc::now(),
        };

        self.rabbit_channel
            .basic_publish(
                RabbitMQConfig::SYSTEM_EVENTS_EXCHANGE,
                "system.alerts",
                BasicPublishOptions::default(),
                &serde_json::to_vec(&alert).map_err(ServiceError::Serialization)?,
                BasicProperties::default(),
            )
            .await
            .map_err(ServiceError::RabbitMQ)?;

        Ok(())
    }

    async fn recover_from_failure(&self) -> Result<(), MonitorError> {
        tracing_info!("Initiating recovery process");

        // Get last known state from Redis
        let recovery_key = RedisKeys::recovery_key(self.partition_config.partition_id);
        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .map_err(ServiceError::Redis)?;

        if let Ok(recovery_data) = redis::cmd("GET")
            .arg(&recovery_key)
            .query_async::<String>(&mut conn)
            .await
        {
            let recovery_state: RecoveryState =
                serde_json::from_str(&recovery_data).map_err(ServiceError::Serialization)?;

            tracing_info!(
                "Found recovery state with {} missed streamers",
                recovery_state.missed_streamers.len()
            );

            // Replay missed checks
            for streamer in recovery_state.missed_streamers {
                if let Err(e) = self.check_streamer(&streamer).await {
                    tracing_warn!("Failed to recover streamer {}: {:?}", streamer, e);
                }
            }
        } else {
            tracing_info!("No recovery state found");
        }

        // Mark service as healthy
        *self.service_status.write().await = ServiceStatus::Healthy;
        tracing_info!("Service marked as healthy after recovery");

        Ok(())
    }

    async fn run(&self) -> Result<(), MonitorError> {
        tracing_info!("Starting monitor service");

        // Attempt recovery on startup
        if let Err(e) = self.recover_from_failure().await {
            tracing_error!("Recovery failed: {:?}", e);
            // Continue anyway, but in degraded state
            *self.service_status.write().await = ServiceStatus::Degraded;
        }

        let mut check_interval = tokio::time::interval(self.health_check_interval);
        let mut heartbeat_interval = tokio::time::interval(self.heartbeat_interval);

        tracing_info!("Service intervals configured");

        loop {
            tokio::select! {
                _ = check_interval.tick() => {
                    let assigned_streamers = self.get_assigned_streamers().await?;
                    tracing_info!("Starting live check cycle for {} streamers", assigned_streamers.len());

                    for streamer in &assigned_streamers {
                        match self.check_streamer(streamer).await {
                            Ok(status) => {
                                tracing_info!("Checked status for {}: is_live={} viewers={}", streamer, status.is_live, status.viewers.unwrap_or(0));
                                   // Publish status to RabbitMQ if streamer is live
                                    if status.is_live {
                                        self.rabbit_channel
                                            .basic_publish(
                                                RabbitMQConfig::STREAMER_STATUS_EXCHANGE,
                                                &format!("streamer.status.{}", streamer),
                                                BasicPublishOptions::default(),
                                                &serde_json::to_vec(&status).map_err(ServiceError::Serialization)?,
                                                BasicProperties::default(),
                                            )
                                            .await
                                            .map_err(ServiceError::RabbitMQ)?;
                                    }
                            }
                            Err(MonitorError::RateLimited { wait_time_secs }) => {
                                tracing_warn!("Rate limited, waiting {} seconds", wait_time_secs);
                                sleep(Duration::from_secs(wait_time_secs)).await;
                            }
                            Err(e) => {
                                tracing_error!("Error checking status for {}: {:?}", streamer, e);
                            }
                        }
                    }
                }

                _ = heartbeat_interval.tick() => {
                    if let Err(e) = self.register_heartbeat().await {
                        tracing_error!("Failed to register heartbeat: {:?}", e);

                        let mut counter = self.error_counter.lock().await;
                        let count = counter.entry("heartbeat".to_string()).or_insert(0);
                        *count += 1;

                        if *count > 3 {
                            tracing_warn!("Multiple heartbeat failures: {}", count);
                            *self.service_status.write().await = ServiceStatus::Degraded;
                        }
                    }
                }
            }
        }
    }

    /// Graceful shutdown
    async fn shutdown(&self) -> Result<(), ServiceError> {
        tracing_info!("Initiating graceful shutdown");

        // Update service status
        *self.service_status.write().await = ServiceStatus::ShuttingDown;

        // Register final heartbeat
        self.register_heartbeat().await?;
        tracing_info!("Final heartbeat registered");

        // Clean up Redis keys
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        redis::cmd("DEL")
            .arg(RedisKeys::heartbeat_key(self.partition_config.partition_id))
            .exec_async(&mut conn)
            .await?;

        tracing_info!("Cleanup completed, shutdown successful");
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
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let settings = ServiceConfig::new().expect("Failed to load configuration");

    // Extract partition ID from pod name (e.g., "huginn-collector-service-2" -> 2)
    let partition_id = settings
        .partition
        .split('-')
        .next_back()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    tracing_info!(
        "Starting monitor service with partition ID: {}",
        partition_id
    );

    let partition_config = PartitionConfig {
        total_partitions: settings.total_partitions,
        partition_id,
    };

    // Create and run service
    let service = MonitorService::new(settings, partition_config).await?;
    let service = Arc::new(tokio::sync::RwLock::new(service));
    let service_clone = service.clone();

    let health_server =
        HttpServer::new(|| App::new().route("/health", web::get().to(health_check)))
            .bind("0.0.0.0:8080")?
            .run();

    // Run both the main service and health check server
    tokio::select! {
        result = health_server => {
            if let Err(e) = result {
                tracing_error!("Health server error: {:?}", e);
            }
        }
        result = async {
            service.write().await.run().await
        } => {
            if let Err(e) = result {
                tracing_error!("Service error: {:?}", e);
            }
        }
        _ = async {
            tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
            if let Err(e) = service_clone.write().await.shutdown().await {
                tracing_error!("Error during shutdown: {:?}", e);
            }
        } => {}
    }

    Ok(())
}
