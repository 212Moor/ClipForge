use crate::error::AppResult;
use crate::store::Database;
use chrono::Utc;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

pub fn compute_payload_hash(
    model: &str,
    prompt_version: &str,
    system_prefix: &str,
    payload: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"\n--PV--\n");
    hasher.update(prompt_version.as_bytes());
    hasher.update(b"\n--SYS--\n");
    hasher.update(system_prefix.as_bytes());
    hasher.update(b"\n--PAY--\n");
    hasher.update(payload.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn lookup_cached_response(db: &Database, payload_hash: &str) -> AppResult<Option<String>> {
    db.with_conn(|conn| {
        let val = conn
            .query_row(
                "SELECT response_json FROM gemini_calls
                 WHERE payload_hash = ?1 AND response_json IS NOT NULL AND length(response_json) > 0
                 ORDER BY created_at DESC LIMIT 1",
                [payload_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(val)
    })
}

pub struct RecordCallParams<'a> {
    pub project_id: Option<&'a str>,
    pub stage: &'a str,
    pub payload_hash: &'a str,
    pub model: &'a str,
    pub prompt_version: &'a str,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub estimated_cost_usd: f64,
    pub cache_hit: bool,
    pub duration_ms: u32,
    pub response_json: Option<&'a str>,
}

pub fn record_call(db: &Database, params: &RecordCallParams<'_>) -> AppResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO gemini_calls
             (id, project_id, stage, payload_hash, model, prompt_version,
              input_tokens, output_tokens, cached_tokens, estimated_cost_usd,
              cache_hit, duration_ms, response_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                id,
                params.project_id,
                params.stage,
                params.payload_hash,
                params.model,
                params.prompt_version,
                params.input_tokens,
                params.output_tokens,
                params.cached_tokens,
                params.estimated_cost_usd,
                if params.cache_hit { 1 } else { 0 },
                params.duration_ms,
                params.response_json,
                now
            ],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_and_reuses_exact_response_with_zero_tokens() {
        let db = Database::open_in_memory().unwrap();
        let hash = compute_payload_hash("gemini-test", "v1", "sys", r#"{"i":1}"#);
        assert!(lookup_cached_response(&db, &hash).unwrap().is_none());

        record_call(
            &db,
            &RecordCallParams {
                project_id: None,
                stage: "stage_a",
                payload_hash: &hash,
                model: "gemini-test",
                prompt_version: "v1",
                input_tokens: 1200,
                output_tokens: 80,
                cached_tokens: 0,
                estimated_cost_usd: 0.0005,
                cache_hit: false,
                duration_ms: 420,
                response_json: Some(r#"[{"i":1,"k":1,"s":88}]"#),
            },
        )
        .unwrap();

        let hit = lookup_cached_response(&db, &hash).unwrap();
        assert_eq!(hit.as_deref(), Some(r#"[{"i":1,"k":1,"s":88}]"#));
    }
}
