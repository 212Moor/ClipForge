use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Erreur I/O : {0}")]
    Io(#[from] std::io::Error),

    #[error("Erreur base de données : {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Erreur sérialisation JSON : {0}")]
    Json(#[from] serde_json::Error),

    #[error("Tâche interrompue : {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("Erreur de configuration : {0}")]
    Config(String),

    #[error("Erreur interne : {0}")]
    Internal(String),

    #[error("Erreur réseau : {0}")]
    Network(String),

    #[error("Erreur sidecar : {0}")]
    Sidecar(String),

    #[error("Échec de vérification de sécurité : {0}")]
    Security(String),

    #[error("Erreur Gestionnaire d'identifiants Windows : {0}")]
    Keyring(String),

    #[error("Erreur API Gemini : {0}")]
    Gemini(String),

    #[error("Plafond de tokens atteint : {0}")]
    BudgetExceeded(String),

    #[error("Traitement mis en pause ou annulé : {0}")]
    Cancelled(String),

    #[error("Validation invalide : {0}")]
    Validation(String),
}

pub type AppResult<T> = Result<T, AppError>;

/// Erreur typée exposée au front via IPC (jamais de trace sensible ni de secret).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

impl IpcError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl From<AppError> for IpcError {
    fn from(err: AppError) -> Self {
        let code = match &err {
            AppError::Io(_) => "IO_ERROR",
            AppError::Database(_) => "DB_ERROR",
            AppError::Json(_) => "JSON_ERROR",
            AppError::Join(_) => "JOIN_ERROR",
            AppError::Config(_) => "CONFIG_ERROR",
            AppError::Internal(_) => "INTERNAL_ERROR",
            AppError::Network(_) => "NETWORK_ERROR",
            AppError::Sidecar(_) => "SIDECAR_ERROR",
            AppError::Security(_) => "SECURITY_ERROR",
            AppError::Keyring(_) => "KEYRING_ERROR",
            AppError::Gemini(_) => "GEMINI_ERROR",
            AppError::BudgetExceeded(_) => "BUDGET_EXCEEDED",
            AppError::Cancelled(_) => "CANCELLED",
            AppError::Validation(_) => "VALIDATION_ERROR",
        };
        Self::new(code, err.to_string())
    }
}

impl From<tokio::task::JoinError> for IpcError {
    fn from(err: tokio::task::JoinError) -> Self {
        AppError::Join(err).into()
    }
}
