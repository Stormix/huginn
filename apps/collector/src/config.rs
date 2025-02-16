use config::{Config, ConfigError, Environment};
use dotenv::dotenv;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub database_url: String,
}

impl Settings {
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
