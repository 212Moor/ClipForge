use crate::error::{AppError, AppResult};
use std::fs;
use std::path::PathBuf;

/// Arborescence disque standardisée sous `%LOCALAPPDATA%/ClipForge/`.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub cache: PathBuf,
    pub projects: PathBuf,
    pub models: PathBuf,
    pub logs: PathBuf,
    pub database: PathBuf,
    pub pricing_file: PathBuf,
}

impl AppPaths {
    pub fn resolve_and_create() -> AppResult<Self> {
        let root = resolve_local_app_data()?.join("ClipForge");

        let paths = Self {
            bin: root.join("bin"),
            cache: root.join("cache"),
            projects: root.join("projects"),
            models: root.join("models"),
            logs: root.join("logs"),
            database: root.join("clipforge.db"),
            pricing_file: root.join("pricing.json"),
            root,
        };

        for dir in [
            &paths.root,
            &paths.bin,
            &paths.cache,
            &paths.projects,
            &paths.models,
            &paths.logs,
        ] {
            fs::create_dir_all(dir)?;
        }

        Ok(paths)
    }
}

fn resolve_local_app_data() -> AppResult<PathBuf> {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        if !local_app_data.is_empty() {
            return Ok(PathBuf::from(local_app_data));
        }
    }

    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .ok_or_else(|| {
            AppError::Config("Impossible de résoudre le dossier %LOCALAPPDATA%".to_string())
        })
}
