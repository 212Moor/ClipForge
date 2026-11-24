use crate::signals::SignalWeightsConfig;
use crate::store::projects::SignalWindowRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredCandidate {
    pub rank_index: u32,
    pub start_sec: f64,
    pub end_sec: f64,
    pub peak_sec: f64,
    pub local_score: f64,
    pub compact_scores: [f64; 2],
    pub signals_summary_json: String,
}

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) * 0.5
    }
}

pub fn rolling_robust_normalize(series: &[f64], half_window: usize) -> Vec<f64> {
    let n = series.len();
    if n == 0 {
        return Vec::new();
    }
    let global_max = series.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let global_min = series.iter().copied().fold(f64::INFINITY, f64::min);
    let global_span = (global_max - global_min).max(1e-6);

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(half_window);
        let hi = (i + half_window + 1).min(n);
        let slice = &series[lo..hi];
        let med = median(slice);
        let deviations: Vec<f64> = slice.iter().map(|&v| (v - med).abs()).collect();
        let mad = median(&deviations);

        let z = if mad > 1e-6 {
            (series[i] - med) / (1.4826 * mad)
        } else {
            ((series[i] - med) / (global_span * 0.25)).clamp(-4.0, 8.0)
        };

        let normalized = 1.0 / (1.0 + (-1.0 * (z - 2.0)).exp());
        out.push(normalized.clamp(0.0, 1.0));
    }
    out
}

pub fn smooth_series(series: &[f64]) -> Vec<f64> {
    let n = series.len();
    if n < 3 {
        return series.to_vec();
    }
    let mut smoothed = Vec::with_capacity(n);
    for i in 0..n {
        let prev = if i > 0 { series[i - 1] } else { series[i] };
        let curr = series[i];
        let next = if i + 1 < n { series[i + 1] } else { series[i] };
        smoothed.push(0.2 * prev + 0.6 * curr + 0.2 * next);
    }
    smoothed
}

pub fn score_and_select_candidates(
    windows: &mut [SignalWindowRecord],
    cfg: &SignalWeightsConfig,
    target_clips: u32,
    target_min_sec: u32,
    target_max_sec: u32,
) -> Vec<ScoredCandidate> {
    if windows.is_empty() {
        return Vec::new();
    }

    let half_window = 180usize.min(windows.len().max(1));

    let chat_raw: Vec<f64> = windows.iter().map(|w| w.chat_rate).collect();
    let audio_raw: Vec<f64> = windows
        .iter()
        .map(|w| (w.audio_lufs + 70.0).max(0.0) + w.audio_peak * 25.0)
        .collect();
    let emote_raw: Vec<f64> = windows.iter().map(|w| w.emote_score).collect();
    let event_raw: Vec<f64> = windows.iter().map(|w| w.event_score).collect();
    let voice_raw: Vec<f64> = windows.iter().map(|w| w.voice_ratio).collect();
    let scene_raw: Vec<f64> = windows.iter().map(|w| w.scene_change).collect();

    // Détection "petit stream" : adapte automatiquement les poids si le chat est très clairsemé
    let non_muted_count = windows.iter().filter(|w| !w.is_muted).count().max(1);
    let chat_active_windows = windows
        .iter()
        .filter(|w| !w.is_muted && w.chat_rate > 0.0)
        .count();
    let total_chat_msgs: f64 = windows
        .iter()
        .filter(|w| !w.is_muted)
        .map(|w| w.chat_rate * (w.end_sec - w.start_sec).max(1.0))
        .sum();
    let sparse_chat =
        total_chat_msgs < 80.0 || (chat_active_windows as f64 / non_muted_count as f64) < 0.04;

    // Dynamique vocale locale : contraste soudain par rapport aux 100 s précédentes, pondéré par la voix
    let mut vocal_burst_raw = Vec::with_capacity(windows.len());
    let mut sustained_voice_raw = Vec::with_capacity(windows.len());
    for i in 0..windows.len() {
        let lo = i.saturating_sub(20);
        let prev_med = if i > lo {
            median(&audio_raw[lo..i])
        } else {
            audio_raw[i]
        };
        let pos_delta = (audio_raw[i] - prev_med).max(0.0);
        let burst = pos_delta * (0.25 + 0.75 * voice_raw[i]) + voice_raw[i] * 10.0;
        vocal_burst_raw.push(burst);

        let slo = i.saturating_sub(1);
        let shi = (i + 2).min(windows.len());
        let mut s_sum = 0.0;
        for j in slo..shi {
            let energy = ((windows[j].audio_lufs + 55.0).max(0.0) / 55.0).clamp(0.0, 1.0);
            s_sum += voice_raw[j] * (0.4 + 0.6 * energy);
        }
        sustained_voice_raw.push(s_sum / (shi - slo).max(1) as f64);
    }

    let chat_n = rolling_robust_normalize(&chat_raw, half_window);
    let audio_n = rolling_robust_normalize(&audio_raw, half_window);
    let emote_n = rolling_robust_normalize(&emote_raw, half_window);
    let event_n = rolling_robust_normalize(&event_raw, half_window);
    let voice_n = rolling_robust_normalize(&voice_raw, half_window);
    let scene_n = rolling_robust_normalize(&scene_raw, half_window);
    let vocal_burst_n = rolling_robust_normalize(&vocal_burst_raw, half_window);
    let sustained_voice_n = rolling_robust_normalize(&sustained_voice_raw, half_window);

    let weight_sum = (cfg.chat + cfg.audio + cfg.emotes + cfg.events + cfg.voice + cfg.scene).max(1e-6);

    let mut raw_composite = Vec::with_capacity(windows.len());
    for i in 0..windows.len() {
        if windows[i].is_muted {
            raw_composite.push(0.0);
            continue;
        }
        let viewer_boost = if windows[i].viewer_clips_count > 0 {
            0.25 * (windows[i].viewer_clips_count as f64).min(3.0)
        } else {
            0.0
        };

        let base_score = if sparse_chat {
            let chat_scale = (total_chat_msgs / 80.0).clamp(0.08, 1.0);
            let w_chat = cfg.chat * chat_scale;
            let w_emotes = cfg.emotes * chat_scale;
            let w_events = cfg.events * chat_scale;
            let freed = (cfg.chat - w_chat) + (cfg.emotes - w_emotes) + (cfg.events - w_events);
            let w_burst = freed * 0.60;
            let w_sustained = freed * 0.40;
            let eff_sum = (w_chat + w_emotes + w_events + cfg.audio + cfg.voice + cfg.scene + w_burst + w_sustained).max(1e-6);

            let mut s = (w_chat * chat_n[i]
                + cfg.audio * audio_n[i]
                + w_emotes * emote_n[i]
                + w_events * event_n[i]
                + cfg.voice * voice_n[i]
                + cfg.scene * scene_n[i]
                + w_burst * vocal_burst_n[i]
                + w_sustained * sustained_voice_n[i])
                / eff_sum;

            // Pénalise les bruits forts du jeu sans voix (explosions, sirènes, crashs muets)
            if voice_raw[i] < 0.15 {
                s *= 0.55;
            }
            // Bonus dès qu'un rare message de chat apparaît autour du pic (±5 s)
            let near_chat = (i.saturating_sub(1)..=(i + 1).min(windows.len() - 1))
                .any(|j| windows[j].chat_rate > 0.0);
            if near_chat {
                s += 0.18;
            }
            s
        } else {
            (cfg.chat * chat_n[i]
                + cfg.audio * audio_n[i]
                + cfg.emotes * emote_n[i]
                + cfg.events * event_n[i]
                + cfg.voice * voice_n[i]
                + cfg.scene * scene_n[i])
                / weight_sum
        };

        raw_composite.push((base_score + viewer_boost).clamp(0.0, 1.0));
    }

    let smoothed = smooth_series(&raw_composite);
    for (i, w) in windows.iter_mut().enumerate() {
        w.composite_score = if w.is_muted { 0.0 } else { smoothed[i] };
    }

    let mut peak_indices: Vec<usize> = Vec::new();
    for i in 0..windows.len() {
        if windows[i].is_muted || windows[i].composite_score <= 0.05 {
            continue;
        }
        let left = if i > 0 { windows[i - 1].composite_score } else { -1.0 };
        let right = if i + 1 < windows.len() {
            windows[i + 1].composite_score
        } else {
            -1.0
        };
        if windows[i].composite_score >= left && windows[i].composite_score >= right {
            peak_indices.push(i);
        }
    }

    peak_indices.sort_by(|&a, &b| {
        windows[b]
            .composite_score
            .partial_cmp(&windows[a].composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let vod_hours = (windows.last().map(|w| w.end_sec).unwrap_or(0.0) / 3600.0).max(0.5); let max_candidates = ((target_clips.max(1) * cfg.candidate_multiplier.max(1)) as usize).max((vod_hours * 8.0).round() as usize).clamp(24, 300);
    let min_dist = cfg.min_peak_distance_sec.max(30.0);
    let total_vod_end = windows.last().map(|w| w.end_sec).unwrap_or(0.0);
    let min_len = (target_min_sec as f64).max(cfg.min_clip_sec).clamp(20.0, 75.0);
    let max_len = (target_max_sec as f64).min(cfg.max_clip_sec).clamp(min_len, 75.0);
    let target_len = ((min_len + max_len) * 0.5).clamp(20.0, 75.0);

    let mut chosen_peaks: Vec<usize> = Vec::new();
    for idx in peak_indices {
        let peak_t = (windows[idx].start_sec + windows[idx].end_sec) * 0.5;
        let too_close = chosen_peaks.iter().any(|&other| {
            let other_t = (windows[other].start_sec + windows[other].end_sec) * 0.5;
            (peak_t - other_t).abs() < min_dist
        });
        if too_close {
            continue;
        }

        let cand_start = (peak_t - cfg.pre_peak_lead_sec).max(0.0);
        let cand_end = (cand_start + target_len).min(total_vod_end);
        let overlaps_muted = windows
            .iter()
            .any(|w| w.is_muted && w.start_sec < cand_end && w.end_sec > cand_start);
        if overlaps_muted {
            continue;
        }

        chosen_peaks.push(idx);
        if chosen_peaks.len() >= max_candidates {
            break;
        }
    }

    let mut candidates = Vec::with_capacity(chosen_peaks.len());
    for (rank, idx) in chosen_peaks.into_iter().enumerate() {
        let peak_sec = (windows[idx].start_sec + windows[idx].end_sec) * 0.5;
        let mut start_sec = (peak_sec - cfg.pre_peak_lead_sec).max(0.0);
        let mut end_sec = (start_sec + target_len).min(total_vod_end);
        if end_sec - start_sec < min_len && total_vod_end >= min_len {
            start_sec = (end_sec - min_len).max(0.0);
            end_sec = (start_sec + min_len).min(total_vod_end);
        }

        let chat_score = (chat_n[idx] * 100.0).round() / 100.0;
        let audio_score = (audio_n[idx] * 100.0).round() / 100.0;
        let summary_json = serde_json::to_string(&serde_json::json!({
            "chat_norm": chat_score,
            "audio_norm": audio_score,
            "emote_norm": (emote_n[idx] * 100.0).round() / 100.0,
            "event_norm": (event_n[idx] * 100.0).round() / 100.0,
            "voice_norm": (voice_n[idx] * 100.0).round() / 100.0,
            "vocal_burst_norm": (vocal_burst_n[idx] * 100.0).round() / 100.0,
            "sparse_chat_mode": sparse_chat,
            "chat_rate": windows[idx].chat_rate,
            "audio_lufs": windows[idx].audio_lufs,
        }))
        .unwrap_or_else(|_| "{}".to_string());

        candidates.push(ScoredCandidate {
            rank_index: (rank + 1) as u32,
            start_sec,
            end_sec,
            peak_sec,
            local_score: (windows[idx].composite_score * 1000.0).round() / 1000.0,
            compact_scores: [chat_score, audio_score],
            signals_summary_json: summary_json,
        });
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_window(i: usize, chat: f64, lufs: f64, muted: bool) -> SignalWindowRecord {
        SignalWindowRecord {
            start_sec: i as f64 * 5.0,
            end_sec: (i + 1) as f64 * 5.0,
            chat_rate: chat,
            emote_score: chat * 0.5,
            event_score: 0.0,
            audio_lufs: lufs,
            audio_peak: if lufs > -20.0 { 0.85 } else { 0.15 },
            voice_ratio: 0.6,
            scene_change: 0.0,
            viewer_clips_count: 0,
            is_muted: muted,
            composite_score: 0.0,
        }
    }

    #[test]
    fn rolling_normalization_highlights_spikes() {
        let mut vals = vec![1.0; 40];
        vals[20] = 25.0;
        let norm = rolling_robust_normalize(&vals, 15);
        assert!(norm[20] > 0.85);
        assert!(norm[5] < 0.3);
    }

    #[test]
    fn enforces_min_distance_and_excludes_muted_zones() {
        let mut windows: Vec<SignalWindowRecord> = (0..120)
            .map(|i| make_window(i, 0.4, -34.0, (60..70).contains(&i)))
            .collect();

        windows[13].chat_rate = 18.0;
        windows[13].emote_score = 12.0;
        windows[13].audio_lufs = -10.0;
        windows[17].chat_rate = 14.0;
        windows[17].emote_score = 9.0;
        windows[17].audio_lufs = -12.0;

        windows[64].chat_rate = 40.0;
        windows[64].audio_lufs = -8.0;

        windows[90].chat_rate = 22.0;
        windows[90].emote_score = 15.0;
        windows[90].audio_lufs = -9.0;

        let cfg = SignalWeightsConfig::default();
        let candidates = score_and_select_candidates(&mut windows, &cfg, 4, 30, 50);

        assert!(candidates.len() >= 2);
        for c in &candidates {
            assert!(c.end_sec <= 300.0 || c.start_sec >= 350.0);
            assert!(c.end_sec - c.start_sec >= 20.0 && c.end_sec - c.start_sec <= 75.0);
        }
        for i in 0..candidates.len() {
            for j in (i + 1)..candidates.len() {
                assert!((candidates[i].peak_sec - candidates[j].peak_sec).abs() >= 90.0);
            }
        }
    }

    #[test]
    fn adapts_to_sparse_chat_and_favors_vocal_reactions() {
        let mut windows: Vec<SignalWindowRecord> = (0..120)
            .map(|i| {
                let mut w = make_window(i, 0.0, -36.0, false);
                w.voice_ratio = 0.25;
                w
            })
            .collect();

        // Explosion forte sans voix
        windows[25].audio_lufs = -8.0;
        windows[25].audio_peak = 0.95;
        windows[25].voice_ratio = 0.05;

        // Vraie réaction vocale du streamer
        windows[80].audio_lufs = -11.0;
        windows[80].audio_peak = 0.88;
        windows[80].voice_ratio = 0.92;
        windows[81].audio_lufs = -13.0;
        windows[81].voice_ratio = 0.88;

        let cfg = SignalWeightsConfig::default();
        let cands = score_and_select_candidates(&mut windows, &cfg, 2, 25, 45);
        assert!(!cands.is_empty());
        assert!(windows[80].composite_score > windows[25].composite_score);
    }
}