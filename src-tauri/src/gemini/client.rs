use crate::error::{AppError, AppResult};
use crate::store::Database;
use crate::util::blocking;
use chrono::Utc;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::{Duration, Instant};

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const SETTING_STAGE_A_MODEL: &str = "gemini.model.stage_a";
const SETTING_STAGE_B_MODEL: &str = "gemini.model.stage_b";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GeminiModelInfo {
    /// Identifiant canonique renvoyé par `models.list` (ex. `models/...`).
    pub name: String,
    /// Identifiant court, sans préfixe `models/`.
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

fn build_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent("ClipForge/0.1")
        .build()
        .map_err(|e| AppError::Network(e.to_string()))
}

/// Appelle l'endpoint officiel `GET /v1beta/models` (paginé). La clé ne transite que par l'en-tête
/// `x-goog-api-key` : jamais dans l'URL ni dans les journaux.
pub async fn list_models(api_key: &str) -> AppResult<Vec<GeminiModelInfo>> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err(AppError::Gemini(
            "Aucune clé API Gemini renseignée. Créez une clé sur Google AI Studio.".to_string(),
        ));
    }

    let client = build_http_client()?;
    let mut collected = Vec::new();
    let mut page_token: Option<String> = None;

    // Garde-fou de pagination : 10 pages de 100 modèles au maximum.
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

/// Valide une clé via `models.list` (aucun token facturé) et consigne l'appel dans `gemini_calls`.
pub async fn test_api_key(db: &Database, api_key: &str) -> AppResult<GeminiKeyTestResult> {
    let start = Instant::now();
    let models = list_models(api_key).await?;
    let latency_ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let usable: Vec<GeminiModelInfo> = models
        .into_iter()
        .filter(|m| m.supports_generate_content)
        .collect();
    let count = usable.len() as u32;

    // Trace d'audit : 0 token, aucune clé stockée. Un échec d'écriture ne doit pas faire échouer le test.
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

    tracing::info!(models_count = count, latency_ms, "Test de clé API Gemini réussi via models.list");

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
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => format!(
            "Clé API refusée par Google AI Studio ({detail}). Utilisez une clé générée sur aistudio.google.com : un abonnement Google AI Pro ne fournit pas d'accès API."
        ),
        StatusCode::TOO_MANY_REQUESTS => {
            "Limite de requêtes (HTTP 429) atteinte sur Google AI Studio. Réessayez dans quelques secondes."
                .to_string()
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
    fn parses_models_list_payload() {
        let json = r#"{"models":[{"name":"models/gemini-x-flash","displayName":"X Flash","inputTokenLimit":1048576,
            "outputTokenLimit":65536,"supportedGenerationMethods":["generateContent","countTokens"]}],"nextPageToken":""}"#;
        let parsed: ModelsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].input_token_limit, 1_048_576);
    }
}
