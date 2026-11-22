use crate::error::{AppError, AppResult};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use specta::Type;

const KEYRING_SERVICE: &str = "app.clipforge.desktop";
const KEYRING_ACCOUNT: &str = "gemini_api_key";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GeminiKeyStatus {
    pub configured: bool,
    pub masked_preview: Option<String>,
    pub storage_backend: String,
}

fn entry() -> AppResult<Entry> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Keyring(format!("initialisation impossible : {e}")))
}

/// Stocke la clé API dans le Gestionnaire d'identifiants Windows (jamais journalisée).
pub fn save_api_key(raw_key: &str) -> AppResult<GeminiKeyStatus> {
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config(
            "La clé API Gemini ne peut pas être vide.".to_string(),
        ));
    }

    entry()?
        .set_password(trimmed)
        .map_err(|e| AppError::Keyring(format!("échec d'enregistrement : {e}")))?;

    tracing::info!("Clé API Gemini enregistrée dans le Gestionnaire d'identifiants Windows");
    Ok(GeminiKeyStatus {
        configured: true,
        masked_preview: Some(mask_key(trimmed)),
        storage_backend: backend_label(),
    })
}

/// Lit la clé API depuis le Gestionnaire d'identifiants Windows.
pub fn load_api_key() -> AppResult<Option<String>> {
    match entry()?.get_password() {
        Ok(secret) => {
            let trimmed = secret.trim().to_string();
            Ok(if trimmed.is_empty() { None } else { Some(trimmed) })
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Keyring(format!("échec de lecture : {e}"))),
    }
}

pub fn delete_api_key() -> AppResult<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => {
            tracing::info!("Clé API Gemini supprimée du Gestionnaire d'identifiants Windows");
            Ok(())
        }
        Err(e) => Err(AppError::Keyring(format!("échec de suppression : {e}"))),
    }
}

pub fn get_key_status() -> AppResult<GeminiKeyStatus> {
    let existing = load_api_key()?;
    Ok(GeminiKeyStatus {
        configured: existing.is_some(),
        masked_preview: existing.as_deref().map(mask_key),
        storage_backend: backend_label(),
    })
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".to_string();
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}••••••••{suffix}")
}

fn backend_label() -> String {
    if cfg!(target_os = "windows") {
        "Windows Credential Manager".to_string()
    } else {
        "OS Keyring".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::mask_key;

    #[test]
    fn masks_long_and_short_keys() {
        assert_eq!(mask_key("AIzaSyABCDEFGHIJKLMNOP"), "AIza••••••••MNOP");
        assert_eq!(mask_key("short"), "••••••••");
    }
}
