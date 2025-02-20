pub mod error;
pub mod flaresolverr;
pub mod recovery;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use thiserror::Error;

#[derive(Debug, Serialize)]
pub struct SystemAlert {
    pub service: String,
    pub partition_id: u32,
    pub alert_type: AlertType,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub enum AlertType {
    ServiceDegraded,
    ServiceRecovered,
    PartitionError,
}

/// Partition configuration for distributing work across service instances
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionConfig {
    pub total_partitions: u32,
    pub partition_id: u32,
}

/// Trait for types that can be partitioned
pub trait Partitionable {
    fn get_partition(&self, total_partitions: u32) -> u32;
}

impl Partitionable for String {
    fn get_partition(&self, total_partitions: u32) -> u32 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        (hasher.finish() % total_partitions as u64) as u32
    }
}

/// Streamer status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerStatus {
    pub username: String,
    pub is_live: bool,
    pub timestamp: DateTime<Utc>,
    pub viewers: Option<i32>,
    pub stream_id: Option<String>,
    pub title: Option<String>,
}

/// Chat message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub streamer: String,
    pub username: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<ChatMessageMetadata>,
}

/// Additional metadata for chat messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageMetadata {
    pub donation_amount: Option<f64>,
    pub subscription_months: Option<i32>,
    pub subscription_tier: Option<String>,
    pub user_badges: Option<Vec<String>>,
}

/// Service health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub partition_id: u32,
    pub total_partitions: u32,
    pub assigned_streamers: Vec<String>,
    pub last_heartbeat: DateTime<Utc>,
    pub status: ServiceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceStatus {
    Healthy,
    Degraded,
    Starting,
    ShuttingDown,
}

/// Recovery state for service restoration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryState {
    pub partition_id: u32,
    pub missed_streamers: Vec<String>,
    pub last_processed_timestamp: DateTime<Utc>,
    pub recovery_attempt: u32,
}

/// Custom error types
#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("RabbitMQ error: {0}")]
    RabbitMQ(#[from] lapin::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Partition error: {0}")]
    Partition(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("APIProxy error: {0}")]
    ApiProxyError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

/// Redis keys and prefixes
pub struct RedisKeys;

impl RedisKeys {
    pub const HEARTBEAT_PREFIX: &'static str = "monitor:heartbeat:";
    pub const RECOVERY_PREFIX: &'static str = "monitor:recovery:";
    pub const STREAMER_STATUS_PREFIX: &'static str = "streamer:status:";

    pub fn heartbeat_key(partition_id: u32) -> String {
        format!("{}{}", Self::HEARTBEAT_PREFIX, partition_id)
    }

    pub fn recovery_key(partition_id: u32) -> String {
        format!("{}{}", Self::RECOVERY_PREFIX, partition_id)
    }

    pub fn streamer_status_key(username: &str) -> String {
        format!("{}{}", Self::STREAMER_STATUS_PREFIX, username)
    }
}

/// RabbitMQ exchanges and queues
pub struct RabbitMQConfig;

impl RabbitMQConfig {
    pub const STREAMER_STATUS_EXCHANGE: &'static str = "streamer.status";
    pub const CHAT_MESSAGES_EXCHANGE: &'static str = "chat.messages";
    pub const SYSTEM_EVENTS_EXCHANGE: &'static str = "system.events";

    pub const STREAMER_STATUS_QUEUE: &'static str = "streamer.status.queue";
    pub const CHAT_MESSAGES_QUEUE: &'static str = "chat.messages.queue";
    pub const SYSTEM_EVENTS_QUEUE: &'static str = "system.events.queue";

    pub const STREAMER_STATUS_ROUTING_KEY: &'static str = "streamer.status.{}";
    pub const CHAT_MESSAGES_ROUTING_KEY: &'static str = "chat.message";
}

/// Utility functions
pub mod utils {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn generate_unique_id() -> String {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{:x}", timestamp)
    }

    pub fn calculate_partition_distribution(
        streamers: &[String],
        total_partitions: u32,
    ) -> Vec<Vec<String>> {
        let mut distribution = vec![Vec::new(); total_partitions as usize];

        for streamer in streamers {
            let partition = streamer.get_partition(total_partitions);
            distribution[partition as usize].push(streamer.clone());
        }

        distribution
    }
}

pub mod logger {
    use slog::{Drain, Logger, o};
    use slog_async;
    use slog_envlogger;
    use slog_term;

    pub fn init_root_logger() -> Logger {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        let drain = slog_envlogger::new(drain).fuse();

        Logger::root(drain, o!("version" => env!("CARGO_PKG_VERSION")))
    }

    pub fn create_child_logger(module: &str) -> Logger {
        let parent = init_root_logger();
        parent.new(o!("module" => module.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_distribution() {
        let streamers = vec![
            "ninja".to_string(),
            "pokimane".to_string(),
            "xqc".to_string(),
            "shroud".to_string(),
            "tfue".to_string(),
        ];

        let total_partitions = 3;
        let distribution = utils::calculate_partition_distribution(&streamers, total_partitions);

        // Verify all streamers are assigned
        let total_assigned: usize = distribution.iter().map(|p| p.len()).sum();
        assert_eq!(total_assigned, streamers.len());

        // Verify distribution is relatively even
        let max_streamers = distribution.iter().map(|p| p.len()).max().unwrap();
        let min_streamers = distribution.iter().map(|p| p.len()).min().unwrap();
        assert!(
            max_streamers - min_streamers <= 1,
            "Uneven distribution detected"
        );
    }

    #[test]
    fn test_consistent_partitioning() {
        let streamer = "ninja".to_string();
        let partition1 = streamer.get_partition(3);
        let partition2 = streamer.get_partition(3);

        assert_eq!(partition1, partition2, "Partitioning should be consistent");
    }
}

pub mod config {
    use config::{Config, ConfigError, Environment};
    use dotenv::dotenv;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct ServiceConfig {
        pub database_url: String,
        pub rabbitmq_url: String,
        pub redis_url: String,
        pub health_check_interval: u64,
        pub heartbeat_interval: u64,
        pub recovery_attempts: u32,
        pub total_partitions: u32,
        pub flaresolverr_url: String,
        pub flaresolverr_max_timeout: u32,
        pub partition: String,
    }

    impl ServiceConfig {
        pub fn new() -> Result<Self, ConfigError> {
            // Load environment variables from .env file
            dotenv().ok();

            let mut s = Config::new();

            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `APP_DATABASE_URL=postgres://... would set the `database_url` key
            s.merge(Environment::with_prefix("APP"))?;

            // Deserialize the configuration into the Settings struct
            s.try_into()
        }
    }
}
