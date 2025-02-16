use crate::error::MonitorError;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

#[derive(Debug)]
pub struct RecoveryManager {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl RecoveryManager {
    pub fn new(max_attempts: u32, base_delay: Duration, max_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            max_delay,
        }
    }

    pub async fn execute<F, Fut, T>(&self, operation: F) -> Result<T, MonitorError>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<T, MonitorError>> + Send,
    {
        let mut attempts = 0;
        let mut delay = self.base_delay;

        loop {
            attempts += 1;
            
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if attempts >= self.max_attempts {
                        return Err(MonitorError::RecoveryFailed {
                            attempts,
                            message: e.to_string(),
                        });
                    }

                    warn!(
                        "Operation failed (attempt {}/{}): {}. Retrying in {:?}",
                        attempts, self.max_attempts, e, delay
                    );

                    sleep(delay).await;
                    delay = std::cmp::min(delay * 2, self.max_delay);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_successful_operation() {
        let recovery = RecoveryManager::new(3, Duration::from_millis(10), Duration::from_millis(100));
        let result = recovery.execute(|| async { Ok::<_, MonitorError>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_then_success() {
        let attempts = Arc::new(AtomicU32::new(0));
        let recovery = RecoveryManager::new(3, Duration::from_millis(10), Duration::from_millis(100));
        
        let attempts_clone = attempts.clone();
        let operation = move || {
            let attempts = attempts_clone.clone();
            async move {
                let current = attempts.fetch_add(1, Ordering::SeqCst);
                if current == 0 {
                    Err(MonitorError::Other("First attempt fails".into()))
                } else {
                    Ok(42)
                }
            }
        };

        let result = recovery.execute(operation).await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_max_attempts_reached() {
        let recovery = RecoveryManager::new(3, Duration::from_millis(10), Duration::from_millis(100));
        
        let result = recovery
            .execute(|| async { Err::<(), MonitorError>(MonitorError::Other("Always fails".into())) })
            .await;

        assert!(matches!(
            result,
            Err(MonitorError::RecoveryFailed { attempts: 3, .. })
        ));
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let recovery = RecoveryManager::new(3, Duration::from_millis(10), Duration::from_millis(30));
        let timestamps = Arc::new(std::sync::Mutex::new(vec![]));
        
        let timestamps_clone = timestamps.clone();
        let operation = move || {
            let timestamps = timestamps_clone.clone();
            async move {
                timestamps.lock().unwrap().push(std::time::Instant::now());
                Err::<(), MonitorError>(MonitorError::Other("Always fails".into()))
            }
        };

        let _: Result<(), MonitorError> = recovery.execute(operation).await;
        
        let timestamps = timestamps.lock().unwrap();
        assert_eq!(timestamps.len(), 3);
        
        // Check that delays between attempts are increasing
        let delay1 = timestamps[1].duration_since(timestamps[0]);
        let delay2 = timestamps[2].duration_since(timestamps[1]);
        
        assert!(delay1 >= Duration::from_millis(10));
        assert!(delay2 >= Duration::from_millis(20));
        assert!(delay2 <= Duration::from_millis(35)); // Add small buffer for timing variations
    }
}
