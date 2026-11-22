use crate::gemini::ratelimit::GeminiRateLimiter;
use crate::paths::AppPaths;
use crate::store::Database;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tracing_appender::non_blocking::WorkerGuard;

pub struct AppState {
    pub paths: AppPaths,
    pub db: Database,
    /// Matériau réellement appliqué à la fenêtre : "mica", "acrylic" ou "solid".
    pub vibrancy: Mutex<String>,
    /// Drapeaux de pause/annulation des analyses en cours par `project_id`.
    pub cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Drapeaux d'annulation des rendus vidéo en cours par `clip_id`.
    pub render_cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Sémaphore limitant la concurrence de rendu FFmpeg (1 à 2 jobs en parallèle selon GPU).
    pub render_semaphore: Arc<Semaphore>,
    /// File unique + limiteur de débit RPM/TPM pour l'API Gemini.
    pub gemini_limiter: Arc<GeminiRateLimiter>,
    _log_guard: WorkerGuard,
}

impl AppState {
    pub fn new(paths: AppPaths, db: Database, log_guard: WorkerGuard) -> Self {
        Self {
            paths,
            db,
            vibrancy: Mutex::new("solid".to_string()),
            cancel_flags: Mutex::new(HashMap::new()),
            render_cancel_flags: Mutex::new(HashMap::new()),
            render_semaphore: Arc::new(Semaphore::new(2)),
            gemini_limiter: Arc::new(GeminiRateLimiter::default()),
            _log_guard: log_guard,
        }
    }

    pub fn register_cancel_flag(&self, project_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.cancel_flags.lock() {
            map.insert(project_id.to_string(), Arc::clone(&flag));
        }
        flag
    }

    pub fn request_cancel(&self, project_id: &str) -> bool {
        if let Ok(map) = self.cancel_flags.lock() {
            if let Some(flag) = map.get(project_id) {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    pub fn clear_cancel_flag(&self, project_id: &str) {
        if let Ok(mut map) = self.cancel_flags.lock() {
            map.remove(project_id);
        }
    }

    pub fn register_render_cancel(&self, clip_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.render_cancel_flags.lock() {
            map.insert(clip_id.to_string(), Arc::clone(&flag));
        }
        flag
    }

    pub fn request_render_cancel(&self, clip_id: &str) -> bool {
        if let Ok(map) = self.render_cancel_flags.lock() {
            if let Some(flag) = map.get(clip_id) {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    pub fn clear_render_cancel(&self, clip_id: &str) {
        if let Ok(mut map) = self.render_cancel_flags.lock() {
            map.remove(clip_id);
        }
    }
}
