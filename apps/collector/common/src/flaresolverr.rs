use crate::ServiceError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

#[derive(Serialize)]
struct FlareSolverrRequest {
    cmd: String,
    url: String,
    #[serde(rename = "maxTimeout")]
    max_timeout: u32,
}

#[derive(Deserialize, Debug)]
struct FlareSolverrResponse {
    status: String,
    message: String,
    solution: FlareSolverrSolution,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct FlareSolverrSolution {
    url: String,
    status: u32,
    response: String,
}

#[derive(Deserialize, Debug)]
struct HealthResponse {
    msg: String,
    version: String,
    #[serde(rename = "userAgent")]
    user_agent: String,
}

pub struct FlareSolverrClient {
    client: Client,
    base_url: String,
}

impl FlareSolverrClient {
    pub fn new(base_url: String) -> Result<Self, ServiceError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| ServiceError::ApiProxyError(e.to_string()))?;

        Ok(Self { client, base_url })
    }

    pub async fn get(&self, url: &str) -> Result<String, ServiceError> {
        let payload = FlareSolverrRequest {
            cmd: "request.get".to_string(),
            url: url.to_string(),
            max_timeout: 5000,
        };

        info!("Sending FlareSolverr (at {}) request to {}", self.base_url, url);

        let status = self
            .client
            .post(&format!("{}/v1", self.base_url))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("FlareSolverr request failed: {}", e);
                ServiceError::ApiProxyError(format!("FlareSolverr request failed: {}", e))
            })?;

        let response_text = status.text().await.map_err(|e| {
            ServiceError::ApiProxyError(format!("Failed to get response text: {}", e))
        })?;

        let flare_response =
            serde_json::from_str::<FlareSolverrResponse>(&response_text).map_err(|e| {
                ServiceError::ApiProxyError(format!(
                    "Failed to parse JSON: {} for response: {}",
                    e, response_text
                ))
            })?;

        if flare_response.status != "ok" {
            return Err(ServiceError::ApiProxyError(format!(
                "FlareSolverr failed with status '{}': {}",
                flare_response.status, flare_response.message
            )));
        }

        let html_response = flare_response.solution.response;
        let json_start = html_response
            .find('{')
            .ok_or_else(|| ServiceError::ApiProxyError("No JSON found in response".to_string()))?;
        let json_end = html_response.rfind('}').ok_or_else(|| {
            ServiceError::ApiProxyError("No JSON end found in response".to_string())
        })?;

        Ok(html_response[json_start..=json_end].to_string())
    }

    pub async fn health_check(&self) -> Result<(), ServiceError> {
        let response = self
            .client
            .get(&self.base_url)
            .send()
            .await
            .map_err(|e| ServiceError::ApiProxyError(format!("Health check failed: {}", e)))?;

        let response_text = response.text().await.map_err(|e| {
            ServiceError::ApiProxyError(format!("Failed to get response text: {}", e))
        })?;

        info!("FlareSolverr health check response: {}", response_text);

        let health_data = serde_json::from_str::<HealthResponse>(&response_text).map_err(|e| {
            ServiceError::ApiProxyError(format!("Failed to parse health check response: {}", e))
        })?;

        if health_data.msg != "FlareSolverr is ready!" {
            return Err(ServiceError::ApiProxyError(
                "FlareSolverr reported not ready".to_string(),
            ));
        }

        Ok(())
    }
}
