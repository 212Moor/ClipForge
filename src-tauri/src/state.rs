use crate::paths::AppPaths;
use crate::store::Database;
use std::sync::Mutex;
use tracing_appender::non_blocking::WorkerGuard;

pub struct AppState {
    pub paths: AppPaths,
    pub db: Database,
    /// Matériau réellement appliqué à la fenêtre : "mica", "acrylic" ou "solid".
    pub vibrancy: Mutex<String>,
    _log_guard: WorkerGuard,
}

impl AppState {
    pub fn new(paths: AppPaths, db: Database, log_guard: WorkerGuard) -> Self {
        Self {
            paths,
            db,
            vibrancy: Mutex::new("solid".to_string()),
            _log_guard: log_guard,
        }
    }
}
