use slog::{Logger, info};

pub struct Collection {
    logger: Logger,
}

impl Collection {
    pub fn new(logger: Logger) -> Self {
        Self { logger }
    }

    pub async fn start(&self) {
        info!(self.logger, "Starting collection");

        // Log the data every second
        // tokio::spawn(Self::log_data(Arc::clone(&data_clone), self.logger.clone()));
    }
}
