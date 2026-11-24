pub mod asr;
pub mod config;
pub mod error;
pub mod gemini;
pub mod ingest;
pub mod ipc;
pub mod jobs;
pub mod logging;
pub mod paths;
pub mod render;
pub mod scoring;
pub mod sidecars;
pub mod signals;
pub mod state;
pub mod store;
pub mod preview;
pub mod tiktok;
pub mod util;

use crate::config::release_config;
use crate::ipc::{apply_native_vibrancy, PipelineLogEvent, ProgressEvent, ThemeMode};
use crate::paths::AppPaths;
use crate::state::AppState;
use crate::store::Database;
#[cfg(debug_assertions)]
use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = AppPaths::resolve_and_create()
        .expect("Échec d'initialisation des dossiers %LOCALAPPDATA%/ClipForge");

    let log_guard = logging::init_logging(&paths.logs);
    tracing::info!(root = %paths.root.display(), "Démarrage de ClipForge");

    let db = Database::open(&paths.database)
        .expect("Échec d'ouverture ou de migration de la base SQLite ClipForge");

    let specta_builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            ipc::get_app_bootstrap,
            ipc::apply_window_theme,
            ipc::get_setting,
            ipc::set_setting,
            ipc::get_sidecars_overview,
            ipc::set_whisper_variant,
            ipc::install_sidecar,
            ipc::test_sidecar,
            ipc::import_sidecars_offline,
            ipc::get_gemini_key_status,
            ipc::save_gemini_api_key,
            ipc::delete_gemini_api_key,
            ipc::test_gemini_api_key,
            ipc::list_gemini_models,
            ipc::get_gemini_model_selection,
            ipc::set_gemini_model_selection,
            ipc::get_pricing_config,
            ipc::set_pricing_config,
            ipc::get_rate_limit_config,
            ipc::set_rate_limit_config,
            ipc::list_streamer_profiles_cmd,
            ipc::save_streamer_profile_cmd,
            ipc::delete_streamer_profile_cmd,
            ipc::get_signal_weights,
            ipc::set_signal_weights,
            ipc::list_whisper_models,
            ipc::get_selected_whisper_model,
            ipc::set_selected_whisper_model,
            ipc::download_whisper_model,
            ipc::probe_vod,
            ipc::estimate_cost_before_launch,
            ipc::create_project,
            ipc::create_demo_project_cmd,
            ipc::list_projects_cmd,
            ipc::delete_project_cmd,
            ipc::update_project_budget_limit,
            ipc::start_project_analysis,
            ipc::pause_project_analysis,
            ipc::get_project_pipeline_status,
            ipc::get_project_timeline,
            ipc::get_token_gauge,
            ipc::get_costs_dashboard_cmd,
            ipc::list_project_clips,
            ipc::promote_project_candidates,
            ipc::set_clip_review_status,
            ipc::save_clip_edits,
            ipc::save_profile_facecam,
            ipc::regenerate_clip_title,
            ipc::list_subtitle_presets,
            ipc::detect_render_encoders,
            ipc::get_render_settings,
            ipc::set_render_settings,
            ipc::enqueue_clip_render,
            ipc::enqueue_project_approved_renders,
            ipc::cancel_clip_render,
            ipc::export_project_csv,
            ipc::reveal_in_explorer,
            ipc::tiktok_get_status,
            ipc::tiktok_save_credentials,
            ipc::tiktok_connect_oauth,
            ipc::tiktok_disconnect,
            ipc::tiktok_upload_clip,
            ipc::tiktok_open_studio_window,
            ipc::tiktok_prepare_studio_upload,
            ipc::tiktok_suggest_copy,
            ipc::tiktok_auto_inject_upload,
            ipc::prepare_clip_preview,
            ipc::open_tiktok_publish,
        ])
        .events(collect_events![ProgressEvent, PipelineLogEvent]);

    #[cfg(debug_assertions)]
    specta_builder
        .export(Typescript::default(), "../src/bindings.ts")
        .expect("Échec d'exportation des bindings TypeScript");

    let mut tauri_builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());

    if release_config().updater_configured {
        tauri_builder = tauri_builder.plugin(tauri_plugin_updater::Builder::new().build());
    } else {
        tracing::info!(
            "Mises à jour automatiques désactivées : clé publique ou dépôt GitHub encore au placeholder dans tauri.conf.json"
        );
    }

    let app_state = AppState::new(paths, db, log_guard);

    tauri_builder
        .manage(app_state)
        .invoke_handler(specta_builder.invoke_handler())
        .setup(move |app| {
            specta_builder.mount_events(app);

            if let Some(window) = app.get_webview_window("main") {
                let material = apply_native_vibrancy(&window, ThemeMode::Dark);
                if let Ok(mut guard) = app.state::<AppState>().vibrancy.lock() {
                    *guard = material.clone();
                }
                tracing::info!(material = %material, "Effet de fenêtre natif initialisé");
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Erreur critique durant l'exécution de ClipForge");
}
