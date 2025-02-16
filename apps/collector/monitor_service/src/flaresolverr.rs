use crate::error::MonitorError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
struct FlareSolverrSolution {
    url: String,
    status: u32,
    response: String,
}

pub struct FlareSolverrClient {
    client: Client,
    base_url: String,
}

impl FlareSolverrClient {
    pub fn new(base_url: String) -> Result<Self, MonitorError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| MonitorError::ApiError(e.to_string()))?;

        Ok(Self {
            client,
            base_url,
        })
    }

    pub async fn get(&self, url: &str) -> Result<String, MonitorError> {
        let payload = FlareSolverrRequest {
            cmd: "request.get".to_string(),
            url: url.to_string(),
            max_timeout: 60000,
        };

        let status = self.client
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| MonitorError::ApiError(format!("FlareSolverr request failed: {}", e)))?;

        let response_text = status.text().await
            .map_err(|e| MonitorError::ApiError(format!("Failed to get response text: {}", e)))?;

        let flare_response = serde_json::from_str::<FlareSolverrResponse>(&response_text)
            .map_err(|e| MonitorError::ApiError(format!("Failed to parse JSON: {} for response: {}", e, response_text)))?;

        let html_response = flare_response.solution.response;
        let json_start = html_response.find('{')
            .ok_or_else(|| MonitorError::ApiError("No JSON found in response".to_string()))?;
        let json_end = html_response.rfind('}')
            .ok_or_else(|| MonitorError::ApiError("No JSON end found in response".to_string()))?;

        Ok(html_response[json_start..=json_end].to_string())
    }
} 