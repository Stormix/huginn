use crate::error::MonitorError;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KickResponse {
    pub success: bool,
    pub is_live: bool,
    pub viewers: i32,
    pub title: String,
    pub chatroom_id: String,
}

#[derive(Clone)]
pub struct KickClient {
    client: Client,
    base_url: String,
}

impl KickClient {
    pub fn new(kick_url: String) -> Result<Self, MonitorError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| MonitorError::ApiError(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            base_url: kick_url,
        })
    }

    pub async fn check_streamer(&self, username: &str) -> Result<KickResponse, MonitorError> {
        let url = format!("{}/check/{}", self.base_url, username);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| MonitorError::ApiError(format!("Request failed: {}", e)))?;

        if response.status() == 429 {
            return Err(MonitorError::RateLimited { wait_time_secs: 30 });
        }

        response
            .json::<KickResponse>()
            .await
            .map_err(|e| MonitorError::ApiError(format!("Failed to parse response: {}", e)))
    }
}
