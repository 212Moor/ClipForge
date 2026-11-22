use crate::error::{AppError, AppResult};
use crate::paths::AppPaths;
use crate::sidecars::download::download_resumable;
use crate::sidecars::{manifest::SidecarId, prepare_verified_executable};
use crate::store::Database;
use crate::util::blocking;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::process::Command;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const SETTING_WHISPER_MODEL: &str = "asr.whisper_model";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WhisperModelSpec {
    pub id: String,
    pub label: String,
    pub filename: String,
    pub size_mb: u32,
    pub url: String,
    pub downloaded: bool,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptWord {
    pub word: String,
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptSegment {
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub words: Vec<TranscriptWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WindowTranscript {
    pub language: String,
    pub window_start_sec: f64,
    pub window_end_sec: f64,
    pub segments: Vec<TranscriptSegment>,
    pub compact_text: String,
}

pub fn available_whisper_models(paths: &AppPaths) -> Vec<WhisperModelSpec> {
    let specs = [
        (
            "tiny",
            "Whisper Tiny (rapide, ~75 Mo)",
            "ggml-tiny.bin",
            75u32,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        ),
        (
            "base",
            "Whisper Base (équilibré, ~148 Mo)",
            "ggml-base.bin",
            148u32,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        ),
        (
            "small",
            "Whisper Small (recommandé FR/EN, ~488 Mo)",
            "ggml-small.bin",
            488u32,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        ),
        (
            "medium",
            "Whisper Medium (haute précision, ~1.5 Go)",
            "ggml-medium.bin",
            1530u32,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        ),
    ];

    specs
        .into_iter()
        .map(|(id, label, filename, size_mb, url)| {
            let p = paths.models.join(filename);
            let downloaded = p.is_file();
            WhisperModelSpec {
                id: id.to_string(),
                label: label.to_string(),
                filename: filename.to_string(),
                size_mb,
                url: url.to_string(),
                downloaded,
                local_path: downloaded.then(|| p.to_string_lossy().to_string()),
            }
        })
        .collect()
}

pub fn load_selected_whisper_model(db: &Database) -> String {
    if let Ok(Some(raw)) = db.get_setting(SETTING_WHISPER_MODEL) {
        if let Ok(id) = serde_json::from_str::<String>(&raw) {
            if !id.trim().is_empty() {
                return id;
            }
        }
    }
    "base".to_string()
}

pub fn save_selected_whisper_model(db: &Database, model_id: &str) -> AppResult<()> {
    db.set_setting(SETTING_WHISPER_MODEL, &serde_json::to_string(model_id)?)
}

pub async fn ensure_whisper_model_downloaded(
    app: &AppHandle,
    paths: &AppPaths,
    model_id: &str,
) -> AppResult<PathBuf> {
    let models = available_whisper_models(paths);
    let spec = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| AppError::Config(format!("Modèle Whisper inconnu : {model_id}")))?;

    let dest = paths.models.join(&spec.filename);
    if dest.is_file() {
        return Ok(dest);
    }

    let job_id = format!("whisper-model-{}", spec.id);
    download_resumable(app, &job_id, &spec.label, &spec.url, &dest).await?;
    Ok(dest)
}

pub async fn transcribe_candidate_window(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    workspace_dir: &Path,
    audio_wav_path: &Path,
    candidate_index: u32,
    start_sec: f64,
    end_sec: f64,
    vod_duration_sec: f64,
    language: &str,
    is_demo: bool,
) -> AppResult<WindowTranscript> {
    if is_demo {
        return Ok(build_demo_transcript(candidate_index, start_sec, end_sec));
    }

    let margin = 15.0f64;
    let slice_start = (start_sec - margin).max(0.0);
    let slice_end = if vod_duration_sec > 0.0 {
        (end_sec + margin).min(vod_duration_sec)
    } else {
        end_sec + margin
    };
    let slice_dur = (slice_end - slice_start).max(1.0);

    let asr_dir = workspace_dir.join("asr");
    fs::create_dir_all(&asr_dir)?;
    let chunk_wav = asr_dir.join(format!("cand_{candidate_index:03}.wav"));
    let out_prefix = asr_dir.join(format!("cand_{candidate_index:03}_out"));
    let out_json = asr_dir.join(format!("cand_{candidate_index:03}_out.json"));

    let ffmpeg = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await?;
    let whisper_exe = prepare_verified_executable(db, paths, SidecarId::WhisperCpp).await?;

    let db_clone = db.clone();
    let model_id = blocking(move || Ok(load_selected_whisper_model(&db_clone))).await?;
    let model_path = ensure_whisper_model_downloaded(app, paths, &model_id).await?;

    let mut ff = Command::new(&ffmpeg);
    ff.args([
        "-y",
        "-ss",
        &format!("{slice_start:.3}"),
        "-t",
        &format!("{slice_dur:.3}"),
        "-i",
    ])
    .arg(audio_wav_path)
    .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
    .arg(&chunk_wav);
    #[cfg(target_os = "windows")]
    ff.creation_flags(CREATE_NO_WINDOW);

    let ff_out = ff
        .output()
        .await
        .map_err(|e| AppError::Sidecar(format!("Découpe audio FFmpeg impossible : {e}")))?;
    if !ff_out.status.success() {
        return Err(AppError::Sidecar(format!(
            "FFmpeg a échoué lors de la découpe de la fenêtre #{candidate_index}"
        )));
    }

    let mut wh = Command::new(&whisper_exe);
    wh.arg("-m")
        .arg(&model_path)
        .arg("-f")
        .arg(&chunk_wav)
        .arg("-ojf")
        .arg("-of")
        .arg(&out_prefix)
        .arg("-l")
        .arg(if language.trim().is_empty() { "auto" } else { language });
    #[cfg(target_os = "windows")]
    wh.creation_flags(CREATE_NO_WINDOW);

    let wh_out = wh
        .output()
        .await
        .map_err(|e| AppError::Sidecar(format!("Échec lancement whisper.cpp : {e}")))?;

    if !wh_out.status.success() || !out_json.is_file() {
        let stderr = String::from_utf8_lossy(&wh_out.stderr);
        return Err(AppError::Sidecar(format!(
            "whisper.cpp a échoué sur la fenêtre #{candidate_index} : {}",
            stderr.lines().last().unwrap_or("erreur inconnue")
        )));
    }

    let raw_json = fs::read_to_string(&out_json)?;
    let _ = fs::remove_file(&chunk_wav);

    parse_whisper_json(&raw_json, slice_start, start_sec, end_sec, language)
}

pub fn parse_whisper_json(
    raw_json: &str,
    slice_origin_sec: f64,
    cand_start_sec: f64,
    cand_end_sec: f64,
    fallback_lang: &str,
) -> AppResult<WindowTranscript> {
    let val: serde_json::Value = serde_json::from_str(raw_json)?;
    let detected_lang = val
        .pointer("/result/language")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_lang)
        .to_string();

    let mut segments = Vec::new();
    if let Some(arr) = val.get("transcription").and_then(|v| v.as_array()) {
        for item in arr {
            let from_ms = item
                .pointer("/offsets/from")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let to_ms = item
                .pointer("/offsets/to")
                .and_then(|v| v.as_f64())
                .unwrap_or(from_ms);
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }

            let mut words = Vec::new();
            if let Some(tokens) = item.get("tokens").and_then(|v| v.as_array()) {
                for tok in tokens {
                    let t_text = tok.get("text").and_then(|v| v.as_str()).unwrap_or("").trim();
                    if t_text.is_empty() || t_text.starts_with("[_") || t_text.starts_with("<|") {
                        continue;
                    }
                    let w_from = tok
                        .pointer("/offsets/from")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(from_ms);
                    let w_to = tok
                        .pointer("/offsets/to")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(w_from + 150.0);
                    words.push(TranscriptWord {
                        word: t_text.to_string(),
                        start_sec: slice_origin_sec + w_from / 1000.0,
                        end_sec: slice_origin_sec + w_to / 1000.0,
                    });
                }
            }

            segments.push(TranscriptSegment {
                start_sec: slice_origin_sec + from_ms / 1000.0,
                end_sec: slice_origin_sec + to_ms / 1000.0,
                text,
                words,
            });
        }
    }

    let compact_text = compress_transcript(&segments, cand_start_sec);

    Ok(WindowTranscript {
        language: detected_lang,
        window_start_sec: cand_start_sec,
        window_end_sec: cand_end_sec,
        segments,
        compact_text,
    })
}

pub fn compress_transcript(segments: &[TranscriptSegment], base_start_sec: f64) -> String {
    const FILLERS: &[&str] = &["euh", "heu", "uh", "um", "hmm", "bah", "ben"];
    let mut parts = Vec::new();

    for seg in segments {
        let mut cleaned_words: Vec<String> = Vec::new();
        let mut prev_lower = String::new();

        for raw_word in seg.text.split_whitespace() {
            let norm = raw_word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_ascii_lowercase();
            if norm.is_empty() || FILLERS.contains(&norm.as_str()) {
                continue;
            }
            if norm == prev_lower {
                continue;
            }
            prev_lower = norm;
            cleaned_words.push(raw_word.to_string());
        }

        if cleaned_words.is_empty() {
            continue;
        }
        let rel = (seg.start_sec - base_start_sec).round() as i32;
        parts.push(format!("[{rel}]{}", cleaned_words.join(" ")));
    }

    let joined = parts.join(" ");
    if joined.chars().count() > 900 {
        let truncated: String = joined.chars().take(897).collect();
        format!("{truncated}...")
    } else {
        joined
    }
}

fn build_demo_transcript(idx: u32, start_sec: f64, end_sec: f64) -> WindowTranscript {
    let lines: &[&str] = match idx % 4 {
        1 => &[
            "Non mais attends, regarde où il est allé se planquer ce gars !",
            "Il croit vraiment que je l'ai pas vu dans le buisson ?",
            "OH NON IL EST TOMBÉ TOUT SEUL DANS LE VIDE !",
            "J'en peux plus, clippez ça immédiatement, c'est le pire play de l'année !",
        ],
        2 => &[
            "Ok un contre quatre, zéro pression, il me reste six balles.",
            "Un de moins à gauche, deuxième tête !",
            "Attends il arrive derrière... BOOM le troisième !",
            "LE CLUTCH 1V4 DE LA MASTERCLASS ! LET'S GO !",
        ],
        3 => &[
            "Si le jeu me fait encore laguer sur ce saut je quitte tout.",
            "Regarde ma connexion, trois cents de ping au moment exact du boss !",
            "NON MAIS C'EST UNE BLAGUE, LE PERSONNAGE A TRAVERSÉ LE SOL !",
            "Ok c'est fini, j'ai failli casser mon clavier là.",
        ],
        _ => &[
            "Attendez les gars, merci mille fois pour le raid de trois cents personnes !",
            "Bienvenue à toute la commu, vous arrivez pile pour la finale.",
            "Le chat part complètement en hype train, vous êtes incroyables !",
            "On va essayer de leur offrir une vraie victoire maintenant.",
        ],
    };

    let dur = (end_sec - start_sec).max(8.0);
    let step = dur / lines.len() as f64;
    let mut segments = Vec::new();
    for (i, &line) in lines.iter().enumerate() {
        let s = start_sec + i as f64 * step;
        let e = s + step * 0.85;
        let word_list: Vec<&str> = line.split_whitespace().collect();
        let w_step = (e - s) / word_list.len().max(1) as f64;
        let words = word_list
            .iter()
            .enumerate()
            .map(|(wi, &w)| TranscriptWord {
                word: w.to_string(),
                start_sec: s + wi as f64 * w_step,
                end_sec: s + (wi + 1) as f64 * w_step,
            })
            .collect();
        segments.push(TranscriptSegment {
            start_sec: s,
            end_sec: e,
            text: line.to_string(),
            words,
        });
    }

    let compact_text = compress_transcript(&segments, start_sec);
    WindowTranscript {
        language: "fr".to_string(),
        window_start_sec: start_sec,
        window_end_sec: end_sec,
        segments,
        compact_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compresses_fillers_and_duplicate_words() {
        let segs = vec![
            TranscriptSegment {
                start_sec: 100.0,
                end_sec: 104.0,
                text: "Euh non non mais regarde regarde ça".to_string(),
                words: vec![],
            },
            TranscriptSegment {
                start_sec: 106.0,
                end_sec: 110.0,
                text: "Bah oh il a fait ça !".to_string(),
                words: vec![],
            },
        ];
        let compact = compress_transcript(&segs, 100.0);
        assert_eq!(compact, "[0]non mais regarde ça [6]oh il a fait ça");
    }
}
