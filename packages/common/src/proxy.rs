use crate::error::MonitorError;
use rand::seq::SliceRandom;
use reqwest::{Client, Proxy};
use std::env;
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub struct ProxyConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl ProxyConfig {
    fn to_url(&self) -> String {
        format!(
            "http://{}:{}@{}:{}",
            self.username, self.password, self.host, self.port
        )
    }
}

pub struct ProxyManager {
    proxies: Vec<ProxyConfig>,
}

impl ProxyManager {
    pub fn new() -> Result<Self, crate::error::ServiceError> {
        let proxies = Self::parse_proxy_list()?;
        if proxies.is_empty() {
            return Err(crate::error::ServiceError::Configuration(
                "No valid proxies found".into(),
            ));
        }
        info!("Loaded {} proxies", proxies.len());

        Ok(Self { proxies })
    }

    fn parse_proxy_list() -> Result<Vec<ProxyConfig>, crate::error::ServiceError> {
        let proxy_list = env::var("PROXY_LIST").map_err(|_| {
            crate::error::ServiceError::Configuration("PROXY_LIST environment variable not found".into())
        })?;

        let proxies = proxy_list
            .split('\n')
            .filter_map(|line| {
                let parts: Vec<&str> = line.trim().split(':').collect();
                if parts.len() == 4 {
                    Some(ProxyConfig {
                        host: parts[0].to_string(),
                        port: parts[1].parse().ok()?,
                        username: parts[2].to_string(),
                        password: parts[3].to_string(),
                    })
                } else {
                    warn!("Invalid proxy format: {}", line);
                    None
                }
            })
            .collect::<Vec<_>>();

        Ok(proxies)
    }

    async fn create_client_with_proxy(&self, proxy: &ProxyConfig) -> Result<Client, MonitorError> {
        let proxy_url = proxy.to_url();
        let proxy = Proxy::http(&proxy_url)
            .map_err(|e| MonitorError::ApiError(format!("Failed to create proxy: {}", e)))?
            .no_proxy();  // Disable proxy bypass

        Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .proxy(proxy)
            .build()
            .map_err(|e| MonitorError::ApiError(format!("Failed to create HTTP client: {}", e)))
    }

    pub async fn make_request(&self, url: &str) -> Result<String, MonitorError> {
        let mut proxies = self.proxies.clone();
        proxies.shuffle(&mut rand::thread_rng());

        let mut last_error = None;
        for proxy in proxies {
            info!("Trying proxy {}:{}", proxy.host, proxy.port);
            match self.create_client_with_proxy(&proxy).await {
                Ok(client) => {
                    match client
                        .get(url)
                        .header(
                            "User-Agent",
                            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
                        )
                        .send()
                        .await
                    {
                        Ok(response) => {
                            if response.status() == 429 {
                                warn!("Rate limited on proxy {}:{}", proxy.host, proxy.port);
                                last_error = Some(MonitorError::RateLimited {
                                    wait_time_secs: 30,
                                });
                                continue;
                            }

                            match response.text().await {
                                Ok(text) => return Ok(text),
                                Err(e) => {
                                    warn!("Failed to read response from proxy {}:{}: {}", proxy.host, proxy.port, e);
                                    last_error = Some(MonitorError::ApiError(format!(
                                        "Failed to read response: {}",
                                        e
                                    )));
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Request failed with proxy {}:{}: {}", proxy.host, proxy.port, e);
                            last_error = Some(MonitorError::ApiError(format!("Request failed: {}", e)));
                            continue;
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to create client with proxy {}:{}: {}", proxy.host, proxy.port, e);
                    last_error = Some(e);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| MonitorError::ApiError("All proxies failed".into())))
    }
} 