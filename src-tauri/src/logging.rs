use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Initialise les journaux structurés JSON (rotation quotidienne) dans `%LOCALAPPDATA%/ClipForge/logs/`.
/// Le `WorkerGuard` retourné doit rester vivant pendant toute la durée du processus.
pub fn init_logging(logs_dir: &Path) -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily(logs_dir, "clipforge.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("clipforge_lib=info,tauri=warn,reqwest=warn"));

    let file_layer = fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(false)
        .with_line_number(false);

    let stdout_layer = fmt::layer()
        .compact()
        .with_target(true)
        .with_ansi(cfg!(debug_assertions));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stdout_layer)
        .init();

    guard
}
