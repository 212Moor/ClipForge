use crate::asr::{TranscriptWord, WindowTranscript};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SubtitleStylePreset {
    pub id: String,
    pub label: String,
    pub font_name: String,
    pub font_size: u32,
    pub primary_color_hex: String,
    pub highlight_color_hex: String,
    pub outline_color_hex: String,
    pub back_color_hex: String,
    pub border_style: u32,
    pub outline_width: u32,
    pub shadow_depth: u32,
    pub bold: bool,
    pub uppercase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SubtitleStyleCustom {
    pub style_id: String,
    pub font_name: Option<String>,
    pub primary_color_hex: Option<String>,
    pub highlight_color_hex: Option<String>,
    pub font_size: Option<u32>,
    #[serde(default)]
    pub hide_subtitles: bool,
    #[serde(default)]
    pub hide_hook: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct EditableSubtitleCue {
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ClipOverridesPayload {
    #[serde(default)]
    pub cues: Vec<EditableSubtitleCue>,
    #[serde(default)]
    pub pan_x: f64,
    pub font_name: Option<String>,
    pub primary_color_hex: Option<String>,
    pub highlight_color_hex: Option<String>,
    pub font_size: Option<u32>,
    #[serde(default)]
    pub hide_subtitles: bool,
    #[serde(default)]
    pub hide_hook: bool,
}

impl Default for ClipOverridesPayload {
    fn default() -> Self {
        Self {
            cues: Vec::new(),
            pan_x: 0.0,
            font_name: None,
            primary_color_hex: None,
            highlight_color_hex: None,
            font_size: None,
            hide_subtitles: false,
            hide_hook: false,
        }
    }
}

/// 5 styles prédéfinis adaptés au format vertical 1080x1920 TikTok.
pub fn builtin_subtitle_presets() -> Vec<SubtitleStylePreset> {
    vec![
        SubtitleStylePreset {
            id: "bold_pop".to_string(),
            label: "Bold Pop (TikTok classique)".to_string(),
            font_name: "Segoe UI".to_string(),
            font_size: 74,
            primary_color_hex: "#FFFFFF".to_string(),
            highlight_color_hex: "#FACC15".to_string(),
            outline_color_hex: "#0A0A0F".to_string(),
            back_color_hex: "#000000".to_string(),
            border_style: 1,
            outline_width: 6,
            shadow_depth: 3,
            bold: true,
            uppercase: true,
        },
        SubtitleStylePreset {
            id: "neon_cyan".to_string(),
            label: "Néon Cyan & Violet".to_string(),
            font_name: "Segoe UI".to_string(),
            font_size: 70,
            primary_color_hex: "#F8FAFC".to_string(),
            highlight_color_hex: "#22D3EE".to_string(),
            outline_color_hex: "#3B0764".to_string(),
            back_color_hex: "#0F172A".to_string(),
            border_style: 1,
            outline_width: 5,
            shadow_depth: 4,
            bold: true,
            uppercase: true,
        },
        SubtitleStylePreset {
            id: "clean_minimal".to_string(),
            label: "Clean Minimal".to_string(),
            font_name: "Segoe UI".to_string(),
            font_size: 62,
            primary_color_hex: "#FFFFFF".to_string(),
            highlight_color_hex: "#A78BFA".to_string(),
            outline_color_hex: "#111827".to_string(),
            back_color_hex: "#000000".to_string(),
            border_style: 1,
            outline_width: 3,
            shadow_depth: 1,
            bold: true,
            uppercase: false,
        },
        SubtitleStylePreset {
            id: "boxed_dark".to_string(),
            label: "Cartouche sombre".to_string(),
            font_name: "Segoe UI".to_string(),
            font_size: 64,
            primary_color_hex: "#F8FAFC".to_string(),
            highlight_color_hex: "#34D399".to_string(),
            outline_color_hex: "#09090B".to_string(),
            back_color_hex: "#09090B".to_string(),
            border_style: 3,
            outline_width: 10,
            shadow_depth: 0,
            bold: true,
            uppercase: false,
        },
        SubtitleStylePreset {
            id: "comic_yellow".to_string(),
            label: "Hype Jaune Punchy".to_string(),
            font_name: "Impact".to_string(),
            font_size: 78,
            primary_color_hex: "#FDE047".to_string(),
            highlight_color_hex: "#FB7185".to_string(),
            outline_color_hex: "#000000".to_string(),
            back_color_hex: "#000000".to_string(),
            border_style: 1,
            outline_width: 7,
            shadow_depth: 4,
            bold: true,
            uppercase: true,
        },
    ]
}

pub fn resolve_preset(style_id: &str, overrides: &ClipOverridesPayload) -> SubtitleStylePreset {
    let mut preset = builtin_subtitle_presets()
        .into_iter()
        .find(|p| p.id == style_id)
        .unwrap_or_else(|| builtin_subtitle_presets().remove(0));

    if let Some(ref font) = overrides.font_name {
        if !font.trim().is_empty() {
            preset.font_name = font.trim().to_string();
        }
    }
    if let Some(ref primary) = overrides.primary_color_hex {
        if is_hex_color(primary) {
            preset.primary_color_hex = primary.trim().to_string();
        }
    }
    if let Some(ref hl) = overrides.highlight_color_hex {
        if is_hex_color(hl) {
            preset.highlight_color_hex = hl.trim().to_string();
        }
    }
    if let Some(size) = overrides.font_size {
        preset.font_size = size.clamp(36, 110);
    }
    preset
}

fn is_hex_color(s: &str) -> bool {
    let hex = s.trim().trim_start_matches('#');
    hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Convertit `#RRGGBB` + alpha (00 = opaque, FF = transparent en ASS) vers `&HAABBGGRR`.
pub fn hex_to_ass_color(hex_rgb: &str, alpha_hex: &str) -> String {
    let clean = hex_rgb.trim().trim_start_matches('#');
    if clean.len() != 6 || !clean.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!("&H{alpha_hex}FFFFFF");
    }
    let r = &clean[0..2];
    let g = &clean[2..4];
    let b = &clean[4..6];
    format!("&H{alpha_hex}{b}{g}{r}").to_ascii_uppercase()
}

/// Formate une durée en secondes vers le format ASS `H:MM:SS.cc` (centièmes de seconde).
pub fn format_ass_time(sec: f64) -> String {
    let total_cs = (sec.max(0.0) * 100.0).round() as u64;
    let cs = total_cs % 100;
    let total_sec = total_cs / 100;
    let s = total_sec % 60;
    let m = (total_sec / 60) % 60;
    let h = total_sec / 3600;
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

/// Échappe tout texte utilisateur/IA avant écriture dans le fichier ASS (aucune injection de tag ASS).
pub fn escape_ass_text(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('{', "(")
        .replace('}', ")")
        .replace('\r', "")
        .replace('\n', "\\N")
}

/// Convertit une transcription Whisper ou des cues édités en liste de mots datés relatifs à `[0, clip_dur]`.
pub fn extract_relative_words(
    transcript: Option<&WindowTranscript>,
    overrides: &ClipOverridesPayload,
    clip_start_sec: f64,
    clip_end_sec: f64,
) -> Vec<TranscriptWord> {
    let clip_dur = (clip_end_sec - clip_start_sec).max(0.5);

    // Priorité aux sous-titres édités manuellement par l'utilisateur
    if !overrides.cues.is_empty() {
        let mut words = Vec::new();
        for cue in &overrides.cues {
            let s = (cue.start_sec - clip_start_sec).clamp(0.0, clip_dur);
            let e = (cue.end_sec - clip_start_sec).clamp(s + 0.1, clip_dur);
            let tokens: Vec<&str> = cue.text.split_whitespace().collect();
            if tokens.is_empty() {
                continue;
            }
            let step = (e - s) / tokens.len() as f64;
            for (i, tok) in tokens.iter().enumerate() {
                words.push(TranscriptWord {
                    word: (*tok).to_string(),
                    start_sec: s + i as f64 * step,
                    end_sec: (s + (i + 1) as f64 * step).min(clip_dur),
                });
            }
        }
        return words;
    }

    let Some(tr) = transcript else {
        return Vec::new();
    };

    let mut words = Vec::new();
    for seg in &tr.segments {
        if seg.end_sec <= clip_start_sec || seg.start_sec >= clip_end_sec {
            continue;
        }
        if !seg.words.is_empty() {
            for w in &seg.words {
                if w.end_sec <= clip_start_sec || w.start_sec >= clip_end_sec {
                    continue;
                }
                let clean = w.word.trim();
                if clean.is_empty() {
                    continue;
                }
                let s = (w.start_sec - clip_start_sec).clamp(0.0, clip_dur);
                let e = (w.end_sec - clip_start_sec).clamp(s + 0.08, clip_dur);
                words.push(TranscriptWord {
                    word: clean.to_string(),
                    start_sec: s,
                    end_sec: e,
                });
            }
        } else {
            let s = (seg.start_sec - clip_start_sec).clamp(0.0, clip_dur);
            let e = (seg.end_sec - clip_start_sec).clamp(s + 0.2, clip_dur);
            let tokens: Vec<&str> = seg.text.split_whitespace().collect();
            if tokens.is_empty() {
                continue;
            }
            let step = (e - s) / tokens.len() as f64;
            for (i, tok) in tokens.iter().enumerate() {
                words.push(TranscriptWord {
                    word: (*tok).to_string(),
                    start_sec: s + i as f64 * step,
                    end_sec: (s + (i + 1) as f64 * step).min(clip_dur),
                });
            }
        }
    }

    words
}

/// Génère un fichier ASS complet 1080×1920 :
/// - Zones de sécurité TikTok respectées (MarginV=430 > 20 % du bas = 384 px ; MarginR=170 pour la bande droite)
/// - Sous-titres mot par mot avec mise en évidence couleur du mot actif
/// - Accroche (hook) affichée pendant les 2,8 premières secondes dans la zone sûre supérieure
pub fn generate_ass_document(
    style_id: &str,
    overrides: &ClipOverridesPayload,
    hook_text: &str,
    transcript: Option<&WindowTranscript>,
    clip_start_sec: f64,
    clip_end_sec: f64,
) -> String {
    let preset = resolve_preset(style_id, overrides);
    let primary_ass = hex_to_ass_color(&preset.primary_color_hex, "00");
    let highlight_ass = hex_to_ass_color(&preset.highlight_color_hex, "00");
    let outline_ass = hex_to_ass_color(&preset.outline_color_hex, "00");
    let back_alpha = if preset.border_style == 3 { "40" } else { "78" };
    let back_ass = hex_to_ass_color(&preset.back_color_hex, back_alpha);
    let bold_flag = if preset.bold { -1 } else { 0 };

    // Zone de sécurité TikTok : rien dans les 20 % du bas (384 px) ni sur les ~15 % de droite (162 px)
    let margin_l = 76;
    let margin_r = 172;
    let margin_v_sub = 435;
    let _margin_v_hook = 290;

    let mut doc = String::new();
    doc.push_str("[Script Info]\n");
    doc.push_str("ScriptType: v4.00+\n");
    doc.push_str("PlayResX: 1080\n");
    doc.push_str("PlayResY: 1920\n");
    doc.push_str("WrapStyle: 2\n");
    doc.push_str("ScaledBorderAndShadow: yes\n\n");

    doc.push_str("[V4+ Styles]\n");
    doc.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    doc.push_str(&format!(
        "Style: WordSub,{},{},{},{},{},{},{},0,0,0,100,100,0,0,{},{},{},2,{},{},{},1\n",
        preset.font_name,
        preset.font_size,
        primary_ass,
        highlight_ass,
        outline_ass,
        back_ass,
        bold_flag,
        preset.border_style,
        preset.outline_width,
        preset.shadow_depth,
        margin_l,
        margin_r,
        margin_v_sub
    ));
    doc.push_str(&format!(
        "Style: HookBanner,{},62,&H00FFFFFF,&H0022D3EE,&H00120B29,&H25120B29,-1,0,0,0,100,100,0,0,3,12,0,8,{},{},{},1\n\n",
        preset.font_name, 72, 72, 220
    ));

    doc.push_str("[Events]\n");
    doc.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    // 1. Accroche (hook) 0.15s -> 2.85s (zone sûre haute)
    let trimmed_hook = hook_text.trim();
    if !overrides.hide_hook && !trimmed_hook.is_empty() {
        let clip_dur = (clip_end_sec - clip_start_sec).max(1.0);
        let hook_end = 2.85f64.min(clip_dur);
        let escaped_hook = escape_ass_text(&trimmed_hook.to_uppercase());
        doc.push_str(&format!(
            "Dialogue: 2,{},{},HookBanner,,0,0,0,,{{\\fad(160,260)}}{}\n",
            format_ass_time(0.15),
            format_ass_time(hook_end),
            escaped_hook
        ));
    }

    // 2. Sous-titres mot par mot regroupés par groupes de 4 mots maximum pour une lecture fluide
    if overrides.hide_subtitles || style_id == "none" { return doc; }
    let words = extract_relative_words(transcript, overrides, clip_start_sec, clip_end_sec);
    // En ASS, \1c&HBBGGRR& utilise uniquement les 6 derniers caractères (sans alpha)
    let primary_bgr = &primary_ass[primary_ass.len().saturating_sub(6)..];
    let highlight_bgr = &highlight_ass[highlight_ass.len().saturating_sub(6)..];

    for chunk in words.chunks(4) {
        for (active_idx, active_word) in chunk.iter().enumerate() {
            let start_s = active_word.start_sec;
            let end_s = if active_idx + 1 < chunk.len() {
                chunk[active_idx + 1].start_sec.max(active_word.end_sec)
            } else {
                active_word.end_sec
            };
            if end_s <= start_s + 0.02 {
                continue;
            }

            let mut line = String::new();
            for (wi, w) in chunk.iter().enumerate() {
                if wi > 0 {
                    line.push(' ');
                }
                let raw_text = if preset.uppercase {
                    w.word.to_uppercase()
                } else {
                    w.word.clone()
                };
                let escaped = escape_ass_text(&raw_text);
                if wi == active_idx {
                    line.push_str(&format!(
                        "{{\\1c&H{highlight_bgr}&\\fscx108\\fscy108}}{escaped}{{\\1c&H{primary_bgr}&\\fscx100\\fscy100}}"
                    ));
                } else {
                    line.push_str(&escaped);
                }
            }

            doc.push_str(&format!(
                "Dialogue: 1,{},{},WordSub,,0,0,0,,{}\n",
                format_ass_time(start_s),
                format_ass_time(end_s),
                line
            ));
        }
    }

    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::TranscriptSegment;

    #[test]
    fn generates_word_highlighted_ass_within_tiktok_safe_zones() {
        let tr = WindowTranscript {
            language: "fr".to_string(),
            window_start_sec: 100.0,
            window_end_sec: 130.0,
            compact_text: "[0]Quel clutch incroyable".to_string(),
            segments: vec![TranscriptSegment {
                start_sec: 100.0,
                end_sec: 103.0,
                text: "Quel clutch incroyable".to_string(),
                words: vec![
                    TranscriptWord { word: "Quel".to_string(), start_sec: 100.0, end_sec: 100.8 },
                    TranscriptWord { word: "clutch".to_string(), start_sec: 100.8, end_sec: 101.6 },
                    TranscriptWord { word: "incroyable".to_string(), start_sec: 101.6, end_sec: 102.9 },
                ],
            }],
        };

        let ass = generate_ass_document(
            "bold_pop",
            &ClipOverridesPayload::default(),
            "LE CLUTCH 1V4",
            Some(&tr),
            100.0,
            130.0,
        );

        assert!(ass.contains("PlayResX: 1080"));
        assert!(ass.contains("PlayResY: 1920"));
        // MarginV = 435 (> 384 px = 20% bas TikTok), MarginR = 172 (> 15% droite)
        assert!(ass.contains(",76,172,435,1"));
        assert!(ass.contains("HookBanner"));
        assert!(ass.contains("LE CLUTCH 1V4"));
        // 3 dialogues pour les 3 mots actifs + 1 pour le hook
        let dialogue_count = ass.lines().filter(|l| l.starts_with("Dialogue:")).count();
        assert_eq!(dialogue_count, 4);
    }

    #[test]
    fn converts_hex_rgb_to_ass_bgr() {
        assert_eq!(hex_to_ass_color("#22D3EE", "00"), "&H00EED322");
        assert_eq!(format_ass_time(65.432), "0:01:05.43");
    }
}
