use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Logs to the terminal and to spotlight/spotlight.log.YYYY-MM-DD (a new file each day, last 7 kept).
/// Keep the returned guard alive until the program exits so buffered logs get written.
pub fn init() -> WorkerGuard {
    // Uses RUST_LOG if set (e.g. RUST_LOG=debug), otherwise defaults to "info".
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("spotlight.log")
        .max_log_files(7) // keep the last 7 days, delete older files
        .build("spotlight")
        .expect("failed to create log file appender");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer()) // terminal
        .with(fmt::layer().with_writer(file_writer).with_ansi(false)) // file, no color codes
        .init();

    guard
}
