use crate::error::{AppError, AppResult};
use crate::gemini::budget::{
    compute_call_cost_usd, enforce_budget_cap, estimate_tokens_local, PricingConfig,
};
use crate::gemini::cache::{compute_payload_hash, lookup_cached_response, record_call, RecordCallParams};
use crate::gemini::ratelimit::{backoff_with_jitter, GeminiRateLimiter};
use crate::signals::CompactChatSummary;
use crate::store::projects::StreamerProfileRecord;
use crate::store::Database;
use crate::util::blocking;
use chrono::Utc;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::time::{Duration, Instant};

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const SETTING_STAGE_A_MODEL: &str = "gemini.model.stage_a";
const SETTING_STAGE_B_MODEL: &str = "gemini.model.stage_b";
const SETTING_OUTPUT_LANGUAGE: &str = "gemini.output_language";

pub const PROMPT_VERSION: &str = "v4";

pub const STAGE_A_SYSTEM_PROMPT: &str = r#"Tu es un monteur TikTok/Shorts spécialisé dans les lives Twitch (y compris petits streams sans chat actif).
Tu reçois un lot de fenêtres candidates issues d'une VOD. Pour chaque candidat :
- i : index du candidat
- d : durée initiale en secondes de la fenêtre [0 .. d]
- t : transcription horodatée en secondes relatives (ex: [-18]... [0]... [42]..., avec 25 s de contexte avant et après)
- c : résumé du chat (NE pénalise jamais l'absence de chat)
- s : [score_chat, score_audio]

RÈGLES DE REJET (k=0, s<=35) :
1. Dialogue pur de PNJ / cinématique passive sans aucune réaction ni commentaire du streamer.
2. Silence, écran de chargement, lecture de menu ou bruit isolé sans parole.

BARÈME (rôle : sélectionner largement les meilleurs passages avant le contrôle visuel Étage B) :
- 0-35 : cinématique pure / menu / vide (k=0)
- 36-54 : moment banal (k=0)
- 55-79 : bon moment de live publiable (fail, réaction drôle, vanne, conduite chaotique, tension, moment absurde) (k=1)
- 80-100 : pépite virale (fou rire, énorme fail, rage culte, punchline, clutch) (k=1)

Sur un lot de 12 candidats, garde en moyenne les 3 à 6 meilleurs moments (k=1) si le live contient du gameplay et des réactions.

DÉCOUPAGE PRÉCIS (a et b en secondes) :
- a (entre -25 et +35) : décalage du début par rapport à [0] pour démarrer PILE sur le setup.
- b (entre -35 et +25) : décalage de la fin par rapport à [d] pour couper PILE 1 à 2 secondes après la chute.
- Vise une durée nerveuse entre 15 s et 55 s.

Renvoie UNIQUEMENT un tableau JSON valide conforme au schéma (i, k, s, c, a, b, r <= 12 mots)."#;

pub const STAGE_B_SYSTEM_PROMPT: &str = r#"Tu es un directeur créatif TikTok/Shorts gaming/Twitch.
Tu reçois des finalistes pré-sélectionnés, accompagnés de 3 VIGNETTES VISUELLES (début, milieu, fin) lorsque disponibles.

TA MISSION : classer ces finalistes de 0 à 100 et rédiger une accroche, un titre, une légende et des hashtags pour CHACUN.
- Mets keep=0 et score<=15 UNIQUEMENT pour une disqualification visuelle indiscutable : écran noir, AFK, menu pause/inventaire/chargement figé, ou cinématique pure avec bandes noires sans réaction drôle.
- Pour TOUT vrai moment de gameplay, conduite, fail, rage ou blague du streamer, mets keep=1 et attribue un score nuancé entre 50 et 100 afin de classer les meilleurs en tête (ne rejette pas un bon clip juste parce que 3 images fixes paraissent calmes).

FINITION ÉDITORIALE :
- hook : accroche choc de 3 à 7 mots MAXIMUM (sans spoiler la chute).
- title : titre accrocheur <= 60 caractères.
- caption : courte légende TikTok engageante (1 à 2 phrases).
- tags : 4 à 6 hashtags pertinents sans #.
- a (entre -20 et +25) et b (entre -25 et +20) : affinage fin du début et de la fin.

Renvoie UNIQUEMENT un tableau JSON valide conforme au schéma."#;
const ALLOWED_CATEGORIES: &[&str] = &[
    "fun",
    "clutch",
    "fail",
    "rage",
    "wholesome",
    "story",
    "skill",
];

fn default_stage_b_keep() -> u32 {
    1
}

fn default_stage_b_score() -> u32 {
    75
}

#[derive(Debug, Clone)]
pub struct VisualInlineImage {
    pub label: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GeminiModelInfo {
    pub name: String,
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub input_token_limit: u32,
    pub output_token_limit: u32,
    pub supported_generation_methods: Vec<String>,
    pub supports_generate_content: bool,
    pub supports_count_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GeminiKeyTestResult {
    pub valid: bool,
    pub latency_ms: u32,
    pub models_count: u32,
    pub models: Vec<GeminiModelInfo>,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct GeminiModelSelection {
    pub stage_a_model: Option<String>,
    pub stage_b_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageACandidatePayload {
    pub i: u32,
    pub d: u32,
    pub t: String,
    pub c: CompactChatSummary,
    pub s: [f64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StageAOutputItem {
    pub i: u32,
    pub k: u32,
    pub s: u32,
    pub c: String,
    pub a: f64,
    pub b: f64,
    pub r: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageBInputItem {
    pub i: u32,
    pub d: u32,
    pub cat: String,
    pub t: String,
    pub c: CompactChatSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StageBOutputItem {
    pub i: u32,
    pub hook: String,
    pub title: String,
    pub caption: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub a: f64,
    #[serde(default)]
    pub b: f64,
    #[serde(default = "default_stage_b_keep")]
    pub keep: u32,
    #[serde(default = "default_stage_b_score")]
    pub score: u32,
}

#[derive(Debug, Clone)]
pub struct StructuredCallOutcome {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub cost_usd: f64,
    pub cache_hit: bool,
    pub duration_ms: u32,
}

#[derive(Debug, Clone)]
pub enum EcoBatchPollResult {
    Pending(String),
    Succeeded(Vec<String>),
    Failed(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsListResponse {
    #[serde(default)]
    models: Vec<RawGeminiModel>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGeminiModel {
    name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    input_token_limit: u32,
    #[serde(default)]
    output_token_limit: u32,
    #[serde(default)]
    supported_generation_methods: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorEnvelope {
    error: Option<GeminiErrorBody>,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorBody {
    message: Option<String>,
    status: Option<String>,
}

fn build_http_client(timeout_secs: u64) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(timeout_secs))
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .user_agent("ClipForge/0.2")
        .build()
        .map_err(|e| AppError::Network(e.to_string()))
}

pub fn load_output_language(db: &Database) -> String {
    if let Ok(Some(raw)) = db.get_setting(SETTING_OUTPUT_LANGUAGE) {
        if let Ok(lang) = serde_json::from_str::<String>(&raw) {
            if !lang.trim().is_empty() {
                return lang;
            }
        }
    }
    "fr".to_string()
}

pub fn save_output_language(db: &Database, lang: &str) -> AppResult<()> {
    db.set_setting(SETTING_OUTPUT_LANGUAGE, &serde_json::to_string(lang)?)
}

pub fn build_system_prefix(
    base_prompt: &str,
    profile: Option<&StreamerProfileRecord>,
    output_language: &str,
) -> String {
    let lang_label = match output_language {
        "en" => "anglais (English)",
        _ => "français",
    };

    let profile_block = match profile {
        Some(p) => {
            let jargon = if p.jargon.is_empty() {
                "aucun".to_string()
            } else {
                p.jargon.join(", ")
            };
            let games = if p.games.is_empty() {
                "variété".to_string()
            } else {
                p.games.join(", ")
            };
            let emotes = if p.emotes.is_empty() {
                "standard Twitch".to_string()
            } else {
                let mut pairs: Vec<String> = p
                    .emotes
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                pairs.sort();
                pairs.join("; ")
            };
            format!(
                "PROFIL STREAMER:\n- Nom: {}\n- Ton: {}\n- Jeux: {}\n- Jargon: {}\n- Emotes: {}\n- Blagues récurrentes: {}\n- INTERDIT EN CLIP: {}",
                p.name, p.tone, games, jargon, emotes, p.inside_jokes, p.forbidden_topics
            )
        }
        None => "PROFIL STREAMER:\n- Streamer Twitch généraliste, ton dynamique et authentique.".to_string(),
    };

    format!("{base_prompt}\nLangue de sortie : {lang_label}.\n\n{profile_block}")
}

pub fn stage_a_response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "ARRAY",
        "items": {
            "type": "OBJECT",
            "properties": {
                "i": { "type": "INTEGER" },
                "k": { "type": "INTEGER" },
                "s": { "type": "INTEGER" },
                "c": {
                    "type": "STRING",
                    "enum": ["fun", "clutch", "fail", "rage", "wholesome", "story", "skill"]
                },
                "a": { "type": "NUMBER" },
                "b": { "type": "NUMBER" },
                "r": { "type": "STRING" }
            },
            "required": ["i", "k", "s", "c", "a", "b", "r"]
        }
    })
}

pub fn stage_b_response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "ARRAY",
        "items": {
            "type": "OBJECT",
            "properties": {
                "i": { "type": "INTEGER" },
                "keep": { "type": "INTEGER" },
                "score": { "type": "INTEGER" },
                "hook": { "type": "STRING" },
                "title": { "type": "STRING" },
                "caption": { "type": "STRING" },
                "tags": {
                    "type": "ARRAY",
                    "items": { "type": "STRING" }
                },
                "a": { "type": "NUMBER" },
                "b": { "type": "NUMBER" }
            },
            "required": ["i", "keep", "score", "hook", "title", "caption", "tags"]
        }
    })
}

pub async fn list_models(api_key: &str) -> AppResult<Vec<GeminiModelInfo>> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err(AppError::Gemini(
            "Aucune clé API Gemini renseignée. Créez une clé sur Google AI Studio.".to_string(),
        ));
    }

    let client = build_http_client(20)?;
    let mut collected = Vec::new();
    let mut page_token: Option<String> = None;

    for _ in 0..10 {
        let mut request = client
            .get(format!("{GEMINI_API_BASE}/models"))
            .header("x-goog-api-key", key)
            .query(&[("pageSize", "100")]);

        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }

        let response = request
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Impossible de joindre Google AI Studio : {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Gemini(format_api_error(status, &body)));
        }

        let payload: ModelsListResponse = response
            .json()
            .await
            .map_err(|e| AppError::Gemini(format!("Réponse models.list illisible : {e}")))?;

        for raw in payload.models {
            let id = raw.name.strip_prefix("models/").unwrap_or(&raw.name).to_string();
            let supports = |method: &str| raw.supported_generation_methods.iter().any(|m| m == method);

            collected.push(GeminiModelInfo {
                supports_generate_content: supports("generateContent"),
                supports_count_tokens: supports("countTokens"),
                display_name: if raw.display_name.is_empty() {
                    id.clone()
                } else {
                    raw.display_name.clone()
                },
                description: raw.description.clone(),
                input_token_limit: raw.input_token_limit,
                output_token_limit: raw.output_token_limit,
                supported_generation_methods: raw.supported_generation_methods.clone(),
                name: raw.name.clone(),
                id,
            });
        }

        match payload.next_page_token {
            Some(next) if !next.is_empty() => page_token = Some(next),
            _ => break,
        }
    }

    collected.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(collected)
}

pub async fn test_api_key(db: &Database, api_key: &str) -> AppResult<GeminiKeyTestResult> {
    let start = Instant::now();
    let models = list_models(api_key).await?;
    let latency_ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let usable: Vec<GeminiModelInfo> = models
        .into_iter()
        .filter(|m| m.supports_generate_content)
        .collect();
    let count = usable.len() as u32;

    let db_clone = db.clone();
    let audit = blocking(move || {
        db_clone.with_conn(|conn| {
            conn.execute(
                "INSERT INTO gemini_calls
                 (id, project_id, stage, payload_hash, model, prompt_version, input_tokens, output_tokens,
                  cached_tokens, estimated_cost_usd, cache_hit, duration_ms, response_json, created_at)
                 VALUES (?1, NULL, 'test', 'models.list', 'models.list', 'p0', 0, 0, 0, 0.0, 0, ?2, NULL, ?3)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    latency_ms,
                    Utc::now().to_rfc3339()
                ],
            )?;
            Ok(())
        })
    })
    .await;
    if let Err(err) = audit {
        tracing::warn!(error = %err, "Trace d'audit du test de clé non enregistrée");
    }

    Ok(GeminiKeyTestResult {
        valid: true,
        latency_ms,
        models_count: count,
        models: usable,
        message: format!(
            "Clé API valide : {count} modèles compatibles generateContent récupérés en {latency_ms} ms."
        ),
    })
}

pub fn load_model_selection(db: &Database) -> AppResult<GeminiModelSelection> {
    let parse = |raw: Option<String>| -> Option<String> {
        raw.and_then(|s| serde_json::from_str::<String>(&s).ok())
            .filter(|s| !s.trim().is_empty())
    };

    Ok(GeminiModelSelection {
        stage_a_model: parse(db.get_setting(SETTING_STAGE_A_MODEL)?),
        stage_b_model: parse(db.get_setting(SETTING_STAGE_B_MODEL)?),
    })
}

pub fn save_model_selection(db: &Database, selection: &GeminiModelSelection) -> AppResult<()> {
    let stage_a = selection.stage_a_model.clone().unwrap_or_default();
    let stage_b = selection.stage_b_model.clone().unwrap_or_default();
    db.set_setting(SETTING_STAGE_A_MODEL, &serde_json::to_string(&stage_a)?)?;
    db.set_setting(SETTING_STAGE_B_MODEL, &serde_json::to_string(&stage_b)?)?;
    Ok(())
}

pub async fn count_tokens_remote(
    api_key: &str,
    model_id: &str,
    system_instruction: &str,
    user_payload: &str,
) -> AppResult<u32> {
    let short = model_id.strip_prefix("models/").unwrap_or(model_id);
    let client = build_http_client(15)?;
    let body = serde_json::json!({
        "generateContentRequest": {
            "model": format!("models/{short}"),
            "systemInstruction": { "parts": [{ "text": system_instruction }] },
            "contents": [{ "role": "user", "parts": [{ "text": user_payload }] }]
        }
    });

    let resp = client
        .post(format!("{GEMINI_API_BASE}/models/{short}:countTokens"))
        .header("x-goog-api-key", api_key.trim())
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("countTokens inaccessible : {e}")))?;

    if !resp.status().is_success() {
        return Ok(estimate_tokens_local(system_instruction) + estimate_tokens_local(user_payload));
    }

    let val: serde_json::Value = resp.json().await.unwrap_or_default();
    let total = val
        .get("totalTokens")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(u64::from(u32::MAX)) as u32)
        .unwrap_or_else(|| estimate_tokens_local(system_instruction) + estimate_tokens_local(user_payload));
    Ok(total)
}

pub async fn call_gemini_structured(
    db: &Database,
    limiter: &GeminiRateLimiter,
    pricing: &PricingConfig,
    api_key: &str,
    project_id: Option<&str>,
    stage: &str,
    model_id: &str,
    system_prefix: &str,
    user_payload: &str,
    response_schema: serde_json::Value,
    max_output_tokens: u32,
    disable_thinking: bool,
) -> AppResult<StructuredCallOutcome> {
    call_gemini_structured_multimodal(
        db,
        limiter,
        pricing,
        api_key,
        project_id,
        stage,
        model_id,
        system_prefix,
        user_payload,
        &[],
        response_schema,
        max_output_tokens,
        disable_thinking,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn call_gemini_structured_multimodal(
    db: &Database,
    limiter: &GeminiRateLimiter,
    pricing: &PricingConfig,
    api_key: &str,
    project_id: Option<&str>,
    stage: &str,
    model_id: &str,
    system_prefix: &str,
    user_payload: &str,
    images: &[VisualInlineImage],
    response_schema: serde_json::Value,
    max_output_tokens: u32,
    disable_thinking: bool,
) -> AppResult<StructuredCallOutcome> {
    let short_model = model_id.strip_prefix("models/").unwrap_or(model_id);
    let cache_payload_str = if images.is_empty() {
        user_payload.to_string()
    } else {
        let mut h = Sha256::new();
        h.update(user_payload.as_bytes());
        for img in images {
            h.update(img.label.as_bytes());
            h.update(&img.bytes);
        }
        format!("{user_payload}#img:{:x}", h.finalize())
    };
    let payload_hash = compute_payload_hash(short_model, PROMPT_VERSION, system_prefix, &cache_payload_str);

    // 1. Cache disque SQLite d'abord : 0 token dépensé
    let db_lookup = db.clone();
    let hash_lookup = payload_hash.clone();
    if let Some(cached_json) = blocking(move || lookup_cached_response(&db_lookup, &hash_lookup)).await? {
        let db_rec = db.clone();
        let pid_owned = project_id.map(ToOwned::to_owned);
        let stage_owned = stage.to_string();
        let hash_owned = payload_hash.clone();
        let model_owned = short_model.to_string();
        let _ = blocking(move || {
            record_call(
                &db_rec,
                &RecordCallParams {
                    project_id: pid_owned.as_deref(),
                    stage: &stage_owned,
                    payload_hash: &hash_owned,
                    model: &model_owned,
                    prompt_version: PROMPT_VERSION,
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_tokens: 0,
                    estimated_cost_usd: 0.0,
                    cache_hit: true,
                    duration_ms: 0,
                    response_json: None,
                },
            )
        })
        .await;

        return Ok(StructuredCallOutcome {
            text: cached_json,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cost_usd: 0.0,
            cache_hit: true,
            duration_ms: 0,
        });
    }

    // 2. Estimation précise + contrôle du plafond dur par projet
    let est_input = if images.is_empty() {
        count_tokens_remote(api_key, short_model, system_prefix, user_payload)
            .await
            .unwrap_or_else(|_| estimate_tokens_local(system_prefix) + estimate_tokens_local(user_payload))
    } else {
        estimate_tokens_local(system_prefix)
            .saturating_add(estimate_tokens_local(user_payload))
            .saturating_add((images.len() as u32).saturating_mul(260))
    };

    if let Some(pid) = project_id {
        let db_cap = db.clone();
        let pid_owned = pid.to_string();
        let projected = est_input.saturating_add(max_output_tokens / 2);
        blocking(move || enforce_budget_cap(&db_cap, &pid_owned, projected)).await?;
    }

    // 3. Construction des parties (texte + vignettes JPEG base64 éventuelles)
    let mut user_parts = Vec::with_capacity(1 + images.len() * 2);
    user_parts.push(serde_json::json!({ "text": user_payload }));
    for img in images {
        user_parts.push(serde_json::json!({ "text": img.label }));
        user_parts.push(serde_json::json!({
            "inlineData": {
                "mimeType": img.mime_type,
                "data": encode_base64(&img.bytes)
            }
        }));
    }

    let client = build_http_client(180)?;
    let max_retries = limiter.max_retries().await;
    let mut use_thinking_zero = disable_thinking;

    let mut attempt = 0u32;
    loop {
        limiter.acquire(est_input).await;
        let started = Instant::now();

        let mut gen_cfg = serde_json::json!({
            "temperature": 0.2,
            "maxOutputTokens": max_output_tokens,
            "responseMimeType": "application/json",
            "responseSchema": response_schema
        });
        if use_thinking_zero {
            gen_cfg["thinkingConfig"] = serde_json::json!({ "thinkingBudget": 0 });
        }

        let req_body = serde_json::json!({
            "systemInstruction": {
                "parts": [{ "text": system_prefix }]
            },
            "contents": [{
                "role": "user",
                "parts": user_parts
            }],
            "generationConfig": gen_cfg
        });

        let resp_res = client
            .post(format!("{GEMINI_API_BASE}/models/{short_model}:generateContent"))
            .header("x-goog-api-key", api_key.trim())
            .json(&req_body)
            .send()
            .await;

        let resp = match resp_res {
            Ok(r) => r,
            Err(net_err) => {
                if attempt < max_retries {
                    attempt += 1;
                    tokio::time::sleep(backoff_with_jitter(attempt)).await;
                    continue;
                }
                return Err(AppError::Network(format!(
                    "Connexion perdue avec Google AI Studio (timeout={}) : {net_err:?}",
                    net_err.is_timeout()
                )));
            }
        };

        let status = resp.status();
        let raw_body = resp.text().await.unwrap_or_default();

        if (status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE)
            && attempt < max_retries
        {
            attempt += 1;
            tracing::warn!(status = %status, attempt, "HTTP 429/503 Gemini — nouvel essai avec backoff");
            tokio::time::sleep(backoff_with_jitter(attempt)).await;
            continue;
        }

        if status == StatusCode::BAD_REQUEST
            && use_thinking_zero
            && !raw_body.to_ascii_lowercase().contains("api key")
        {
            use_thinking_zero = false;
            continue;
        }

        if !status.is_success() {
            return Err(AppError::Gemini(format!("[{short_model}] {}", format_api_error(status, &raw_body))));
        }

        let duration_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        let parsed: serde_json::Value = serde_json::from_str(&raw_body)?;

        let text = extract_candidate_text(&parsed).ok_or_else(|| {
            AppError::Gemini("Réponse Gemini vide ou bloquée par les filtres de sécurité.".to_string())
        })?;

        let input_tokens = parsed
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u64::from(u32::MAX)) as u32)
            .unwrap_or(est_input);
        let output_tokens = parsed
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u64::from(u32::MAX)) as u32)
            .unwrap_or_else(|| estimate_tokens_local(&text));
        let cached_tokens = parsed
            .pointer("/usageMetadata/cachedContentTokenCount")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u64::from(u32::MAX)) as u32)
            .unwrap_or(0);

        let cost_usd = compute_call_cost_usd(
            pricing,
            short_model,
            input_tokens,
            output_tokens,
            cached_tokens,
            false,
        );

        let db_rec = db.clone();
        let pid_owned = project_id.map(ToOwned::to_owned);
        let stage_owned = stage.to_string();
        let hash_owned = payload_hash.clone();
        let model_owned = short_model.to_string();
        let text_owned = text.clone();
        blocking(move || {
            record_call(
                &db_rec,
                &RecordCallParams {
                    project_id: pid_owned.as_deref(),
                    stage: &stage_owned,
                    payload_hash: &hash_owned,
                    model: &model_owned,
                    prompt_version: PROMPT_VERSION,
                    input_tokens,
                    output_tokens,
                    cached_tokens,
                    estimated_cost_usd: cost_usd,
                    cache_hit: false,
                    duration_ms,
                    response_json: Some(&text_owned),
                },
            )
        })
        .await?;

        return Ok(StructuredCallOutcome {
            text,
            input_tokens,
            output_tokens,
            cached_tokens,
            cost_usd,
            cache_hit: false,
            duration_ms,
        });
    }
}

pub async fn submit_eco_batch_stage_a(
    api_key: &str,
    model_id: &str,
    system_prefix: &str,
    batch_payloads: &[String],
) -> AppResult<String> {
    let short_model = model_id.strip_prefix("models/").unwrap_or(model_id);
    let client = build_http_client(45)?;
    let schema = stage_a_response_schema();

    let requests: Vec<serde_json::Value> = batch_payloads
        .iter()
        .enumerate()
        .map(|(idx, payload)| {
            serde_json::json!({
                "request": {
                    "systemInstruction": { "parts": [{ "text": system_prefix }] },
                    "contents": [{ "role": "user", "parts": [{ "text": payload }] }],
                    "generationConfig": {
                        "temperature": 0.2,
                        "maxOutputTokens": 1400,
                        "responseMimeType": "application/json",
                        "responseSchema": schema
                    }
                },
                "metadata": { "key": format!("batch-{idx}") }
            })
        })
        .collect();

    let body = serde_json::json!({
        "batch": {
            "displayName": format!("clipforge-eco-{}", Utc::now().timestamp()),
            "inputConfig": {
                "requests": { "requests": requests }
            }
        }
    });

    let resp = client
        .post(format!("{GEMINI_API_BASE}/models/{short_model}:batchGenerateContent"))
        .header("x-goog-api-key", api_key.trim())
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Soumission Batch API échouée : {e}")))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Gemini(format_api_error(status, &text)));
    }

    let val: serde_json::Value = serde_json::from_str(&text)?;
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Gemini("Identifiant de lot Batch API absent de la réponse.".to_string()))?;
    Ok(name.to_string())
}

pub async fn poll_eco_batch_job(api_key: &str, batch_name: &str) -> AppResult<EcoBatchPollResult> {
    let client = build_http_client(25)?;
    let resp = client
        .get(format!("{GEMINI_API_BASE}/{batch_name}"))
        .header("x-goog-api-key", api_key.trim())
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Suivi Batch API impossible : {e}")))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Gemini(format_api_error(status, &text)));
    }

    let val: serde_json::Value = serde_json::from_str(&text)?;
    let done = val.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
    let state = val
        .pointer("/metadata/state")
        .and_then(|v| v.as_str())
        .unwrap_or("JOB_STATE_RUNNING");

    if !done && state != "JOB_STATE_SUCCEEDED" {
        if state == "JOB_STATE_FAILED" || state == "JOB_STATE_CANCELLED" {
            return Ok(EcoBatchPollResult::Failed(format!("Lot Batch API terminé en état {state}")));
        }
        return Ok(EcoBatchPollResult::Pending(state.to_string()));
    }

    if let Some(err_msg) = val.pointer("/error/message").and_then(|v| v.as_str()) {
        return Ok(EcoBatchPollResult::Failed(err_msg.to_string()));
    }

    let mut texts = Vec::new();
    if let Some(inlined) = val
        .pointer("/response/inlinedResponses/inlinedResponses")
        .and_then(|v| v.as_array())
    {
        for item in inlined {
            if let Some(resp_obj) = item.get("response") {
                if let Some(t) = extract_candidate_text(resp_obj) {
                    texts.push(t);
                }
            }
        }
    }
    Ok(EcoBatchPollResult::Succeeded(texts))
}

pub fn parse_and_validate_stage_a(raw_json: &str) -> AppResult<Vec<StageAOutputItem>> {
    let cleaned = strip_markdown_fences(raw_json);
    let mut items: Vec<StageAOutputItem> = serde_json::from_str(cleaned)
        .map_err(|e| AppError::Validation(format!("Schéma JSON Étage A invalide : {e}")))?;

    for item in &mut items {
        if item.k > 1 {
            return Err(AppError::Validation(format!(
                "Champ 'k' invalide ({}) pour le candidat {}",
                item.k, item.i
            )));
        }
        item.s = item.s.min(100);
        let cat_lower = item.c.trim().to_ascii_lowercase();
        item.c = if ALLOWED_CATEGORIES.contains(&cat_lower.as_str()) {
            cat_lower
        } else {
            "fun".to_string()
        };
        item.a = item.a.clamp(-25.0, 35.0);
        item.b = item.b.clamp(-35.0, 25.0);
        let words: Vec<&str> = item.r.split_whitespace().take(12).collect();
        item.r = words.join(" ");
    }
    Ok(items)
}

pub fn parse_and_validate_stage_b(raw_json: &str) -> AppResult<Vec<StageBOutputItem>> {
    let cleaned = strip_markdown_fences(raw_json);
    let mut items: Vec<StageBOutputItem> = serde_json::from_str(cleaned)
        .map_err(|e| AppError::Validation(format!("Schéma JSON Étage B invalide : {e}")))?;

    for item in &mut items {
        let hook_words: Vec<&str> = item.hook.split_whitespace().take(8).collect();
        item.hook = hook_words.join(" ");
        if item.title.chars().count() > 60 {
            item.title = item.title.chars().take(60).collect();
        }
        item.tags = item
            .tags
            .iter()
            .map(|t| t.trim().trim_start_matches('#').to_string())
            .filter(|t| !t.is_empty())
            .take(8)
            .collect();
        item.a = item.a.clamp(-20.0, 25.0);
        item.b = item.b.clamp(-25.0, 20.0);
        item.keep = if item.keep > 0 { 1 } else { 0 };
        item.score = item.score.min(100);
    }
    Ok(items)
}

pub async fn regenerate_single_clip_metadata(
    db: &Database,
    limiter: &GeminiRateLimiter,
    pricing: &PricingConfig,
    api_key: &str,
    project_id: &str,
    model_id: &str,
    profile: Option<&StreamerProfileRecord>,
    input_item: &StageBInputItem,
) -> AppResult<StageBOutputItem> {
    let db_lang = db.clone();
    let out_lang = blocking(move || Ok(load_output_language(&db_lang))).await?;
    let system_prefix = build_system_prefix(STAGE_B_SYSTEM_PROMPT, profile, &out_lang);
    let payload = serde_json::to_string(&vec![input_item])?;

    let outcome = call_gemini_structured(
        db,
        limiter,
        pricing,
        api_key,
        Some(project_id),
        "regen",
        model_id,
        &system_prefix,
        &payload,
        stage_b_response_schema(),
        600,
        true,
    )
    .await?;

    let mut items = parse_and_validate_stage_b(&outcome.text)?;
    items
        .pop()
        .ok_or_else(|| AppError::Validation("Réponse vide lors de la régénération ciblée.".to_string()))
}

fn extract_candidate_text(val: &serde_json::Value) -> Option<String> {
    if let Some(parts) = val
        .pointer("/candidates/0/content/parts")
        .and_then(|v| v.as_array())
    {
        for part in parts.iter().rev() {
            if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn strip_markdown_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim().strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim().strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    }
}

fn format_api_error(status: StatusCode, body: &str) -> String {
    let api_msg = serde_json::from_str::<GeminiErrorEnvelope>(body)
        .ok()
        .and_then(|env| env.error)
        .and_then(|err| match (err.status, err.message) {
            (Some(s), Some(m)) => Some(format!("{s} : {m}")),
            (None, Some(m)) => Some(m),
            (Some(s), None) => Some(s),
            (None, None) => None,
        });
    let detail = api_msg.unwrap_or_else(|| format!("HTTP {}", status.as_u16()));

    match status {
        StatusCode::BAD_REQUEST if !body.to_ascii_lowercase().contains("api key") => format!(
            "Requête rejetée par Google AI Studio ({detail}). Vérifiez le modèle sélectionné dans Paramètres."
        ),
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => format!(
            "Clé API refusée par Google AI Studio ({detail}). Utilisez une clé générée sur aistudio.google.com : un abonnement Google AI Pro ne fournit pas d'accès API."
        ),
        StatusCode::TOO_MANY_REQUESTS => {
            format!("Limite de requêtes (HTTP 429) atteinte sur Google AI Studio ({detail}).")
        }
        _ => format!("Erreur Google AI Studio : {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_pro_subscription_on_auth_errors() {
        let body = r#"{"error":{"code":400,"message":"API key not valid.","status":"INVALID_ARGUMENT"}}"#;
        let msg = format_api_error(StatusCode::BAD_REQUEST, body);
        assert!(msg.contains("INVALID_ARGUMENT"));
        assert!(msg.contains("Google AI Pro"));
    }

    #[test]
    fn validates_and_normalizes_stage_a_and_b_schemas() {
        let a_raw = r#"[{"i":12,"k":1,"s":115,"c":"CLUTCH","a":-40.0,"b":1.5,"r":"un deux trois quatre cinq six sept huit neuf dix onze douze treize"}]"#;
        let a = parse_and_validate_stage_a(a_raw).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].s, 100);
        assert_eq!(a[0].c, "clutch");
        assert_eq!(a[0].a, -25.0);
        assert_eq!(a[0].r.split_whitespace().count(), 12);

        let b_raw = r##"[{"i":12,"hook":"un deux trois quatre cinq six sept huit neuf","title":"Titre très long qui dépasse largement la limite autorisée de soixante caractères maximum","caption":"Incroyable action","tags":["#clutch","gaming"]}]"##;
        let b = parse_and_validate_stage_b(b_raw).unwrap();
        assert_eq!(b[0].hook.split_whitespace().count(), 8);
        assert!(b[0].title.chars().count() <= 60);
        assert_eq!(b[0].tags[0], "clutch");
        assert_eq!(b[0].keep, 1);
        assert_eq!(encode_base64(b"Man"), "TWFu");
        assert_eq!(encode_base64(b"Ma"), "TWE=");
    }
}