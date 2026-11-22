use crate::error::AppResult;

/// Exécute une tâche bloquante (SQLite, disque, hachage…) hors du thread async
/// afin de ne jamais figer l'interface.
pub async fn blocking<T, F>(f: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await?
}
