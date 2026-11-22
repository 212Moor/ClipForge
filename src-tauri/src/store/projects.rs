use crate::error::{AppError, AppResult};
use crate::store::Database;
use chrono::{Datelike, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisMode {
    Eco,
    Standard,
    Quality,
}

impl AnalysisMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eco => "eco",
            Self::Standard => "standard",
            Self::Quality => "quality",
        }
    }

    pub fn from_str(raw: &str) -> Self {
        match raw {
            "eco" => Self::Eco,
            "quality" => Self::Quality,
            _ => Self::Standard,
        }
    }
}

/// Zone facecam normalisée (0.0 à 1.0 par rapport au cadre 16:9 source), mémorisée par streamer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct FacecamRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Default for FacecamRect {
    fn default() -> Self {
        // Coin bas-gauche typique des streams gaming
        Self {
            x: 0.02,
            y: 0.62,
            w: 0.24,
            h: 0.34,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StreamerProfileRecord {
    pub id: String,
    pub name: String,
    pub tone: String,
    pub jargon: Vec<String>,
    pub emotes: HashMap<String, String>,
    pub games: Vec<String>,
    pub inside_jokes: String,
    pub forbidden_topics: String,
    pub facecam_rect: Option<FacecamRect>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SaveStreamerProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub tone: String,
    pub jargon: Vec<String>,
    pub emotes: HashMap<String, String>,
    pub games: Vec<String>,
    pub inside_jokes: String,
    pub forbidden_topics: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MutedSegment {
    pub start_sec: f64,
    pub end_sec: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VodRecord {
    pub id: String,
    pub project_id: String,
    pub source_url: String,
    pub twitch_vod_id: Option<String>,
    pub channel_name: Option<String>,
    pub title: Option<String>,
    pub duration_sec: f64,
    pub thumbnail_url: Option<String>,
    pub audio_path: Option<String>,
    pub chat_path: Option<String>,
    pub muted_segments: Vec<MutedSegment>,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub streamer_profile_id: Option<String>,
    pub mode: AnalysisMode,
    pub target_clips: u32,
    pub target_min_sec: u32,
    pub target_max_sec: u32,
    pub token_budget_limit: u32,
    pub status: String,
    pub workspace_dir: String,
    pub created_at: String,
    pub updated_at: String,
    pub vod_title: Option<String>,
    pub vod_channel: Option<String>,
    pub vod_duration_sec: f64,
    pub vod_thumbnail_url: Option<String>,
    pub candidates_count: u32,
    pub clips_count: u32,
    pub spent_input_tokens: u32,
    pub spent_output_tokens: u32,
    pub spent_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SignalWindowRecord {
    pub start_sec: f64,
    pub end_sec: f64,
    pub chat_rate: f64,
    pub emote_score: f64,
    pub event_score: f64,
    pub audio_lufs: f64,
    pub audio_peak: f64,
    pub voice_ratio: f64,
    pub scene_change: f64,
    pub viewer_clips_count: u32,
    pub is_muted: bool,
    pub composite_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CandidateRecord {
    pub id: String,
    pub project_id: String,
    pub vod_id: String,
    pub rank_index: u32,
    pub start_sec: f64,
    pub end_sec: f64,
    pub peak_sec: f64,
    pub local_score: f64,
    pub signals_summary_json: String,
    pub transcript_json: Option<String>,
    pub chat_summary_json: Option<String>,
    pub stage_a_keep: Option<bool>,
    pub stage_a_score: Option<u32>,
    pub stage_a_category: Option<String>,
    pub stage_a_adj_start: Option<f64>,
    pub stage_a_adj_end: Option<f64>,
    pub stage_a_reason: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ClipRecord {
    pub id: String,
    pub project_id: String,
    pub candidate_id: Option<String>,
    pub start_sec: f64,
    pub end_sec: f64,
    pub hook: String,
    pub title: String,
    pub caption: String,
    pub hashtags: Vec<String>,
    pub category: String,
    pub layout_mode: String,
    pub subtitle_style: String,
    pub subtitles_override_json: Option<String>,
    pub review_status: String,
    pub render_status: String,
    pub output_mp4_path: Option<String>,
    pub output_caption_path: Option<String>,
    pub score: Option<u32>,
    pub transcript_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateClipInput {
    pub id: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub hook: String,
    pub title: String,
    pub caption: String,
    pub hashtags: Vec<String>,
    pub category: String,
    pub layout_mode: String,
    pub subtitle_style: String,
    pub subtitles_override_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProjectTimelineData {
    pub project: ProjectSummary,
    pub vod: VodRecord,
    pub signal_windows: Vec<SignalWindowRecord>,
    pub candidates: Vec<CandidateRecord>,
    pub clips: Vec<ClipRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GeminiCallRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub stage: String,
    pub model: String,
    pub prompt_version: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub estimated_cost_usd: f64,
    pub cache_hit: bool,
    pub duration_ms: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StageCostSummary {
    pub stage: String,
    pub calls: u32,
    pub cache_hits: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProjectCostSummary {
    pub project_id: String,
    pub project_name: String,
    pub mode: AnalysisMode,
    pub token_budget_limit: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub calls: u32,
    pub cache_hits: u32,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TokenUsageGauge {
    pub today_input_tokens: u32,
    pub today_output_tokens: u32,
    pub today_cost_usd: f64,
    pub month_input_tokens: u32,
    pub month_output_tokens: u32,
    pub month_cost_usd: f64,
    pub total_cache_hits: u32,
    pub total_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CostsDashboardData {
    pub gauge: TokenUsageGauge,
    pub by_stage: Vec<StageCostSummary>,
    pub by_project: Vec<ProjectCostSummary>,
    pub recent_calls: Vec<GeminiCallRecord>,
    pub usd_to_eur: f64,
}

pub fn list_streamer_profiles(db: &Database) -> AppResult<Vec<StreamerProfileRecord>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, tone, jargon_json, emotes_json, games_json, inside_jokes, forbidden_topics, facecam_rect_json, updated_at
             FROM streamer_profiles ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let jargon_raw: String = row.get(3)?;
                let emotes_raw: String = row.get(4)?;
                let games_raw: String = row.get(5)?;
                let facecam_raw: Option<String> = row.get(8)?;
                Ok(StreamerProfileRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    tone: row.get(2)?,
                    jargon: serde_json::from_str(&jargon_raw).unwrap_or_default(),
                    emotes: serde_json::from_str(&emotes_raw).unwrap_or_default(),
                    games: serde_json::from_str(&games_raw).unwrap_or_default(),
                    inside_jokes: row.get(6)?,
                    forbidden_topics: row.get(7)?,
                    facecam_rect: facecam_raw.and_then(|s| serde_json::from_str(&s).ok()),
                    updated_at: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

pub fn get_streamer_profile(db: &Database, id: &str) -> AppResult<Option<StreamerProfileRecord>> {
    db.with_conn(|conn| {
        let row = conn
            .query_row(
                "SELECT id, name, tone, jargon_json, emotes_json, games_json, inside_jokes, forbidden_topics, facecam_rect_json, updated_at
                 FROM streamer_profiles WHERE id = ?1",
                [id],
                |row| {
                    let jargon_raw: String = row.get(3)?;
                    let emotes_raw: String = row.get(4)?;
                    let games_raw: String = row.get(5)?;
                    let facecam_raw: Option<String> = row.get(8)?;
                    Ok(StreamerProfileRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        tone: row.get(2)?,
                        jargon: serde_json::from_str(&jargon_raw).unwrap_or_default(),
                        emotes: serde_json::from_str(&emotes_raw).unwrap_or_default(),
                        games: serde_json::from_str(&games_raw).unwrap_or_default(),
                        inside_jokes: row.get(6)?,
                        forbidden_topics: row.get(7)?,
                        facecam_rect: facecam_raw.and_then(|s| serde_json::from_str(&s).ok()),
                        updated_at: row.get(9)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    })
}

pub fn save_streamer_profile(
    db: &Database,
    input: &SaveStreamerProfileInput,
) -> AppResult<StreamerProfileRecord> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "Le nom du profil streamer est obligatoire.".to_string(),
        ));
    }
    let id = input
        .id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = Utc::now().to_rfc3339();
    let jargon_json = serde_json::to_string(&input.jargon)?;
    let emotes_json = serde_json::to_string(&input.emotes)?;
    let games_json = serde_json::to_string(&input.games)?;

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO streamer_profiles
             (id, name, tone, jargon_json, emotes_json, games_json, inside_jokes, forbidden_topics, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                tone = excluded.tone,
                jargon_json = excluded.jargon_json,
                emotes_json = excluded.emotes_json,
                games_json = excluded.games_json,
                inside_jokes = excluded.inside_jokes,
                forbidden_topics = excluded.forbidden_topics,
                updated_at = excluded.updated_at",
            rusqlite::params![
                id,
                name,
                input.tone.trim(),
                jargon_json,
                emotes_json,
                games_json,
                input.inside_jokes.trim(),
                input.forbidden_topics.trim(),
                now
            ],
        )?;
        Ok(())
    })?;

    Ok(StreamerProfileRecord {
        id,
        name: name.to_string(),
        tone: input.tone.trim().to_string(),
        jargon: input.jargon.clone(),
        emotes: input.emotes.clone(),
        games: input.games.clone(),
        inside_jokes: input.inside_jokes.trim().to_string(),
        forbidden_topics: input.forbidden_topics.trim().to_string(),
        facecam_rect: None,
        updated_at: now,
    })
}

pub fn delete_streamer_profile(db: &Database, id: &str) -> AppResult<()> {
    db.with_conn(|conn| {
        conn.execute("DELETE FROM streamer_profiles WHERE id = ?1", [id])?;
        Ok(())
    })
}

pub fn list_projects(db: &Database) -> AppResult<Vec<ProjectSummary>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                p.id, p.name, p.streamer_profile_id, p.mode, p.target_clips,
                p.target_min_sec, p.target_max_sec, p.token_budget_limit, p.status,
                p.workspace_dir, p.created_at, p.updated_at,
                v.title, v.channel_name, COALESCE(v.duration_sec, 0.0), v.thumbnail_url,
                (SELECT COUNT(*) FROM candidates c WHERE c.project_id = p.id),
                (SELECT COUNT(*) FROM clips cl WHERE cl.project_id = p.id),
                COALESCE((SELECT SUM(g.input_tokens) FROM gemini_calls g WHERE g.project_id = p.id), 0),
                COALESCE((SELECT SUM(g.output_tokens) FROM gemini_calls g WHERE g.project_id = p.id), 0),
                COALESCE((SELECT SUM(g.estimated_cost_usd) FROM gemini_calls g WHERE g.project_id = p.id), 0.0)
             FROM projects p
             LEFT JOIN vods v ON v.project_id = p.id
             ORDER BY p.updated_at DESC",
        )?;

        let rows = stmt
            .query_map([], |row| {
                let mode_str: String = row.get(3)?;
                let in_tok: i64 = row.get(18)?;
                let out_tok: i64 = row.get(19)?;
                Ok(ProjectSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    streamer_profile_id: row.get(2)?,
                    mode: AnalysisMode::from_str(&mode_str),
                    target_clips: row.get(4)?,
                    target_min_sec: row.get(5)?,
                    target_max_sec: row.get(6)?,
                    token_budget_limit: row.get(7)?,
                    status: row.get(8)?,
                    workspace_dir: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    vod_title: row.get(12)?,
                    vod_channel: row.get(13)?,
                    vod_duration_sec: row.get(14)?,
                    vod_thumbnail_url: row.get(15)?,
                    candidates_count: row.get(16)?,
                    clips_count: row.get(17)?,
                    spent_input_tokens: in_tok.clamp(0, i64::from(u32::MAX)) as u32,
                    spent_output_tokens: out_tok.clamp(0, i64::from(u32::MAX)) as u32,
                    spent_cost_usd: row.get(20)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    })
}

pub fn get_project_summary(db: &Database, project_id: &str) -> AppResult<ProjectSummary> {
    list_projects(db)?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| AppError::Config(format!("Projet introuvable : {project_id}")))
}

pub fn get_vod_by_project(db: &Database, project_id: &str) -> AppResult<VodRecord> {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT id, project_id, source_url, twitch_vod_id, channel_name, title,
                    duration_sec, thumbnail_url, audio_path, chat_path, muted_segments_json, metadata_json
             FROM vods WHERE project_id = ?1",
            [project_id],
            |row| {
                let muted_raw: String = row.get(10)?;
                Ok(VodRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    source_url: row.get(2)?,
                    twitch_vod_id: row.get(3)?,
                    channel_name: row.get(4)?,
                    title: row.get(5)?,
                    duration_sec: row.get(6)?,
                    thumbnail_url: row.get(7)?,
                    audio_path: row.get(8)?,
                    chat_path: row.get(9)?,
                    muted_segments: serde_json::from_str(&muted_raw).unwrap_or_default(),
                    metadata_json: row.get(11)?,
                })
            },
        )
        .map_err(AppError::Database)
    })
}

pub fn update_project_status(db: &Database, project_id: &str, status: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE projects SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![status, now, project_id],
        )?;
        Ok(())
    })
}

pub fn update_project_budget(db: &Database, project_id: &str, new_limit: u32) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE projects SET token_budget_limit = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_limit.max(5_000), now, project_id],
        )?;
        Ok(())
    })
}

pub fn delete_project(db: &Database, project_id: &str) -> AppResult<Option<String>> {
    let workspace = db.with_conn(|conn| {
        let dir: Option<String> = conn
            .query_row(
                "SELECT workspace_dir FROM projects WHERE id = ?1",
                [project_id],
                |row| row.get(0),
            )
            .optional()?;
        conn.execute("DELETE FROM projects WHERE id = ?1", [project_id])?;
        Ok(dir)
    })?;
    Ok(workspace)
}

pub fn replace_signal_windows(
    db: &Database,
    vod_id: &str,
    windows: &[SignalWindowRecord],
) -> AppResult<()> {
    db.with_conn_mut(|conn| {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM signal_windows WHERE vod_id = ?1", [vod_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO signal_windows
                 (vod_id, start_sec, end_sec, chat_rate, emote_score, event_score,
                  audio_lufs, audio_peak, voice_ratio, scene_change, viewer_clips_count,
                  is_muted, composite_score)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            for w in windows {
                stmt.execute(rusqlite::params![
                    vod_id,
                    w.start_sec,
                    w.end_sec,
                    w.chat_rate,
                    w.emote_score,
                    w.event_score,
                    w.audio_lufs,
                    w.audio_peak,
                    w.voice_ratio,
                    w.scene_change,
                    w.viewer_clips_count,
                    if w.is_muted { 1 } else { 0 },
                    w.composite_score,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
}

pub fn list_signal_windows(db: &Database, vod_id: &str) -> AppResult<Vec<SignalWindowRecord>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT start_sec, end_sec, chat_rate, emote_score, event_score,
                    audio_lufs, audio_peak, voice_ratio, scene_change, viewer_clips_count,
                    is_muted, composite_score
             FROM signal_windows WHERE vod_id = ?1 ORDER BY start_sec ASC",
        )?;
        let rows = stmt
            .query_map([vod_id], |row| {
                let muted_int: i32 = row.get(10)?;
                Ok(SignalWindowRecord {
                    start_sec: row.get(0)?,
                    end_sec: row.get(1)?,
                    chat_rate: row.get(2)?,
                    emote_score: row.get(3)?,
                    event_score: row.get(4)?,
                    audio_lufs: row.get(5)?,
                    audio_peak: row.get(6)?,
                    voice_ratio: row.get(7)?,
                    scene_change: row.get(8)?,
                    viewer_clips_count: row.get(9)?,
                    is_muted: muted_int != 0,
                    composite_score: row.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

pub fn list_candidates(db: &Database, project_id: &str) -> AppResult<Vec<CandidateRecord>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, project_id, vod_id, rank_index, start_sec, end_sec, peak_sec,
                    local_score, signals_summary_json, transcript_json, chat_summary_json,
                    stage_a_keep, stage_a_score, stage_a_category, stage_a_adj_start,
                    stage_a_adj_end, stage_a_reason, status
             FROM candidates WHERE project_id = ?1 ORDER BY rank_index ASC",
        )?;
        let rows = stmt
            .query_map([project_id], |row| {
                let keep_int: Option<i32> = row.get(11)?;
                let score_int: Option<i32> = row.get(12)?;
                Ok(CandidateRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    vod_id: row.get(2)?,
                    rank_index: row.get(3)?,
                    start_sec: row.get(4)?,
                    end_sec: row.get(5)?,
                    peak_sec: row.get(6)?,
                    local_score: row.get(7)?,
                    signals_summary_json: row.get(8)?,
                    transcript_json: row.get(9)?,
                    chat_summary_json: row.get(10)?,
                    stage_a_keep: keep_int.map(|v| v != 0),
                    stage_a_score: score_int.map(|v| v.max(0) as u32),
                    stage_a_category: row.get(13)?,
                    stage_a_adj_start: row.get(14)?,
                    stage_a_adj_end: row.get(15)?,
                    stage_a_reason: row.get(16)?,
                    status: row.get(17)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

fn map_clip_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipRecord> {
    let tags_raw: String = row.get(8)?;
    let stage_a_score: Option<i32> = row.get(17)?;
    let local_score: Option<f64> = row.get(18)?;
    let score = stage_a_score
        .map(|s| s.clamp(0, 100) as u32)
        .or_else(|| local_score.map(|ls| (ls * 100.0).round().clamp(0.0, 100.0) as u32));

    Ok(ClipRecord {
        id: row.get(0)?,
        project_id: row.get(1)?,
        candidate_id: row.get(2)?,
        start_sec: row.get(3)?,
        end_sec: row.get(4)?,
        hook: row.get(5)?,
        title: row.get(6)?,
        caption: row.get(7)?,
        hashtags: serde_json::from_str(&tags_raw).unwrap_or_default(),
        category: row.get(9)?,
        layout_mode: row.get(10)?,
        subtitle_style: row.get(11)?,
        subtitles_override_json: row.get(12)?,
        review_status: row.get(13)?,
        render_status: row.get(14)?,
        output_mp4_path: row.get(15)?,
        output_caption_path: row.get(16)?,
        score,
        transcript_json: row.get(19)?,
    })
}

pub fn list_clips(db: &Database, project_id: &str) -> AppResult<Vec<ClipRecord>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT cl.id, cl.project_id, cl.candidate_id, cl.start_sec, cl.end_sec,
                    cl.hook, cl.title, cl.caption, cl.hashtags_json, cl.category,
                    cl.layout_mode, cl.subtitle_style, cl.subtitles_override_json,
                    cl.review_status, cl.render_status, cl.output_mp4_path, cl.output_caption_path,
                    c.stage_a_score, c.local_score, c.transcript_json
             FROM clips cl
             LEFT JOIN candidates c ON c.id = cl.candidate_id
             WHERE cl.project_id = ?1
             ORDER BY cl.start_sec ASC",
        )?;
        let rows = stmt
            .query_map([project_id], map_clip_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

pub fn get_clip(db: &Database, clip_id: &str) -> AppResult<ClipRecord> {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT cl.id, cl.project_id, cl.candidate_id, cl.start_sec, cl.end_sec,
                    cl.hook, cl.title, cl.caption, cl.hashtags_json, cl.category,
                    cl.layout_mode, cl.subtitle_style, cl.subtitles_override_json,
                    cl.review_status, cl.render_status, cl.output_mp4_path, cl.output_caption_path,
                    c.stage_a_score, c.local_score, c.transcript_json
             FROM clips cl
             LEFT JOIN candidates c ON c.id = cl.candidate_id
             WHERE cl.id = ?1",
            [clip_id],
            map_clip_row,
        )
        .map_err(AppError::Database)
    })
}

pub fn update_clip_review_status(
    db: &Database,
    clip_id: &str,
    review_status: &str,
) -> AppResult<ClipRecord> {
    if !matches!(review_status, "pending" | "approved" | "rejected") {
        return Err(AppError::Validation(format!(
            "Statut de revue invalide : {review_status}"
        )));
    }
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE clips SET review_status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![review_status, now, clip_id],
        )?;
        Ok(())
    })?;
    get_clip(db, clip_id)
}

pub fn update_clip_edits(db: &Database, input: &UpdateClipInput) -> AppResult<ClipRecord> {
    if !matches!(input.layout_mode.as_str(), "split" | "fit" | "fill") {
        return Err(AppError::Validation(format!(
            "Mise en page invalide : {}",
            input.layout_mode
        )));
    }
    let start = input.start_sec.max(0.0);
    let end = input.end_sec.max(start + 2.0);
    let tags_json = serde_json::to_string(&input.hashtags)?;
    let now = Utc::now().to_rfc3339();

    db.with_conn(|conn| {
        conn.execute(
            "UPDATE clips
             SET start_sec = ?1, end_sec = ?2, hook = ?3, title = ?4, caption = ?5,
                 hashtags_json = ?6, category = ?7, layout_mode = ?8, subtitle_style = ?9,
                 subtitles_override_json = ?10, updated_at = ?11
             WHERE id = ?12",
            rusqlite::params![
                start,
                end,
                input.hook.trim(),
                input.title.trim(),
                input.caption.trim(),
                tags_json,
                input.category.trim(),
                input.layout_mode,
                input.subtitle_style,
                input.subtitles_override_json,
                now,
                input.id
            ],
        )?;
        Ok(())
    })?;
    get_clip(db, &input.id)
}

pub fn update_clip_render_result(
    db: &Database,
    clip_id: &str,
    render_status: &str,
    output_mp4_path: Option<&str>,
    output_caption_path: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE clips
             SET render_status = ?1,
                 output_mp4_path = COALESCE(?2, output_mp4_path),
                 output_caption_path = COALESCE(?3, output_caption_path),
                 updated_at = ?4
             WHERE id = ?5",
            rusqlite::params![render_status, output_mp4_path, output_caption_path, now, clip_id],
        )?;
        Ok(())
    })
}

pub fn save_streamer_facecam_rect(
    db: &Database,
    profile_id: &str,
    rect: FacecamRect,
) -> AppResult<()> {
    let rect_json = serde_json::to_string(&rect)?;
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE streamer_profiles SET facecam_rect_json = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![rect_json, now, profile_id],
        )?;
        Ok(())
    })
}

/// Crée des clips éditables depuis les meilleurs candidats locaux lorsqu'aucun clip Gemini n'a encore été généré
/// (pratique en mode 0 token / local seul ou sur le jeu de test 10 min).
pub fn promote_candidates_to_clips(db: &Database, project_id: &str) -> AppResult<Vec<ClipRecord>> {
    let existing = list_clips(db, project_id)?;
    if !existing.is_empty() {
        return Ok(existing);
    }
    let project = get_project_summary(db, project_id)?;
    let mut cands = list_candidates(db, project_id)?;
    cands.sort_by(|a, b| {
        b.local_score
            .partial_cmp(&a.local_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    cands.truncate(project.target_clips.max(1) as usize);

    let now = Utc::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        let tx = conn.transaction()?;
        for c in &cands {
            let hook = format!("Moment fort #{}", c.rank_index);
            let title = format!("{} — Temps fort #{}", project.name, c.rank_index);
            let caption = "Extrait détecté automatiquement par signaux locaux ClipForge.";
            tx.execute(
                r#"INSERT INTO clips
                 (id, project_id, candidate_id, start_sec, end_sec, hook, title, caption, hashtags_json,
                  category, layout_mode, subtitle_style, review_status, render_status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '["twitch","clip","gaming"]', 'fun', 'split', 'bold_pop', 'pending', 'idle', ?9, ?9)"#,
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    project_id,
                    c.id,
                    c.start_sec,
                    c.end_sec,
                    hook,
                    title,
                    caption,
                    now
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    })?;

    list_clips(db, project_id)
}

pub fn get_project_timeline_data(
    db: &Database,
    project_id: &str,
) -> AppResult<ProjectTimelineData> {
    let project = get_project_summary(db, project_id)?;
    let vod = get_vod_by_project(db, project_id)?;
    let signal_windows = list_signal_windows(db, &vod.id)?;
    let candidates = list_candidates(db, project_id)?;
    let clips = list_clips(db, project_id)?;
    Ok(ProjectTimelineData {
        project,
        vod,
        signal_windows,
        candidates,
        clips,
    })
}

pub fn get_token_usage_gauge(db: &Database) -> AppResult<TokenUsageGauge> {
    let now = Utc::now();
    let day_prefix = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
    let month_prefix = format!("{:04}-{:02}", now.year(), now.month());

    db.with_conn(|conn| {
        let (today_in, today_out, today_cost): (i64, i64, f64) = conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM gemini_calls WHERE created_at LIKE ?1",
            [format!("{day_prefix}%")],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        let (month_in, month_out, month_cost): (i64, i64, f64) = conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM gemini_calls WHERE created_at LIKE ?1",
            [format!("{month_prefix}%")],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        let (total_calls, total_hits): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(cache_hit), 0) FROM gemini_calls WHERE stage != 'test'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        Ok(TokenUsageGauge {
            today_input_tokens: today_in.clamp(0, i64::from(u32::MAX)) as u32,
            today_output_tokens: today_out.clamp(0, i64::from(u32::MAX)) as u32,
            today_cost_usd: today_cost,
            month_input_tokens: month_in.clamp(0, i64::from(u32::MAX)) as u32,
            month_output_tokens: month_out.clamp(0, i64::from(u32::MAX)) as u32,
            month_cost_usd: month_cost,
            total_cache_hits: total_hits.clamp(0, i64::from(u32::MAX)) as u32,
            total_calls: total_calls.clamp(0, i64::from(u32::MAX)) as u32,
        })
    })
}

pub fn get_costs_dashboard(db: &Database, usd_to_eur: f64) -> AppResult<CostsDashboardData> {
    let gauge = get_token_usage_gauge(db)?;

    let (by_stage, by_project, recent_calls) = db.with_conn(|conn| {
        let mut stage_stmt = conn.prepare(
            "SELECT stage, COUNT(*), COALESCE(SUM(cache_hit), 0),
                    COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0), COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM gemini_calls
             GROUP BY stage
             ORDER BY stage ASC",
        )?;
        let by_stage = stage_stmt
            .query_map([], |row| {
                let calls: i64 = row.get(1)?;
                let hits: i64 = row.get(2)?;
                let inp: i64 = row.get(3)?;
                let out: i64 = row.get(4)?;
                let cached: i64 = row.get(5)?;
                Ok(StageCostSummary {
                    stage: row.get(0)?,
                    calls: calls.max(0) as u32,
                    cache_hits: hits.max(0) as u32,
                    input_tokens: inp.clamp(0, i64::from(u32::MAX)) as u32,
                    output_tokens: out.clamp(0, i64::from(u32::MAX)) as u32,
                    cached_tokens: cached.clamp(0, i64::from(u32::MAX)) as u32,
                    cost_usd: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut proj_stmt = conn.prepare(
            "SELECT p.id, p.name, p.mode, p.token_budget_limit,
                    COALESCE(SUM(g.input_tokens), 0), COALESCE(SUM(g.output_tokens), 0),
                    COALESCE(SUM(g.cached_tokens), 0), COUNT(g.id),
                    COALESCE(SUM(g.cache_hit), 0), COALESCE(SUM(g.estimated_cost_usd), 0.0)
             FROM projects p
             LEFT JOIN gemini_calls g ON g.project_id = p.id
             GROUP BY p.id
             ORDER BY p.updated_at DESC",
        )?;
        let by_project = proj_stmt
            .query_map([], |row| {
                let mode_str: String = row.get(2)?;
                let inp: i64 = row.get(4)?;
                let out: i64 = row.get(5)?;
                let cached: i64 = row.get(6)?;
                let calls: i64 = row.get(7)?;
                let hits: i64 = row.get(8)?;
                Ok(ProjectCostSummary {
                    project_id: row.get(0)?,
                    project_name: row.get(1)?,
                    mode: AnalysisMode::from_str(&mode_str),
                    token_budget_limit: row.get(3)?,
                    input_tokens: inp.clamp(0, i64::from(u32::MAX)) as u32,
                    output_tokens: out.clamp(0, i64::from(u32::MAX)) as u32,
                    cached_tokens: cached.clamp(0, i64::from(u32::MAX)) as u32,
                    calls: calls.max(0) as u32,
                    cache_hits: hits.max(0) as u32,
                    cost_usd: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut calls_stmt = conn.prepare(
            "SELECT g.id, g.project_id, p.name, g.stage, g.model, g.prompt_version,
                    g.input_tokens, g.output_tokens, g.cached_tokens, g.estimated_cost_usd,
                    g.cache_hit, g.duration_ms, g.created_at
             FROM gemini_calls g
             LEFT JOIN projects p ON p.id = g.project_id
             ORDER BY g.created_at DESC
             LIMIT 100",
        )?;
        let recent_calls = calls_stmt
            .query_map([], |row| {
                let hit_int: i32 = row.get(10)?;
                let inp: i64 = row.get(6)?;
                let out: i64 = row.get(7)?;
                let cached: i64 = row.get(8)?;
                let dur: i64 = row.get(11)?;
                Ok(GeminiCallRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    project_name: row.get(2)?,
                    stage: row.get(3)?,
                    model: row.get(4)?,
                    prompt_version: row.get(5)?,
                    input_tokens: inp.clamp(0, i64::from(u32::MAX)) as u32,
                    output_tokens: out.clamp(0, i64::from(u32::MAX)) as u32,
                    cached_tokens: cached.clamp(0, i64::from(u32::MAX)) as u32,
                    estimated_cost_usd: row.get(9)?,
                    cache_hit: hit_int != 0,
                    duration_ms: dur.clamp(0, i64::from(u32::MAX)) as u32,
                    created_at: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok((by_stage, by_project, recent_calls))
    })?;

    Ok(CostsDashboardData {
        gauge,
        by_stage,
        by_project,
        recent_calls,
        usd_to_eur,
    })
}
