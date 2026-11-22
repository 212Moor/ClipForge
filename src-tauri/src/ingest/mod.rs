use crate::error::{AppError, AppResult};
use crate::ipc::emit_progress;
use crate::paths::AppPaths;
use crate::sidecars::{manifest::SidecarId, prepare_verified_executable};
use crate::store::projects::{AnalysisMode, MutedSegment};
use crate::store::Database;
use crate::util::blocking;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::process::Command;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VodProbeMetadata {
    pub source_url: String,
    pub twitch_vod_id: Option<String>,
    pub channel_name: String,
    pub title: String,
    pub duration_sec: f64,
    pub thumbnail_url: Option<String>,
    pub muted_segments: Vec<MutedSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateProjectInput {
    pub name: Option<String>,
    pub source_url: String,
    pub streamer_profile_id: Option<String>,
    pub mode: AnalysisMode,
    pub target_clips: u32,
    pub target_min_sec: u32,
    pub target_max_sec: u32,
    pub token_budget_limit: u32,
    pub asr_language: Option<String>,
    pub subscriber_cookie: Option<String>,
    pub probe: Option<VodProbeMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestArtifacts {
    pub audio_wav_path: PathBuf,
    pub chat_json_path: PathBuf,
    pub muted_segments: Vec<MutedSegment>,
}

pub fn parse_twitch_vod_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) && trimmed.len() >= 6 {
        return Some(trimmed.to_string());
    }
    let lowered = trimmed.to_ascii_lowercase();
    if let Some(pos) = lowered.find("/videos/") {
        let rest = &trimmed[pos + "/videos/".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 6 {
            return Some(digits);
        }
    }
    None
}

pub fn normalize_vod_url(raw: &str) -> AppResult<(String, Option<String>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(
            "Veuillez coller une URL ou un ID de VOD Twitch.".to_string(),
        ));
    }
    if let Some(vod_id) = parse_twitch_vod_id(trimmed) {
        return Ok((format!("https://www.twitch.tv/videos/{vod_id}"), Some(vod_id)));
    }
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        return Ok((trimmed.to_string(), None));
    }
    Err(AppError::Validation(
        "Format d'URL Twitch non reconnu (attendu : https://www.twitch.tv/videos/<ID> ou un ID numérique)."
            .to_string(),
    ))
}

pub async fn probe_vod_metadata(
    db: &Database,
    paths: &AppPaths,
    raw_url: &str,
    cookie_path: Option<&Path>,
) -> AppResult<VodProbeMetadata> {
    let (url, twitch_vod_id) = normalize_vod_url(raw_url)?;
    let yt_dlp = prepare_verified_executable(db, paths, SidecarId::YtDlp).await?;

    let mut cmd = Command::new(&yt_dlp);
    cmd.args(["--dump-single-json", "--no-download", "--no-warnings"]);
    if let Some(cookies) = cookie_path {
        cmd.arg("--cookies").arg(cookies);
    }
    cmd.arg(&url);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::Sidecar(format!("Impossible de lancer yt-dlp : {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Network(format!(
            "yt-dlp n'a pas pu lire cette VOD : {}",
            stderr.lines().next().unwrap_or("erreur inconnue")
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let channel_name = json
        .get("uploader")
        .or_else(|| json.get("channel"))
        .and_then(|v| v.as_str())
        .unwrap_or("Chaîne Twitch")
        .to_string();
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("VOD Twitch")
        .to_string();
    let duration_sec = json.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let thumbnail_url = json
        .get("thumbnail")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);

    let mut muted_segments = Vec::new();
    if let Some(arr) = json.get("muted_segments").and_then(|v| v.as_array()) {
        for seg in arr {
            let start = seg.get("offset").or_else(|| seg.get("start")).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let dur = seg.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if dur > 0.0 {
                muted_segments.push(MutedSegment {
                    start_sec: start,
                    end_sec: start + dur,
                    reason: "Musique protégée (Twitch muted)".to_string(),
                });
            }
        }
    }

    Ok(VodProbeMetadata {
        source_url: url,
        twitch_vod_id: twitch_vod_id.or_else(|| {
            json.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim_start_matches('v').to_string())
        }),
        channel_name,
        title,
        duration_sec,
        thumbnail_url,
        muted_segments,
    })
}

pub async fn create_project_in_db(
    db: &Database,
    paths: &AppPaths,
    input: CreateProjectInput,
) -> AppResult<String> {
    let (url, parsed_id) = normalize_vod_url(&input.source_url)?;
    let project_id = uuid::Uuid::new_v4().to_string();
    let vod_id = uuid::Uuid::new_v4().to_string();
    let workspace_dir = paths.projects.join(&project_id);
    fs::create_dir_all(&workspace_dir)?;

    if let Some(cookie) = input.subscriber_cookie.as_deref() {
        let trimmed = cookie.trim();
        if !trimmed.is_empty() {
            fs::write(workspace_dir.join("cookies.txt"), trimmed)?;
        }
    }

    let probe = input.probe.clone();
    let title = probe
        .as_ref()
        .map(|p| p.title.clone())
        .unwrap_or_else(|| format!("VOD {}", parsed_id.as_deref().unwrap_or("Twitch")));
    let name = input
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| title.clone());
    let channel = probe.as_ref().map(|p| p.channel_name.clone());
    let duration_sec = probe.as_ref().map(|p| p.duration_sec).unwrap_or(0.0);
    let thumbnail_url = probe.as_ref().and_then(|p| p.thumbnail_url.clone());
    let twitch_vod_id = probe
        .as_ref()
        .and_then(|p| p.twitch_vod_id.clone())
        .or(parsed_id);
    let muted_json = serde_json::to_string(
        &probe
            .as_ref()
            .map(|p| p.muted_segments.clone())
            .unwrap_or_default(),
    )?;

    let metadata_json = serde_json::to_string(&serde_json::json!({
        "asr_language": input.asr_language.unwrap_or_else(|| "auto".to_string()),
    }))?;

    let now = Utc::now().to_rfc3339();
    let workspace_str = workspace_dir.to_string_lossy().to_string();
    let db_clone = db.clone();
    let project_id_ret = project_id.clone();

    blocking(move || {
        db_clone.with_conn_mut(|conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO projects
                 (id, name, streamer_profile_id, mode, target_clips, target_min_sec, target_max_sec,
                  token_budget_limit, status, workspace_dir, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'draft', ?9, ?10, ?10)",
                rusqlite::params![
                    project_id,
                    name,
                    input.streamer_profile_id,
                    input.mode.as_str(),
                    input.target_clips.clamp(1, 30),
                    input.target_min_sec.clamp(15, 60),
                    input.target_max_sec.clamp(25, 90),
                    input.token_budget_limit.max(5_000),
                    workspace_str,
                    now
                ],
            )?;
            tx.execute(
                "INSERT INTO vods
                 (id, project_id, source_url, twitch_vod_id, channel_name, title, duration_sec,
                  thumbnail_url, audio_path, chat_path, muted_segments_json, metadata_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10, ?11, ?11)",
                rusqlite::params![
                    vod_id,
                    project_id,
                    url,
                    twitch_vod_id,
                    channel,
                    title,
                    duration_sec,
                    thumbnail_url,
                    muted_json,
                    metadata_json,
                    now
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    })
    .await?;

    Ok(project_id_ret)
}

pub async fn download_audio_and_chat(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    project_id: &str,
    workspace_dir: &Path,
    source_url: &str,
    twitch_vod_id: Option<&str>,
    existing_muted: &[MutedSegment],
) -> AppResult<IngestArtifacts> {
    fs::create_dir_all(workspace_dir)?;
    let audio_wav_path = workspace_dir.join("audio_16k.wav");
    let raw_audio_path = workspace_dir.join("audio_raw.m4a");
    let chat_json_path = workspace_dir.join("chat.json");
    let cookie_file = workspace_dir.join("cookies.txt");
    let cookie_opt = cookie_file.is_file().then_some(cookie_file.as_path());

    if !audio_wav_path.is_file() {
        emit_progress(
            app,
            "pipeline",
            project_id,
            "ingest",
            10.0,
            "Téléchargement du flux audio seul (yt-dlp)…",
        );
        let yt_dlp = prepare_verified_executable(db, paths, SidecarId::YtDlp).await?;
        let ffmpeg = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await?;

        if !raw_audio_path.is_file() {
            let mut cmd = Command::new(&yt_dlp);
            cmd.args([
                "-f",
                "bestaudio/audio_only/worst",
                "--no-playlist",
                "--continue",
                "-o",
            ])
            .arg(&raw_audio_path);
            if let Some(cookies) = cookie_opt {
                cmd.arg("--cookies").arg(cookies);
            }
            cmd.arg(source_url);
            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);

            let out = cmd
                .output()
                .await
                .map_err(|e| AppError::Sidecar(format!("Échec lancement yt-dlp : {e}")))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                return Err(AppError::Network(format!(
                    "Échec téléchargement audio yt-dlp : {}",
                    err.lines().next().unwrap_or("erreur inconnue")
                )));
            }
        }

        emit_progress(
            app,
            "pipeline",
            project_id,
            "ingest",
            45.0,
            "Conversion audio 16 kHz mono pour analyse locale et Whisper…",
        );

        let mut ff = Command::new(&ffmpeg);
        ff.args(["-y", "-i"])
            .arg(&raw_audio_path)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
            .arg(&audio_wav_path);
        #[cfg(target_os = "windows")]
        ff.creation_flags(CREATE_NO_WINDOW);

        let out = ff
            .output()
            .await
            .map_err(|e| AppError::Sidecar(format!("Échec lancement FFmpeg : {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(AppError::Sidecar(format!(
                "Conversion audio FFmpeg échouée : {}",
                err.lines().last().unwrap_or("erreur inconnue")
            )));
        }
        let _ = fs::remove_file(&raw_audio_path);
    }

    if !chat_json_path.is_file() {
        if let Some(vod_id) = twitch_vod_id {
            emit_progress(
                app,
                "pipeline",
                project_id,
                "ingest",
                70.0,
                "Téléchargement du chat Twitch (TwitchDownloaderCLI)…",
            );
            let td = prepare_verified_executable(db, paths, SidecarId::TwitchDownloader).await?;
            let mut cmd = Command::new(&td);
            cmd.args(["chatdownload", "--id", vod_id, "-o"])
                .arg(&chat_json_path);
            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);

            let out = cmd.output().await.map_err(|e| {
                AppError::Sidecar(format!("Échec lancement TwitchDownloaderCLI : {e}"))
            })?;
            if !out.status.success() {
                tracing::warn!(
                    stderr = %String::from_utf8_lossy(&out.stderr),
                    "Chat Twitch indisponible — poursuite avec chat vide"
                );
                fs::write(&chat_json_path, r#"{"comments":[]}"#)?;
            }
        } else {
            fs::write(&chat_json_path, r#"{"comments":[]}"#)?;
        }
    }

    emit_progress(
        app,
        "pipeline",
        project_id,
        "ingest",
        90.0,
        "Vérification des segments mutés (musique protégée)…",
    );
    let mut muted_segments = existing_muted.to_vec();
    let wav_clone = audio_wav_path.clone();
    let detected_silences = blocking(move || detect_long_silences_wav(&wav_clone, 25.0)).await?;
    for seg in detected_silences {
        let already_covered = muted_segments
            .iter()
            .any(|m| m.start_sec <= seg.start_sec + 5.0 && m.end_sec >= seg.end_sec - 5.0);
        if !already_covered {
            muted_segments.push(seg);
        }
    }

    let audio_str = audio_wav_path.to_string_lossy().to_string();
    let chat_str = chat_json_path.to_string_lossy().to_string();
    let muted_json = serde_json::to_string(&muted_segments)?;
    let db_clone = db.clone();
    let pid = project_id.to_string();
    blocking(move || {
        db_clone.with_conn(|conn| {
            conn.execute(
                "UPDATE vods SET audio_path = ?1, chat_path = ?2, muted_segments_json = ?3, updated_at = ?4
                 WHERE project_id = ?5",
                rusqlite::params![audio_str, chat_str, muted_json, Utc::now().to_rfc3339(), pid],
            )?;
            Ok(())
        })
    })
    .await?;

    emit_progress(
        app,
        "pipeline",
        project_id,
        "ingest",
        100.0,
        "Ingestion audio + chat terminée.",
    );

    Ok(IngestArtifacts {
        audio_wav_path,
        chat_json_path,
        muted_segments,
    })
}

pub fn detect_long_silences_wav(wav_path: &Path, min_duration_sec: f64) -> AppResult<Vec<MutedSegment>> {
    let data = fs::read(wav_path)?;
    let (sample_rate, pcm) = parse_pcm16_mono_wav(&data)?;
    if sample_rate == 0 || pcm.is_empty() {
        return Ok(Vec::new());
    }

    let sec_samples = sample_rate as usize;
    let total_secs = pcm.len() / sec_samples;
    let mut segments = Vec::new();
    let mut run_start: Option<usize> = None;

    for s in 0..total_secs {
        let slice = &pcm[s * sec_samples..(s + 1) * sec_samples];
        let mut sum_sq = 0.0f64;
        for &sample in slice {
            let norm = sample as f64 / 32768.0;
            sum_sq += norm * norm;
        }
        let rms = (sum_sq / slice.len() as f64).sqrt();
        let db = if rms > 1e-9 { 20.0 * rms.log10() } else { -120.0 };

        if db < -55.0 {
            if run_start.is_none() {
                run_start = Some(s);
            }
        } else if let Some(start) = run_start.take() {
            let dur = (s - start) as f64;
            if dur >= min_duration_sec {
                segments.push(MutedSegment {
                    start_sec: start as f64,
                    end_sec: s as f64,
                    reason: "Silence prolongé détecté (probable zone mutée)".to_string(),
                });
            }
        }
    }

    if let Some(start) = run_start {
        let dur = (total_secs - start) as f64;
        if dur >= min_duration_sec {
            segments.push(MutedSegment {
                start_sec: start as f64,
                end_sec: total_secs as f64,
                reason: "Silence prolongé détecté (probable zone mutée)".to_string(),
            });
        }
    }

    Ok(segments)
}

pub fn parse_pcm16_mono_wav(bytes: &[u8]) -> AppResult<(u32, Vec<i16>)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AppError::Sidecar("Fichier WAV invalide ou corrompu.".to_string()));
    }
    let mut offset = 12usize;
    let mut sample_rate = 16000u32;
    let mut channels = 1u16;
    let mut data_slice: Option<&[u8]> = None;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body_start = offset + 8;
        let body_end = (body_start + chunk_size).min(bytes.len());

        if chunk_id == b"fmt " && body_end - body_start >= 16 {
            channels = u16::from_le_bytes([bytes[body_start + 2], bytes[body_start + 3]]);
            sample_rate = u32::from_le_bytes([
                bytes[body_start + 4],
                bytes[body_start + 5],
                bytes[body_start + 6],
                bytes[body_start + 7],
            ]);
        } else if chunk_id == b"data" {
            data_slice = Some(&bytes[body_start..body_end]);
            break;
        }
        offset = body_end + (chunk_size % 2);
    }

    let raw = data_slice.ok_or_else(|| AppError::Sidecar("Chunk 'data' introuvable dans le WAV.".to_string()))?;
    let step = (channels.max(1) as usize) * 2;
    let mut samples = Vec::with_capacity(raw.len() / step);
    for frame in raw.chunks_exact(step) {
        samples.push(i16::from_le_bytes([frame[0], frame[1]]));
    }
    Ok((sample_rate, samples))
}

pub async fn create_demo_project(db: &Database, paths: &AppPaths) -> AppResult<String> {
    let project_id = uuid::Uuid::new_v4().to_string();
    let vod_id = uuid::Uuid::new_v4().to_string();
    let workspace_dir = paths.projects.join(&project_id);
    fs::create_dir_all(&workspace_dir)?;

    let audio_wav_path = workspace_dir.join("audio_16k.wav");
    let chat_json_path = workspace_dir.join("chat.json");

    let audio_clone = audio_wav_path.clone();
    let chat_clone = chat_json_path.clone();
    blocking(move || generate_demo_media(&audio_clone, &chat_clone)).await?;

    let muted_segments = vec![MutedSegment {
        start_sec: 300.0,
        end_sec: 355.0,
        reason: "Musique protégée (démo)".to_string(),
    }];
    let muted_json = serde_json::to_string(&muted_segments)?;
    let now = Utc::now().to_rfc3339();
    let workspace_str = workspace_dir.to_string_lossy().to_string();
    let audio_str = audio_wav_path.to_string_lossy().to_string();
    let chat_str = chat_json_path.to_string_lossy().to_string();

    let db_clone = db.clone();
    let pid_ret = project_id.clone();

    blocking(move || {
        db_clone.with_conn_mut(|conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO projects
                 (id, name, streamer_profile_id, mode, target_clips, target_min_sec, target_max_sec,
                  token_budget_limit, status, workspace_dir, created_at, updated_at)
                 VALUES (?1, 'Démo ClipForge (VOD 10 min)', NULL, 'standard', 4, 25, 55, 50000, 'draft', ?2, ?3, ?3)",
                rusqlite::params![project_id, workspace_str, now],
            )?;
            tx.execute(
                r#"INSERT INTO vods
                 (id, project_id, source_url, twitch_vod_id, channel_name, title, duration_sec,
                  thumbnail_url, audio_path, chat_path, muted_segments_json, metadata_json, created_at, updated_at)
                 VALUES (?1, ?2, 'https://www.twitch.tv/videos/999000010', '999000010', 'ClipForgeDemo',
                         'Session Ranked — Jeu de test 10 min', 600.0, NULL, ?3, ?4, ?5, '{"demo":true,"asr_language":"fr"}', ?6, ?6)"#,
                rusqlite::params![vod_id, project_id, audio_str, chat_str, muted_json, now],
            )?;
            tx.commit()?;
            Ok(())
        })
    })
    .await?;

    Ok(pid_ret)
}

fn generate_demo_media(wav_path: &Path, chat_path: &Path) -> AppResult<()> {
    let sample_rate = 16_000u32;
    let duration_sec = 600u32;
    let num_samples = (sample_rate * duration_sec) as usize;
    let peak_centers = [68.0f64, 195.0, 412.0, 528.0];

    let data_bytes_len = (num_samples * 2) as u32;
    let mut file = File::create(wav_path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_bytes_len).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&(sample_rate * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes_len.to_le_bytes())?;

    let mut chunk = vec![0u8; sample_rate as usize * 2];
    for sec in 0..duration_sec {
        let t_sec = sec as f64;
        let is_muted = (300.0..355.0).contains(&t_sec);
        let peak_boost = peak_centers
            .iter()
            .map(|&c| {
                let d = (t_sec - c).abs();
                if d < 14.0 { (1.0 - d / 14.0) * 0.72 } else { 0.0 }
            })
            .fold(0.0f64, f64::max);

        let amp = if is_muted { 0.0 } else { 0.06 + peak_boost };
        for i in 0..(sample_rate as usize) {
            let t = t_sec + (i as f64 / sample_rate as f64);
            let wave = (2.0 * std::f64::consts::PI * 220.0 * t).sin() * 0.5
                + (2.0 * std::f64::consts::PI * 480.0 * t).sin() * 0.35
                + (2.0 * std::f64::consts::PI * 920.0 * t).sin() * 0.15;
            let sample = (wave * amp * 28000.0).clamp(-32767.0, 32767.0) as i16;
            let bytes = sample.to_le_bytes();
            chunk[i * 2] = bytes[0];
            chunk[i * 2 + 1] = bytes[1];
        }
        file.write_all(&chunk)?;
    }

    let mut comments = Vec::new();
    let baseline_msgs = [
        "Salut tout le monde !",
        "Bonne game",
        "Ça part bien cette session",
        "GG",
        "Let's go",
    ];
    for sec in (0..600).step_by(4) {
        comments.push(serde_json::json!({
            "content_offset_seconds": sec as f64,
            "commenter": { "display_name": format!("viewer_{}", sec % 17) },
            "message": { "body": baseline_msgs[(sec / 4) % baseline_msgs.len()], "bits_spent": 0 }
        }));
    }

    let peak_scenarios: &[(f64, &[&str], u32)] = &[
        (68.0, &["KEKW", "OMEGALUL il est tombé tout seul", "LULW CLIPPEZ ÇA", "MDRRR KEKW", "NO WAY KEKW"], 0),
        (195.0, &["PogChamp QUEL CLUTCH", "INSANE 1v4 Pog", "MASTERCLASS CLUTCH", "GOAT PogChamp", "CLIP IT"], 500),
        (412.0, &["RAGE QUIT KEKW", "IL A CASSÉ SON CLAVIER LUL", "PTDRRR KEKW", "ICANT OMEGALUL"], 0),
        (528.0, &["WHOLESOME <3", "MERCI POUR LE RAID Pog", "HYPE TRAIN LET'S GO", "GG GOAT W"], 1000),
    ];

    for &(center, msgs, bits) in peak_scenarios {
        for k in 0..42 {
            let offset = (center - 6.0 + (k as f64 * 0.38)).clamp(0.0, 599.0);
            comments.push(serde_json::json!({
                "content_offset_seconds": offset,
                "commenter": { "display_name": format!("chatter_{k}") },
                "message": {
                    "body": msgs[k % msgs.len()],
                    "bits_spent": if k == 10 { bits } else { 0 }
                }
            }));
        }
    }

    comments.sort_by(|a, b| {
        let ta = a["content_offset_seconds"].as_f64().unwrap_or(0.0);
        let tb = b["content_offset_seconds"].as_f64().unwrap_or(0.0);
        ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
    });

    fs::write(chat_path, serde_json::to_string(&serde_json::json!({ "comments": comments }))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_twitch_vod_urls_and_ids() {
        assert_eq!(
            parse_twitch_vod_id("https://www.twitch.tv/videos/2145678901?t=01h12m"),
            Some("2145678901".to_string())
        );
        assert_eq!(parse_twitch_vod_id("2145678901"), Some("2145678901".to_string()));
        assert_eq!(parse_twitch_vod_id("invalid"), None);
    }
}
