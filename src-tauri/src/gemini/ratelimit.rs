use crate::error::AppResult;
use crate::store::Database;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const SETTING_RATE_LIMIT: &str = "gemini.ratelimit";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RateLimitConfig {
    pub max_rpm: u32,
    pub max_tpm: u32,
    pub max_retries: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_rpm: 15,
            max_tpm: 250_000,
            max_retries: 4,
        }
    }
}

pub fn load_rate_limit_config(db: &Database) -> RateLimitConfig {
    if let Ok(Some(raw)) = db.get_setting(SETTING_RATE_LIMIT) {
        if let Ok(cfg) = serde_json::from_str::<RateLimitConfig>(&raw) {
            return cfg;
        }
    }
    RateLimitConfig::default()
}

pub fn save_rate_limit_config(db: &Database, cfg: &RateLimitConfig) -> AppResult<()> {
    db.set_setting(SETTING_RATE_LIMIT, &serde_json::to_string(cfg)?)
}

struct WindowState {
    config: RateLimitConfig,
    history: VecDeque<(Instant, u32)>,
}

pub struct GeminiRateLimiter {
    inner: Mutex<WindowState>,
}

impl Default for GeminiRateLimiter {
    fn default() -> Self {
        Self {
            inner: Mutex::new(WindowState {
                config: RateLimitConfig::default(),
                history: VecDeque::new(),
            }),
        }
    }
}

impl GeminiRateLimiter {
    pub async fn update_config(&self, config: RateLimitConfig) {
        let mut guard = self.inner.lock().await;
        guard.config = config;
    }

    pub async fn max_retries(&self) -> u32 {
        self.inner.lock().await.config.max_retries.clamp(1, 6)
    }

    pub async fn acquire(&self, estimated_tokens: u32) {
        loop {
            let wait_duration = {
                let mut guard = self.inner.lock().await;
                let now = Instant::now();
                let window = Duration::from_secs(60);
                while let Some(&(ts, _)) = guard.history.front() {
                    if now.duration_since(ts) >= window {
                        guard.history.pop_front();
                    } else {
                        break;
                    }
                }

                let current_rpm = guard.history.len() as u32;
                let current_tpm: u32 = guard.history.iter().map(|&(_, t)| t).sum();
                let rpm_ok = current_rpm < guard.config.max_rpm.max(1);
                let tpm_ok = current_tpm.saturating_add(estimated_tokens)
                    <= guard.config.max_tpm.max(10_000)
                    || guard.history.is_empty();

                if rpm_ok && tpm_ok {
                    guard.history.push_back((now, estimated_tokens));
                    None
                } else {
                    let oldest = guard.history.front().map(|&(ts, _)| ts).unwrap_or(now);
                    let elapsed = now.duration_since(oldest);
                    Some(window.saturating_sub(elapsed) + Duration::from_millis(250))
                }
            };

            match wait_duration {
                None => return,
                Some(dur) => tokio::time::sleep(dur).await,
            }
        }
    }
}

pub fn backoff_with_jitter(attempt: u32) -> Duration {
    let base_ms = 1_200u64.saturating_mul(1u64 << attempt.min(5));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let jitter_ms = nanos % 750;
    Duration::from_millis((base_ms + jitter_ms).min(45_000))
}
