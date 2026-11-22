pub mod download;
pub mod gpu;
pub mod manifest;
pub mod verify;

use crate::error::{AppError, AppResult};
use crate::ipc::emit_progress;
use crate::paths::AppPaths;
use crate::sidecars::gpu::{detect_gpu_capabilities, recommend_variant, GpuCapabilities, WhisperVariant};
use crate::sidecars::manifest::{
    resolve_manifest, ArchiveKind, ManifestSource, SidecarId, SidecarManifest, ToolManifestEntry,
};
use crate::sidecars::verify::{
    compute_file_sha256, is_valid_sha256_hex, verify_managed_file_cached,
    verify_managed_file_or_delete,
};
use crate::store::Database;
use crate::util::blocking;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const SETTING_INSTALLED_SIDECARS: &str = "sidecars.installed";
const SETTING_WHISPER_VARIANT: &str = "sidecars.whisper_variant";

/// Garde-fous contre les archives piégées et les imports de dossiers volumineux.
const MAX_UNZIPPED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_IMPORT_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn install_lock() -> &'static Mutex<()> {
    INSTALL_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InstalledVersion {
    pub version: String,
    pub variant: String,
    #[serde(default)]
    pub install_dir: String,
    pub executable_path: String,
    pub executable_sha256: String,
    pub self_test_args: Vec<String>,
    pub installed_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct InstalledToolSlot {
    #[serde(default)]
    active: Option<InstalledVersion>,
    #[serde(default)]
    previous: Option<InstalledVersion>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct InstalledRegistry {
    #[serde(default)]
    tools: HashMap<SidecarId, InstalledToolSlot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SidecarStatusItem {
    pub id: SidecarId,
    pub display_name: String,
    pub target_version: String,
    pub target_variant: String,
    pub manifest_ready: bool,
    pub installed: Option<InstalledVersion>,
    pub previous: Option<InstalledVersion>,
    pub is_installed_and_present: bool,
    pub needs_update: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SidecarsOverview {
    pub gpu: GpuCapabilities,
    pub recommended_whisper_variant: WhisperVariant,
    pub active_whisper_variant: WhisperVariant,
    pub available_whisper_variants: Vec<WhisperVariant>,
    pub manifest_source: ManifestSource,
    pub manifest_warning: Option<String>,
    pub manifest_published_at: String,
    pub all_ready: bool,
    pub items: Vec<SidecarStatusItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SidecarOperationReport {
    pub id: SidecarId,
    pub success: bool,
    pub rolled_back: bool,
    pub active_version: Option<String>,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Registre des versions installées (stocké dans la table `settings`)
// ---------------------------------------------------------------------------

fn load_registry(db: &Database) -> AppResult<InstalledRegistry> {
    match db.get_setting(SETTING_INSTALLED_SIDECARS)? {
        Some(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        None => Ok(InstalledRegistry::default()),
    }
}

fn save_registry(db: &Database, registry: &InstalledRegistry) -> AppResult<()> {
    db.set_setting(SETTING_INSTALLED_SIDECARS, &serde_json::to_string(registry)?)
}

fn load_whisper_pref(db: &Database) -> Option<WhisperVariant> {
    db.get_setting(SETTING_WHISPER_VARIANT)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub fn save_whisper_variant(db: &Database, variant: WhisperVariant) -> AppResult<()> {
    db.set_setting(SETTING_WHISPER_VARIANT, &serde_json::to_string(&variant)?)
}

fn job_id_for(id: SidecarId) -> String {
    format!("sidecar-{}", id.dir_name())
}

async fn detect_gpu() -> AppResult<GpuCapabilities> {
    blocking(|| Ok(detect_gpu_capabilities())).await
}

/// Variante whisper.cpp effective : préférence enregistrée si elle est proposée, sinon recommandation matérielle.
fn effective_variant(
    pref: Option<WhisperVariant>,
    gpu: &GpuCapabilities,
    manifest: &SidecarManifest,
) -> (WhisperVariant, WhisperVariant) {
    let available = manifest.available_whisper_variants();
    let recommended = recommend_variant(gpu, &available);
    let active = pref.filter(|v| available.contains(v)).unwrap_or(recommended);
    (active, recommended)
}

// ---------------------------------------------------------------------------
// Vue d'ensemble pour l'UI
// ---------------------------------------------------------------------------

pub async fn get_overview(db: &Database, try_remote_manifest: bool) -> AppResult<SidecarsOverview> {
    let gpu = detect_gpu().await?;
    let resolved = resolve_manifest(try_remote_manifest).await?;

    let db_clone = db.clone();
    let (registry, pref) =
        blocking(move || Ok((load_registry(&db_clone)?, load_whisper_pref(&db_clone)))).await?;

    let (active_variant, recommended) = effective_variant(pref, &gpu, &resolved.manifest);
    let available = resolved.manifest.available_whisper_variants();

    let mut items = Vec::with_capacity(SidecarId::ALL.len());
    let mut all_ready = true;

    for id in SidecarId::ALL {
        let entry = resolved.manifest.find_entry(id, active_variant);
        let slot = registry.tools.get(&id).cloned().unwrap_or_default();

        let is_present = slot
            .active
            .as_ref()
            .map(|inst| Path::new(&inst.executable_path).is_file())
            .unwrap_or(false);
        if !is_present {
            all_ready = false;
        }

        let manifest_ready = entry
            .map(|e| is_valid_sha256_hex(&e.download_sha256) && is_valid_sha256_hex(&e.executable_sha256))
            .unwrap_or(false);

        let (target_version, target_variant) = entry
            .map(|e| (e.version.clone(), e.variant.clone()))
            .unwrap_or_else(|| ("—".to_string(), "—".to_string()));

        let needs_update = match (&slot.active, entry) {
            (Some(inst), Some(e)) if manifest_ready => {
                inst.version != e.version
                    || !inst.variant.eq_ignore_ascii_case(&e.variant)
                    || !inst.executable_sha256.eq_ignore_ascii_case(&e.executable_sha256)
            }
            _ => false,
        };

        items.push(SidecarStatusItem {
            id,
            display_name: id.display_name().to_string(),
            target_version,
            target_variant,
            manifest_ready,
            installed: slot.active,
            previous: slot.previous,
            is_installed_and_present: is_present,
            needs_update,
        });
    }

    Ok(SidecarsOverview {
        gpu,
        recommended_whisper_variant: recommended,
        active_whisper_variant: active_variant,
        available_whisper_variants: available,
        manifest_source: resolved.source,
        manifest_warning: resolved.warning,
        manifest_published_at: resolved.manifest.published_at,
        all_ready,
        items,
    })
}

// ---------------------------------------------------------------------------
// Utilisation par les autres modules
// ---------------------------------------------------------------------------

/// Point d'entrée unique utilisé par tous les modules (ingest, asr, render…) avant de lancer un sidecar.
/// Revérifie le SHA-256 (avec cache sur taille + date de modification) ; en cas d'écart,
/// supprime le binaire et renvoie une erreur claire.
pub async fn prepare_verified_executable(
    db: &Database,
    paths: &AppPaths,
    id: SidecarId,
) -> AppResult<PathBuf> {
    let db_clone = db.clone();
    let registry = blocking(move || load_registry(&db_clone)).await?;

    let active = registry
        .tools
        .get(&id)
        .and_then(|slot| slot.active.clone())
        .ok_or_else(|| {
            AppError::Sidecar(format!(
                "{} n'est pas installé. Ouvrez l'écran Premier lancement pour l'installer.",
                id.display_name()
            ))
        })?;

    let exe_path = PathBuf::from(&active.executable_path);
    if !exe_path.starts_with(&paths.bin) {
        return Err(AppError::Security(format!(
            "Chemin de {} hors du dossier géré %LOCALAPPDATA%/ClipForge/bin",
            id.display_name()
        )));
    }
    if !exe_path.is_file() {
        return Err(AppError::Sidecar(format!(
            "Binaire {} introuvable ({}). Réinstallez-le depuis Premier lancement.",
            id.display_name(),
            exe_path.display()
        )));
    }

    let to_verify = exe_path.clone();
    let expected = active.executable_sha256.clone();
    blocking(move || verify_managed_file_cached(&to_verify, &expected).map(|_| ())).await?;

    Ok(exe_path)
}

pub async fn verify_and_self_test_installed(
    db: &Database,
    paths: &AppPaths,
    id: SidecarId,
) -> AppResult<SidecarOperationReport> {
    let exe_path = prepare_verified_executable(db, paths, id).await?;

    let db_clone = db.clone();
    let registry = blocking(move || load_registry(&db_clone)).await?;
    let active = registry
        .tools
        .get(&id)
        .and_then(|slot| slot.active.clone())
        .ok_or_else(|| AppError::Sidecar("Sidecar non enregistré".to_string()))?;

    run_self_test(&exe_path, &active.self_test_args).await?;

    Ok(SidecarOperationReport {
        id,
        success: true,
        rolled_back: false,
        active_version: Some(active.version),
        message: format!("{} vérifié (SHA-256 et auto-test OK).", id.display_name()),
    })
}

// ---------------------------------------------------------------------------
// Installation en ligne
// ---------------------------------------------------------------------------

async fn select_manifest_entry(
    db: &Database,
    manifest: &SidecarManifest,
    id: SidecarId,
    whisper_override: Option<WhisperVariant>,
) -> AppResult<ToolManifestEntry> {
    let gpu = detect_gpu().await?;
    let db_clone = db.clone();
    let pref = blocking(move || Ok(load_whisper_pref(&db_clone))).await?;

    if let Some(wanted) = whisper_override {
        if !manifest.available_whisper_variants().contains(&wanted) {
            return Err(AppError::Sidecar(format!(
                "La variante whisper.cpp « {} » n'est pas proposée par le manifeste.",
                wanted.as_str()
            )));
        }
        let db_save = db.clone();
        blocking(move || save_whisper_variant(&db_save, wanted)).await?;
    }

    let (active, _) = effective_variant(whisper_override.or(pref), &gpu, manifest);

    manifest.find_entry(id, active).cloned().ok_or_else(|| {
        AppError::Sidecar(format!(
            "Aucune entrée dans le manifeste pour {} (variante {}).",
            id.display_name(),
            active.as_str()
        ))
    })
}

fn require_synced(entry: &ToolManifestEntry) -> AppResult<()> {
    if is_valid_sha256_hex(&entry.download_sha256) && is_valid_sha256_hex(&entry.executable_sha256) {
        Ok(())
    } else {
        Err(AppError::Security(format!(
            "Empreintes SHA-256 non initialisées pour {}. Exécutez scripts/sync-sidecar-manifest.ps1, puis relancez l'application.",
            entry.id.display_name()
        )))
    }
}

/// Télécharge (reprenable), vérifie le SHA-256, extrait, revérifie l'exécutable, exécute l'auto-test
/// puis seulement alors remplace la version active. Une installation qui échoue ne touche jamais
/// la version en place.
pub async fn install_sidecar_online(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    id: SidecarId,
    whisper_variant_override: Option<WhisperVariant>,
) -> AppResult<SidecarOperationReport> {
    let _guard = install_lock().lock().await;

    let resolved = resolve_manifest(true).await?;
    let entry = select_manifest_entry(db, &resolved.manifest, id, whisper_variant_override).await?;
    require_synced(&entry)?;

    let filename = entry
        .url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("archive.bin");
    let download_dest = paths.cache.join("downloads").join(sanitize_path_segment(&format!(
        "{}-{}-{}",
        id.dir_name(),
        entry.version,
        filename
    )));

    download::download_resumable(
        app,
        &job_id_for(id),
        id.display_name(),
        &entry.url,
        &download_dest,
    )
    .await?;

    install_from_staged_artifact(app, db, paths, &entry, &download_dest, true).await
}

// ---------------------------------------------------------------------------
// Import hors ligne
// ---------------------------------------------------------------------------

/// Mode hors ligne : repère dans un dossier choisi par l'utilisateur les fichiers dont le SHA-256 correspond
/// au manifeste, puis applique exactement les mêmes contrôles que l'installation en ligne.
pub async fn import_sidecars_from_directory(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    source_dir: &Path,
) -> AppResult<Vec<SidecarOperationReport>> {
    let _guard = install_lock().lock().await;

    if !source_dir.is_dir() {
        return Err(AppError::Sidecar(
            "Le dossier sélectionné n'existe pas ou est inaccessible.".to_string(),
        ));
    }

    let resolved = resolve_manifest(false).await?;
    let gpu = detect_gpu().await?;
    let db_clone = db.clone();
    let pref = blocking(move || Ok(load_whisper_pref(&db_clone))).await?;
    let (active_variant, _) = effective_variant(pref, &gpu, &resolved.manifest);

    // Indexation SHA-256 des seuls .zip/.exe raisonnables (niveau 1 du dossier).
    let dir = source_dir.to_path_buf();
    let indexed: Vec<(PathBuf, String)> = blocking(move || {
        let mut results = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let is_candidate = path.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
                && fs::metadata(&path).map(|m| m.len() <= MAX_IMPORT_FILE_BYTES).unwrap_or(false);
            if is_candidate {
                let hash = compute_file_sha256(&path)?;
                results.push((path, hash));
            }
        }
        Ok(results)
    })
    .await?;

    let staging_dir = paths.cache.join("offline-import");
    tokio::fs::create_dir_all(&staging_dir).await?;

    let mut reports = Vec::new();

    for id in SidecarId::ALL {
        let Some(entry) = resolved.manifest.find_entry(id, active_variant) else {
            continue;
        };

        if let Err(err) = require_synced(entry) {
            reports.push(failed_report(id, err.to_string()));
            continue;
        }

        let matched = indexed.iter().find(|(_, hash)| {
            hash.eq_ignore_ascii_case(&entry.download_sha256)
                || hash.eq_ignore_ascii_case(&entry.executable_sha256)
        });

        let Some((src_file, matched_hash)) = matched else {
            reports.push(failed_report(
                id,
                format!(
                    "Aucun fichier correspondant au SHA-256 attendu pour {} dans ce dossier.",
                    id.display_name()
                ),
            ));
            continue;
        };

        let staged = staging_dir.join(
            src_file
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("imported.bin")),
        );
        tokio::fs::copy(src_file, &staged).await?;

        // Binaire déjà extrait fourni à la place de l'archive.
        let mut effective = entry.clone();
        if matched_hash.eq_ignore_ascii_case(&entry.executable_sha256)
            && !matched_hash.eq_ignore_ascii_case(&entry.download_sha256)
        {
            effective.archive_kind = ArchiveKind::Binary;
            effective.download_sha256 = entry.executable_sha256.clone();
        }

        match install_from_staged_artifact(app, db, paths, &effective, &staged, true).await {
            Ok(report) => reports.push(report),
            Err(err) => reports.push(failed_report(id, err.to_string())),
        }
    }

    Ok(reports)
}

fn failed_report(id: SidecarId, message: String) -> SidecarOperationReport {
    SidecarOperationReport {
        id,
        success: false,
        rolled_back: false,
        active_version: None,
        message,
    }
}

// ---------------------------------------------------------------------------
// Cœur de l'installation (partagé en ligne / hors ligne)
// ---------------------------------------------------------------------------

async fn install_from_staged_artifact(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    entry: &ToolManifestEntry,
    artifact_path: &Path,
    delete_artifact_after: bool,
) -> AppResult<SidecarOperationReport> {
    let job_id = job_id_for(entry.id);
    let label = entry.id.display_name();

    // 1. SHA-256 de l'archive ou du binaire téléchargé (supprimé s'il est invalide).
    emit_progress(app, "sidecars", &job_id, "verify_download", 5.0, format!("Vérification SHA-256 de {label}…"));
    {
        let path = artifact_path.to_path_buf();
        let expected = entry.download_sha256.clone();
        blocking(move || verify_managed_file_or_delete(&path, &expected).map(|_| ())).await?;
    }

    // 2. Extraction dans un dossier de préparation : la version active n'est jamais touchée avant la réussite.
    let folder = sanitize_path_segment(&format!("{}-{}", entry.version, entry.variant));
    let tool_dir = paths.bin.join(entry.id.dir_name());
    let final_dir = tool_dir.join(&folder);
    let staging_dir = tool_dir.join(format!(".staging-{folder}"));

    emit_progress(app, "sidecars", &job_id, "extract", 40.0, format!("Extraction de {label}…"));
    let staged_exe = {
        let artifact = artifact_path.to_path_buf();
        let staging = staging_dir.clone();
        let kind = entry.archive_kind;
        let rel = entry.executable_rel_path.clone();
        blocking(move || prepare_staging(&artifact, &staging, kind, &rel)).await
    };
    if delete_artifact_after {
        let _ = tokio::fs::remove_file(artifact_path).await;
    }
    let staged_exe = match staged_exe {
        Ok(path) => path,
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            return Err(err);
        }
    };

    // 3. SHA-256 de l'exécutable final, avant tout lancement.
    emit_progress(app, "sidecars", &job_id, "verify_exe", 70.0, format!("Vérification SHA-256 du binaire {label}…"));
    {
        let exe = staged_exe.clone();
        let expected = entry.executable_sha256.clone();
        if let Err(err) = blocking(move || verify_managed_file_or_delete(&exe, &expected).map(|_| ())).await {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            return Err(err);
        }
    }

    // 4. Auto-test. En cas d'échec, la version active (s'il y en a une) reste en place.
    emit_progress(app, "sidecars", &job_id, "self_test", 85.0, format!("Auto-test de {label}…"));
    if let Err(test_err) = run_self_test(&staged_exe, &entry.self_test_args).await {
        tracing::error!(tool = entry.id.dir_name(), version = %entry.version, error = %test_err, "Auto-test échoué");
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        return report_failed_install(db, entry, &test_err.to_string()).await;
    }

    // 5. Promotion atomique du dossier de préparation vers le dossier définitif.
    let rel_exe = staged_exe
        .strip_prefix(&staging_dir)
        .map_err(|_| AppError::Internal("Chemin d'exécutable hors du dossier de préparation".to_string()))?
        .to_path_buf();
    {
        let staging = staging_dir.clone();
        let target = final_dir.clone();
        blocking(move || promote_dir(&staging, &target)).await?;
    }
    let exe_path = final_dir.join(rel_exe);

    // 6. Enregistrement : l'ancienne version devient « précédente », l'avant-dernière est supprimée du disque.
    let installed = InstalledVersion {
        version: entry.version.clone(),
        variant: entry.variant.clone(),
        install_dir: final_dir.to_string_lossy().to_string(),
        executable_path: exe_path.to_string_lossy().to_string(),
        executable_sha256: entry.executable_sha256.to_ascii_lowercase(),
        self_test_args: entry.self_test_args.clone(),
        installed_at: Utc::now().to_rfc3339(),
    };

    let obsolete_dir: Option<PathBuf> = {
        let db_clone = db.clone();
        let installed_clone = installed.clone();
        let id = entry.id;
        blocking(move || {
            let mut registry = load_registry(&db_clone)?;
            let slot = registry.tools.entry(id).or_default();
            let mut obsolete = None;

            if let Some(old_active) = slot.active.take() {
                if old_active.install_dir != installed_clone.install_dir {
                    if let Some(old_previous) = slot.previous.take() {
                        if !old_previous.install_dir.is_empty()
                            && old_previous.install_dir != installed_clone.install_dir
                        {
                            obsolete = Some(PathBuf::from(old_previous.install_dir));
                        }
                    }
                    slot.previous = Some(old_active);
                }
            }
            if slot
                .previous
                .as_ref()
                .map(|p| p.install_dir == installed_clone.install_dir)
                .unwrap_or(false)
            {
                slot.previous = None;
            }
            slot.active = Some(installed_clone);

            save_registry(&db_clone, &registry)?;
            Ok(obsolete)
        })
        .await?
    };

    if let Some(dir) = obsolete_dir {
        if dir.starts_with(&paths.bin) {
            let _ = tokio::fs::remove_dir_all(dir).await;
        }
    }

    emit_progress(app, "sidecars", &job_id, "done", 100.0, format!("{label} {} prêt.", entry.version));

    Ok(SidecarOperationReport {
        id: entry.id,
        success: true,
        rolled_back: false,
        active_version: Some(entry.version.clone()),
        message: format!(
            "{label} {} ({}) installé et vérifié.",
            entry.version, entry.variant
        ),
    })
}

/// Auto-test échoué : si une version valide est déjà active, on la conserve et on le signale ;
/// sinon l'erreur remonte telle quelle.
async fn report_failed_install(
    db: &Database,
    entry: &ToolManifestEntry,
    reason: &str,
) -> AppResult<SidecarOperationReport> {
    let db_clone = db.clone();
    let registry = blocking(move || load_registry(&db_clone)).await?;

    if let Some(active) = registry.tools.get(&entry.id).and_then(|s| s.active.clone()) {
        if Path::new(&active.executable_path).is_file() {
            return Ok(SidecarOperationReport {
                id: entry.id,
                success: false,
                rolled_back: true,
                active_version: Some(active.version.clone()),
                message: format!(
                    "Auto-test échoué pour {} {} ({reason}). La version {} reste active.",
                    entry.id.display_name(),
                    entry.version,
                    active.version
                ),
            });
        }
    }

    Err(AppError::Sidecar(format!(
        "Auto-test échoué pour {} : {reason}",
        entry.id.display_name()
    )))
}

/// Un auto-test réussit si le programme se termine normalement (code 0), ou si, comme TwitchDownloaderCLI
/// (bibliothèque d'arguments CommandLineParser), il affiche son propre nom et sa version puis sort avec le code 1.
/// Un plantage au chargement (DLL manquante : code négatif, aucune sortie) reste un échec.
fn self_test_passed(success: bool, code: Option<i32>, output: &str, expected_name: &str) -> bool {
    success
        || (code == Some(1)
            && !expected_name.is_empty()
            && output
                .to_ascii_lowercase()
                .contains(&expected_name.to_ascii_lowercase()))
}

async fn run_self_test(exe_path: &Path, args: &[String]) -> AppResult<()> {
    let mut cmd = Command::new(exe_path);
    cmd.args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = timeout(Duration::from_secs(15), cmd.output())
        .await
        .map_err(|_| AppError::Sidecar("Délai dépassé (15 s) lors de l'auto-test".to_string()))?
        .map_err(|e| AppError::Sidecar(format!("Impossible de lancer l'exécutable : {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    let expected_name = exe_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();

    if self_test_passed(output.status.success(), output.status.code(), &combined, expected_name) {
        return Ok(());
    }

    let first_line = stderr
        .lines()
        .chain(stdout.lines())
        .find(|line| !line.trim().is_empty())
        .unwrap_or("aucune sortie");
    Err(AppError::Sidecar(format!(
        "code de sortie {:?} : {first_line}",
        output.status.code()
    )))
}

// ---------------------------------------------------------------------------
// Fichiers : extraction, recherche de l'exécutable, promotion
// ---------------------------------------------------------------------------

fn prepare_staging(
    artifact: &Path,
    staging: &Path,
    kind: ArchiveKind,
    rel_exe: &str,
) -> AppResult<PathBuf> {
    if staging.exists() {
        fs::remove_dir_all(staging)?;
    }
    fs::create_dir_all(staging)?;

    match kind {
        ArchiveKind::Binary => {
            let name = Path::new(rel_exe)
                .file_name()
                .ok_or_else(|| AppError::Sidecar("Nom d'exécutable invalide".to_string()))?;
            let target = staging.join(name);
            fs::copy(artifact, &target)?;
            Ok(target)
        }
        ArchiveKind::Zip => {
            extract_zip_safely(artifact, staging)?;
            find_extracted_executable(staging, rel_exe)
        }
    }
}

fn extract_zip_safely(zip_path: &Path, target_dir: &Path) -> AppResult<()> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Sidecar(format!("Archive ZIP invalide : {e}")))?;

    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Sidecar(format!("Lecture d'une entrée ZIP impossible : {e}")))?;

        total = total.saturating_add(entry.size());
        if total > MAX_UNZIPPED_BYTES {
            return Err(AppError::Security(
                "Archive ZIP rejetée : taille décompressée anormalement élevée.".to_string(),
            ));
        }

        let Some(rel_path) = entry.enclosed_name() else {
            return Err(AppError::Security(
                "Archive ZIP rejetée : chemin relatif dangereux détecté (Zip Slip).".to_string(),
            ));
        };

        let out_path = target_dir.join(rel_path);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            io::copy(&mut entry, &mut out_file)?;
        }
    }
    Ok(())
}

/// Cherche l'exécutable par chemin relatif exact, sinon par nom de fichier dans l'arborescence extraite
/// (cas courant des archives qui contiennent un dossier racine unique).
fn find_extracted_executable(root: &Path, expected_rel_path: &str) -> AppResult<PathBuf> {
    let direct = root.join(expected_rel_path);
    if direct.is_file() {
        return Ok(direct);
    }

    let wanted = Path::new(expected_rel_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(expected_rel_path)
        .to_ascii_lowercase();

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_ascii_lowercase() == wanted)
                .unwrap_or(false)
            {
                return Ok(path);
            }
        }
    }

    Err(AppError::Sidecar(format!(
        "Exécutable {expected_rel_path} introuvable après extraction."
    )))
}

fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut last_err = None;
    for attempt in 0..5 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if attempt < 4 {
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("renommage impossible")))
}

/// Remplace `final_dir` par `staging` en conservant une sauvegarde le temps du remplacement.
fn promote_dir(staging: &Path, final_dir: &Path) -> AppResult<()> {
    let had_final = final_dir.exists();
    let backup = final_dir.with_file_name(format!(
        ".old-{}",
        final_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "tool".to_string())
    ));

    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    if had_final {
        rename_with_retry(final_dir, &backup)?;
    }
    if let Err(err) = rename_with_retry(staging, final_dir) {
        if had_final {
            let _ = fs::rename(&backup, final_dir);
        }
        return Err(err.into());
    }
    if had_final {
        let _ = fs::remove_dir_all(&backup);
    }
    Ok(())
}

fn sanitize_path_segment(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clipforge-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn build_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn self_test_accepts_exit_code_one_only_with_tool_banner() {
        assert!(self_test_passed(true, Some(0), "", "ffmpeg"));
        assert!(self_test_passed(false, Some(1), "TwitchDownloaderCLI 1.56.5+abc", "TwitchDownloaderCLI"));
        assert!(!self_test_passed(false, Some(1), "erreur inconnue", "TwitchDownloaderCLI"));
        assert!(!self_test_passed(false, Some(-1073741515), "TwitchDownloaderCLI", "TwitchDownloaderCLI"));
        assert!(!self_test_passed(false, Some(1), "TwitchDownloaderCLI", ""));
    }

    #[test]
    fn sanitizes_path_segments() {
        assert_eq!(sanitize_path_segment("v1.2/../x y"), "v1.2_.._x_y");
    }

    #[test]
    fn extracts_zip_and_finds_nested_executable() {
        let dir = temp_dir();
        let zip_path = dir.join("tool.zip");
        build_zip(&zip_path, &[("pkg-1.0/bin/tool.exe", b"binary"), ("pkg-1.0/readme.txt", b"hi")]);

        let staging = dir.join("staging");
        let exe = prepare_staging(&zip_path, &staging, ArchiveKind::Zip, "tool.exe").unwrap();
        assert!(exe.is_file());
        assert!(exe.starts_with(&staging));
    }

    #[test]
    fn rejects_zip_slip() {
        let dir = temp_dir();
        let zip_path = dir.join("evil.zip");
        build_zip(&zip_path, &[("../evil.exe", b"x")]);
        let err = extract_zip_safely(&zip_path, &dir.join("out")).unwrap_err();
        assert!(matches!(err, AppError::Security(_)));
    }

    #[test]
    fn promote_replaces_existing_directory() {
        let dir = temp_dir();
        let staging = dir.join("staging");
        let target = dir.join("final");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(staging.join("new.txt"), b"new").unwrap();
        fs::write(target.join("old.txt"), b"old").unwrap();

        promote_dir(&staging, &target).unwrap();
        assert!(target.join("new.txt").exists());
        assert!(!target.join("old.txt").exists());
        assert!(!staging.exists());
    }
}
