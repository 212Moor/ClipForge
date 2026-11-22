use crate::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

pub fn is_valid_sha256_hex(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn compute_file_sha256(path: &Path) -> AppResult<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Vérifie le SHA-256 d'un fichier géré par ClipForge.
/// En cas d'écart, supprime immédiatement le fichier et renvoie une erreur explicite.
pub fn verify_managed_file_or_delete(path: &Path, expected_sha256: &str) -> AppResult<String> {
    if !is_valid_sha256_hex(expected_sha256) {
        return Err(AppError::Security(
            "Empreinte SHA-256 absente ou non initialisée dans le manifeste. \
             Exécutez scripts/sync-sidecar-manifest.ps1 pour renseigner les empreintes officielles."
                .to_string(),
        ));
    }

    let actual = compute_file_sha256(path)?;
    let expected = expected_sha256.trim().to_ascii_lowercase();

    if actual != expected {
        let deleted = fs::remove_file(path).is_ok();
        tracing::error!(
            path = %path.display(),
            expected = %expected,
            actual = %actual,
            deleted,
            "Échec de vérification SHA-256, fichier supprimé"
        );
        return Err(AppError::Security(format!(
            "Intégrité SHA-256 invalide pour {}. Le fichier a été supprimé par sécurité (attendu : {}, reçu : {}).",
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string()),
            expected,
            actual
        )));
    }

    Ok(actual)
}

struct VerifiedEntry {
    len: u64,
    modified: Option<SystemTime>,
    sha256: String,
}

static VERIFIED: OnceLock<Mutex<HashMap<PathBuf, VerifiedEntry>>> = OnceLock::new();

/// Comme `verify_managed_file_or_delete`, mais évite de re-hacher un binaire volumineux (FFmpeg ≈ 100 Mo)
/// à chaque lancement tant que sa taille et sa date de modification n'ont pas changé.
pub fn verify_managed_file_cached(path: &Path, expected_sha256: &str) -> AppResult<String> {
    let expected = expected_sha256.trim().to_ascii_lowercase();
    let meta = fs::metadata(path)?;
    let modified = meta.modified().ok();
    let cache = VERIFIED.get_or_init(|| Mutex::new(HashMap::new()));

    if is_valid_sha256_hex(&expected) {
        if let Ok(guard) = cache.lock() {
            if let Some(entry) = guard.get(path) {
                if entry.len == meta.len() && entry.modified == modified && entry.sha256 == expected {
                    return Ok(expected);
                }
            }
        }
    }

    let actual = verify_managed_file_or_delete(path, &expected)?;

    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            path.to_path_buf(),
            VerifiedEntry {
                len: meta.len(),
                modified,
                sha256: actual.clone(),
            },
        );
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(content: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clipforge-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.bin");
        fs::write(&path, content).unwrap();
        path
    }

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn validates_hex_format() {
        assert!(is_valid_sha256_hex(ABC_SHA256));
        assert!(!is_valid_sha256_hex(""));
        assert!(!is_valid_sha256_hex("A_GENERER_PAR_SCRIPT_SYNC"));
        assert!(!is_valid_sha256_hex(&"z".repeat(64)));
    }

    #[test]
    fn computes_known_digest() {
        let path = temp_file(b"abc");
        assert_eq!(compute_file_sha256(&path).unwrap(), ABC_SHA256);
    }

    #[test]
    fn accepts_matching_hash_case_insensitively() {
        let path = temp_file(b"abc");
        assert!(verify_managed_file_or_delete(&path, &ABC_SHA256.to_uppercase()).is_ok());
        assert!(path.exists());
    }

    #[test]
    fn deletes_file_on_mismatch() {
        let path = temp_file(b"abc");
        let err = verify_managed_file_or_delete(&path, &"0".repeat(64)).unwrap_err();
        assert!(matches!(err, AppError::Security(_)));
        assert!(!path.exists());
    }

    #[test]
    fn refuses_placeholder_hash_without_touching_file() {
        let path = temp_file(b"abc");
        assert!(verify_managed_file_or_delete(&path, "A_GENERER_PAR_SCRIPT_SYNC").is_err());
        assert!(path.exists());
    }

    #[test]
    fn cached_verification_works_twice() {
        let path = temp_file(b"abc");
        assert!(verify_managed_file_cached(&path, ABC_SHA256).is_ok());
        assert!(verify_managed_file_cached(&path, ABC_SHA256).is_ok());
    }
}
