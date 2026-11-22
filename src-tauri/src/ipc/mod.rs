use crate::asr::{
    available_whisper_models, ensure_whisper_model_downloaded, load_selected_whisper_model,
    save_selected_whisper_model, WhisperModelSpec,
};
use crate::config::release_config;
use crate::error::IpcError;
use crate::gemini::budget::{
    estimate_project_cost, load_pricing_config, save_pricing_config, PricingConfig,
    ProjectCostEstimate,
};
use crate::gemini::client::{
    self as gemini_client, GeminiKeyTestResult, GeminiModelInfo, GeminiModelSelection,
};
use crate::gemini::keyring_store::{self as gemini_keyring, GeminiKeyStatus};
use crate::gemini::ratelimit::{load_rate_limit_config, save_rate_limit_config, RateLimitConfig};
use crate::ingest::{
    create_demo_project, create_project_in_db, probe_vod_metadata, CreateProjectInput,
    VodProbeMetadata,
};
use crate::jobs::{get_pipeline_status, run_analysis_pipeline, PipelineStatusSnapshot};
use crate::sidecars::{
    self, gpu::WhisperVariant, manifest::SidecarId, SidecarOperationReport, SidecarsOverview,
};
use crate::signals::{load_signal_config, save_signal_config, SignalWeightsConfig};
use crate::state::AppState;
use crate::store::migrations::{list_applied_migrations, AppliedMigration};
use crate::asr::WindowTranscript;
use crate::gemini::client::StageBInputItem;
use crate::render::{
    self,
    ass::{builtin_subtitle_presets, SubtitleStylePreset},
    DetectedEncoders, RenderSettings,
};
use crate::signals::CompactChatSummary;
use crate::store::projects::{
    delete_project, delete_streamer_profile, get_clip, get_costs_dashboard, get_project_summary,
    get_project_timeline_data, get_streamer_profile, get_token_usage_gauge, list_candidates,
    list_clips, list_projects, list_streamer_profiles, promote_candidates_to_clips,
    save_streamer_facecam_rect, save_streamer_profile, update_clip_edits,
    update_clip_review_status, update_project_budget, AnalysisMode, ClipRecord, CostsDashboardData,
    FacecamRect, ProjectSummary, ProjectTimelineData, SaveStreamerProfileInput,
    StreamerProfileRecord, TokenUsageGauge, UpdateClipInput,
};
use crate::util::blocking;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

// ---------------------------------------------------------------------------
// Événements poussés vers l'UI (zéro polling)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ProgressEvent {
    pub job_id: String,
    pub scope: String,
    pub step: String,
    pub current: f64,
    pub total: f64,
    pub percent: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct PipelineLogEvent {
    pub project_id: String,
    pub level: String,
    pub timestamp: String,
    pub message: String,
}

pub fn emit_progress(
    app: &AppHandle,
    scope: &str,
    job_id: &str,
    step: &str,
    percent: f64,
    message: impl Into<String>,
) {
    let _ = ProgressEvent {
        job_id: job_id.to_string(),
        scope: scope.to_string(),
        step: step.to_string(),
        current: percent,
        total: 100.0,
        percent,
        message: message.into(),
    }
    .emit(app);
}

pub fn emit_bytes_progress(
    app: &AppHandle,
    scope: &str,
    job_id: &str,
    step: &str,
    current: u64,
    total: u64,
    message: impl Into<String>,
) {
    let percent = if total > 0 {
        ((current as f64 / total as f64) * 100.0).min(100.0)
    } else {
        0.0
    };
    let _ = ProgressEvent {
        job_id: job_id.to_string(),
        scope: scope.to_string(),
        step: step.to_string(),
        current: current as f64,
        total: total as f64,
        percent,
        message: message.into(),
    }
    .emit(app);
}

pub fn emit_pipeline_log(
    app: &AppHandle,
    project_id: &str,
    level: &str,
    message: impl Into<String>,
) {
    let _ = PipelineLogEvent {
        project_id: project_id.to_string(),
        level: level.to_string(),
        timestamp: Utc::now().format("%H:%M:%S").to_string(),
        message: message.into(),
    }
    .emit(app);
}

// ---------------------------------------------------------------------------
// Application : démarrage, thème, réglages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppBootstrapInfo {
    pub app_version: String,
    pub github_repo: String,
    pub updater_configured: bool,
    pub data_root: String,
    pub logs_dir: String,
    pub db_path: String,
    pub applied_migrations: Vec<AppliedMigration>,
    pub platform_vibrancy: String,
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_bootstrap(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppBootstrapInfo, IpcError> {
    let db = state.db.clone();
    let applied_migrations = blocking(move || db.with_conn(list_applied_migrations)).await?;

    let cfg = release_config();
    let platform_vibrancy = state
        .vibrancy
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| "unknown".to_string());

    Ok(AppBootstrapInfo {
        app_version: app.package_info().version.to_string(),
        github_repo: cfg.github_repo.clone(),
        updater_configured: cfg.updater_configured,
        data_root: state.paths.root.to_string_lossy().to_string(),
        logs_dir: state.paths.logs.to_string_lossy().to_string(),
        db_path: state.paths.database.to_string_lossy().to_string(),
        applied_migrations,
        platform_vibrancy,
    })
}

#[tauri::command]
#[specta::specta]
pub fn apply_window_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    theme: ThemeMode,
) -> Result<String, IpcError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| IpcError::new("WINDOW_NOT_FOUND", "Fenêtre principale introuvable"))?;

    let material = apply_native_vibrancy(&window, theme);
    if let Ok(mut guard) = state.vibrancy.lock() {
        *guard = material.clone();
    }
    Ok(material)
}

#[tauri::command]
#[specta::specta]
pub async fn get_setting(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || db.get_setting(&key)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_setting(
    state: State<'_, AppState>,
    key: String,
    value_json: String,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || db.set_setting(&key, &value_json)).await?)
}

#[cfg(target_os = "windows")]
pub fn apply_native_vibrancy(window: &tauri::WebviewWindow, theme: ThemeMode) -> String {
    let dark = match theme {
        ThemeMode::Dark => Some(true),
        ThemeMode::Light => Some(false),
        ThemeMode::System => None,
    };

    if window_vibrancy::apply_mica(window, dark).is_ok() {
        return "mica".to_string();
    }

    let tint = match theme {
        ThemeMode::Light => Some((244, 244, 248, 210)),
        ThemeMode::Dark | ThemeMode::System => Some((14, 15, 22, 215)),
    };
    if window_vibrancy::apply_acrylic(window, tint).is_ok() {
        return "acrylic".to_string();
    }

    "solid".to_string()
}

#[cfg(not(target_os = "windows"))]
pub fn apply_native_vibrancy(_window: &tauri::WebviewWindow, _theme: ThemeMode) -> String {
    "solid".to_string()
}

// ---------------------------------------------------------------------------
// Sidecars
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_sidecars_overview(
    state: State<'_, AppState>,
    try_remote_manifest: bool,
) -> Result<SidecarsOverview, IpcError> {
    Ok(sidecars::get_overview(&state.db, try_remote_manifest).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_whisper_variant(
    state: State<'_, AppState>,
    variant: WhisperVariant,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || sidecars::save_whisper_variant(&db, variant)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn install_sidecar(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SidecarId,
    whisper_variant: Option<WhisperVariant>,
) -> Result<SidecarOperationReport, IpcError> {
    Ok(
        sidecars::install_sidecar_online(&app, &state.db, &state.paths, id, whisper_variant)
            .await?,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn test_sidecar(
    state: State<'_, AppState>,
    id: SidecarId,
) -> Result<SidecarOperationReport, IpcError> {
    Ok(sidecars::verify_and_self_test_installed(&state.db, &state.paths, id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn import_sidecars_offline(
    app: AppHandle,
    state: State<'_, AppState>,
    directory_path: String,
) -> Result<Vec<SidecarOperationReport>, IpcError> {
    Ok(sidecars::import_sidecars_from_directory(
        &app,
        &state.db,
        &state.paths,
        &PathBuf::from(directory_path),
    )
    .await?)
}

// ---------------------------------------------------------------------------
// Gemini & Tarifs
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_gemini_key_status() -> Result<GeminiKeyStatus, IpcError> {
    Ok(blocking(gemini_keyring::get_key_status).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn save_gemini_api_key(api_key: String) -> Result<GeminiKeyStatus, IpcError> {
    Ok(blocking(move || gemini_keyring::save_api_key(&api_key)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_gemini_api_key() -> Result<(), IpcError> {
    Ok(blocking(gemini_keyring::delete_api_key).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn test_gemini_api_key(
    state: State<'_, AppState>,
    api_key_override: Option<String>,
) -> Result<GeminiKeyTestResult, IpcError> {
    let key = match api_key_override.filter(|k| !k.trim().is_empty()) {
        Some(key) => key,
        None => stored_api_key().await?,
    };
    Ok(gemini_client::test_api_key(&state.db, &key).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_gemini_models() -> Result<Vec<GeminiModelInfo>, IpcError> {
    let key = stored_api_key().await?;
    let models = gemini_client::list_models(&key).await?;
    Ok(models
        .into_iter()
        .filter(|m| m.supports_generate_content)
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn get_gemini_model_selection(
    state: State<'_, AppState>,
) -> Result<GeminiModelSelection, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || gemini_client::load_model_selection(&db)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_gemini_model_selection(
    state: State<'_, AppState>,
    selection: GeminiModelSelection,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || gemini_client::save_model_selection(&db, &selection)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_pricing_config(state: State<'_, AppState>) -> Result<PricingConfig, IpcError> {
    let paths = state.paths.clone();
    Ok(blocking(move || Ok(load_pricing_config(&paths))).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_pricing_config(
    state: State<'_, AppState>,
    pricing: PricingConfig,
) -> Result<(), IpcError> {
    let paths = state.paths.clone();
    Ok(blocking(move || save_pricing_config(&paths, &pricing)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_rate_limit_config(state: State<'_, AppState>) -> Result<RateLimitConfig, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || Ok(load_rate_limit_config(&db))).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_rate_limit_config(
    state: State<'_, AppState>,
    config: RateLimitConfig,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    let cfg_clone = config.clone();
    blocking(move || save_rate_limit_config(&db, &cfg_clone)).await?;
    state.gemini_limiter.update_config(config).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Partie 2 : Profils streamer, Projets, Ingestion, Signaux, Whisper, Pipeline & Coûts
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn list_streamer_profiles_cmd(
    state: State<'_, AppState>,
) -> Result<Vec<StreamerProfileRecord>, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || list_streamer_profiles(&db)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn save_streamer_profile_cmd(
    state: State<'_, AppState>,
    input: SaveStreamerProfileInput,
) -> Result<StreamerProfileRecord, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || save_streamer_profile(&db, &input)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_streamer_profile_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || delete_streamer_profile(&db, &id)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_signal_weights(state: State<'_, AppState>) -> Result<SignalWeightsConfig, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || Ok(load_signal_config(&db))).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_signal_weights(
    state: State<'_, AppState>,
    config: SignalWeightsConfig,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || save_signal_config(&db, &config)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_whisper_models(state: State<'_, AppState>) -> Result<Vec<WhisperModelSpec>, IpcError> {
    let paths = state.paths.clone();
    Ok(blocking(move || Ok(available_whisper_models(&paths))).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_selected_whisper_model(state: State<'_, AppState>) -> Result<String, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || Ok(load_selected_whisper_model(&db))).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_selected_whisper_model(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || save_selected_whisper_model(&db, &model_id)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn download_whisper_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<String, IpcError> {
    let path = ensure_whisper_model_downloaded(&app, &state.paths, &model_id).await?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn probe_vod(
    state: State<'_, AppState>,
    url: String,
) -> Result<VodProbeMetadata, IpcError> {
    Ok(probe_vod_metadata(&state.db, &state.paths, &url, None).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn estimate_cost_before_launch(
    state: State<'_, AppState>,
    vod_duration_sec: f64,
    target_clips: u32,
    mode: AnalysisMode,
) -> Result<ProjectCostEstimate, IpcError> {
    let db = state.db.clone();
    let paths = state.paths.clone();
    Ok(blocking(move || {
        let pricing = load_pricing_config(&paths);
        let sel = gemini_client::load_model_selection(&db).unwrap_or_default();
        Ok(estimate_project_cost(
            &pricing,
            vod_duration_sec,
            target_clips,
            mode,
            sel.stage_a_model.as_deref(),
            sel.stage_b_model.as_deref(),
        ))
    })
    .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn create_project(
    state: State<'_, AppState>,
    input: CreateProjectInput,
) -> Result<String, IpcError> {
    Ok(create_project_in_db(&state.db, &state.paths, input).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn create_demo_project_cmd(state: State<'_, AppState>) -> Result<String, IpcError> {
    Ok(create_demo_project(&state.db, &state.paths).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_projects_cmd(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || list_projects(&db)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_project_cmd(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<(), IpcError> {
    state.request_cancel(&project_id);
    let db = state.db.clone();
    let workspace = blocking(move || delete_project(&db, &project_id)).await?;
    if let Some(dir) = workspace {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_project_budget_limit(
    state: State<'_, AppState>,
    project_id: String,
    new_limit: u32,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || update_project_budget(&db, &project_id, new_limit)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn start_project_analysis(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    local_only: bool,
) -> Result<(), IpcError> {
    let cancel_flag = state.register_cancel_flag(&project_id);
    let db = state.db.clone();
    let paths = state.paths.clone();
    let limiter = Arc::clone(&state.gemini_limiter);
    let app_clone = app.clone();
    let pid_clone = project_id.clone();

    tokio::spawn(async move {
        let _ = run_analysis_pipeline(
            app_clone.clone(),
            db,
            paths,
            limiter,
            pid_clone.clone(),
            local_only,
            cancel_flag,
        )
        .await;
        if let Some(st) = app_clone.try_state::<AppState>() {
            st.clear_cancel_flag(&pid_clone);
        }
    });

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn pause_project_analysis(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<bool, IpcError> {
    Ok(state.request_cancel(&project_id))
}

#[tauri::command]
#[specta::specta]
pub async fn get_project_pipeline_status(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<PipelineStatusSnapshot, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || get_pipeline_status(&db, &project_id)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_project_timeline(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectTimelineData, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || get_project_timeline_data(&db, &project_id)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_token_gauge(state: State<'_, AppState>) -> Result<TokenUsageGauge, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || get_token_usage_gauge(&db)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_costs_dashboard_cmd(
    state: State<'_, AppState>,
) -> Result<CostsDashboardData, IpcError> {
    let db = state.db.clone();
    let paths = state.paths.clone();
    Ok(blocking(move || {
        let pricing = load_pricing_config(&paths);
        get_costs_dashboard(&db, pricing.usd_to_eur)
    })
    .await?)
}


// ---------------------------------------------------------------------------
// Partie 3 : Revue des clips, édition, régénération ciblée & Moteur de rendu FFmpeg
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn list_project_clips(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<ClipRecord>, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || list_clips(&db, &project_id)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn promote_project_candidates(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<ClipRecord>, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || promote_candidates_to_clips(&db, &project_id)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_clip_review_status(
    state: State<'_, AppState>,
    clip_id: String,
    review_status: String,
) -> Result<ClipRecord, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || update_clip_review_status(&db, &clip_id, &review_status)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn save_clip_edits(
    state: State<'_, AppState>,
    input: UpdateClipInput,
) -> Result<ClipRecord, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || update_clip_edits(&db, &input)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn save_profile_facecam(
    state: State<'_, AppState>,
    profile_id: String,
    rect: FacecamRect,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || save_streamer_facecam_rect(&db, &profile_id, rect)).await?)
}

/// Régénération ciblée Gemini sur UN SEUL clip (accroche, titre, description, hashtags).
#[tauri::command]
#[specta::specta]
pub async fn regenerate_clip_title(
    state: State<'_, AppState>,
    clip_id: String,
) -> Result<ClipRecord, IpcError> {
    let api_key = stored_api_key().await?;
    let db = state.db.clone();
    let paths = state.paths.clone();
    let cid = clip_id.clone();

    let (clip, project, profile, pricing, mut selection, candidate_opt) = blocking(move || {
        let clip = get_clip(&db, &cid)?;
        let project = get_project_summary(&db, &clip.project_id)?;
        let profile = match project.streamer_profile_id.as_deref() {
            Some(pid) => get_streamer_profile(&db, pid)?,
            None => None,
        };
        let pricing = load_pricing_config(&paths);
        let selection = gemini_client::load_model_selection(&db).unwrap_or_default();
        let cands = list_candidates(&db, &clip.project_id)?;
        let cand = clip
            .candidate_id
            .as_deref()
            .and_then(|id| cands.into_iter().find(|c| c.id == id));
        Ok((clip, project, profile, pricing, selection, cand))
    })
    .await?;

    if selection.stage_b_model.is_none() {
        let live = gemini_client::list_models(&api_key).await?;
        let usable: Vec<String> = live
            .into_iter()
            .filter(|m| m.supports_generate_content)
            .map(|m| m.id)
            .collect();
        selection.stage_b_model = usable
            .iter()
            .find(|id| id.contains("flash") && !id.contains("lite"))
            .or_else(|| usable.iter().find(|id| id.contains("flash")))
            .or_else(|| usable.first())
            .cloned();
    }

    let model_id = selection
        .stage_b_model
        .or(selection.stage_a_model)
        .ok_or_else(|| IpcError::new("CONFIG_ERROR", "Aucun modèle Gemini disponible dans Paramètres."))?;

    let compact_text = clip
        .transcript_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<WindowTranscript>(s).ok())
        .map(|tr| tr.compact_text)
        .unwrap_or_else(|| clip.title.clone());

    let chat_sum: CompactChatSummary = candidate_opt
        .as_ref()
        .and_then(|c| c.chat_summary_json.as_deref())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(CompactChatSummary {
            r: 0.0,
            e: vec![],
            m: vec![],
        });

    let input_item = StageBInputItem {
        i: candidate_opt.as_ref().map(|c| c.rank_index).unwrap_or(1),
        d: (clip.end_sec - clip.start_sec).round().max(1.0) as u32,
        cat: clip.category.clone(),
        t: compact_text,
        c: chat_sum,
    };

    let regenerated = gemini_client::regenerate_single_clip_metadata(
        &state.db,
        &state.gemini_limiter,
        &pricing,
        &api_key,
        &project.id,
        &model_id,
        profile.as_ref(),
        &input_item,
    )
    .await?;

    let db_up = state.db.clone();
    let updated = blocking(move || {
        update_clip_edits(
            &db_up,
            &UpdateClipInput {
                id: clip.id,
                start_sec: clip.start_sec,
                end_sec: clip.end_sec,
                hook: regenerated.hook,
                title: regenerated.title,
                caption: regenerated.caption,
                hashtags: regenerated.tags,
                category: clip.category,
                layout_mode: clip.layout_mode,
                subtitle_style: clip.subtitle_style,
                subtitles_override_json: clip.subtitles_override_json,
            },
        )
    })
    .await?;

    Ok(updated)
}

#[tauri::command]
#[specta::specta]
pub fn list_subtitle_presets() -> Vec<SubtitleStylePreset> {
    builtin_subtitle_presets()
}

#[tauri::command]
#[specta::specta]
pub async fn detect_render_encoders(state: State<'_, AppState>) -> Result<DetectedEncoders, IpcError> {
    Ok(render::detect_hardware_encoders(&state.db, &state.paths).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_render_settings(state: State<'_, AppState>) -> Result<RenderSettings, IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || Ok(render::load_render_settings(&db))).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn set_render_settings(
    state: State<'_, AppState>,
    settings: RenderSettings,
) -> Result<(), IpcError> {
    let db = state.db.clone();
    Ok(blocking(move || render::save_render_settings(&db, &settings)).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn enqueue_clip_render(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: String,
) -> Result<(), IpcError> {
    let cancel_flag = state.register_render_cancel(&clip_id);
    let db = state.db.clone();
    let paths = state.paths.clone();
    let sem = Arc::clone(&state.render_semaphore);
    let app_clone = app.clone();
    let cid = clip_id.clone();

    tokio::spawn(async move {
        let _ = render::render_single_clip(
            app_clone.clone(),
            db,
            paths,
            sem,
            cid.clone(),
            cancel_flag,
        )
        .await;
        if let Some(st) = app_clone.try_state::<AppState>() {
            st.clear_render_cancel(&cid);
        }
    });

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn enqueue_project_approved_renders(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
) -> Result<u32, IpcError> {
    let db_list = state.db.clone();
    let pid = project_id.clone();
    let approved_ids: Vec<String> = blocking(move || {
        let clips = list_clips(&db_list, &pid)?;
        Ok(clips
            .into_iter()
            .filter(|c| c.review_status == "approved" && c.render_status != "rendering")
            .map(|c| c.id)
            .collect())
    })
    .await?;

    let count = approved_ids.len() as u32;
    for clip_id in approved_ids {
        let cancel_flag = state.register_render_cancel(&clip_id);
        let db = state.db.clone();
        let paths = state.paths.clone();
        let sem = Arc::clone(&state.render_semaphore);
        let app_clone = app.clone();
        let cid = clip_id.clone();

        tokio::spawn(async move {
            let _ = render::render_single_clip(
                app_clone.clone(),
                db,
                paths,
                sem,
                cid.clone(),
                cancel_flag,
            )
            .await;
            if let Some(st) = app_clone.try_state::<AppState>() {
                st.clear_render_cancel(&cid);
            }
        });
    }

    Ok(count)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_clip_render(
    state: State<'_, AppState>,
    clip_id: String,
) -> Result<bool, IpcError> {
    Ok(state.request_render_cancel(&clip_id))
}

#[tauri::command]
#[specta::specta]
pub async fn export_project_csv(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<String, IpcError> {
    let db = state.db.clone();
    let path = blocking(move || render::write_project_csv(&db, &project_id)).await?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
#[specta::specta]
pub fn reveal_in_explorer(path: String) -> Result<(), IpcError> {
    Ok(render::reveal_path_in_explorer(&path)?)
}

async fn stored_api_key() -> Result<String, IpcError> {
    blocking(gemini_keyring::load_api_key)
        .await?
        .ok_or_else(|| {
            IpcError::new(
                "KEY_MISSING",
                "Aucune clé API Gemini enregistrée dans le Gestionnaire d'identifiants Windows.",
            )
        })
}
