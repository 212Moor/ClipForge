use crate::config::release_config;
use crate::error::{AppError, AppResult};
use crate::sidecars::gpu::WhisperVariant;
use chrono::DateTime;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Component, Path};
use std::time::Duration;

const EMBEDDED_MANIFEST_RAW: &str = include_str!("../../resources/sidecars-manifest.json");
const MAX_MANIFEST_BYTES: usize = 1_000_000;

/// Clé publique Ed25519 (64 caractères hex = 32 octets) qui signe le manifeste distant.
/// Générez-la avec `npm run manifest:keygen`. Tant qu'elle reste au placeholder,
/// seul le manifeste embarqué est utilisé.
pub const SIDECAR_MANIFEST_PUBKEY_HEX: &str = "REMPLACER_PAR_CLE_PUBLIQUE_ED25519_HEX_64_CHARS";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SidecarId {
    Ffmpeg,
    YtDlp,
    TwitchDownloader,
    WhisperCpp,
}

impl SidecarId {
    pub const ALL: [SidecarId; 4] = [
        SidecarId::Ffmpeg,
        SidecarId::YtDlp,
        SidecarId::TwitchDownloader,
        SidecarId::WhisperCpp,
    ];

    pub fn dir_name(&self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::YtDlp => "yt-dlp",
            Self::TwitchDownloader => "twitch-downloader",
            Self::WhisperCpp => "whisper-cpp",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ffmpeg => "FFmpeg",
            Self::YtDlp => "yt-dlp",
            Self::TwitchDownloader => "TwitchDownloaderCLI",
            Self::WhisperCpp => "whisper.cpp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    Binary,
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ToolManifestEntry {
    pub id: SidecarId,
    pub variant: String,
    pub version: String,
    pub url: String,
    pub archive_kind: ArchiveKind,
    pub download_sha256: String,
    pub executable_rel_path: String,
    pub executable_sha256: String,
    pub self_test_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SidecarManifest {
    pub schema_version: u32,
    pub published_at: String,
    pub tools: Vec<ToolManifestEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ManifestSource {
    Embedded,
    RemoteSigned,
}

#[derive(Debug, Clone)]
pub struct ResolvedManifest {
    pub source: ManifestSource,
    pub warning: Option<String>,
    pub manifest: SidecarManifest,
}

impl SidecarManifest {
    pub fn find_entry(&self, id: SidecarId, whisper_variant: WhisperVariant) -> Option<&ToolManifestEntry> {
        let wanted = match id {
            SidecarId::WhisperCpp => whisper_variant.as_str(),
            _ => "default",
        };
        self.tools
            .iter()
            .find(|t| t.id == id && t.variant.eq_ignore_ascii_case(wanted))
    }

    /// Variantes de whisper.cpp réellement proposées (il n'existe pas de binaire Vulkan officiel).
    pub fn available_whisper_variants(&self) -> Vec<WhisperVariant> {
        WhisperVariant::ALL
            .into_iter()
            .filter(|variant| {
                self.tools.iter().any(|t| {
                    t.id == SidecarId::WhisperCpp && t.variant.eq_ignore_ascii_case(variant.as_str())
                })
            })
            .collect()
    }

    fn published_at(&self) -> AppResult<DateTime<chrono::FixedOffset>> {
        DateTime::parse_from_rfc3339(&self.published_at)
            .map_err(|e| AppError::Config(format!("published_at invalide dans le manifeste : {e}")))
    }

    /// Contrôles de cohérence appliqués à tout manifeste, embarqué ou distant.
    pub fn validate(&self) -> AppResult<()> {
        if self.schema_version != 1 {
            return Err(AppError::Config(format!(
                "Version de schéma du manifeste non supportée : {}",
                self.schema_version
            )));
        }
        self.published_at()?;

        for tool in &self.tools {
            if !tool.url.starts_with("https://") {
                return Err(AppError::Security(format!(
                    "URL non HTTPS refusée dans le manifeste pour {}",
                    tool.id.display_name()
                )));
            }
            let rel = Path::new(&tool.executable_rel_path);
            let unsafe_path = rel.as_os_str().is_empty()
                || rel
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)));
            if unsafe_path {
                return Err(AppError::Security(format!(
                    "Chemin d'exécutable dangereux dans le manifeste pour {}",
                    tool.id.display_name()
                )));
            }
        }
        Ok(())
    }
}

pub fn load_embedded_manifest() -> AppResult<SidecarManifest> {
    let manifest: SidecarManifest = serde_json::from_str(EMBEDDED_MANIFEST_RAW)?;
    manifest.validate()?;
    Ok(manifest)
}

/// Vérifie une signature Ed25519 (128 caractères hex = 64 octets) sur les octets bruts du manifeste.
pub fn verify_ed25519_signature(payload: &[u8], signature_hex: &str, pubkey_hex: &str) -> AppResult<()> {
    let pubkey_hex = pubkey_hex.trim();
    if pubkey_hex.starts_with("REMPLACER_") || pubkey_hex.len() != 64 {
        return Err(AppError::Security(
            "Clé publique Ed25519 du manifeste distant non configurée".to_string(),
        ));
    }

    let pk_arr: [u8; 32] = hex::decode(pubkey_hex)
        .map_err(|_| AppError::Security("Clé publique Ed25519 invalide (hex)".to_string()))?
        .try_into()
        .map_err(|_| AppError::Security("Longueur de clé publique Ed25519 invalide".to_string()))?;

    let sig_arr: [u8; 64] = hex::decode(signature_hex.trim())
        .map_err(|_| AppError::Security("Signature Ed25519 invalide (hex)".to_string()))?
        .try_into()
        .map_err(|_| AppError::Security("Longueur de signature Ed25519 invalide".to_string()))?;

    let verifying_key = VerifyingKey::from_bytes(&pk_arr)
        .map_err(|e| AppError::Security(format!("Clé publique Ed25519 rejetée : {e}")))?;

    verifying_key
        .verify(payload, &Signature::from_bytes(&sig_arr))
        .map_err(|_| AppError::Security("Signature Ed25519 du manifeste distant invalide".to_string()))
}

/// Récupère le manifeste distant signé s'il est demandé et configuré ; sinon (ou en cas d'échec)
/// bascule sur le manifeste embarqué. Un manifeste distant plus ancien que l'embarqué est refusé
/// (protection contre le rejeu d'un ancien manifeste signé).
pub async fn resolve_manifest(try_remote: bool) -> AppResult<ResolvedManifest> {
    let embedded = load_embedded_manifest()?;

    if !try_remote {
        return Ok(ResolvedManifest {
            source: ManifestSource::Embedded,
            warning: None,
            manifest: embedded,
        });
    }

    if SIDECAR_MANIFEST_PUBKEY_HEX.starts_with("REMPLACER_") {
        return Ok(ResolvedManifest {
            source: ManifestSource::Embedded,
            warning: Some(
                "Manifeste embarqué utilisé : la clé publique Ed25519 du manifeste distant n'est pas encore configurée."
                    .to_string(),
            ),
            manifest: embedded,
        });
    }

    let cfg = release_config();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("ClipForge-SidecarManager/0.1")
        .build()
        .map_err(|e| AppError::Network(e.to_string()))?;

    match fetch_and_verify_remote(
        &client,
        &cfg.sidecar_manifest_url,
        &cfg.sidecar_manifest_sig_url,
        &embedded,
    )
    .await
    {
        Ok(remote) => {
            tracing::info!(published_at = %remote.published_at, "Manifeste distant Ed25519 vérifié");
            Ok(ResolvedManifest {
                source: ManifestSource::RemoteSigned,
                warning: None,
                manifest: remote,
            })
        }
        Err(err) => {
            tracing::warn!(error = %err, "Repli sur le manifeste embarqué");
            Ok(ResolvedManifest {
                source: ManifestSource::Embedded,
                warning: Some(format!("Manifeste distant ignoré ({err}). Manifeste embarqué actif.")),
                manifest: embedded,
            })
        }
    }
}

async fn download_small(client: &reqwest::Client, url: &str, what: &str) -> AppResult<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "HTTP {} lors du téléchargement de {what}",
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Network(e.to_string()))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(AppError::Security(format!("{what} anormalement volumineux")));
    }
    Ok(bytes.to_vec())
}

async fn fetch_and_verify_remote(
    client: &reqwest::Client,
    manifest_url: &str,
    sig_url: &str,
    embedded: &SidecarManifest,
) -> AppResult<SidecarManifest> {
    let manifest_bytes = download_small(client, manifest_url, "le manifeste distant").await?;
    let sig_bytes = download_small(client, sig_url, "la signature Ed25519").await?;
    let sig_text = String::from_utf8_lossy(&sig_bytes).to_string();

    verify_ed25519_signature(&manifest_bytes, &sig_text, SIDECAR_MANIFEST_PUBKEY_HEX)?;

    let remote: SidecarManifest = serde_json::from_slice(&manifest_bytes)?;
    remote.validate()?;

    if remote.published_at()? < embedded.published_at()? {
        return Err(AppError::Security(
            "Le manifeste distant est plus ancien que le manifeste embarqué (rétrogradation refusée)."
                .to_string(),
        ));
    }
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn embedded_manifest_parses_and_validates() {
        let manifest = load_embedded_manifest().expect("manifeste embarqué valide");
        assert!(manifest.find_entry(SidecarId::Ffmpeg, WhisperVariant::Cpu).is_some());
        assert!(manifest.find_entry(SidecarId::YtDlp, WhisperVariant::Cpu).is_some());
        assert!(manifest.find_entry(SidecarId::TwitchDownloader, WhisperVariant::Cpu).is_some());
        assert!(manifest.find_entry(SidecarId::WhisperCpp, WhisperVariant::Cpu).is_some());
        assert!(!manifest.available_whisper_variants().is_empty());
    }

    #[test]
    fn ed25519_roundtrip_and_tamper_detection() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
        let payload = br#"{"schema_version":1}"#;
        let sig_hex = hex::encode(signing.sign(payload).to_bytes());

        assert!(verify_ed25519_signature(payload, &sig_hex, &pubkey_hex).is_ok());
        assert!(verify_ed25519_signature(b"autre contenu", &sig_hex, &pubkey_hex).is_err());
        assert!(verify_ed25519_signature(payload, &sig_hex, "REMPLACER_PAR_CLE").is_err());
    }

    #[test]
    fn rejects_unsafe_manifest_content() {
        let mut manifest = load_embedded_manifest().unwrap();
        manifest.tools[0].url = "http://exemple.test/a.zip".to_string();
        assert!(manifest.validate().is_err());

        let mut manifest = load_embedded_manifest().unwrap();
        manifest.tools[0].executable_rel_path = "../evil.exe".to_string();
        assert!(manifest.validate().is_err());
    }
}
