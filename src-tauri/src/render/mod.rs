pub mod ass;

use crate::asr::WindowTranscript;
use crate::error::{AppError, AppResult};
use crate::ipc::emit_progress;
use crate::paths::AppPaths;
use crate::render::ass::{generate_ass_document, ClipOverridesPayload};
use crate::sidecars::{manifest::SidecarId, prepare_verified_executable};
use crate::store::projects::{
    get_clip, get_project_summary, get_streamer_profile, get_vod_by_project, list_clips,
    update_clip_render_result, ClipRecord, FacecamRect,
};
use crate::store::Database;
use crate::util::blocking;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Semaphore;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const SETTING_RENDER_CONFIG: &str = "render.config";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DetectedEncoders {
    pub nvenc: bool,
    pub amf: bool,
    pub qsv: bool,
    pub libx264: bool,
    pub recommended: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenderSettings {
    /// "auto" | "h264_nvenc" | "h264_amf" | "h264_qsv" | "libx264"
    pub encoder: String,
    /// 30 ou 60
    pub fps: u32,
    /// 1 ou 2 jobs en parallèle
    pub max_parallel: u32,
    pub default_facecam: FacecamRect,
    pub custom_export_dir: Option<String>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            encoder: "auto".to_string(),
            fps: 30,
            max_parallel: 2,
            default_facecam: FacecamRect::default(),
            custom_export_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ClipRenderResult {
    pub clip_id: String,
    pub encoder_used: String,
    pub mp4_path: String,
    pub caption_txt_path: String,
    pub csv_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SegmentMeta {
    seg_start_sec: f64,
    seg_end_sec: f64,
    is_demo: bool,
}

pub fn load_render_settings(db: &Database) -> RenderSettings {
    if let Ok(Some(raw)) = db.get_setting(SETTING_RENDER_CONFIG) {
        if let Ok(cfg) = serde_json::from_str::<RenderSettings>(&raw) {
            return cfg;
        }
    }
    RenderSettings::default()
}

pub fn save_render_settings(db: &Database, settings: &RenderSettings) -> AppResult<()> {
    let mut normalized = settings.clone();
    normalized.fps = if normalized.fps >= 55 { 60 } else { 30 };
    normalized.max_parallel = normalized.max_parallel.clamp(1, 2);
    db.set_setting(SETTING_RENDER_CONFIG, &serde_json::to_string(&normalized)?)
}

/// Détecte les encodeurs H.264 disponibles dans le sidecar FFmpeg (NVENC -> AMF -> QSV -> libx264).
pub async fn detect_hardware_encoders(db: &Database, paths: &AppPaths) -> AppResult<DetectedEncoders> {
    let ffmpeg = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(["-hide_banner", "-encoders"]);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd
        .output()
        .await
        .map_err(|e| AppError::Sidecar(format!("Impossible d'interroger FFmpeg : {e}")))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(parse_ffmpeg_encoders(&stdout))
}

pub fn parse_ffmpeg_encoders(ffmpeg_encoders_stdout: &str) -> DetectedEncoders {
    let nvenc = ffmpeg_encoders_stdout.contains("h264_nvenc");
    let amf = ffmpeg_encoders_stdout.contains("h264_amf");
    let qsv = ffmpeg_encoders_stdout.contains("h264_qsv");
    let libx264 = ffmpeg_encoders_stdout.contains("libx264");

    let recommended = if nvenc {
        "h264_nvenc"
    } else if amf {
        "h264_amf"
    } else if qsv {
        "h264_qsv"
    } else {
        "libx264"
    }
    .to_string();

    DetectedEncoders {
        nvenc,
        amf,
        qsv,
        libx264,
        recommended,
    }
}

pub fn resolve_encoder_choice(settings: &RenderSettings, detected: &DetectedEncoders) -> String {
    match settings.encoder.as_str() {
        "h264_nvenc" if detected.nvenc => "h264_nvenc".to_string(),
        "h264_amf" if detected.amf => "h264_amf".to_string(),
        "h264_qsv" if detected.qsv => "h264_qsv".to_string(),
        "libx264" => "libx264".to_string(),
        _ => detected.recommended.clone(),
    }
}

/// Échappe un chemin de fichier pour le filtre FFmpeg `ass='...'` (notamment `C:\...` sous Windows).
pub fn escape_ffmpeg_filter_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    raw.replace(':', "\\:").replace('\'', "\\'")
}

/// Construit le filtergraph FFmpeg 1080×1920 pour Split / Fit / Fill + incrustation ASS.
/// Aucun texte utilisateur/IA n'est jamais interpolé dans la ligne de commande : uniquement le chemin ASS.
pub fn build_layout_filtergraph(
    layout_mode: &str,
    facecam: FacecamRect,
    pan_x: f64,
    ass_path: &Path,
) -> String {
    let w = facecam.w.clamp(0.08, 0.95);
    let h = facecam.h.clamp(0.08, 0.95);
    let x = facecam.x.clamp(0.0, (1.0 - w).max(0.0));
    let y = facecam.y.clamp(0.0, (1.0 - h).max(0.0));
    let ass_esc = escape_ffmpeg_filter_path(ass_path);

    match layout_mode {
        "fit" => format!(
            "[0:v]split=3[vbg][vgame][vcam];\
             [vbg]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=24:4,setsar=1[bg];\
             [vgame]scale=1080:608:force_original_aspect_ratio=decrease,pad=1080:608:(ow-iw)/2:(oh-ih)/2:black,setsar=1[fg];\
             [vcam]crop=iw*{w:.4}:ih*{h:.4}:iw*{x:.4}:ih*{y:.4},scale=280:280:force_original_aspect_ratio=increase,crop=280:280,setsar=1[cam];\
             [bg][fg]overlay=(W-w)/2:(H-h)/2[mid];\
             [mid][cam]overlay=56:110,ass='{ass_esc}'[vout]"
        ),
        "fill" => {
            let pan_norm = ((pan_x.clamp(-1.0, 1.0) + 1.0) * 0.5).clamp(0.0, 1.0);
            format!(
                "[0:v]scale=-2:1920,crop=1080:1920:(iw-1080)*{pan_norm:.4}:0,setsar=1,ass='{ass_esc}'[vout]"
            )
        }
        // "split" par défaut : facecam en haut (1080x640), jeu en bas (1080x1280)
        _ => format!(
            "[0:v]split=2[vcam][vgame];\
             [vcam]crop=iw*{w:.4}:ih*{h:.4}:iw*{x:.4}:ih*{y:.4},scale=1080:640:force_original_aspect_ratio=increase,crop=1080:640,setsar=1[top];\
             [vgame]scale=1080:1280:force_original_aspect_ratio=increase,crop=1080:1280,setsar=1[bot];\
             [top][bot]vstack=inputs=2,ass='{ass_esc}'[vout]"
        ),
    }
}

pub fn format_caption_txt(clip: &ClipRecord) -> String {
    let tags = clip
        .hashtags
        .iter()
        .map(|t| {
            let clean = t.trim().trim_start_matches('#');
            format!("#{clean}")
        })
        .filter(|t| t.len() > 1)
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = String::new();
    if !clip.title.trim().is_empty() {
        out.push_str(clip.title.trim());
        out.push_str("\n\n");
    }
    if !clip.caption.trim().is_empty() {
        out.push_str(clip.caption.trim());
        out.push_str("\n\n");
    }
    if !tags.is_empty() {
        out.push_str(&tags);
        out.push('\n');
    }
    out
}

pub fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

pub fn build_project_csv(clips: &[ClipRecord]) -> String {
    let mut csv = String::from(
        "clip_id,title,hook,category,start_sec,end_sec,duration_sec,layout_mode,subtitle_style,review_status,render_status,hashtags,caption,mp4_path,caption_txt_path\n",
    );
    for c in clips {
        let tags = c
            .hashtags
            .iter()
            .map(|t| format!("#{}", t.trim().trim_start_matches('#')))
            .collect::<Vec<_>>()
            .join(" ");
        let dur = (c.end_sec - c.start_sec).max(0.0);
        let row = [
            escape_csv_field(&c.id),
            escape_csv_field(&c.title),
            escape_csv_field(&c.hook),
            escape_csv_field(&c.category),
            format!("{:.2}", c.start_sec),
            format!("{:.2}", c.end_sec),
            format!("{:.2}", dur),
            escape_csv_field(&c.layout_mode),
            escape_csv_field(&c.subtitle_style),
            escape_csv_field(&c.review_status),
            escape_csv_field(&c.render_status),
            escape_csv_field(&tags),
            escape_csv_field(&c.caption),
            escape_csv_field(c.output_mp4_path.as_deref().unwrap_or("")),
            escape_csv_field(c.output_caption_path.as_deref().unwrap_or("")),
        ]
        .join(",");
        csv.push_str(&row);
        csv.push('\n');
    }
    csv
}

pub fn slugify_filename(raw: &str, fallback_id: &str) -> String {
    let mut slug: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    while slug.contains("__") {
        slug = slug.replace("__", "_");
    }
    let trimmed = slug.trim_matches('_');
    let short_id: String = fallback_id.chars().take(8).collect();
    if trimmed.is_empty() {
        format!("clip_{short_id}")
    } else {
        let capped: String = trimmed.chars().take(48).collect();
        format!("{capped}_{short_id}")
    }
}

pub fn resolve_export_dir(project_workspace: &Path, settings: &RenderSettings) -> AppResult<PathBuf> {
    let dir = match settings.custom_export_dir.as_deref() {
        Some(custom) if !custom.trim().is_empty() => PathBuf::from(custom.trim()),
        _ => project_workspace.join("exports"),
    };
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Exporte ou met à jour le fichier CSV récapitulatif du projet dans son dossier d'export.
pub fn write_project_csv(db: &Database, project_id: &str) -> AppResult<PathBuf> {
    let project = get_project_summary(db, project_id)?;
    let settings = load_render_settings(db);
    let clips = list_clips(db, project_id)?;
    let export_dir = resolve_export_dir(Path::new(&project.workspace_dir), &settings)?;
    let csv_path = export_dir.join("clips_export.csv");
    fs::write(&csv_path, build_project_csv(&clips))?;
    Ok(csv_path)
}

/// Télécharge uniquement le segment vidéo du clip retenu (marge ±5 s) via `yt-dlp --download-sections`
/// ou génère un segment vidéo de test 16:9 si le projet est une démo locale.
/// Reprenable : réutilise le segment s'il couvre déjà `[start_sec, end_sec]`.
pub async fn ensure_clip_video_segment(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    clip: &ClipRecord,
    workspace_dir: &Path,
    source_url: &str,
    vod_duration_sec: f64,
    audio_wav_path: Option<&Path>,
    is_demo: bool,
) -> AppResult<(PathBuf, f64)> {
    let seg_dir = workspace_dir.join("segments");
    fs::create_dir_all(&seg_dir)?;
    let seg_path = seg_dir.join(format!("clip_{}.mp4", clip.id));
    let meta_path = seg_dir.join(format!("clip_{}.meta.json", clip.id));

    // Reprise : si le segment déjà présent couvre les bornes actuelles du clip, on ne retélécharge pas
    if seg_path.is_file() && meta_path.is_file() {
        if let Ok(raw) = fs::read_to_string(&meta_path) {
            if let Ok(meta) = serde_json::from_str::<SegmentMeta>(&raw) {
                if meta.seg_start_sec <= clip.start_sec + 0.1
                    && meta.seg_end_sec >= clip.end_sec - 0.1
                {
                    return Ok((seg_path, meta.seg_start_sec));
                }
            }
        }
    }

    let margin = 5.0f64;
    let seg_start = (clip.start_sec - margin).max(0.0);
    let seg_end = if vod_duration_sec > 0.0 {
        (clip.end_sec + margin).min(vod_duration_sec)
    } else {
        clip.end_sec + margin
    };
    let seg_dur = (seg_end - seg_start).max(2.0);

    let ffmpeg = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await?;

    if is_demo {
        emit_progress(
            app,
            "render",
            &clip.id,
            "segment",
            12.0,
            "Génération du segment vidéo démo 16:9 (test local)…",
        );

        let mut cmd = Command::new(&ffmpeg);
        cmd.args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=1280x720:rate=30:duration={seg_dur:.2}"),
        ]);
        if let Some(wav) = audio_wav_path.filter(|p| p.is_file()) {
            cmd.args(["-ss", &format!("{seg_start:.3}"), "-t", &format!("{seg_dur:.3}"), "-i"])
                .arg(wav)
                .args(["-map", "0:v:0", "-map", "1:a:0"]);
        } else {
            cmd.args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=440:duration={seg_dur:.2}"),
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
            ]);
        }
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-shortest",
        ])
        .arg(&seg_path);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let out = cmd
            .output()
            .await
            .map_err(|e| AppError::Sidecar(format!("Génération vidéo démo impossible : {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(AppError::Sidecar(format!(
                "FFmpeg démo a échoué : {}",
                stderr.lines().last().unwrap_or("erreur inconnue")
            )));
        }
    } else {
        emit_progress(
            app,
            "render",
            &clip.id,
            "segment",
            10.0,
            format!(
                "Téléchargement du segment vidéo seul ({:.0}s–{:.0}s, marge ±5s)…",
                seg_start, seg_end
            ),
        );

        let yt_dlp = prepare_verified_executable(db, paths, SidecarId::YtDlp).await?;
        let cookie_file = workspace_dir.join("cookies.txt");

        if seg_path.is_file() {
            let _ = fs::remove_file(&seg_path);
        }

        let mut cmd = Command::new(&yt_dlp);
        cmd.args([
            "--force-ipv4", "--retries", "10", "--fragment-retries", "10", "--retry-sleep", "2", "--socket-timeout", "30", "--download-sections",
            &format!("*{seg_start:.2}-{seg_end:.2}"),
            "--force-keyframes-at-cuts",
            "-f",
            "bestvideo[height<=1080][ext=mp4]+bestaudio[ext=m4a]/best[height<=1080]/best",
            "--merge-output-format",
            "mp4",
            "--no-playlist",
        ]);
        if let Some(ff_dir) = ffmpeg.parent() {
            cmd.arg("--ffmpeg-location").arg(ff_dir);
        }
        if cookie_file.is_file() {
            cmd.arg("--cookies").arg(&cookie_file);
        }
        cmd.arg("-o").arg(&seg_path).arg(source_url);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let out = cmd
            .output()
            .await
            .map_err(|e| AppError::Sidecar(format!("Impossible de lancer yt-dlp : {e}")))?;
        if !out.status.success() || !seg_path.is_file() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(AppError::Network(format!(
                "Téléchargement du segment vidéo échoué : {}",
                stderr.lines().last().unwrap_or("erreur inconnue")
            )));
        }
    }

    let meta = SegmentMeta {
        seg_start_sec: seg_start,
        seg_end_sec: seg_end,
        is_demo,
    };
    fs::write(&meta_path, serde_json::to_string(&meta)?)?;

    Ok((seg_path, seg_start))
}

/// Exécute le rendu complet d'un clip en 1080×1920 H.264 / AAC 192k / loudnorm −14 LUFS + ASS mot à mot,
/// puis exporte le MP4, le fichier `.txt` de légende et met à jour le CSV récapitulatif.
pub async fn render_single_clip(
    app: AppHandle,
    db: Database,
    paths: AppPaths,
    semaphore: Arc<Semaphore>,
    clip_id: String,
    cancel_flag: Arc<AtomicBool>,
) -> AppResult<ClipRenderResult> {
    let db_q = db.clone();
    let cid_q = clip_id.clone();
    blocking(move || update_clip_render_result(&db_q, &cid_q, "queued", None, None)).await?;
    emit_progress(&app, "render", &clip_id, "queued", 2.0, "En file d'attente GPU/CPU…");

    // Limite à 1 ou 2 jobs parallèles (vérifie la config utilisateur)
    let _permit = semaphore
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Sémaphore de rendu fermé".to_string()))?;

    if cancel_flag.load(Ordering::SeqCst) {
        let db_c = db.clone();
        let cid_c = clip_id.clone();
        let _ = blocking(move || update_clip_render_result(&db_c, &cid_c, "idle", None, None)).await;
        return Err(AppError::Cancelled("Rendu annulé avant démarrage.".to_string()));
    }

    let db_load = db.clone();
    let cid_load = clip_id.clone();
    let (clip, project, vod, settings, profile_facecam) = blocking(move || {
        update_clip_render_result(&db_load, &cid_load, "rendering", None, None)?;
        let clip = get_clip(&db_load, &cid_load)?;
        let project = get_project_summary(&db_load, &clip.project_id)?;
        let vod = get_vod_by_project(&db_load, &clip.project_id)?;
        let settings = load_render_settings(&db_load);
        let profile_facecam = match project.streamer_profile_id.as_deref() {
            Some(pid) => get_streamer_profile(&db_load, pid)?.and_then(|p| p.facecam_rect),
            None => None,
        };
        Ok((clip, project, vod, settings, profile_facecam))
    })
    .await?;

    let result: AppResult<ClipRenderResult> = async {
        let workspace_dir = PathBuf::from(&project.workspace_dir);
        let export_dir = resolve_export_dir(&workspace_dir, &settings)?;
        let meta_val: serde_json::Value =
            serde_json::from_str(&vod.metadata_json).unwrap_or(serde_json::Value::Null);
        let is_demo = meta_val.get("demo").and_then(|v| v.as_bool()).unwrap_or(false);
        let audio_path_opt = vod.audio_path.as_deref().map(PathBuf::from);

        // 1. Segment vidéo (±5 s)
        let (segment_path, seg_origin_sec) = ensure_clip_video_segment(
            &app,
            &db,
            &paths,
            &clip,
            &workspace_dir,
            &vod.source_url,
            vod.duration_sec,
            audio_path_opt.as_deref(),
            is_demo,
        )
        .await?;

        if cancel_flag.load(Ordering::SeqCst) {
            return Err(AppError::Cancelled("Rendu annulé.".to_string()));
        }

        // 2. Génération du fichier sous-titres ASS mot à mot + accroche
        emit_progress(&app, "render", &clip.id, "subtitles", 24.0, "Génération des sous-titres ASS mot à mot…");
        let overrides: ClipOverridesPayload = clip
            .subtitles_override_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        let transcript: Option<WindowTranscript> = clip
            .transcript_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok());

        let ass_content = generate_ass_document(
            &clip.subtitle_style,
            &overrides,
            &clip.hook,
            transcript.as_ref(),
            clip.start_sec,
            clip.end_sec,
        );
        let ass_dir = workspace_dir.join("ass");
        fs::create_dir_all(&ass_dir)?;
        let ass_path = ass_dir.join(format!("clip_{}.ass", clip.id));
        fs::write(&ass_path, ass_content)?;

        // 3. Détection de l'encodeur matériel (NVENC -> AMF -> QSV -> libx264)
        let detected = detect_hardware_encoders(&db, &paths).await?;
        let chosen_encoder = resolve_encoder_choice(&settings, &detected);
        let ffmpeg = prepare_verified_executable(&db, &paths, SidecarId::Ffmpeg).await?;

        let facecam = profile_facecam.unwrap_or(settings.default_facecam);
        let filter_complex = build_layout_filtergraph(
            &clip.layout_mode,
            facecam,
            overrides.pan_x,
            &ass_path,
        );

        let slug = slugify_filename(&clip.title, &clip.id);
        let output_mp4 = export_dir.join(format!("{slug}.mp4"));
        let output_txt = export_dir.join(format!("{slug}.txt"));

        let trim_start = (clip.start_sec - seg_origin_sec).max(0.0);
        let clip_dur = (clip.end_sec - clip.start_sec).max(1.0);

        emit_progress(
            &app,
            "render",
            &clip.id,
            "encoding",
            30.0,
            format!("Encodage 1080×1920 ({chosen_encoder}, {} fps, −14 LUFS)…", settings.fps),
        );

        // Tente d'abord l'encodeur matériel choisi ; si un GPU est absent/pilote incompatible, repli automatique sur libx264
        let mut encoder_used = chosen_encoder.clone();
        let encode_res = run_ffmpeg_encode(
            &app,
            &ffmpeg,
            &clip.id,
            &segment_path,
            trim_start,
            clip_dur,
            &filter_complex,
            &encoder_used,
            settings.fps,
            &output_mp4,
            &cancel_flag,
        )
        .await;

        if let Err(err) = encode_res {
            if matches!(err, AppError::Cancelled(_)) {
                let _ = fs::remove_file(&output_mp4);
                return Err(err);
            }
            if encoder_used != "libx264" {
                tracing::warn!(
                    encoder = %encoder_used,
                    error = %err,
                    "Encodeur matériel indisponible à l'exécution — repli automatique sur libx264"
                );
                encoder_used = "libx264".to_string();
                emit_progress(
                    &app,
                    "render",
                    &clip.id,
                    "encoding",
                    32.0,
                    "Repli automatique sur libx264…",
                );
                run_ffmpeg_encode(
                    &app,
                    &ffmpeg,
                    &clip.id,
                    &segment_path,
                    trim_start,
                    clip_dur,
                    &filter_complex,
                    &encoder_used,
                    settings.fps,
                    &output_mp4,
                    &cancel_flag,
                )
                .await?;
            } else {
                return Err(err);
            }
        }

        // 4. Export du fichier légende .txt + mise à jour SQLite + CSV récapitulatif
        fs::write(&output_txt, format_caption_txt(&clip))?;

        let mp4_str = output_mp4.to_string_lossy().to_string();
        let txt_str = output_txt.to_string_lossy().to_string();

        let db_done = db.clone();
        let cid_done = clip.id.clone();
        let pid_done = clip.project_id.clone();
        let mp4_c = mp4_str.clone();
        let txt_c = txt_str.clone();
        let csv_path = blocking(move || {
            update_clip_render_result(&db_done, &cid_done, "done", Some(&mp4_c), Some(&txt_c))?;
            write_project_csv(&db_done, &pid_done)
        })
        .await?;

        emit_progress(
            &app,
            "render",
            &clip.id,
            "done",
            100.0,
            format!("Export terminé ({encoder_used})"),
        );

        Ok(ClipRenderResult {
            clip_id: clip.id.clone(),
            encoder_used,
            mp4_path: mp4_str,
            caption_txt_path: txt_str,
            csv_path: csv_path.to_string_lossy().to_string(),
        })
    }
    .await;

    match result {
        Ok(res) => Ok(res),
        Err(AppError::Cancelled(msg)) => {
            let db_c = db.clone();
            let cid_c = clip_id.clone();
            let _ = blocking(move || update_clip_render_result(&db_c, &cid_c, "idle", None, None)).await;
            emit_progress(&app, "render", &clip_id, "cancelled", 0.0, &msg);
            Err(AppError::Cancelled(msg))
        }
        Err(err) => {
            let msg = err.to_string();
            let db_f = db.clone();
            let cid_f = clip_id.clone();
            let _ = blocking(move || update_clip_render_result(&db_f, &cid_f, "failed", None, None)).await;
            emit_progress(&app, "render", &clip_id, "failed", 0.0, &msg);
            Err(err)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_ffmpeg_encode(
    app: &AppHandle,
    ffmpeg: &Path,
    clip_id: &str,
    input_segment: &Path,
    trim_start_sec: f64,
    clip_dur_sec: f64,
    filter_complex: &str,
    encoder: &str,
    fps: u32,
    output_mp4: &Path,
    cancel_flag: &AtomicBool,
) -> AppResult<()> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-ss",
        &format!("{trim_start_sec:.3}"),
        "-t",
        &format!("{clip_dur_sec:.3}"),
        "-i",
    ])
    .arg(input_segment)
    .args(["-filter_complex", filter_complex, "-map", "[vout]", "-map", "0:a:0?"]);

    match encoder {
        "h264_nvenc" => {
            cmd.args(["-c:v", "h264_nvenc", "-preset", "p4", "-cq", "22", "-b:v", "0"]);
        }
        "h264_amf" => {
            cmd.args([
                "-c:v",
                "h264_amf",
                "-quality",
                "balanced",
                "-rc",
                "cqp",
                "-qp_i",
                "22",
                "-qp_p",
                "22",
            ]);
        }
        "h264_qsv" => {
            cmd.args(["-c:v", "h264_qsv", "-preset", "medium", "-global_quality", "22"]);
        }
        _ => {
            cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", "20"]);
        }
    }

    cmd.args([
        "-r",
        &fps.to_string(),
        "-pix_fmt",
        "yuv420p",
        "-af",
        "loudnorm=I=-14:TP=-1.5:LRA=11",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-ar",
        "48000",
        "-movflags",
        "+faststart",
    ])
    .arg(output_mp4)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Sidecar(format!("Lancement FFmpeg impossible : {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Sidecar("Flux stdout FFmpeg indisponible".to_string()))?;
    let mut reader = BufReader::new(stdout).lines();
    let total_us = (clip_dur_sec * 1_000_000.0).max(1.0);
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            return Err(AppError::Cancelled("Rendu annulé par l'utilisateur.".to_string()));
        }

        match tokio::time::timeout(Duration::from_millis(250), reader.next_line()).await {
            Ok(Ok(Some(line))) => {
                if let Some(us_str) = line.strip_prefix("out_time_us=") {
                    if let Ok(us) = us_str.trim().parse::<f64>() {
                        if last_emit.elapsed() >= Duration::from_millis(120) {
                            let ratio = (us / total_us).clamp(0.0, 1.0);
                            let pct = 30.0 + ratio * 66.0;
                            emit_progress(
                                app,
                                "render",
                                clip_id,
                                "encoding",
                                pct,
                                format!("Encodage {encoder} : {:.0} %", ratio * 100.0),
                            );
                            last_emit = Instant::now();
                        }
                    }
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| AppError::Sidecar(format!("Attente FFmpeg interrompue : {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Sidecar(format!(
            "FFmpeg ({encoder}) a échoué : {}",
            stderr.lines().last().unwrap_or("erreur inconnue")
        )));
    }

    Ok(())
}

/// Ouvre un fichier ou dossier directement dans l'Explorateur Windows.
pub fn reveal_path_in_explorer(path_str: &str) -> AppResult<()> {
    let target = PathBuf::from(path_str);
    if !target.exists() {
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Chemin introuvable : {path_str}"),
        )));
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer");
        if target.is_file() {
            cmd.arg(format!("/select,{}", target.display()));
        } else {
            cmd.arg(&target);
        }
        cmd.spawn()?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = target;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_windows_paths_for_ffmpeg_ass_filter() {
        let p = Path::new(r"C:\Users\Alex\AppData\Local\ClipForge\clip_1.ass");
        assert_eq!(
            escape_ffmpeg_filter_path(p),
            r"C\:/Users/Alex/AppData/Local/ClipForge/clip_1.ass"
        );
    }

    #[test]
    fn builds_split_fit_fill_1080x1920_filtergraphs() {
        let ass = Path::new(r"C:\tmp\sub.ass");
        let fc = FacecamRect::default();

        let split = build_layout_filtergraph("split", fc, 0.0, ass);
        assert!(split.contains("1080:640"));
        assert!(split.contains("1080:1280"));
        assert!(split.contains("vstack=inputs=2"));

        let fit = build_layout_filtergraph("fit", fc, 0.0, ass);
        assert!(fit.contains("boxblur=24:4"));
        assert!(fit.contains("overlay=56:110"));

        let fill = build_layout_filtergraph("fill", fc, -0.5, ass);
        assert!(fill.contains("crop=1080:1920:(iw-1080)*0.2500:0"));
    }

    #[test]
    fn ranks_hardware_encoders_nvenc_amf_qsv_x264() {
        let all = parse_ffmpeg_encoders(" V....D h264_nvenc \n V....D h264_amf \n V....D libx264 ");
        assert_eq!(all.recommended, "h264_nvenc");

        let amd = parse_ffmpeg_encoders(" V....D h264_amf \n V....D libx264 ");
        assert_eq!(amd.recommended, "h264_amf");

        let cpu = parse_ffmpeg_encoders(" V....D libx264 ");
        assert_eq!(cpu.recommended, "libx264");
    }

    #[test]
    fn escapes_csv_quotes_and_commas() {
        assert_eq!(escape_csv_field("simple"), "simple");
        assert_eq!(
            escape_csv_field("Incroyable, \"clutch\" 1v4"),
            "\"Incroyable, \"\"clutch\"\" 1v4\""
        );
    }
}
