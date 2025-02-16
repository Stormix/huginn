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
