use thiserror::Error;

#[derive(Error, Debug)]
pub enum MonitorError {
    #[error("Service error: {0}")]
    Service(#[from] common::ServiceError),
    
    #[error("Recovery failed after {attempts} attempts: {message}")]
    RecoveryFailed {
        attempts: u32,
        message: String,
    },
    
    #[error("Partition error: {0}")]
    PartitionError(String),
    
    #[error("Rate limited: must wait {wait_time_secs} seconds")]
    RateLimited {
        wait_time_secs: u64,
    },

    #[error("Other error: {0}")]
    Other(String),

    #[error("API error: {0}")]
    ApiError(String),
    
    #[error("API proxy error: {0}")]
    ApiProxyError(String),
}