use crate::error::{AppError, AppResult};
use crate::paths::AppPaths;
use crate::store::projects::AnalysisMode;
use crate::store::Database;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const EMBEDDED_PRICING_RAW: &str = include_str!("../../resources/pricing.json");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelRate {
    pub input_per_million_usd: f64,
    pub output_per_million_usd: f64,
    pub cached_input_per_million_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PricingConfig {
    pub schema_version: u32,
    pub usd_to_eur: f64,
    pub batch_discount: f64,
    #[serde(default)]
    pub note: String,
    pub defaults_by_family: HashMap<String, ModelRate>,
    #[serde(default)]
    pub models: HashMap<String, ModelRate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProjectCostEstimate {
    pub mode: AnalysisMode,
    pub estimated_candidates: u32,
    pub estimated_input_tokens: u32,
    pub estimated_output_tokens: u32,
    pub estimated_cost_usd: f64,
    pub estimated_cost_eur: f64,
    pub stage_a_model: String,
    pub stage_b_model: Option<String>,
    pub uses_batch_discount: bool,
    pub within_target_budget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProjectBudgetSnapshot {
    pub project_id: String,
    pub budget_limit_tokens: u32,
    pub used_input_tokens: u32,
    pub used_output_tokens: u32,
    pub remaining_tokens: u32,
    pub spent_cost_usd: f64,
}

impl Default for PricingConfig {
    fn default() -> Self {
        serde_json::from_str(EMBEDDED_PRICING_RAW).expect("pricing.json embarqué invalide")
    }
}

pub fn load_pricing_config(paths: &AppPaths) -> PricingConfig {
    if !paths.pricing_file.is_file() {
        let _ = fs::write(&paths.pricing_file, EMBEDDED_PRICING_RAW);
    }
    load_pricing_from_path(&paths.pricing_file).unwrap_or_default()
}

pub fn load_pricing_from_path(path: &Path) -> AppResult<PricingConfig> {
    let raw = fs::read_to_string(path)?;
    let parsed: PricingConfig = serde_json::from_str(&raw)?;
    Ok(parsed)
}

pub fn save_pricing_config(paths: &AppPaths, pricing: &PricingConfig) -> AppResult<()> {
    let pretty = serde_json::to_string_pretty(pricing)?;
    fs::write(&paths.pricing_file, pretty)?;
    Ok(())
}

pub fn resolve_model_rate(pricing: &PricingConfig, model_id: &str) -> ModelRate {
    let short_id = model_id.strip_prefix("models/").unwrap_or(model_id);
    if let Some(exact) = pricing.models.get(short_id) {
        return exact.clone();
    }
    let lower = short_id.to_ascii_lowercase();
    let family = if lower.contains("flash-lite") || lower.contains("lite") {
        "flash-lite"
    } else if lower.contains("flash") {
        "flash"
    } else if lower.contains("pro") {
        "pro"
    } else {
        "fallback"
    };

    pricing
        .defaults_by_family
        .get(family)
        .or_else(|| pricing.defaults_by_family.get("fallback"))
        .cloned()
        .unwrap_or(ModelRate {
            input_per_million_usd: 0.30,
            output_per_million_usd: 2.50,
            cached_input_per_million_usd: 0.075,
        })
}

pub fn compute_call_cost_usd(
    pricing: &PricingConfig,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
    is_batch: bool,
) -> f64 {
    let rate = resolve_model_rate(pricing, model_id);
    let uncached_input = input_tokens.saturating_sub(cached_tokens) as f64;
    let cached_input = cached_tokens as f64;
    let output = output_tokens as f64;

    let raw_usd = (uncached_input / 1_000_000.0) * rate.input_per_million_usd
        + (cached_input / 1_000_000.0) * rate.cached_input_per_million_usd
        + (output / 1_000_000.0) * rate.output_per_million_usd;

    if is_batch {
        raw_usd * pricing.batch_discount.clamp(0.1, 1.0)
    } else {
        raw_usd
    }
}

pub fn estimate_tokens_local(text: &str) -> u32 {
    let chars = text.chars().count();
    ((chars as f64 / 3.7).ceil() as u32).max(1)
}

pub fn estimate_project_cost(
    pricing: &PricingConfig,
    vod_duration_sec: f64,
    target_clips: u32,
    mode: AnalysisMode,
    stage_a_model: Option<&str>,
    stage_b_model: Option<&str>,
) -> ProjectCostEstimate {
    let clamped_clips = target_clips.clamp(1, 100);
    let max_by_duration = ((vod_duration_sec.max(600.0) / 110.0).floor() as u32).max(4);
    let estimated_candidates = (clamped_clips * 3).min(300).min(max_by_duration);

    let batch_size = 12u32;
    let num_stage_a_batches = ((estimated_candidates + batch_size - 1) / batch_size).max(1);

    let stage_a_input = num_stage_a_batches * 650 + estimated_candidates * 420;
    let stage_a_output = estimated_candidates * 48;

    let (stage_b_input, stage_b_output) = match mode {
        AnalysisMode::Eco => (0u32, 0u32),
        AnalysisMode::Standard => (700 + clamped_clips * 460, clamped_clips * 165),
        AnalysisMode::Quality => (700 + clamped_clips * 720, clamped_clips * 195),
    };

    let model_a = stage_a_model
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("flash-lite")
        .to_string();
    let model_b = match mode {
        AnalysisMode::Eco => None,
        _ => Some(
            stage_b_model
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("flash")
                .to_string(),
        ),
    };

    let uses_batch = mode == AnalysisMode::Eco;
    let cost_a = compute_call_cost_usd(pricing, &model_a, stage_a_input, stage_a_output, 0, uses_batch);
    let cost_b = match &model_b {
        Some(mb) => compute_call_cost_usd(pricing, mb, stage_b_input, stage_b_output, 0, false),
        None => 0.0,
    };

    let total_in = stage_a_input + stage_b_input;
    let total_out = stage_a_output + stage_b_output;
    let total_usd = cost_a + cost_b;
    let total_eur = total_usd * pricing.usd_to_eur;
    let target_limit = if mode == AnalysisMode::Eco { 60_000 } else { 150_000 };

    ProjectCostEstimate {
        mode,
        estimated_candidates,
        estimated_input_tokens: total_in,
        estimated_output_tokens: total_out,
        estimated_cost_usd: (total_usd * 10_000.0).round() / 10_000.0,
        estimated_cost_eur: (total_eur * 10_000.0).round() / 10_000.0,
        stage_a_model: model_a,
        stage_b_model: model_b,
        uses_batch_discount: uses_batch,
        within_target_budget: total_in <= target_limit,
    }
}

pub fn get_project_budget_snapshot(
    db: &Database,
    project_id: &str,
) -> AppResult<ProjectBudgetSnapshot> {
    db.with_conn(|conn| {
        let limit: u32 = conn.query_row(
            "SELECT token_budget_limit FROM projects WHERE id = ?1",
            [project_id],
            |row| row.get(0),
        )?;
        let (used_in, used_out, spent_usd): (i64, i64, f64) = conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM gemini_calls WHERE project_id = ?1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let in_u32 = used_in.clamp(0, i64::from(u32::MAX)) as u32;
        let out_u32 = used_out.clamp(0, i64::from(u32::MAX)) as u32;
        let used_total = in_u32.saturating_add(out_u32);
        Ok(ProjectBudgetSnapshot {
            project_id: project_id.to_string(),
            budget_limit_tokens: limit,
            used_input_tokens: in_u32,
            used_output_tokens: out_u32,
            remaining_tokens: limit.saturating_sub(used_total),
            spent_cost_usd: spent_usd,
        })
    })
}

pub fn enforce_budget_cap(
    db: &Database,
    project_id: &str,
    projected_tokens: u32,
) -> AppResult<ProjectBudgetSnapshot> {
    let snap = get_project_budget_snapshot(db, project_id)?;
    let used_total = snap.used_input_tokens.saturating_add(snap.used_output_tokens);
    if used_total.saturating_add(projected_tokens) > snap.budget_limit_tokens {
        return Err(AppError::BudgetExceeded(format!(
            "Plafond atteint ({used_total} + {projected_tokens} estimés > limite de {} tokens). Augmentez le plafond du projet pour reprendre au point exact.",
            snap.budget_limit_tokens
        )));
    }
    Ok(snap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_hour_vod_stays_well_under_token_targets() {
        let pricing = PricingConfig::default();
        let std_est = estimate_project_cost(&pricing, 4.0 * 3600.0, 12, AnalysisMode::Standard, None, None);
        assert!(std_est.estimated_input_tokens < 150_000);
        assert!(std_est.estimated_output_tokens < 25_000);
        assert!(std_est.within_target_budget);

        let eco_est = estimate_project_cost(&pricing, 4.0 * 3600.0, 12, AnalysisMode::Eco, None, None);
        assert!(eco_est.estimated_input_tokens < 60_000);
        assert!(eco_est.uses_batch_discount);
        assert!(eco_est.estimated_cost_usd < std_est.estimated_cost_usd);
    }

    #[test]
    fn enforces_hard_budget_cap() {
        let db = Database::open_in_memory().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO projects (id, name, mode, target_clips, target_min_sec, target_max_sec, token_budget_limit, status, workspace_dir, created_at, updated_at)
                 VALUES ('p1', 'Test', 'standard', 10, 30, 60, 1000, 'running', '.', ?1, ?1)",
                [&now],
            )?;
            conn.execute(
                "INSERT INTO gemini_calls (id, project_id, stage, payload_hash, model, prompt_version, input_tokens, output_tokens, cached_tokens, estimated_cost_usd, cache_hit, duration_ms, response_json, created_at)
                 VALUES ('c1', 'p1', 'stage_a', 'h1', 'm1', 'v1', 800, 100, 0, 0.001, 0, 100, '[]', ?1)",
                [&now],
            )?;
            Ok(())
        })
        .unwrap();

        assert!(enforce_budget_cap(&db, "p1", 50).is_ok());
        let err = enforce_budget_cap(&db, "p1", 200).unwrap_err();
        assert!(matches!(err, AppError::BudgetExceeded(_)));
    }
}
