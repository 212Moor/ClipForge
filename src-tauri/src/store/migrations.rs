use crate::error::AppResult;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppliedMigration {
    pub version: u32,
    pub name: String,
    pub applied_at: String,
}

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_initial_schema",
    sql: r#"
CREATE TABLE IF NOT EXISTS streamer_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    tone TEXT NOT NULL DEFAULT '',
    jargon_json TEXT NOT NULL DEFAULT '[]',
    emotes_json TEXT NOT NULL DEFAULT '{}',
    games_json TEXT NOT NULL DEFAULT '[]',
    inside_jokes TEXT NOT NULL DEFAULT '',
    forbidden_topics TEXT NOT NULL DEFAULT '',
    facecam_rect_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    streamer_profile_id TEXT REFERENCES streamer_profiles(id) ON DELETE SET NULL,
    mode TEXT NOT NULL DEFAULT 'standard' CHECK (mode IN ('eco', 'standard', 'quality')),
    target_clips INTEGER NOT NULL DEFAULT 12,
    target_min_sec INTEGER NOT NULL DEFAULT 30,
    target_max_sec INTEGER NOT NULL DEFAULT 60,
    token_budget_limit INTEGER NOT NULL DEFAULT 150000,
    status TEXT NOT NULL DEFAULT 'draft',
    workspace_dir TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vods (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL UNIQUE REFERENCES projects(id) ON DELETE CASCADE,
    source_url TEXT NOT NULL,
    twitch_vod_id TEXT,
    channel_name TEXT,
    title TEXT,
    duration_sec REAL NOT NULL DEFAULT 0,
    thumbnail_url TEXT,
    audio_path TEXT,
    chat_path TEXT,
    muted_segments_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS signal_windows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vod_id TEXT NOT NULL REFERENCES vods(id) ON DELETE CASCADE,
    start_sec REAL NOT NULL,
    end_sec REAL NOT NULL,
    chat_rate REAL NOT NULL DEFAULT 0,
    emote_score REAL NOT NULL DEFAULT 0,
    event_score REAL NOT NULL DEFAULT 0,
    audio_lufs REAL NOT NULL DEFAULT -70,
    audio_peak REAL NOT NULL DEFAULT 0,
    voice_ratio REAL NOT NULL DEFAULT 0,
    scene_change REAL NOT NULL DEFAULT 0,
    viewer_clips_count INTEGER NOT NULL DEFAULT 0,
    is_muted INTEGER NOT NULL DEFAULT 0,
    composite_score REAL NOT NULL DEFAULT 0,
    UNIQUE(vod_id, start_sec)
);

CREATE INDEX IF NOT EXISTS idx_signal_windows_vod_start
    ON signal_windows(vod_id, start_sec);

CREATE TABLE IF NOT EXISTS candidates (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    vod_id TEXT NOT NULL REFERENCES vods(id) ON DELETE CASCADE,
    rank_index INTEGER NOT NULL,
    start_sec REAL NOT NULL,
    end_sec REAL NOT NULL,
    peak_sec REAL NOT NULL,
    local_score REAL NOT NULL,
    signals_summary_json TEXT NOT NULL DEFAULT '{}',
    transcript_json TEXT,
    chat_summary_json TEXT,
    stage_a_keep INTEGER,
    stage_a_score INTEGER,
    stage_a_category TEXT,
    stage_a_adj_start REAL,
    stage_a_adj_end REAL,
    stage_a_reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_candidates_project_rank
    ON candidates(project_id, rank_index);

CREATE TABLE IF NOT EXISTS clips (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    candidate_id TEXT REFERENCES candidates(id) ON DELETE SET NULL,
    start_sec REAL NOT NULL,
    end_sec REAL NOT NULL,
    hook TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    caption TEXT NOT NULL DEFAULT '',
    hashtags_json TEXT NOT NULL DEFAULT '[]',
    category TEXT NOT NULL DEFAULT 'fun',
    layout_mode TEXT NOT NULL DEFAULT 'split' CHECK (layout_mode IN ('split', 'fit', 'fill')),
    subtitle_style TEXT NOT NULL DEFAULT 'bold_pop',
    subtitles_override_json TEXT,
    review_status TEXT NOT NULL DEFAULT 'pending' CHECK (review_status IN ('pending', 'approved', 'rejected')),
    render_status TEXT NOT NULL DEFAULT 'idle' CHECK (render_status IN ('idle', 'queued', 'rendering', 'done', 'failed')),
    output_mp4_path TEXT,
    output_caption_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_clips_project_status
    ON clips(project_id, review_status);

CREATE TABLE IF NOT EXISTS gemini_calls (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    stage TEXT NOT NULL CHECK (stage IN ('stage_a', 'stage_b', 'stage_c', 'regen', 'test')),
    payload_hash TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_cost_usd REAL NOT NULL DEFAULT 0,
    cache_hit INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    response_json TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_gemini_calls_hash
    ON gemini_calls(payload_hash);
CREATE INDEX IF NOT EXISTS idx_gemini_calls_project
    ON gemini_calls(project_id, stage);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'paused', 'completed', 'failed', 'cancelled')),
    progress REAL NOT NULL DEFAULT 0,
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_jobs_project_state
    ON jobs(project_id, state);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#,
}];

pub fn run_migrations(conn: &mut Connection) -> AppResult<Vec<AppliedMigration>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;

    let current_max: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;

    for migration in MIGRATIONS {
        if migration.version > current_max {
            let tx = conn.transaction()?;
            tx.execute_batch(migration.sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![migration.version, migration.name, Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
            tracing::info!(
                version = migration.version,
                name = migration.name,
                "Migration SQLite appliquée"
            );
        }
    }

    list_applied_migrations(conn)
}

pub fn list_applied_migrations(conn: &Connection) -> AppResult<Vec<AppliedMigration>> {
    let mut stmt =
        conn.prepare("SELECT version, name, applied_at FROM schema_migrations ORDER BY version ASC")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AppliedMigration {
                version: row.get(0)?,
                name: row.get(1)?,
                applied_at: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        let first = run_migrations(&mut conn).unwrap();
        assert_eq!(first.len(), MIGRATIONS.len());

        let second = run_migrations(&mut conn).unwrap();
        assert_eq!(second.len(), first.len());

        let tables: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
                 ('projects','vods','signal_windows','candidates','clips','gemini_calls','jobs','settings','streamer_profiles')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 9);
    }
}
