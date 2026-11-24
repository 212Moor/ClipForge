use crate::asr::WindowTranscript;
use crate::error::{AppError, AppResult};
use crate::gemini::budget::PricingConfig;
use crate::gemini::client::{
    build_system_prefix, call_gemini_structured, encode_base64, load_output_language,
};
use crate::gemini::ratelimit::GeminiRateLimiter;
use crate::store::projects::{ClipRecord, StreamerProfileRecord};
use crate::store::Database;
use crate::util::blocking;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager};

static HAS_INJECTED_ONCE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TikTokAiSuggestion {
    pub descriptions: Vec<String>,
    pub hashtags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TikTokAutoInjectResult {
    pub mp4_path: String,
    pub file_name: String,
    pub file_size_bytes: u32,
    pub auto_publish: bool,
    pub message: String,
}

const TIKTOK_COPY_SYSTEM_PROMPT: &str = r#"Tu es un community manager TikTok/Shorts spécialisé gaming et lives Twitch.
À partir d'un extrait de live (titre, accroche, catégorie, transcription), propose :
1. "descriptions" : exactement 3 variantes de description TikTok courtes et percutantes (sans hashtags dedans) :
   - Variante 1 : drôle / punchy / réaction
   - Variante 2 : ultra-courte et mystérieuse (1 phrase)
   - Variante 3 : question ouverte pour déclencher des commentaires
2. "hashtags" : 8 hashtags pertinents sans le symbole # (mélange jeu/thème + gaming/twitch/fyp).
Renvoie UNIQUEMENT un objet JSON conforme au schéma."#;

pub fn tiktok_copy_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "OBJECT",
        "properties": {
            "descriptions": {
                "type": "ARRAY",
                "items": { "type": "STRING" }
            },
            "hashtags": {
                "type": "ARRAY",
                "items": { "type": "STRING" }
            }
        },
        "required": ["descriptions", "hashtags"]
    })
}

/// Génère 3 variantes de description TikTok + 8 suggestions de hashtags pour UN seul clip.
pub async fn suggest_tiktok_post_copy(
    db: &Database,
    limiter: &GeminiRateLimiter,
    pricing: &PricingConfig,
    api_key: &str,
    project_id: &str,
    model_id: &str,
    profile: Option<&StreamerProfileRecord>,
    clip: &ClipRecord,
) -> AppResult<TikTokAiSuggestion> {
    let db_lang = db.clone();
    let out_lang = blocking(move || Ok(load_output_language(&db_lang))).await?;
    let system_prefix = build_system_prefix(TIKTOK_COPY_SYSTEM_PROMPT, profile, &out_lang);

    let compact_text = clip
        .transcript_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<WindowTranscript>(s).ok())
        .map(|tr| tr.compact_text)
        .unwrap_or_default();

    let payload = serde_json::to_string(&serde_json::json!({
        "title": clip.title,
        "hook": clip.hook,
        "caption_current": clip.caption,
        "category": clip.category,
        "duration_sec": (clip.end_sec - clip.start_sec).round().max(1.0) as u32,
        "existing_tags": clip.hashtags,
        "transcript": compact_text
    }))?;

    let outcome = call_gemini_structured(
        db,
        limiter,
        pricing,
        api_key,
        Some(project_id),
        "tiktok_copy",
        model_id,
        &system_prefix,
        &payload,
        tiktok_copy_schema(),
        650,
        true,
    )
    .await?;

    let cleaned = outcome
        .text
        .trim()
        .strip_prefix("```json")
        .or_else(|| outcome.text.trim().strip_prefix("```"))
        .unwrap_or(outcome.text.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

    let mut parsed: TikTokAiSuggestion = serde_json::from_str(cleaned)
        .map_err(|e| AppError::Validation(format!("Suggestions TikTok IA invalides : {e}")))?;

    parsed.descriptions = parsed
        .descriptions
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(3)
        .collect();

    parsed.hashtags = parsed
        .hashtags
        .into_iter()
        .map(|t| {
            t.trim()
                .trim_start_matches('#')
                .replace(' ', "")
                .to_string()
        })
        .filter(|t| !t.is_empty())
        .take(10)
        .collect();

    Ok(parsed)
}

/// Ouvre ou met au premier plan la fenêtre TikTok Studio intégrée à ClipForge (cookies persistants WebView2).
pub fn open_or_focus_studio_window(app: &AppHandle) -> AppResult<(tauri::WebviewWindow, bool)> {
    if let Some(win) = app.get_webview_window("tiktok-studio") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return Ok((win, false));
    }

    let url = tauri::Url::parse("https://www.tiktok.com/tiktokstudio/upload")
        .map_err(|e| AppError::Config(format!("URL TikTok invalide : {e}")))?;

    let win = tauri::WebviewWindowBuilder::new(app, "tiktok-studio", tauri::WebviewUrl::External(url))
        .title("ClipForge — Compte TikTok lié")
        .inner_size(1180.0, 840.0)
        .center()
        .build()
        .map_err(|e| AppError::Internal(format!("Impossible d'ouvrir TikTok intégré : {e}")))?;

    Ok((win, true))
}

/// Injecte directement le fichier MP4 + la description + les hashtags dans la fenêtre TikTok Studio
/// intégrée via WebView2 (zéro clé API, zéro CSP/CORS, zéro glisser-déposer manuel).
pub async fn inject_clip_into_studio_window(
    app: &AppHandle,
    mp4_path: &Path,
    caption_text: &str,
    auto_publish: bool,
) -> AppResult<TikTokAutoInjectResult> {
    if !mp4_path.is_file() {
        return Err(AppError::Config(
            "Fichier MP4 introuvable pour l'injection TikTok.".to_string(),
        ));
    }

    let path_owned = mp4_path.to_path_buf();
    let bytes = blocking(move || std::fs::read(&path_owned).map_err(AppError::Io)).await?;
    if bytes.is_empty() {
        return Err(AppError::Validation("Le fichier MP4 est vide.".to_string()));
    }
    let file_size_bytes = bytes.len().min(u32::MAX as usize) as u32;
    let file_name = mp4_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clipforge_clip.mp4")
        .to_string();

    let (win, was_newly_created) = open_or_focus_studio_window(app)?;

    let current_url = win.url().map(|u| u.to_string()).unwrap_or_default();
    let is_on_login = current_url.contains("/login");
    let is_on_upload =
        current_url.contains("tiktokstudio/upload") || current_url.contains("creator-center/upload");

    let needs_fresh_upload = was_newly_created
        || (!is_on_login && !is_on_upload)
        || (is_on_upload && HAS_INJECTED_ONCE.load(Ordering::SeqCst));

    if !was_newly_created && needs_fresh_upload && !is_on_login {
        let _ = win.eval("window.location.href = 'https://www.tiktok.com/tiktokstudio/upload';");
    }

    if needs_fresh_upload {
        tokio::time::sleep(Duration::from_millis(4200)).await;
    } else {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // Initialise le tampon en mémoire dans la WebView TikTok (aucun appel réseau => jamais bloqué par CSP)
    win.eval("window.__cfChunks = [];")
        .map_err(|e| AppError::Internal(format!("Init injection TikTok : {e}")))?;

    // Découpe en morceaux de 768 KiB (multiple exact de 3 octets pour un base64 sans padding intermédiaire)
    const CHUNK_BYTES: usize = 768 * 1024;
    for chunk in bytes.chunks(CHUNK_BYTES) {
        let b64 = encode_base64(chunk);
        let push_js = format!("if (window.__cfChunks) {{ window.__cfChunks.push('{b64}'); }}");
        win.eval(&push_js)
            .map_err(|e| AppError::Internal(format!("Transfert vidéo vers TikTok : {e}")))?;
    }

    let file_name_json = serde_json::to_string(&file_name)?;
    let caption_json = serde_json::to_string(caption_text.trim())?;
    let auto_pub_js = if auto_publish { "true" } else { "false" };

    let injector_js = build_tiktok_injector_js(&file_name_json, &caption_json, auto_pub_js);
    win.eval(&injector_js)
        .map_err(|e| AppError::Internal(format!("Exécution script injection TikTok : {e}")))?;

    HAS_INJECTED_ONCE.store(true, Ordering::SeqCst);

    let message = if auto_publish {
        "MP4 + description + hashtags injectés dans TikTok Studio (publication auto activée).".to_string()
    } else {
        "MP4 + description + hashtags injectés dans TikTok Studio ! Vérifie et clique sur Publier.".to_string()
    };

    Ok(TikTokAutoInjectResult {
        mp4_path: mp4_path.to_string_lossy().to_string(),
        file_name,
        file_size_bytes,
        auto_publish,
        message,
    })
}

fn build_tiktok_injector_js(file_name_json: &str, caption_json: &str, auto_pub_js: &str) -> String {
    format!(
        r##"(function() {{
  const FILE_NAME = {file_name_json};
  const CAPTION = {caption_json};
  const AUTO_PUBLISH = {auto_pub_js};

  function setHud(msg, tone) {{
    let el = document.getElementById('__clipforge_hud');
    if (!el) {{
      el = document.createElement('div');
      el.id = '__clipforge_hud';
      el.style.cssText = 'position:fixed;top:14px;right:14px;z-index:2147483647;max-width:380px;padding:10px 14px;border-radius:12px;font-family:Segoe UI,system-ui,sans-serif;font-size:12px;font-weight:600;line-height:1.4;color:#f8fafc;background:rgba(12,14,24,0.94);border:1px solid rgba(34,211,238,0.65);box-shadow:0 12px 32px rgba(0,0,0,0.45);display:flex;align-items:center;gap:10px;';
      document.documentElement.appendChild(el);
    }}
    const border = tone === 'ok' ? 'rgba(52,211,153,0.85)' : tone === 'warn' ? 'rgba(250,204,21,0.85)' : 'rgba(34,211,238,0.75)';
    el.style.borderColor = border;
    el.innerHTML = '';
    const span = document.createElement('span');
    span.textContent = msg;
    const btn = document.createElement('button');
    btn.textContent = '×';
    btn.style.cssText = 'background:transparent;border:none;color:#94a3b8;font-size:16px;cursor:pointer;padding:0 2px;line-height:1;';
    btn.onclick = () => el.remove();
    el.appendChild(span);
    el.appendChild(btn);
  }}

  function allDocs() {{
    const docs = [document];
    try {{
      const iframes = document.querySelectorAll('iframe');
      for (const f of iframes) {{
        try {{
          if (f.contentDocument) docs.push(f.contentDocument);
        }} catch (_) {{}}
      }}
    }} catch (_) {{}}
    return docs;
  }}

  function findFileInput() {{
    for (const d of allDocs()) {{
      const vidInput = d.querySelector('input[type="file"][accept*="video"]');
      if (vidInput) return {{ doc: d, input: vidInput }};
      const anyInput = d.querySelector('input[type="file"]');
      if (anyInput) return {{ doc: d, input: anyInput }};
    }}
    return null;
  }}

  function findCaptionEditor() {{
    const selectors = [
      '[data-e2e="caption-editor"] [contenteditable="true"]',
      '.caption-editor [contenteditable="true"]',
      '.public-DraftEditor-content[contenteditable="true"]',
      '[role="textbox"][contenteditable="true"]',
      'div[contenteditable="true"]'
    ];
    for (const d of allDocs()) {{
      for (const sel of selectors) {{
        const list = d.querySelectorAll(sel);
        for (const el of list) {{
          const rect = el.getBoundingClientRect();
          if (rect.width >= 120 && rect.height >= 20) {{
            return {{ doc: d, editor: el }};
          }}
        }}
      }}
    }}
    return null;
  }}

  function findPublishButton() {{
    for (const d of allDocs()) {{
      const direct = d.querySelector('button[data-e2e="post_video_button"], [data-e2e="post_video_button"] button');
      if (direct) return direct;
      const buttons = d.querySelectorAll('button');
      for (const b of buttons) {{
        const t = (b.innerText || b.textContent || '').trim().toLowerCase();
        if (t === 'publier' || t === 'post' || t === 'poster') {{
          const rect = b.getBoundingClientRect();
          if (rect.width >= 60 && rect.height >= 28) return b;
        }}
      }}
    }}
    return null;
  }}

  function applyCaption(doc, editor, text) {{
    try {{
      editor.focus();
      const win = doc.defaultView || window;
      const sel = win.getSelection();
      const range = doc.createRange();
      range.selectNodeContents(editor);
      sel.removeAllRanges();
      sel.addRange(range);
      doc.execCommand('selectAll', false, null);
      doc.execCommand('delete', false, null);
      const inserted = doc.execCommand('insertText', false, text);
      const current = (editor.innerText || '').trim();
      if (!inserted || !current || current.length < Math.min(8, text.length)) {{
        const dt = new DataTransfer();
        dt.setData('text/plain', text);
        const pasteEv = new ClipboardEvent('paste', {{
          bubbles: true,
          cancelable: true,
          clipboardData: dt
        }});
        editor.dispatchEvent(pasteEv);
      }}
      editor.dispatchEvent(new Event('input', {{ bubbles: true }}));
    }} catch (_) {{}}
  }}

  function buildFileFromChunks() {{
    const chunks = window.__cfChunks || [];
    window.__cfChunks = [];
    const parts = [];
    for (let i = 0; i < chunks.length; i++) {{
      const bin = atob(chunks[i]);
      const arr = new Uint8Array(bin.length);
      for (let j = 0; j < bin.length; j++) arr[j] = bin.charCodeAt(j);
      parts.push(arr);
    }}
    const blob = new Blob(parts, {{ type: 'video/mp4' }});
    return new File([blob], FILE_NAME, {{ type: 'video/mp4', lastModified: Date.now() }});
  }}

  if (window.location.href.includes('/login')) {{
    window.__cfChunks = [];
    setHud('ClipForge : connecte-toi une fois à ton compte TikTok ici, puis reclique sur Uploader dans ClipForge.', 'warn');
    return;
  }}

  const file = buildFileFromChunks();
  const sizeMb = (file.size / (1024 * 1024)).toFixed(1);
  setHud('ClipForge : injection de ' + FILE_NAME + ' (' + sizeMb + ' Mo)…', 'info');

  let inputTries = 0;
  const inputTimer = setInterval(() => {{
    inputTries++;
    if (window.location.href.includes('/login')) {{
      clearInterval(inputTimer);
      setHud('ClipForge : connecte-toi à ton compte TikTok ici, puis relance Uploader dans ClipForge.', 'warn');
      return;
    }}
    const found = findFileInput();
    if (!found) {{
      if (inputTries >= 70) {{
        clearInterval(inputTimer);
        setHud('ClipForge : champ upload non détecté (assure-toi d\'être connecté sur TikTok Studio).', 'warn');
      }}
      return;
    }}
    clearInterval(inputTimer);

    try {{
      const dt = new DataTransfer();
      dt.items.add(file);
      found.input.files = dt.files;
      found.input.dispatchEvent(new Event('input', {{ bubbles: true }}));
      found.input.dispatchEvent(new Event('change', {{ bubbles: true }}));
      setHud('ClipForge : vidéo envoyée ! Attente du champ description…', 'info');
    }} catch (err) {{
      setHud('ClipForge : injection fichier bloquée (' + String(err) + ').', 'warn');
      return;
    }}

    if (!CAPTION) {{
      setHud('ClipForge : vidéo injectée ! Tu peux publier.', 'ok');
      return;
    }}

    let capTries = 0;
    const capTimer = setInterval(() => {{
      capTries++;
      const cap = findCaptionEditor();
      if (!cap) {{
        if (capTries >= 70) {{
          clearInterval(capTimer);
          setHud('ClipForge : vidéo injectée ! Colle ta description (Ctrl+V) puis clique sur Publier.', 'ok');
        }}
        return;
      }}
      clearInterval(capTimer);

      setTimeout(() => {{
        applyCaption(cap.doc, cap.editor, CAPTION);
        // Second passage de sécurité si TikTok ré-écrit le nom du fichier après traitement
        setTimeout(() => {{
          const nowTxt = (cap.editor.innerText || '').trim();
          const stem = FILE_NAME.replace(/\.mp4$/i, '');
          if (!nowTxt || nowTxt === stem || nowTxt.length < Math.min(6, CAPTION.length)) {{
            applyCaption(cap.doc, cap.editor, CAPTION);
          }}

          if (!AUTO_PUBLISH) {{
            const pubBtn = findPublishButton();
            if (pubBtn) {{
              pubBtn.style.outline = '3px solid #22d3ee';
              pubBtn.style.outlineOffset = '3px';
            }}
            setHud('ClipForge : vidéo + description + # prêts ! Vérifie (ou ajoute un son) puis clique sur Publier.', 'ok');
            return;
          }}

          setHud('ClipForge : traitement vidéo TikTok en cours avant publication auto…', 'info');
          let pubTries = 0;
          const pubTimer = setInterval(() => {{
            pubTries++;
            const btn = findPublishButton();
            const isDisabled = !btn || btn.disabled || btn.getAttribute('aria-disabled') === 'true';
            if (!isDisabled) {{
              clearInterval(pubTimer);
              setTimeout(() => {{
                btn.click();
                setHud('ClipForge : publication lancée sur TikTok !', 'ok');
              }}, 1500);
            }} else if (pubTries >= 120) {{
              clearInterval(pubTimer);
              setHud('ClipForge : vidéo prête — clique sur Publier pour finaliser.', 'ok');
            }}
          }}, 1000);
        }}, 1800);
      }}, 900);
    }}, 600);
  }}, 500);
}})();"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_injector_js_with_escaped_caption() {
        let js = build_tiktok_injector_js("\"clip.mp4\"", "\"Incroyable fail! #gta #twitch\"", "false");
        assert!(js.contains("FILE_NAME = \"clip.mp4\""));
        assert!(js.contains("AUTO_PUBLISH = false"));
        assert!(js.contains("DataTransfer"));
    }
}
