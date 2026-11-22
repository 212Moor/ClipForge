use crate::config::release_config;
use crate::error::IpcError;
use crate::gemini::client::{
    self as gemini_client, GeminiKeyTestResult, GeminiModelInfo, GeminiModelSelection,
};
use crate::gemini::keyring_store::{self as gemini_keyring, GeminiKeyStatus};
use crate::sidecars::{
    self, gpu::WhisperVariant, manifest::SidecarId, SidecarOperationReport, SidecarsOverview,
};
use crate::state::AppState;
use crate::store::migrations::{list_applied_migrations, AppliedMigration};
use crate::util::blocking;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

// ---------------------------------------------------------------------------
// Événements de progression (poussés vers l'UI, sans polling)
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

/// Progression exprimée en pourcentage (étapes d'un traitement).
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

/// Progression exprimée en octets (téléchargements).
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
    tracing::info!(?theme, material = %material, "Matériau de fenêtre appliqué");
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

/// Applique Mica (Windows 11), sinon Acrylic (Windows 10), sinon un fond uni.
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
// Gemini
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

/// Sans `api_key_override`, teste la clé déjà enregistrée dans le Gestionnaire d'identifiants Windows.
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
