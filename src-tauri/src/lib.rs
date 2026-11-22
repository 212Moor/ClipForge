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
pub mod util;

use crate::config::release_config;
use crate::ipc::{apply_native_vibrancy, ProgressEvent, ThemeMode};
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

    // Les commandes doivent être désignées par leur chemin de module (ipc::nom).
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
        ])
        .events(collect_events![ProgressEvent]);

    // Régénère src/bindings.ts à chaque lancement en debug ; à committer ensuite.
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
