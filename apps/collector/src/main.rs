mod config;
mod constants;
mod models;
mod monitor;
mod utils;

use config::Settings;
use monitor::collection::Collection;
use slog::info;
use tokio;
use utils::logger::create_child_logger;

#[tokio::main]
async fn main() {
    let settings = Settings::new().expect("Failed to load configuration");
    let app_logger = create_child_logger("app");

    info!(
        app_logger,
        "Application started with settings: {:?}", settings
    );

    let collection = Collection::new(app_logger);

    collection.start().await;

    // Wait for a termination signal
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for termination signal");
}
