use crate::error::AppResult;
use crate::ingest::parse_pcm16_mono_wav;
use crate::store::projects::{MutedSegment, SignalWindowRecord};
use crate::store::Database;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const DEFAULT_WEIGHTS_RAW: &str = include_str!("../../resources/signal_weights.json");
const SETTING_SIGNAL_CONFIG: &str = "signals.config";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SignalWeightsConfig {
    pub chat: f64,
    pub audio: f64,
    pub emotes: f64,
    pub events: f64,
    pub voice: f64,
    pub scene: f64,
    pub min_peak_distance_sec: f64,
    pub pre_peak_lead_sec: f64,
    pub min_clip_sec: f64,
    pub max_clip_sec: f64,
    pub candidate_multiplier: u32,
    pub emotes_list: Vec<String>,
    pub keywords_list: Vec<String>,
}

impl Default for SignalWeightsConfig {
    fn default() -> Self {
        serde_json::from_str(DEFAULT_WEIGHTS_RAW).expect("signal_weights.json embarqué invalide")
    }
}

pub fn load_signal_config(db: &Database) -> SignalWeightsConfig {
    if let Ok(Some(raw)) = db.get_setting(SETTING_SIGNAL_CONFIG) {
        if let Ok(cfg) = serde_json::from_str::<SignalWeightsConfig>(&raw) {
            return cfg;
        }
    }
    SignalWeightsConfig::default()
}

pub fn save_signal_config(db: &Database, cfg: &SignalWeightsConfig) -> AppResult<()> {
    db.set_setting(SETTING_SIGNAL_CONFIG, &serde_json::to_string(cfg)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageLite {
    pub offset_sec: f64,
    pub author: String,
    pub body: String,
    pub bits_spent: u32,
    pub is_event: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactChatSummary {
    pub r: f64,
    pub e: Vec<String>,
    pub m: Vec<String>,
}

pub fn load_chat_messages(chat_path: &Path) -> AppResult<Vec<ChatMessageLite>> {
    if !chat_path.is_file() {
        return Ok(Vec::new());
    }
    let raw_bytes = fs::read(chat_path)?; let raw = String::from_utf8_lossy(&raw_bytes).trim_start_matches('\u{feff}').to_string();
    let val: serde_json::Value = serde_json::from_str(&raw)?;
    let Some(comments) = val.get("comments").and_then(|c| c.as_array()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::with_capacity(comments.len());
    for item in comments {
        let offset_sec = item
            .get("content_offset_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let author = item
            .pointer("/commenter/display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("anon")
            .to_string();
        let body = item
            .pointer("/message/body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if body.is_empty() {
            continue;
        }
        let bits_spent = item
            .pointer("/message/bits_spent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32;
        let has_notice = item
            .pointer("/message/user_notice_params")
            .and_then(|v| v.as_object())
            .map(|m| !m.is_empty())
            .unwrap_or(false);
        let lower = body.to_ascii_lowercase();
        let is_event = bits_spent > 0
            || has_notice
            || lower.contains("subscribed")
            || lower.contains("gifted")
            || lower.contains("raid")
            || lower.contains("hype train");

        out.push(ChatMessageLite {
            offset_sec,
            author,
            body,
            bits_spent,
            is_event,
        });
    }
    Ok(out)
}

pub fn extract_local_signals(
    audio_wav_path: &Path,
    chat_messages: &[ChatMessageLite],
    muted_segments: &[MutedSegment],
    cfg: &SignalWeightsConfig,
    fallback_duration_sec: f64,
) -> AppResult<Vec<SignalWindowRecord>> {
    let step_sec = 5.0f64;

    let (audio_duration, audio_metrics) = if audio_wav_path.is_file() {
        let wav_bytes = fs::read(audio_wav_path)?;
        let (sr, samples) = parse_pcm16_mono_wav(&wav_bytes)?;
        let dur = if sr > 0 {
            samples.len() as f64 / sr as f64
        } else {
            fallback_duration_sec
        };
        let win_samples = (sr as f64 * step_sec) as usize;
        let n_win = ((dur / step_sec).ceil() as usize).max(1);
        let mut metrics = Vec::with_capacity(n_win);

        for w in 0..n_win {
            let start = w * win_samples;
            let end = ((w + 1) * win_samples).min(samples.len());
            if start >= end {
                metrics.push((-70.0, 0.0, 0.0));
                continue;
            }
            let slice = &samples[start..end];
            metrics.push(analyze_audio_chunk(slice, sr));
        }
        (dur, metrics)
    } else {
        let n_win = ((fallback_duration_sec / step_sec).ceil() as usize).max(1);
        (fallback_duration_sec, vec![(-70.0, 0.0, 0.0); n_win])
    };

    let chat_max = chat_messages
        .iter()
        .map(|m| m.offset_sec)
        .fold(0.0f64, f64::max);
    let total_duration = audio_duration.max(chat_max).max(fallback_duration_sec).max(step_sec);
    let num_windows = (total_duration / step_sec).ceil() as usize;

    let emotes_set: HashSet<String> = cfg
        .emotes_list
        .iter()
        .map(|e| e.to_ascii_lowercase())
        .collect();
    let keywords_lower: Vec<String> = cfg
        .keywords_list
        .iter()
        .map(|k| k.to_ascii_lowercase())
        .collect();

    let mut chat_counts = vec![0u32; num_windows];
    let mut emote_hits = vec![0.0f64; num_windows];
    let mut event_hits = vec![0.0f64; num_windows];

    for msg in chat_messages {
        let idx = ((msg.offset_sec / step_sec).floor() as isize).clamp(0, num_windows as isize - 1) as usize;
        chat_counts[idx] += 1;

        let lower = msg.body.to_ascii_lowercase();
        let mut msg_emotes = 0.0f64;
        for token in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if !token.is_empty() && emotes_set.contains(token) {
                msg_emotes += 1.0;
            }
        }
        for kw in &keywords_lower {
            if lower.contains(kw) {
                msg_emotes += 0.6;
            }
        }
        emote_hits[idx] += msg_emotes.min(4.0);

        if msg.is_event {
            let bits_bonus = (msg.bits_spent as f64 / 200.0).min(5.0);
            event_hits[idx] += 1.0 + bits_bonus;
        }
    }

    let mut windows = Vec::with_capacity(num_windows);
    for i in 0..num_windows {
        let start_sec = i as f64 * step_sec;
        let end_sec = ((i + 1) as f64 * step_sec).min(total_duration);
        let (audio_lufs, audio_peak, voice_ratio) = audio_metrics
            .get(i)
            .copied()
            .unwrap_or((-70.0, 0.0, 0.0));

        let is_muted = muted_segments
            .iter()
            .any(|seg| seg.start_sec < end_sec && seg.end_sec > start_sec);

        windows.push(SignalWindowRecord {
            start_sec,
            end_sec,
            chat_rate: chat_counts[i] as f64 / step_sec,
            emote_score: emote_hits[i] / step_sec,
            event_score: event_hits[i],
            audio_lufs,
            audio_peak,
            voice_ratio,
            scene_change: 0.0,
            viewer_clips_count: 0,
            is_muted,
            composite_score: 0.0,
        });
    }

    Ok(windows)
}

fn analyze_audio_chunk(samples: &[i16], sample_rate: u32) -> (f64, f64, f64) {
    if samples.is_empty() || sample_rate == 0 {
        return (-70.0, 0.0, 0.0);
    }

    let mut sum_sq = 0.0f64;
    let mut peak = 0.0f64;
    for &s in samples {
        let norm = s as f64 / 32768.0;
        let abs = norm.abs();
        if abs > peak {
            peak = abs;
        }
        sum_sq += norm * norm;
    }
    let rms = (sum_sq / samples.len() as f64).sqrt();
    let lufs = if rms > 1e-6 {
        (20.0 * rms.log10() - 0.691).clamp(-70.0, 0.0)
    } else {
        -70.0
    };

    let frame_len = ((sample_rate as usize) * 30 / 1000).max(160);
    let mut voice_frames = 0usize;
    let mut total_frames = 0usize;

    for frame in samples.chunks_exact(frame_len) {
        total_frames += 1;
        let mut f_sq = 0.0f64;
        let mut zcr = 0usize;
        let mut prev_pos = frame[0] >= 0;
        for &s in frame {
            let n = s as f64 / 32768.0;
            f_sq += n * n;
            let pos = s >= 0;
            if pos != prev_pos {
                zcr += 1;
                prev_pos = pos;
            }
        }
        let f_rms = (f_sq / frame_len as f64).sqrt();
        let zcr_rate = zcr as f64 / frame_len as f64;
        if f_rms > 0.014 && (0.015..=0.35).contains(&zcr_rate) {
            voice_frames += 1;
        }
    }

    let voice_ratio = if total_frames > 0 {
        voice_frames as f64 / total_frames as f64
    } else {
        0.0
    };

    (lufs, peak, voice_ratio)
}

pub fn summarize_chat_for_window(
    messages: &[ChatMessageLite],
    start_sec: f64,
    end_sec: f64,
    emotes_list: &[String],
) -> CompactChatSummary {
    let dur = (end_sec - start_sec).max(1.0);
    let emotes_lookup: HashMap<String, String> = emotes_list
        .iter()
        .map(|e| (e.to_ascii_lowercase(), e.clone()))
        .collect();

    let mut count = 0usize;
    let mut emote_counts: HashMap<String, usize> = HashMap::new();
    let mut seen_norm = HashSet::new();
    let mut sample_msgs: Vec<String> = Vec::new();

    for msg in messages {
        if msg.offset_sec < start_sec || msg.offset_sec > end_sec {
            continue;
        }
        count += 1;

        for token in msg.body.split(|c: char| !c.is_alphanumeric() && c != '_') {
            let lower = token.to_ascii_lowercase();
            if let Some(canonical) = emotes_lookup.get(&lower) {
                *emote_counts.entry(canonical.clone()).or_insert(0) += 1;
            }
        }

        let cleaned = msg.body.split_whitespace().collect::<Vec<_>>().join(" ");
        let norm = cleaned.to_ascii_lowercase();
        if sample_msgs.len() < 3
            && cleaned.len() >= 10
            && cleaned.len() <= 80
            && seen_norm.insert(norm)
        {
            sample_msgs.push(cleaned);
        }
    }

    let mut top_emotes: Vec<(String, usize)> = emote_counts.into_iter().collect();
    top_emotes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let e = top_emotes
        .into_iter()
        .take(5)
        .map(|(name, c)| format!("{name} x{c}"))
        .collect();

    CompactChatSummary {
        r: ((count as f64 / dur) * 10.0).round() / 10.0,
        e,
        m: sample_msgs,
    }
}
