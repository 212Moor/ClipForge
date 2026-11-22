use crate::error::{AppError, AppResult};
use crate::ipc::emit_bytes_progress;
use futures_util::StreamExt;
use reqwest::header::RANGE;
use reqwest::StatusCode;
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Téléchargement HTTPS reprenable (fichier `.part` + en-tête `Range`).
/// Émet des événements de progression vers l'UI, sans polling.
pub async fn download_resumable(
    app: &AppHandle,
    job_id: &str,
    tool_label: &str,
    url: &str,
    dest_path: &Path,
) -> AppResult<()> {
    if !url.starts_with("https://") {
        return Err(AppError::Security(
            "Téléchargement refusé : l'URL n'est pas en HTTPS.".to_string(),
        ));
    }

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut part_name = dest_path
        .file_name()
        .ok_or_else(|| AppError::Internal("Nom de fichier de destination invalide".to_string()))?
        .to_os_string();
    part_name.push(".part");
    let part_path = dest_path.with_file_name(part_name);

    let existing_len = fs::metadata(&part_path).await.map(|m| m.len()).unwrap_or(0);

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent("ClipForge-Downloader/0.1")
        .build()
        .map_err(|e| AppError::Network(e.to_string()))?;

    let mut request = client.get(url);
    if existing_len > 0 {
        request = request.header(RANGE, format!("bytes={existing_len}-"));
    }

    let response = request
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Connexion échouée pour {tool_label} : {e}")))?;

    let (mut downloaded, total_bytes, append_mode) = match response.status() {
        StatusCode::PARTIAL_CONTENT => {
            let remaining = response.content_length().unwrap_or(0);
            (existing_len, existing_len + remaining, true)
        }
        StatusCode::OK => (0, response.content_length().unwrap_or(0), false),
        StatusCode::RANGE_NOT_SATISFIABLE if existing_len > 0 => {
            // Le .part est déjà complet : la vérification SHA-256 tranchera.
            if dest_path.exists() {
                let _ = fs::remove_file(dest_path).await;
            }
            fs::rename(&part_path, dest_path).await?;
            emit_bytes_progress(
                app,
                "sidecars",
                job_id,
                "download",
                existing_len,
                existing_len,
                format!("{tool_label} : reprise terminée"),
            );
            return Ok(());
        }
        other => {
            return Err(AppError::Network(format!(
                "Téléchargement refusé pour {tool_label} (HTTP {other})"
            )));
        }
    };

    let mut file = if append_mode {
        OpenOptions::new().create(true).append(true).open(&part_path).await?
    } else {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part_path)
            .await?
    };

    let mut stream = response.bytes_stream();
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    loop {
        let next = timeout(STALL_TIMEOUT, stream.next()).await.map_err(|_| {
            AppError::Network(format!(
                "Téléchargement de {tool_label} bloqué (aucune donnée depuis 30 s). Relancez pour reprendre."
            ))
        })?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| {
            AppError::Network(format!("Coupure durant le téléchargement de {tool_label} : {e}"))
        })?;

        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(80) {
            let size_label = if total_bytes > 0 {
                format!(
                    "{:.1} Mo / {:.1} Mo",
                    downloaded as f64 / 1_048_576.0,
                    total_bytes as f64 / 1_048_576.0
                )
            } else {
                format!("{:.1} Mo", downloaded as f64 / 1_048_576.0)
            };
            emit_bytes_progress(
                app,
                "sidecars",
                job_id,
                "download",
                downloaded,
                total_bytes,
                format!("Téléchargement de {tool_label} ({size_label})"),
            );
            last_emit = Instant::now();
        }
    }

    file.flush().await?;
    drop(file);

    if dest_path.exists() {
        let _ = fs::remove_file(dest_path).await;
    }
    fs::rename(&part_path, dest_path).await?;

    emit_bytes_progress(
        app,
        "sidecars",
        job_id,
        "download",
        downloaded,
        total_bytes.max(downloaded),
        format!("{tool_label} téléchargé"),
    );

    Ok(())
}
