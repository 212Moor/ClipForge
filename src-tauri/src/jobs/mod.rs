use crate::error::AppResult;
use crate::store::Database;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct JobRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub kind: String,
    pub state: JobState,
    pub progress: f64,
    pub checkpoint_json: String,
    pub error_message: Option<String>,
    pub updated_at: String,
}

/// Persistance atomique d'un checkpoint de job pour garantir la reprise après plantage.
pub fn upsert_job_checkpoint(db: &Database, record: &JobRecord) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO jobs (id, project_id, kind, state, progress, checkpoint_json, error_message, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
                state = excluded.state,
                progress = excluded.progress,
                checkpoint_json = excluded.checkpoint_json,
                error_message = excluded.error_message,
                updated_at = excluded.updated_at",
            rusqlite::params![
                record.id,
                record.project_id,
                record.kind,
                record.state.as_str(),
                record.progress,
                record.checkpoint_json,
                record.error_message,
                now
            ],
        )?;
        Ok(())
    })
}
