use serde_json::Value;
use std::sync::OnceLock;

const TAURI_CONF_RAW: &str = include_str!("../tauri.conf.json");
const FALLBACK_REPO: &str = "VOTRE_COMPTE/clipforge";

#[derive(Debug, Clone)]
pub struct ReleaseConfig {
    pub github_repo: String,
    pub updater_configured: bool,
    pub sidecar_manifest_url: String,
    pub sidecar_manifest_sig_url: String,
}

static RELEASE_CONFIG: OnceLock<ReleaseConfig> = OnceLock::new();

/// Lit une seule fois `owner/repo` depuis `tauri.conf.json` (plugins.updater.endpoints) :
/// un seul remplacement dans ce fichier met à jour toute l'application.
pub fn release_config() -> &'static ReleaseConfig {
    RELEASE_CONFIG.get_or_init(|| {
        let parsed: Value = serde_json::from_str(TAURI_CONF_RAW).unwrap_or(Value::Null);

        let endpoint = parsed
            .pointer("/plugins/updater/endpoints/0")
            .and_then(Value::as_str)
            .unwrap_or_default();

        let github_repo =
            extract_github_repo(endpoint).unwrap_or_else(|| FALLBACK_REPO.to_string());

        let pubkey = parsed
            .pointer("/plugins/updater/pubkey")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();

        let updater_configured = !pubkey.is_empty()
            && !pubkey.starts_with("REMPLACER_")
            && !github_repo.starts_with("VOTRE_COMPTE/");

        ReleaseConfig {
            sidecar_manifest_url: format!(
                "https://github.com/{github_repo}/releases/latest/download/sidecars-manifest.json"
            ),
            sidecar_manifest_sig_url: format!(
                "https://github.com/{github_repo}/releases/latest/download/sidecars-manifest.json.sig"
            ),
            github_repo,
            updater_configured,
        }
    })
}

fn extract_github_repo(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_owner_and_repo() {
        assert_eq!(
            extract_github_repo("https://github.com/acme/clipforge/releases/latest/download/latest.json"),
            Some("acme/clipforge".to_string())
        );
        assert_eq!(extract_github_repo("https://example.com/a/b"), None);
        assert_eq!(extract_github_repo("https://github.com//b"), None);
    }

    #[test]
    fn embedded_tauri_conf_is_valid_json() {
        let parsed: Value = serde_json::from_str(TAURI_CONF_RAW).expect("tauri.conf.json valide");
        assert!(parsed.pointer("/plugins/updater/endpoints/0").is_some());
    }
}
