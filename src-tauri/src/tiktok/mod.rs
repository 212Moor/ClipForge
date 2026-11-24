pub mod studio;
use crate::error::{AppError, AppResult};
use crate::util::blocking;
use chrono::Utc;
use keyring::{Entry, Error as KeyringError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

const KEYRING_SERVICE: &str = "app.clipforge.desktop";
const KEYRING_TIKTOK_APP: &str = "tiktok_app_config";
const KEYRING_TIKTOK_TOKENS: &str = "tiktok_oauth_tokens";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:48321/tiktok/callback";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAppConfig {
    client_key: String,
    client_secret: String,
    redirect_uri: String,
    #[serde(default = "default_true")]
    use_pkce: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: String,
    open_id: String,
    expires_at_unix: i64,
    creator_nickname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TikTokStatus {
    pub client_configured: bool,
    pub connected: bool,
    pub client_key_preview: Option<String>,
    pub redirect_uri: String,
    pub use_pkce: bool,
    pub creator_nickname: Option<String>,
    pub open_id: Option<String>,
    pub expires_at_iso: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TikTokCredentialsInput {
    pub client_key: String,
    pub client_secret: String,
    pub redirect_uri: Option<String>,
    pub use_pkce: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TikTokUploadInput {
    pub clip_id: String,
    /// "inbox" (brouillon TikTok, fonctionne immédiatement sans audit) ou "direct" (publication sur le profil)
    pub mode: String,
    pub title: String,
    /// "SELF_ONLY", "MUTUAL_FOLLOW_FRIENDS", "FOLLOWER_OF_CREATOR" ou "PUBLIC_TO_EVERYONE"
    pub privacy_level: String,
    pub disable_comment: bool,
    pub disable_duet: bool,
    pub disable_stitch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TikTokUploadResult {
    pub publish_id: String,
    pub mode: String,
    pub status: String,
    pub message: String,
}

fn keyring_entry(account: &str) -> AppResult<Entry> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| AppError::Keyring(format!("initialisation TikTok impossible : {e}")))
}

fn load_app_config() -> AppResult<Option<StoredAppConfig>> {
    match keyring_entry(KEYRING_TIKTOK_APP)?.get_password() {
        Ok(raw) => serde_json::from_str::<StoredAppConfig>(&raw)
            .map(Some)
            .map_err(|e| AppError::Config(format!("Configuration TikTok corrompue : {e}"))),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Keyring(format!("lecture config TikTok : {e}"))),
    }
}

fn load_tokens() -> AppResult<Option<StoredTokens>> {
    match keyring_entry(KEYRING_TIKTOK_TOKENS)?.get_password() {
        Ok(raw) => serde_json::from_str::<StoredTokens>(&raw)
            .map(Some)
            .map_err(|e| AppError::Config(format!("Jetons TikTok corrompus : {e}"))),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Keyring(format!("lecture jetons TikTok : {e}"))),
    }
}

fn save_tokens(tokens: &StoredTokens) -> AppResult<()> {
    let raw = serde_json::to_string(tokens)?;
    keyring_entry(KEYRING_TIKTOK_TOKENS)?
        .set_password(&raw)
        .map_err(|e| AppError::Keyring(format!("enregistrement jetons TikTok : {e}")))
}

pub fn save_credentials(input: TikTokCredentialsInput) -> AppResult<TikTokStatus> {
    let client_key = input.client_key.trim().to_string();
    let client_secret = input.client_secret.trim().to_string();
    if client_key.is_empty() || client_secret.is_empty() {
        return Err(AppError::Config(
            "Renseignez à la fois la Client Key et le Client Secret TikTok.".to_string(),
        ));
    }
    let redirect_uri = input
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_REDIRECT_URI)
        .to_string();

    let cfg = StoredAppConfig {
        client_key,
        client_secret,
        redirect_uri,
        use_pkce: input.use_pkce.unwrap_or(true),
    };
    let raw = serde_json::to_string(&cfg)?;
    keyring_entry(KEYRING_TIKTOK_APP)?
        .set_password(&raw)
        .map_err(|e| AppError::Keyring(format!("enregistrement config TikTok : {e}")))?;

    get_status()
}

pub fn disconnect() -> AppResult<TikTokStatus> {
    match keyring_entry(KEYRING_TIKTOK_TOKENS)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => {}
        Err(e) => return Err(AppError::Keyring(format!("déconnexion TikTok : {e}"))),
    }
    get_status()
}

pub fn get_status() -> AppResult<TikTokStatus> {
    let cfg = load_app_config()?;
    let tokens = load_tokens()?;

    let client_key_preview = cfg.as_ref().map(|c| mask_value(&c.client_key));
    let redirect_uri = cfg
        .as_ref()
        .map(|c| c.redirect_uri.clone())
        .unwrap_or_else(|| DEFAULT_REDIRECT_URI.to_string());
    let use_pkce = cfg.as_ref().map(|c| c.use_pkce).unwrap_or(true);

    let expires_at_iso = tokens
        .as_ref()
        .and_then(|t| chrono::DateTime::from_timestamp(t.expires_at_unix, 0))
        .map(|dt| dt.to_rfc3339());

    Ok(TikTokStatus {
        client_configured: cfg.is_some(),
        connected: tokens.is_some(),
        client_key_preview,
        redirect_uri,
        use_pkce,
        creator_nickname: tokens.as_ref().and_then(|t| t.creator_nickname.clone()),
        open_id: tokens.as_ref().map(|t| t.open_id.clone()),
        expires_at_iso,
    })
}

fn mask_value(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".to_string();
    }
    let pre: String = chars[..4].iter().collect();
    let suf: String = chars[chars.len() - 4..].iter().collect();
    format!("{pre}••••{suf}")
}

fn build_client(timeout_secs: u64) -> AppResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(timeout_secs))
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .user_agent("ClipForge/0.2")
        .build()
        .map_err(|e| AppError::Network(e.to_string()))
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex_str, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn extract_bind_addr(redirect_uri: &str) -> String {
    let stripped = redirect_uri
        .strip_prefix("http://")
        .or_else(|| redirect_uri.strip_prefix("https://"))
        .unwrap_or("127.0.0.1:48321/tiktok/callback");
    let host_port = stripped.split('/').next().unwrap_or("127.0.0.1:48321");
    if host_port.starts_with("localhost:") {
        host_port.replacen("localhost:", "127.0.0.1:", 1)
    } else if host_port.contains(':') {
        host_port.to_string()
    } else {
        "127.0.0.1:48321".to_string()
    }
}

pub fn extract_callback_query_params(http_request: &str) -> Vec<(String, String)> {
    let first_line = http_request.lines().next().unwrap_or_default();
    let target = first_line.split_whitespace().nth(1).unwrap_or_default();
    let Some((_, query)) = target.split_once('?') else {
        return Vec::new();
    };
    query
        .split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((url_decode(k), url_decode(v)))
        })
        .collect()
}

fn sha256_hex(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Lance le flux OAuth 2.0 officiel TikTok dans le navigateur et récupère automatiquement le code
/// via un serveur HTTP local temporaire sur `127.0.0.1:48321`.
pub async fn start_oauth_login() -> AppResult<TikTokStatus> {
    let cfg = blocking(load_app_config).await?.ok_or_else(|| {
        AppError::Config(
            "Enregistrez d'abord votre Client Key et Client Secret TikTok (developers.tiktok.com)."
                .to_string(),
        )
    })?;

    let state = Uuid::new_v4().to_string().replace('-', "");
    let code_verifier = format!(
        "{}{}",
        Uuid::new_v4().to_string().replace('-', ""),
        Uuid::new_v4().to_string().replace('-', "")
    );
    let code_challenge = sha256_hex(&code_verifier);
    let scopes = "user.info.basic,video.upload,video.publish";

    let mut auth_url = format!(
        "https://www.tiktok.com/v2/auth/authorize/?client_key={}&scope={}&response_type=code&redirect_uri={}&state={}",
        url_encode(&cfg.client_key),
        url_encode(scopes),
        url_encode(&cfg.redirect_uri),
        url_encode(&state),
    );
    if cfg.use_pkce {
        auth_url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            url_encode(&code_challenge)
        ));
    }

    let bind_addr = extract_bind_addr(&cfg.redirect_uri);
    let listener = TcpListener::bind(&bind_addr).map_err(|e| {
        AppError::Network(format!(
            "Impossible d'écouter sur {bind_addr} pour le retour TikTok : {e}"
        ))
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::Network(format!("Configuration socket OAuth : {e}")))?;

    // Ouvre la page de connexion TikTok dans le navigateur par défaut
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &auth_url])
            .spawn();
    }

    let expected_state = state.clone();
    let auth_code = blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if Instant::now() > deadline {
                return Err(AppError::Network(
                    "Délai expiré (3 min) en attendant la validation sur TikTok.".to_string(),
                ));
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let params = extract_callback_query_params(&req);

                    let err_param = params
                        .iter()
                        .find(|(k, _)| k == "error_description" || k == "error")
                        .map(|(_, v)| v.clone());
                    let code_param = params
                        .iter()
                        .find(|(k, _)| k == "code")
                        .map(|(_, v)| v.clone());
                    let ret_state = params
                        .iter()
                        .find(|(k, _)| k == "state")
                        .map(|(_, v)| v.clone());

                    let (html_body, outcome) = if let Some(err_msg) = err_param {
                        (
                            format!("<html><body style='font-family:sans-serif;background:#0b0b12;color:#fff;padding:40px'><h2>Connexion TikTok annulée</h2><p>{err_msg}</p></body></html>"),
                            Err(AppError::Network(format!("Autorisation TikTok refusée : {err_msg}"))),
                        )
                    } else if let Some(code) = code_param {
                        if ret_state.as_deref() != Some(expected_state.as_str()) {
                            (
                                "<html><body style='font-family:sans-serif;background:#0b0b12;color:#fff;padding:40px'><h2>État OAuth invalide</h2></body></html>".to_string(),
                                Err(AppError::Security("Paramètre state OAuth TikTok invalide.".to_string())),
                            )
                        } else {
                            (
                                "<html><body style='font-family:sans-serif;background:#0b0b12;color:#fff;padding:40px'><h2>Connexion TikTok réussie !</h2><p>Vous pouvez fermer cet onglet et revenir dans ClipForge.</p></body></html>".to_string(),
                                Ok(code),
                            )
                        }
                    } else {
                        // Ignore les requêtes parasites (ex. /favicon.ico)
                        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                        continue;
                    };

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html_body.len(),
                        html_body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                    return outcome;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(120));
                }
                Err(e) => {
                    return Err(AppError::Network(format!("Erreur serveur local OAuth : {e}")));
                }
            }
        }
    })
    .await?;

    // Échange du code contre access_token + refresh_token
    let mut form_parts = vec![
        format!("client_key={}", url_encode(&cfg.client_key)),
        format!("client_secret={}", url_encode(&cfg.client_secret)),
        format!("code={}", url_encode(&auth_code)),
        "grant_type=authorization_code".to_string(),
        format!("redirect_uri={}", url_encode(&cfg.redirect_uri)),
    ];
    if cfg.use_pkce {
        form_parts.push(format!("code_verifier={}", url_encode(&code_verifier)));
    }
    let form_body = form_parts.join("&");

    let client = build_client(30)?;
    let resp = client
        .post("https://open.tiktokapis.com/v2/oauth/token/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Échange de jeton TikTok impossible : {e}")))?;

    let status = resp.status();
    let raw = resp.text().await.unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    if !status.is_success() || access_token.is_none() {
        let err_desc = json
            .get("error_description")
            .or_else(|| json.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or(&raw);
        return Err(AppError::Network(format!(
            "Connexion TikTok refusée (HTTP {}) : {}",
            status.as_u16(),
            err_desc.chars().take(240).collect::<String>()
        )));
    }

    let access_token = access_token.unwrap_or_default();
    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let open_id = json
        .get("open_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(86_400);

    // Récupère le pseudo créateur (optionnel)
    let creator_nickname = fetch_creator_nickname(&client, &access_token).await;

    let tokens = StoredTokens {
        access_token,
        refresh_token,
        open_id,
        expires_at_unix: Utc::now().timestamp() + expires_in,
        creator_nickname,
    };
    blocking(move || save_tokens(&tokens)).await?;
    blocking(get_status).await
}

async fn fetch_creator_nickname(client: &Client, access_token: &str) -> Option<String> {
    let resp = client
        .post("https://open.tiktokapis.com/v2/post/publish/creator_info/query/")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json; charset=UTF-8")
        .body("{}")
        .send()
        .await
        .ok()?;
    let val: serde_json::Value = resp.json().await.ok()?;
    val.pointer("/data/creator_nickname")
        .or_else(|| val.pointer("/data/creator_username"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

async fn ensure_valid_access_token() -> AppResult<String> {
    let (cfg, tokens) = blocking(|| Ok((load_app_config()?, load_tokens()?))).await?;
    let Some(mut tokens) = tokens else {
        return Err(AppError::Config(
            "Aucun compte TikTok connecté. Connectez votre compte dans le menu TikTok.".to_string(),
        ));
    };

    if tokens.expires_at_unix > Utc::now().timestamp() + 300 {
        return Ok(tokens.access_token);
    }

    let Some(cfg) = cfg else {
        return Ok(tokens.access_token);
    };
    if tokens.refresh_token.is_empty() {
        return Err(AppError::Config(
            "Session TikTok expirée : reconnectez votre compte TikTok.".to_string(),
        ));
    }

    let form_body = format!(
        "client_key={}&client_secret={}&grant_type=refresh_token&refresh_token={}",
        url_encode(&cfg.client_key),
        url_encode(&cfg.client_secret),
        url_encode(&tokens.refresh_token)
    );
    let client = build_client(30)?;
    let resp = client
        .post("https://open.tiktokapis.com/v2/oauth/token/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Rafraîchissement session TikTok : {e}")))?;

    let raw = resp.text().await.unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    let Some(new_access) = json.get("access_token").and_then(|v| v.as_str()) else {
        return Err(AppError::Config(
            "Impossible de rafraîchir la session TikTok : reconnectez votre compte.".to_string(),
        ));
    };

    tokens.access_token = new_access.to_string();
    if let Some(new_ref) = json.get("refresh_token").and_then(|v| v.as_str()) {
        tokens.refresh_token = new_ref.to_string();
    }
    let exp = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(86_400);
    tokens.expires_at_unix = Utc::now().timestamp() + exp;

    let access_out = tokens.access_token.clone();
    blocking(move || save_tokens(&tokens)).await?;
    Ok(access_out)
}

/// Calcule le découpage de chunks conforme aux règles strictes TikTok Content Posting API v2 :
/// - <= 64 Mo : 1 seul chunk de taille exacte `video_size`
/// - > 64 Mo : chunks de 10 Mo, le dernier absorbant le reste (garanti >= 5 Mo et < 20 Mo)
pub fn compute_tiktok_chunk_plan(video_size: u64) -> (u64, u64) {
    const MAX_SINGLE: u64 = 64 * 1024 * 1024;
    const MULTI_CHUNK: u64 = 10 * 1024 * 1024;
    if video_size <= MAX_SINGLE {
        (video_size.max(1), 1)
    } else {
        let count = (video_size / MULTI_CHUNK).max(1);
        (MULTI_CHUNK, count)
    }
}

/// Envoie directement un fichier MP4 vers TikTok via Content Posting API v2 (Brouillon Inbox ou Publication directe).
pub async fn upload_mp4_to_tiktok(
    mp4_path: &Path,
    input: &TikTokUploadInput,
) -> AppResult<TikTokUploadResult> {
    if !mp4_path.is_file() {
        return Err(AppError::Config(
            "Fichier MP4 introuvable pour l'envoi TikTok.".to_string(),
        ));
    }

    let access_token = ensure_valid_access_token().await?;
    let video_size = std::fs::metadata(mp4_path)?.len();
    if video_size == 0 {
        return Err(AppError::Validation("Le fichier MP4 est vide.".to_string()));
    }

    let (chunk_size, total_chunk_count) = compute_tiktok_chunk_plan(video_size);
    let is_direct = input.mode == "direct";

    let (init_url, init_body) = if is_direct {
        let safe_title: String = input.title.trim().chars().take(2150).collect();
        let privacy = match input.privacy_level.as_str() {
            "PUBLIC_TO_EVERYONE"
            | "MUTUAL_FOLLOW_FRIENDS"
            | "FOLLOWER_OF_CREATOR"
            | "SELF_ONLY" => input.privacy_level.as_str(),
            _ => "SELF_ONLY",
        };
        (
            "https://open.tiktokapis.com/v2/post/publish/video/init/",
            serde_json::json!({
                "post_info": {
                    "title": safe_title,
                    "privacy_level": privacy,
                    "disable_duet": input.disable_duet,
                    "disable_comment": input.disable_comment,
                    "disable_stitch": input.disable_stitch
                },
                "source_info": {
                    "source": "FILE_UPLOAD",
                    "video_size": video_size,
                    "chunk_size": chunk_size,
                    "total_chunk_count": total_chunk_count
                }
            }),
        )
    } else {
        (
            "https://open.tiktokapis.com/v2/post/publish/inbox/video/init/",
            serde_json::json!({
                "source_info": {
                    "source": "FILE_UPLOAD",
                    "video_size": video_size,
                    "chunk_size": chunk_size,
                    "total_chunk_count": total_chunk_count
                }
            }),
        )
    };

    let client = build_client(300)?;
    let init_resp = client
        .post(init_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json; charset=UTF-8")
        .json(&init_body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Initialisation upload TikTok impossible : {e}")))?;

    let init_status = init_resp.status();
    let init_raw = init_resp.text().await.unwrap_or_default();
    let init_json: serde_json::Value = serde_json::from_str(&init_raw).unwrap_or_default();

    let api_code = init_json
        .pointer("/error/code")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if !init_status.is_success() || (api_code != "ok" && !api_code.is_empty()) {
        let api_msg = init_json
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or(&init_raw);
        let hint = if api_code == "unaudited_client_can_only_post_to_private_accounts" {
            " (Astuce : utilisez le mode Brouillon ou Confidentialité 'Moi uniquement' tant que votre app TikTok n'est pas auditée.)"
        } else {
            ""
        };
        return Err(AppError::Network(format!(
            "TikTok a refusé l'initialisation ({api_code}) : {api_msg}{hint}"
        )));
    }

    let publish_id = init_json
        .pointer("/data/publish_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Network("publish_id absent de la réponse TikTok.".to_string()))?
        .to_string();
    let upload_url = init_json
        .pointer("/data/upload_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Network("upload_url absent de la réponse TikTok.".to_string()))?
        .to_string();

    // Envoi séquentiel des chunks MP4 vers upload_url
    for idx in 0..total_chunk_count {
        let start = idx * chunk_size;
        let end_exclusive = if idx + 1 == total_chunk_count {
            video_size
        } else {
            ((idx + 1) * chunk_size).min(video_size)
        };
        let len = (end_exclusive - start) as usize;
        let mp4_owned = mp4_path.to_path_buf();
        let bytes = blocking(move || {
            let mut file = std::fs::File::open(&mp4_owned)?;
            file.seek(SeekFrom::Start(start))?;
            let mut buf = vec![0u8; len];
            file.read_exact(&mut buf)?;
            Ok(buf)
        })
        .await?;

        let content_range = format!("bytes {start}-{}/{video_size}", end_exclusive - 1);
        let put_resp = client
            .put(&upload_url)
            .header("Content-Type", "video/mp4")
            .header("Content-Range", &content_range)
            .header("Content-Length", len.to_string())
            .body(bytes)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Transfert chunk MP4 vers TikTok : {e}")))?;

        if !put_resp.status().is_success() {
            let put_status = put_resp.status().as_u16(); let err_txt = put_resp.text().await.unwrap_or_default();
            return Err(AppError::Network(format!(
                "Échec upload MP4 TikTok (HTTP {}) : {}",
                put_status,
                err_txt.chars().take(200).collect::<String>()
            )));
        }
    }

    // Interroge le statut de traitement côté TikTok
    let mut final_status = if is_direct {
        "PROCESSING_UPLOAD".to_string()
    } else {
        "SEND_TO_USER_INBOX".to_string()
    };
    if let Ok(st_resp) = client
        .post("https://open.tiktokapis.com/v2/post/publish/status/fetch/")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json; charset=UTF-8")
        .json(&serde_json::json!({ "publish_id": publish_id }))
        .send()
        .await
    {
        if let Ok(st_json) = st_resp.json::<serde_json::Value>().await {
            if let Some(st) = st_json.pointer("/data/status").and_then(|v| v.as_str()) {
                final_status = st.to_string();
            }
        }
    }

    let message = if is_direct {
        format!("Vidéo envoyée sur TikTok (statut : {final_status}).")
    } else {
        format!(
            "Vidéo envoyée dans vos Brouillons / Notifications TikTok (statut : {final_status}). Ouvrez l'app TikTok pour finaliser."
        )
    };

    Ok(TikTokUploadResult {
        publish_id,
        mode: input.mode.clone(),
        status: final_status,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_parses_oauth_params() {
        assert_eq!(url_encode("http://127.0.0.1:48321/a b"), "http%3A%2F%2F127.0.0.1%3A48321%2Fa%20b");
        let req = "GET /tiktok/callback?code=abc%2D123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let params = extract_callback_query_params(req);
        assert!(params.iter().any(|(k, v)| k == "code" && v == "abc-123"));
        assert!(params.iter().any(|(k, v)| k == "state" && v == "xyz"));
    }

    #[test]
    fn computes_valid_tiktok_chunks() {
        assert_eq!(compute_tiktok_chunk_plan(12_000_000), (12_000_000, 1));
        assert_eq!(compute_tiktok_chunk_plan(75_000_000), (10 * 1024 * 1024, 7));
    }
}
