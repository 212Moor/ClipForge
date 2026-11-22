pub mod migrations;
pub mod projects;

use crate::error::{AppError, AppResult};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(db_path: &Path) -> AppResult<Self> {
        let mut conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        migrations::run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> AppResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn with_conn<F, T>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&Connection) -> AppResult<T>,
    {
        let guard = self
            .conn
            .lock()
            .map_err(|_| AppError::Internal("Verrou SQLite empoisonné".to_string()))?;
        f(&guard)
    }

    pub fn with_conn_mut<F, T>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&mut Connection) -> AppResult<T>,
    {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| AppError::Internal("Verrou SQLite empoisonné".to_string()))?;
        f(&mut guard)
    }

    pub fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        self.with_conn(|conn| {
            let value = conn
                .query_row(
                    "SELECT value_json FROM settings WHERE key = ?1",
                    [key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            Ok(value)
        })
    }

    pub fn set_setting(&self, key: &str, value_json: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at",
                rusqlite::params![key, value_json, now],
            )?;
            Ok(())
        })
    }
}
