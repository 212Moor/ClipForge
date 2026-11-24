use crate::asr::{transcribe_candidate_window, WindowTranscript};
use crate::error::{AppError, AppResult};
use crate::gemini::budget::{get_project_budget_snapshot, load_pricing_config};
use crate::gemini::client::{
    build_system_prefix, call_gemini_structured, call_gemini_structured_multimodal, VisualInlineImage, list_models, load_model_selection,
    load_output_language, parse_and_validate_stage_a, parse_and_validate_stage_b,
    poll_eco_batch_job, stage_a_response_schema, stage_b_response_schema,
    submit_eco_batch_stage_a, StageACandidatePayload, StageAOutputItem, StageBInputItem,
    STAGE_A_SYSTEM_PROMPT, STAGE_B_SYSTEM_PROMPT,
};
use crate::gemini::keyring_store::load_api_key;
use crate::gemini::ratelimit::{load_rate_limit_config, GeminiRateLimiter};
use crate::ingest::download_audio_and_chat;
use crate::ipc::{emit_pipeline_log, emit_progress};
use crate::paths::AppPaths;
use crate::scoring::score_and_select_candidates;
use crate::sidecars::{manifest::SidecarId, prepare_verified_executable};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
use crate::signals::{
    extract_local_signals, load_chat_messages, load_signal_config, summarize_chat_for_window,
    CompactChatSummary,
};
use crate::store::projects::{
    get_project_summary, get_streamer_profile, get_vod_by_project, list_candidates,
    replace_signal_windows, update_project_status, AnalysisMode,
};
use crate::store::Database;
use crate::util::blocking;
use chrono::Utc;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct JobRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub kind: String,
    pub state: JobState,
    pub progress: f64,
    pub checkpoint_json: String,
    pub error_message: Option<String>,
    pub updated_at: String,
}

/// Checkpoint atomique persisté dans `jobs.checkpoint_json` pour garantir la reprise exacte après plantage,
/// coupure réseau ou plafond de tokens atteint.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct PipelineCheckpoint {
    pub ingest_done: bool,
    pub signals_done: bool,
    pub scoring_done: bool,
    pub asr_completed_ids: Vec<String>,
    pub stage_a_completed_batches: Vec<u32>,
    pub eco_batch_name: Option<String>,
    pub stage_b_done: bool,
    pub current_step: String,
    pub total_candidates: u32,
    pub eta_sec: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PipelineStatusSnapshot {
    pub job: Option<JobRecord>,
    pub checkpoint: PipelineCheckpoint,
    pub spent_input_tokens: u32,
    pub spent_output_tokens: u32,
    pub budget_limit_tokens: u32,
    pub spent_cost_usd: f64,
}

pub fn upsert_job_checkpoint(db: &Database, record: &JobRecord) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO jobs (id, project_id, kind, state, progress, checkpoint_json, error_message, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
                state = excluded.state,
                progress = excluded.progress,
                checkpoint_json = excluded.checkpoint_json,
                error_message = excluded.error_message,
                updated_at = excluded.updated_at",
            rusqlite::params![
                record.id,
                record.project_id,
                record.kind,
                record.state.as_str(),
                record.progress,
                record.checkpoint_json,
                record.error_message,
                now
            ],
        )?;
        Ok(())
    })
}

pub fn get_latest_analysis_job(db: &Database, project_id: &str) -> AppResult<Option<JobRecord>> {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT id, project_id, kind, state, progress, checkpoint_json, error_message, updated_at
             FROM jobs WHERE project_id = ?1 AND kind = 'analysis_pipeline'
             ORDER BY updated_at DESC LIMIT 1",
            [project_id],
            |row| {
                let state_str: String = row.get(3)?;
                Ok(JobRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    kind: row.get(2)?,
                    state: JobState::from_str(&state_str),
                    progress: row.get(4)?,
                    checkpoint_json: row.get(5)?,
                    error_message: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(AppError::Database)
    })
}

pub fn get_pipeline_status(db: &Database, project_id: &str) -> AppResult<PipelineStatusSnapshot> {
    let job = get_latest_analysis_job(db, project_id)?;
    let checkpoint = job
        .as_ref()
        .and_then(|j| serde_json::from_str::<PipelineCheckpoint>(&j.checkpoint_json).ok())
        .unwrap_or_default();
    let budget = get_project_budget_snapshot(db, project_id)?;
    Ok(PipelineStatusSnapshot {
        job,
        checkpoint,
        spent_input_tokens: budget.used_input_tokens,
        spent_output_tokens: budget.used_output_tokens,
        budget_limit_tokens: budget.budget_limit_tokens,
        spent_cost_usd: budget.spent_cost_usd,
    })
}

fn check_cancelled(flag: &AtomicBool) -> AppResult<()> {
    if flag.load(Ordering::SeqCst) {
        return Err(AppError::Cancelled(
            "Analyse mise en pause par l'utilisateur (progression sauvegardée).".to_string(),
        ));
    }
    Ok(())
}

/// Exécute ou reprend le pipeline complet d'analyse (étapes 1 à 6) en tâche asynchrone hors thread UI.
pub async fn run_analysis_pipeline(
    app: AppHandle,
    db: Database,
    paths: AppPaths,
    limiter: Arc<GeminiRateLimiter>,
    project_id: String,
    local_only: bool,
    cancel_flag: Arc<AtomicBool>,
) -> AppResult<()> {
    let job_id = format!("analysis-{project_id}");

    // Charge le checkpoint existant pour reprise exacte sans perte
    let db_init = db.clone();
    let pid_init = project_id.clone();
    let (project, vod, mut cp, rl_cfg) = blocking(move || {
        let project = get_project_summary(&db_init, &pid_init)?;
        let vod = get_vod_by_project(&db_init, &pid_init)?;
        let prev_job = get_latest_analysis_job(&db_init, &pid_init)?;
        let cp = prev_job
            .and_then(|j| serde_json::from_str::<PipelineCheckpoint>(&j.checkpoint_json).ok())
            .unwrap_or_default();
        let rl_cfg = load_rate_limit_config(&db_init);
        update_project_status(&db_init, &pid_init, "running")?;
        Ok((project, vod, cp, rl_cfg))
    })
    .await?;

    limiter.update_config(rl_cfg).await;
    if cp.scoring_done && vod.duration_sec > 21_600.0 && cp.total_candidates <= 60 { cp.scoring_done = false; cp.asr_completed_ids.clear(); cp.stage_a_completed_batches.clear(); cp.eco_batch_name = None; cp.stage_b_done = false; }
    if cp.stage_b_done && !local_only { cp.stage_a_completed_batches.clear(); cp.eco_batch_name = None; cp.stage_b_done = false; }

    let save_cp = |db_ref: &Database, state: JobState, progress: f64, cp_ref: &PipelineCheckpoint, err: Option<String>| {
        let rec = JobRecord {
            id: job_id.clone(),
            project_id: Some(project_id.clone()),
            kind: "analysis_pipeline".to_string(),
            state,
            progress,
            checkpoint_json: serde_json::to_string(cp_ref).unwrap_or_else(|_| "{}".to_string()),
            error_message: err,
            updated_at: Utc::now().to_rfc3339(),
        };
        let _ = upsert_job_checkpoint(db_ref, &rec);
    };

    let result: AppResult<()> = async {
        let workspace_dir = PathBuf::from(&project.workspace_dir);
        let meta_val: serde_json::Value =
            serde_json::from_str(&vod.metadata_json).unwrap_or(serde_json::Value::Null);
        let is_demo = meta_val.get("demo").and_then(|v| v.as_bool()).unwrap_or(false);
        let asr_language = meta_val
            .get("asr_language")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_string();

        // ------------------------------------------------------------------
        // ÉTAPE 1 : INGESTION (audio seul + chat JSON + zones mutées)
        // ------------------------------------------------------------------
        check_cancelled(&cancel_flag)?;
        let (audio_path, chat_path, muted_segments) = if cp.ingest_done
            && vod.audio_path.as_deref().map(|p| PathBuf::from(p).is_file()).unwrap_or(false)
        {
            emit_pipeline_log(&app, &project_id, "info", "Étape 1/6 (Ingestion) déjà en checkpoint — reprise.");
            (
                PathBuf::from(vod.audio_path.clone().unwrap_or_default()),
                PathBuf::from(vod.chat_path.clone().unwrap_or_default()),
                vod.muted_segments.clone(),
            )
        } else {
            cp.current_step = "ingest".to_string();
            save_cp(&db, JobState::Running, 5.0, &cp, None);
            emit_pipeline_log(&app, &project_id, "info", "Étape 1/6 : Ingestion audio seul + chat JSON…");

            let artifacts = download_audio_and_chat(
                &app,
                &db,
                &paths,
                &project_id,
                &workspace_dir,
                &vod.source_url,
                vod.twitch_vod_id.as_deref(),
                &vod.muted_segments,
            )
            .await?;

            cp.ingest_done = true;
            save_cp(&db, JobState::Running, 18.0, &cp, None);
            (
                artifacts.audio_wav_path,
                artifacts.chat_json_path,
                artifacts.muted_segments,
            )
        };

        // ------------------------------------------------------------------
        // ÉTAPE 2 & 3 : SIGNAUX LOCAUX (5 s) + SCORING MÉDIANE/MAD (0 token)
        // ------------------------------------------------------------------
        check_cancelled(&cancel_flag)?;
        let chat_for_summary = {
            let cp_chat = chat_path.clone();
            blocking(move || load_chat_messages(&cp_chat)).await?
        };
        let db_cfg = db.clone();
        let signal_cfg = blocking(move || Ok(load_signal_config(&db_cfg))).await?;

        if !cp.signals_done || !cp.scoring_done {
            cp.current_step = "signals".to_string();
            save_cp(&db, JobState::Running, 22.0, &cp, None);
            emit_progress(&app, "pipeline", &project_id, "signals", 25.0, "Calcul des signaux locaux par pas de 5 s (0 token)…");
            emit_pipeline_log(&app, &project_id, "info", format!("Étape 2/6 : Analyse audio + {} messages de chat…", chat_for_summary.len()));

            let audio_clone = audio_path.clone();
            let msgs_clone = chat_for_summary.clone();
            let muted_clone = muted_segments.clone();
            let cfg_clone = signal_cfg.clone();
            let fallback_dur = vod.duration_sec;

            let mut windows = blocking(move || {
                extract_local_signals(&audio_clone, &msgs_clone, &muted_clone, &cfg_clone, fallback_dur)
            })
            .await?;

            check_cancelled(&cancel_flag)?;
            cp.signals_done = true;
            cp.current_step = "scoring".to_string();
            emit_progress(&app, "pipeline", &project_id, "scoring", 35.0, "Scoring robuste (médiane/MAD) et sélection des pics…");

            let cfg_score = signal_cfg.clone();
            let target_clips = project.target_clips;
            let t_min = project.target_min_sec;
            let t_max = project.target_max_sec;
            let (scored_windows, scored_candidates) = blocking(move || {
                let cands = score_and_select_candidates(&mut windows, &cfg_score, target_clips, t_min, t_max);
                Ok((windows, cands))
            })
            .await?;

            // Persiste signal_windows + candidates (avec résumé chat compact)
            let db_save = db.clone();
            let vod_id_owned = vod.id.clone();
            let pid_owned = project_id.clone();
            let msgs_sum = chat_for_summary.clone();
            let emotes_sum = signal_cfg.emotes_list.clone();
            let cands_len = scored_candidates.len() as u32;

            blocking(move || {
                replace_signal_windows(&db_save, &vod_id_owned, &scored_windows)?;
                let now = Utc::now().to_rfc3339();
                db_save.with_conn_mut(|conn| {
                    let tx = conn.transaction()?;
                    tx.execute("DELETE FROM candidates WHERE project_id = ?1", [&pid_owned])?;
                    for c in &scored_candidates {
                        let chat_sum = summarize_chat_for_window(&msgs_sum, c.start_sec, c.end_sec, &emotes_sum);
                        let chat_sum_json = serde_json::to_string(&chat_sum)?;
                        tx.execute(
                            "INSERT INTO candidates
                             (id, project_id, vod_id, rank_index, start_sec, end_sec, peak_sec,
                              local_score, signals_summary_json, transcript_json, chat_summary_json,
                              status, created_at, updated_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, 'pending', ?11, ?11)",
                            rusqlite::params![
                                uuid::Uuid::new_v4().to_string(),
                                pid_owned,
                                vod_id_owned,
                                c.rank_index,
                                c.start_sec,
                                c.end_sec,
                                c.peak_sec,
                                c.local_score,
                                c.signals_summary_json,
                                chat_sum_json,
                                now
                            ],
                        )?;
                    }
                    tx.commit()?;
                    Ok(())
                })
            })
            .await?;

            cp.scoring_done = true;
            cp.total_candidates = cands_len;
            save_cp(&db, JobState::Running, 42.0, &cp, None);
            emit_pipeline_log(&app, &project_id, "info", format!("Étape 3/6 : {cands_len} candidats sélectionnés (0 token dépensé)."));
        } else {
            emit_pipeline_log(&app, &project_id, "info", "Étapes 2–3 (Signaux & Scoring) déjà en checkpoint — reprise.");
        }

        // ------------------------------------------------------------------
        // ÉTAPE 4 : TRANSCRIPTION LOCALE WHISPER.CPP PAR FENÊTRES (+15 s)
        // ------------------------------------------------------------------
        let db_cands = db.clone();
        let pid_cands = project_id.clone();
        let candidates = blocking(move || list_candidates(&db_cands, &pid_cands)).await?;
        let done_asr: HashSet<String> = cp.asr_completed_ids.iter().cloned().collect();
        let total_cands = candidates.len().max(1);

        cp.current_step = "asr".to_string();
        for (idx, cand) in candidates.iter().enumerate() {
            check_cancelled(&cancel_flag)?;
            if done_asr.contains(&cand.id) && cand.transcript_json.is_some() {
                continue;
            }
            let pct = 42.0 + ((idx as f64 / total_cands as f64) * 26.0);
            emit_progress(
                &app,
                "pipeline",
                &project_id,
                "asr",
                pct,
                format!("Transcription locale whisper.cpp : fenêtre {}/{total_cands}…", idx + 1),
            );

            let transcript = transcribe_candidate_window(
                &app,
                &db,
                &paths,
                &workspace_dir,
                &audio_path,
                cand.rank_index,
                cand.start_sec,
                cand.end_sec,
                vod.duration_sec,
                &asr_language,
                is_demo,
            )
            .await?;

            let tr_json = serde_json::to_string(&transcript)?;
            let db_up = db.clone();
            let cid = cand.id.clone();
            blocking(move || {
                db_up.with_conn(|conn| {
                    conn.execute(
                        "UPDATE candidates SET transcript_json = ?1, status = 'transcribed', updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![tr_json, Utc::now().to_rfc3339(), cid],
                    )?;
                    Ok(())
                })
            })
            .await?;

            cp.asr_completed_ids.push(cand.id.clone());
            save_cp(&db, JobState::Running, pct, &cp, None);
        }
        emit_pipeline_log(&app, &project_id, "info", format!("Étape 4/6 : {} fenêtres transcrites localement (0 token).", candidates.len()));

        if local_only {
            cp.current_step = "completed".to_string();
            save_cp(&db, JobState::Completed, 100.0, &cp, None);
            let db_done = db.clone();
            let pid_done = project_id.clone();
            blocking(move || update_project_status(&db_done, &pid_done, "local_done")).await?;
            emit_progress(&app, "pipeline", &project_id, "completed", 100.0, "Analyse locale terminée (0 token Gemini consommé).");
            return Ok(());
        }

        // ------------------------------------------------------------------
        // ÉTAPE 5 : TRI ÉDITORIAL GEMINI — ÉTAGE A (lots de 12 ou Batch API Éco)
        // ------------------------------------------------------------------
        check_cancelled(&cancel_flag)?;
        let api_key = blocking(load_api_key).await?.ok_or_else(|| {
            AppError::Config(
                "Aucune clé API Gemini enregistrée. Ouvrez Paramètres pour enregistrer votre clé Google AI Studio, ou lancez en mode Analyse locale seule (0 token)."
                    .to_string(),
            )
        })?;

        let db_meta = db.clone();
        let prof_id = project.streamer_profile_id.clone();
        let paths_pricing = paths.clone();
        let (mut selection, profile, out_lang, pricing) = blocking(move || {
            let sel = load_model_selection(&db_meta)?;
            let prof = match prof_id {
                Some(ref pid) => get_streamer_profile(&db_meta, pid)?,
                None => None,
            };
            let lang = load_output_language(&db_meta);
            let pr = load_pricing_config(&paths_pricing);
            Ok((sel, prof, lang, pr))
        })
        .await?;

        // Si aucun modèle n'est encore sélectionné dans Paramètres, résout dynamiquement via models.list
        if selection.stage_a_model.is_none() || selection.stage_b_model.is_none() {
            let live_models = list_models(&api_key).await?;
            let usable: Vec<String> = live_models
                .into_iter()
                .filter(|m| m.supports_generate_content && m.id.starts_with("gemini-") && !m.id.contains("image") && !m.id.contains("tts") && !m.id.contains("audio") && !m.id.contains("live") && !m.id.contains("robotics") && !m.id.contains("computer"))
                .map(|m| m.id)
                .collect();
            if selection.stage_a_model.is_none() {
                selection.stage_a_model = usable
                    .iter()
                    .rev().find(|id| id.contains("flash-lite") && !id.contains("preview")).or_else(|| usable.iter().rev().find(|id| id.contains("flash-lite")))
                    .or_else(|| usable.iter().find(|id| id.contains("flash")))
                    .or_else(|| usable.first())
                    .cloned();
            }
            if selection.stage_b_model.is_none() {
                selection.stage_b_model = usable
                    .iter()
                    .rev().find(|id| id.contains("flash") && !id.contains("lite") && !id.contains("preview")).or_else(|| usable.iter().rev().find(|id| id.contains("flash") && !id.contains("lite")))
                    .or_else(|| usable.iter().find(|id| id.contains("flash")))
                    .or_else(|| usable.first())
                    .cloned();
            }
        }

        let stage_a_model = selection.stage_a_model.clone().ok_or_else(|| {
            AppError::Config("Aucun modèle Gemini disponible pour l'étage A (vérifiez Paramètres > models.list).".to_string())
        })?;
        let stage_b_model = selection
            .stage_b_model
            .clone()
            .unwrap_or_else(|| stage_a_model.clone());

        let db_reload = db.clone();
        let pid_reload = project_id.clone();
        let candidates = blocking(move || list_candidates(&db_reload, &pid_reload)).await?;

        let sys_a = build_system_prefix(STAGE_A_SYSTEM_PROMPT, profile.as_ref(), &out_lang);
        let batch_size = 12usize;
        let payloads_by_batch: Vec<Vec<StageACandidatePayload>> = candidates
            .chunks(batch_size)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|c| {
                        let compact_text = c
                            .transcript_json
                            .as_deref()
                            .and_then(|s| serde_json::from_str::<WindowTranscript>(s).ok())
                            .map(|tr| tr.compact_text)
                            .unwrap_or_default();
                        let chat_sum: CompactChatSummary = c
                            .chat_summary_json
                            .as_deref()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or(CompactChatSummary {
                                r: 0.0,
                                e: vec![],
                                m: vec![],
                            });
                        let sig_val: serde_json::Value =
                            serde_json::from_str(&c.signals_summary_json).unwrap_or_default();
                        let chat_n = sig_val.get("chat_norm").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let audio_n = sig_val.get("audio_norm").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        StageACandidatePayload {
                            i: c.rank_index,
                            d: (c.end_sec - c.start_sec).round().max(1.0) as u32,
                            t: compact_text,
                            c: chat_sum,
                            s: [chat_n, audio_n],
                        }
                    })
                    .collect()
            })
            .collect();

        cp.current_step = "stage_a".to_string();
        if project.mode == AnalysisMode::Eco {
            // Mode Éco : Batch API asynchrone (-50 % sur le coût) avec reprise possible
            let raw_batches: Vec<String> = payloads_by_batch
                .iter()
                .map(|b| serde_json::to_string(b).unwrap_or_else(|_| "[]".to_string()))
                .collect();

            let batch_name = match cp.eco_batch_name.clone() {
                Some(existing) => {
                    emit_pipeline_log(&app, &project_id, "info", format!("Mode Éco : vérification du lot Batch API existant ({existing})…"));
                    existing
                }
                None => {
                    emit_progress(&app, "pipeline", &project_id, "stage_a", 72.0, "Mode Éco : soumission du lot Batch API (-50 %)…");
                    let name = submit_eco_batch_stage_a(&api_key, &stage_a_model, &sys_a, &raw_batches).await?;
                    cp.eco_batch_name = Some(name.clone());
                    save_cp(&db, JobState::Running, 74.0, &cp, None);
                    emit_pipeline_log(&app, &project_id, "info", format!("Lot Batch API créé : {name}"));
                    name
                }
            };

            let poll_res = poll_eco_batch_job(&api_key, &batch_name).await?;
            match poll_res {
                crate::gemini::client::EcoBatchPollResult::Pending(state_label) => {
                    emit_pipeline_log(
                        &app,
                        &project_id,
                        "info",
                        format!("Lot Batch API en attente ({state_label}). Vous pouvez fermer l'application et cliquer sur Reprendre plus tard."),
                    );
                    save_cp(&db, JobState::Paused, 78.0, &cp, None);
                    let db_p = db.clone();
                    let pid_p = project_id.clone();
                    blocking(move || update_project_status(&db_p, &pid_p, "eco_batch_pending")).await?;
                    emit_progress(&app, "pipeline", &project_id, "stage_a", 78.0, format!("Batch API Éco en cours côté Google ({state_label}) — cliquez sur Reprendre pour vérifier."));
                    return Ok(());
                }
                crate::gemini::client::EcoBatchPollResult::Failed(err) => {
                    return Err(AppError::Gemini(err));
                }
                crate::gemini::client::EcoBatchPollResult::Succeeded(texts) => {
                    for text in texts {
                        if let Ok(items) = parse_and_validate_stage_a(&text) {
                            apply_stage_a_results(&db, &project_id, &items).await?;
                        }
                    }
                    cp.stage_a_completed_batches = (0..payloads_by_batch.len() as u32).collect();
                }
            }
        } else {
            let done_batches: HashSet<u32> = cp.stage_a_completed_batches.iter().copied().collect();
            let total_b = payloads_by_batch.len().max(1);

            for (b_idx, batch) in payloads_by_batch.iter().enumerate() {
                check_cancelled(&cancel_flag)?;
                let b_u32 = b_idx as u32;
                if done_batches.contains(&b_u32) {
                    continue;
                }
                let pct = 68.0 + ((b_idx as f64 / total_b as f64) * 16.0);
                emit_progress(
                    &app,
                    "pipeline",
                    &project_id,
                    "stage_a",
                    pct,
                    format!("Tri éditorial Gemini Étage A (lot {}/{total_b})…", b_idx + 1),
                );

                let payload_json = serde_json::to_string(batch)?;
                let outcome = call_gemini_structured(
                    &db,
                    &limiter,
                    &pricing,
                    &api_key,
                    Some(&project_id),
                    "stage_a",
                    &stage_a_model,
                    &sys_a,
                    &payload_json,
                    stage_a_response_schema(),
                    1400,
                    true,
                )
                .await?;

                let parsed = match parse_and_validate_stage_a(&outcome.text) {
                    Ok(items) => Ok(items),
                    Err(first_err) => {
                        // Une seule relance ciblée en cas d'échec de schéma
                        emit_pipeline_log(&app, &project_id, "warn", format!("Validation Étage A échouée ({first_err}) — 1 relance ciblée…"));
                        let retry_payload = format!(
                            "CORRECTION JSON STRICTE REQUISE. Renvoie uniquement un tableau JSON conforme.\n{payload_json}"
                        );
                        let retry_out = call_gemini_structured(
                            &db,
                            &limiter,
                            &pricing,
                            &api_key,
                            Some(&project_id),
                            "stage_a",
                            &stage_a_model,
                            &sys_a,
                            &retry_payload,
                            stage_a_response_schema(),
                            1400,
                            true,
                        )
                        .await?;
                        parse_and_validate_stage_a(&retry_out.text)
                    }
                };

                match parsed {
                    Ok(items) => {
                        apply_stage_a_results(&db, &project_id, &items).await?;
                        emit_pipeline_log(
                            &app,
                            &project_id,
                            "info",
                            format!(
                                "Étage A lot {}/{total_b} : {} candidats évalués ({})",
                                b_idx + 1,
                                items.len(),
                                if outcome.cache_hit {
                                    "100% cache hit, 0 token".to_string()
                                } else {
                                    format!("{} in / {} out tokens", outcome.input_tokens, outcome.output_tokens)
                                }
                            ),
                        );
                    }
                    Err(err) => {
                        // Marquage "à revoir" sans faire planter le projet entier
                        let ranks: Vec<u32> = batch.iter().map(|c| c.i).collect();
                        mark_candidates_needs_review(&db, &project_id, &ranks, &err.to_string()).await?;
                        emit_pipeline_log(&app, &project_id, "warn", format!("Lot {} marqué 'à revoir' après 2e échec de schéma.", b_idx + 1));
                    }
                }

                cp.stage_a_completed_batches.push(b_u32);
                save_cp(&db, JobState::Running, pct, &cp, None);
            }
        }

        // ------------------------------------------------------------------
        // ÉTAPE 6 : FINITION VISUELLE & ÉDITORIALE GEMINI — ÉTAGE B (lots de 4 avec 3 vignettes JPEG)
        // ------------------------------------------------------------------
        check_cancelled(&cancel_flag)?;
        cp.current_step = "stage_b".to_string();

        let db_top = db.clone();
        let pid_top = project_id.clone();
        let all_cands = blocking(move || list_candidates(&db_top, &pid_top)).await?;

        // Pool de finalistes pour l'Étage B : tous les k=1, complété par les meilleurs scores >= 45
        let target_k = (project.target_clips as usize).max(1);
        let finalist_pool = if project.mode == AnalysisMode::Eco {
            target_k
        } else {
            (target_k + (target_k / 2).clamp(8, 25)).clamp(12, 125)
        };
        let mut kept: Vec<_> = all_cands
            .iter()
            .filter(|c| c.stage_a_keep == Some(true))
            .cloned()
            .collect();
        kept.sort_by(|a, b| {
            b.stage_a_score
                .unwrap_or(0)
                .cmp(&a.stage_a_score.unwrap_or(0))
                .then_with(|| b.local_score.partial_cmp(&a.local_score).unwrap_or(std::cmp::Ordering::Equal))
        });
        if kept.len() < finalist_pool {
            let kept_ids: HashSet<String> = kept.iter().map(|c| c.id.clone()).collect();
            let mut extras: Vec<_> = all_cands
                .iter()
                .filter(|c| !kept_ids.contains(&c.id) && c.stage_a_score.unwrap_or(0) >= 25)
                .cloned()
                .collect();
            extras.sort_by(|a, b| {
                b.stage_a_score
                    .unwrap_or(0)
                    .cmp(&a.stage_a_score.unwrap_or(0))
                    .then_with(|| b.local_score.partial_cmp(&a.local_score).unwrap_or(std::cmp::Ordering::Equal))
            });
            for extra in extras {
                if kept.len() >= finalist_pool {
                    break;
                }
                kept.push(extra);
            }
        }
        if kept.is_empty() {
            kept = all_cands.clone();
        }
        kept.truncate(finalist_pool);

        if project.mode == AnalysisMode::Eco || kept.is_empty() {
            create_clips_from_stage_a_only(&db, &project_id, &kept, vod.duration_sec).await?;
            cp.stage_b_done = true;
        } else if !cp.stage_b_done {
            // Extraction rapide de 3 vignettes JPEG 384px par finaliste (hors démo)
            emit_progress(
                &app,
                "pipeline",
                &project_id,
                "stage_b",
                85.0,
                format!("Extraction des vignettes visuelles pour {} finalistes…", kept.len()),
            );
            let previews_map = extract_visual_previews_for_candidates(
                &app,
                &db,
                &paths,
                &project_id,
                &workspace_dir,
                &vod.source_url,
                vod.duration_sec,
                &kept,
                is_demo,
            )
            .await;
            let total_imgs: usize = previews_map.values().map(|v| v.len()).sum();
            emit_pipeline_log(
                &app,
                &project_id,
                "info",
                format!("Analyse visuelle prête : {total_imgs} vignettes extraites sur {} finalistes.", kept.len()),
            );

            let sys_b = build_system_prefix(STAGE_B_SYSTEM_PROMPT, profile.as_ref(), &out_lang);
            let mut all_b_items: Vec<crate::gemini::client::StageBOutputItem> = Vec::new();
            let mut total_in_b = 0u32;
            let mut total_out_b = 0u32;
            let mut all_cache_hit = true;

            let chunks: Vec<&[crate::store::projects::CandidateRecord]> = kept.chunks(4).collect();
            let total_chunks = chunks.len().max(1);

            for (c_idx, chunk) in chunks.iter().enumerate() {
                check_cancelled(&cancel_flag)?;
                let pct = 88.0 + ((c_idx as f64 / total_chunks as f64) * 10.0);
                emit_progress(
                    &app,
                    "pipeline",
                    &project_id,
                    "stage_b",
                    pct,
                    format!("Finition visuelle Gemini Étage B (lot {}/{total_chunks})…", c_idx + 1),
                );

                let inputs_b: Vec<StageBInputItem> = chunk
                    .iter()
                    .map(|c| {
                        let compact_text = c
                            .transcript_json
                            .as_deref()
                            .and_then(|s| serde_json::from_str::<WindowTranscript>(s).ok())
                            .map(|tr| tr.compact_text)
                            .unwrap_or_default();
                        let chat_sum: CompactChatSummary = c
                            .chat_summary_json
                            .as_deref()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or(CompactChatSummary {
                                r: 0.0,
                                e: vec![],
                                m: vec![],
                            });
                        StageBInputItem {
                            i: c.rank_index,
                            d: (c.end_sec - c.start_sec).round().max(1.0) as u32,
                            cat: c.stage_a_category.clone().unwrap_or_else(|| "fun".to_string()),
                            t: compact_text,
                            c: chat_sum,
                        }
                    })
                    .collect();

                let mut batch_images: Vec<VisualInlineImage> = Vec::new();
                for c in *chunk {
                    if let Some(imgs) = previews_map.get(&c.rank_index) {
                        batch_images.extend(imgs.iter().cloned());
                    }
                }

                let payload_b = serde_json::to_string(&inputs_b)?;
                let outcome = call_gemini_structured_multimodal(
                    &db,
                    &limiter,
                    &pricing,
                    &api_key,
                    Some(&project_id),
                    "stage_b",
                    &stage_b_model,
                    &sys_b,
                    &payload_b,
                    &batch_images,
                    stage_b_response_schema(),
                    2400,
                    true,
                )
                .await?;

                total_in_b = total_in_b.saturating_add(outcome.input_tokens);
                total_out_b = total_out_b.saturating_add(outcome.output_tokens);
                if !outcome.cache_hit {
                    all_cache_hit = false;
                }

                let b_items = match parse_and_validate_stage_b(&outcome.text) {
                    Ok(items) => items,
                    Err(_) => {
                        let retry = call_gemini_structured_multimodal(
                            &db,
                            &limiter,
                            &pricing,
                            &api_key,
                            Some(&project_id),
                            "stage_b",
                            &stage_b_model,
                            &sys_b,
                            &format!("CORRECTION JSON STRICTE REQUISE.\n{payload_b}"),
                            &batch_images,
                            stage_b_response_schema(),
                            2400,
                            true,
                        )
                        .await?;
                        parse_and_validate_stage_b(&retry.text)?
                    }
                };
                all_b_items.extend(b_items);
            }

            // Conserve tous les keep == 1, puis complète jusqu'à target_k avec les meilleurs scores >= 20
            // (les vrais écrans noirs / menus / cinématiques notés <= 15 restent exclus)
            let mut validated_b: Vec<_> = all_b_items.iter().filter(|b| b.keep == 1).cloned().collect();
            validated_b.sort_by(|a, b| b.score.cmp(&a.score));
            if validated_b.len() < target_k {
                let already: HashSet<u32> = validated_b.iter().map(|b| b.i).collect();
                let mut rescue: Vec<_> = all_b_items
                    .iter()
                    .filter(|b| !already.contains(&b.i) && b.score >= 20)
                    .cloned()
                    .collect();
                rescue.sort_by(|a, b| b.score.cmp(&a.score));
                for item in rescue {
                    if validated_b.len() >= target_k {
                        break;
                    }
                    validated_b.push(item);
                }
            }
            if validated_b.is_empty() {
                validated_b = all_b_items.clone();
                validated_b.sort_by(|a, b| b.score.cmp(&a.score));
            }
            validated_b.truncate(target_k);

            let selected_ranks: HashSet<u32> = validated_b.iter().map(|b| b.i).collect();
            let final_kept: Vec<_> = kept
                .iter()
                .filter(|c| selected_ranks.contains(&c.rank_index))
                .cloned()
                .collect();
            let rejected_visual = all_b_items.len().saturating_sub(validated_b.len());

            save_clips_from_stage_b(&db, &project_id, &final_kept, &validated_b, vod.duration_sec).await?;
            emit_pipeline_log(
                &app,
                &project_id,
                "info",
                format!(
                    "Étage B visuel terminé : {} clips retenus ({} écartés après contrôle visuel/qualité, {}).",
                    final_kept.len(),
                    rejected_visual,
                    if all_cache_hit {
                        "100% cache hit, 0 token".to_string()
                    } else {
                        format!("{total_in_b} in / {total_out_b} out tokens")
                    }
                ),
            );
            cp.stage_b_done = true;
        }

        cp.current_step = "completed".to_string();
        save_cp(&db, JobState::Completed, 100.0, &cp, None);
        let db_fin = db.clone();
        let pid_fin = project_id.clone();
        blocking(move || update_project_status(&db_fin, &pid_fin, "ready")).await?;
        emit_progress(&app, "pipeline", &project_id, "completed", 100.0, "Analyse terminée — clips prêts !");
        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(()),
        Err(AppError::BudgetExceeded(msg)) => {
            save_cp(&db, JobState::Paused, 75.0, &cp, Some(msg.clone()));
            let db_p = db.clone();
            let pid_p = project_id.clone();
            let _ = blocking(move || update_project_status(&db_p, &pid_p, "budget_paused")).await;
            emit_pipeline_log(&app, &project_id, "warn", &msg);
            emit_progress(&app, "pipeline", &project_id, "budget_paused", 75.0, &msg);
            Err(AppError::BudgetExceeded(msg))
        }
        Err(AppError::Cancelled(msg)) => {
            save_cp(&db, JobState::Paused, 50.0, &cp, Some(msg.clone()));
            let db_p = db.clone();
            let pid_p = project_id.clone();
            let _ = blocking(move || update_project_status(&db_p, &pid_p, "paused")).await;
            emit_pipeline_log(&app, &project_id, "info", &msg);
            emit_progress(&app, "pipeline", &project_id, "paused", 50.0, &msg);
            Ok(())
        }
        Err(err) => {
            let msg = err.to_string();
            save_cp(&db, JobState::Failed, 0.0, &cp, Some(msg.clone()));
            let db_f = db.clone();
            let pid_f = project_id.clone();
            let _ = blocking(move || update_project_status(&db_f, &pid_f, "failed")).await;
            emit_pipeline_log(&app, &project_id, "error", &msg);
            emit_progress(&app, "pipeline", &project_id, "failed", 0.0, &msg);
            Err(err)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn extract_visual_previews_for_candidates(
    app: &AppHandle,
    db: &Database,
    paths: &AppPaths,
    project_id: &str,
    workspace_dir: &Path,
    source_url: &str,
    vod_duration_sec: f64,
    candidates: &[crate::store::projects::CandidateRecord],
    is_demo: bool,
) -> HashMap<u32, Vec<VisualInlineImage>> {
    let mut out: HashMap<u32, Vec<VisualInlineImage>> = HashMap::new();
    if is_demo || source_url.trim().is_empty() || candidates.is_empty() {
        return out;
    }

    let Ok(yt_dlp) = prepare_verified_executable(db, paths, SidecarId::YtDlp).await else {
        return out;
    };
    let Ok(ffmpeg) = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await else {
        return out;
    };

    let previews_dir = workspace_dir.join("previews");
    let _ = std::fs::create_dir_all(&previews_dir);

    // 1. Vérifie d'abord si toutes les vignettes sont déjà en cache local
    let mut missing_any = false;
    for c in candidates {
        for idx in 1..=3u8 {
            let p = previews_dir.join(format!("cand_{:03}_f{idx}.jpg", c.rank_index));
            if !p.is_file() {
                missing_any = true;
            }
        }
    }

    let mut stream_url: Option<String> = None;
    if missing_any {
        let mut cmd = Command::new(&yt_dlp);
        cmd.env("PYTHONUTF8", "1").args([
            "--force-ipv4",
            "--retries",
            "5",
            "--socket-timeout",
            "20",
            "-f",
            "360p/480p/worstvideo[height>=320]/best[height<=480]/worst",
            "-g",
            "--no-playlist",
        ]);
        let cookie_file = workspace_dir.join("cookies.txt");
        if cookie_file.is_file() {
            cmd.arg("--cookies").arg(&cookie_file);
        }
        cmd.arg(source_url);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match tokio::time::timeout(Duration::from_secs(25), cmd.output()).await {
            Ok(Ok(res)) if res.status.success() => {
                let s = String::from_utf8_lossy(&res.stdout);
                stream_url = s.lines().map(str::trim).find(|l| l.starts_with("http")).map(ToOwned::to_owned);
            }
            _ => {
                emit_pipeline_log(
                    app,
                    project_id,
                    "warn",
                    "Flux vidéo basse résolution inaccessible — repli automatique Étage B texte seul.",
                );
            }
        }
    }

    let mut consecutive_failures = 0usize;
    for c in candidates {
        let est_start = (c.start_sec + c.stage_a_adj_start.unwrap_or(0.0)).max(0.0);
        let est_end = (c.end_sec + c.stage_a_adj_end.unwrap_or(0.0))
            .min(if vod_duration_sec > 0.0 { vod_duration_sec } else { c.end_sec + 25.0 })
            .max(est_start + 12.0);
        let dur = est_end - est_start;
        let sample_times = [
            est_start + dur * 0.15,
            est_start + dur * 0.50,
            est_start + dur * 0.85,
        ];

        let mut imgs = Vec::new();
        for (idx, &t_sec) in sample_times.iter().enumerate() {
            let jpg_path = previews_dir.join(format!("cand_{:03}_f{}.jpg", c.rank_index, idx + 1));
            if !jpg_path.is_file() {
                let Some(ref url) = stream_url else {
                    continue;
                };
                if consecutive_failures >= 3 {
                    continue;
                }
                let mut ff = Command::new(&ffmpeg);
                ff.args([
                    "-y",
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-ss",
                    &format!("{t_sec:.2}"),
                    "-i",
                    url,
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=384:-2",
                    "-q:v",
                    "6",
                ])
                .arg(&jpg_path);
                #[cfg(target_os = "windows")]
                ff.creation_flags(CREATE_NO_WINDOW);

                match tokio::time::timeout(Duration::from_secs(15), ff.output()).await {
                    Ok(Ok(r)) if r.status.success() && jpg_path.is_file() => {
                        consecutive_failures = 0;
                    }
                    _ => {
                        let _ = std::fs::remove_file(&jpg_path);
                        consecutive_failures += 1;
                        continue;
                    }
                }
            }

            if let Ok(bytes) = std::fs::read(&jpg_path) {
                if bytes.len() > 500 {
                    imgs.push(VisualInlineImage {
                        label: format!(
                            "Vignette candidat #{} ({}/3, t={:+.0}s)",
                            c.rank_index,
                            idx + 1,
                            t_sec - c.start_sec
                        ),
                        mime_type: "image/jpeg".to_string(),
                        bytes,
                    });
                }
            }
        }
        if !imgs.is_empty() {
            out.insert(c.rank_index, imgs);
        }
    }

    out
}

async fn apply_stage_a_results(
    db: &Database,
    project_id: &str,
    items: &[StageAOutputItem],
) -> AppResult<()> {
    let db_clone = db.clone();
    let pid = project_id.to_string();
    let items_owned = items.to_vec();
    blocking(move || {
        let now = Utc::now().to_rfc3339();
        db_clone.with_conn_mut(|conn| {
            let tx = conn.transaction()?;
            for item in &items_owned {
                tx.execute(
                    "UPDATE candidates
                     SET stage_a_keep = ?1, stage_a_score = ?2, stage_a_category = ?3,
                         stage_a_adj_start = ?4, stage_a_adj_end = ?5, stage_a_reason = ?6,
                         status = 'triaged', updated_at = ?7
                     WHERE project_id = ?8 AND rank_index = ?9",
                    rusqlite::params![
                        item.k,
                        item.s,
                        item.c,
                        item.a,
                        item.b,
                        item.r,
                        now,
                        pid,
                        item.i
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    })
    .await
}

async fn mark_candidates_needs_review(
    db: &Database,
    project_id: &str,
    ranks: &[u32],
    reason: &str,
) -> AppResult<()> {
    let db_clone = db.clone();
    let pid = project_id.to_string();
    let ranks_owned = ranks.to_vec();
    let reason_owned = reason.chars().take(120).collect::<String>();
    blocking(move || {
        let now = Utc::now().to_rfc3339();
        db_clone.with_conn_mut(|conn| {
            let tx = conn.transaction()?;
            for rank in ranks_owned {
                tx.execute(
                    "UPDATE candidates SET status = 'needs_review', stage_a_reason = ?1, updated_at = ?2
                     WHERE project_id = ?3 AND rank_index = ?4",
                    rusqlite::params![reason_owned, now, pid, rank],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    })
    .await
}

async fn create_clips_from_stage_a_only(
    db: &Database,
    project_id: &str,
    kept: &[crate::store::projects::CandidateRecord],
    vod_duration_sec: f64,
) -> AppResult<()> {
    let db_clone = db.clone();
    let pid = project_id.to_string();
    let kept_owned = kept.to_vec();
    blocking(move || {
        let now = Utc::now().to_rfc3339();
        db_clone.with_conn_mut(|conn| {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM clips WHERE project_id = ?1", [&pid])?;
            for c in &kept_owned {
                let start = (c.start_sec + c.stage_a_adj_start.unwrap_or(0.0)).max(0.0);
                let max_end = if vod_duration_sec > 0.0 { vod_duration_sec } else { c.end_sec + 25.0 };
                let raw_end = (c.end_sec + c.stage_a_adj_end.unwrap_or(0.0) + 3.0).min(max_end);
                let end = raw_end.max(start + 15.0).min(start + 70.0).min(max_end);
                let cat = c.stage_a_category.clone().unwrap_or_else(|| "fun".to_string());
                let title = c.stage_a_reason.clone().unwrap_or_else(|| format!("Clip #{}", c.rank_index));
                tx.execute(
                    r#"INSERT INTO clips
                     (id, project_id, candidate_id, start_sec, end_sec, hook, title, caption, hashtags_json,
                      category, layout_mode, subtitle_style, review_status, render_status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', '["twitch","clip"]', ?8, 'fill', 'bold_pop', 'pending', 'idle', ?9, ?9)"#,
                    rusqlite::params![
                        uuid::Uuid::new_v4().to_string(),
                        pid,
                        c.id,
                        start,
                        end,
                        title,
                        title,
                        cat,
                        now
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    })
    .await
}

async fn save_clips_from_stage_b(
    db: &Database,
    project_id: &str,
    kept: &[crate::store::projects::CandidateRecord],
    b_items: &[crate::gemini::client::StageBOutputItem],
    vod_duration_sec: f64,
) -> AppResult<()> {
    let db_clone = db.clone();
    let pid = project_id.to_string();
    let kept_owned = kept.to_vec();
    let b_owned = b_items.to_vec();
    blocking(move || {
        let now = Utc::now().to_rfc3339();
        db_clone.with_conn_mut(|conn| {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM clips WHERE project_id = ?1", [&pid])?;
            for c in &kept_owned {
                let b = b_owned.iter().find(|item| item.i == c.rank_index);
                let adj_s = c.stage_a_adj_start.unwrap_or(0.0) + b.map(|x| x.a).unwrap_or(0.0);
                let adj_e = c.stage_a_adj_end.unwrap_or(0.0) + b.map(|x| x.b).unwrap_or(0.0);
                let start = (c.start_sec + adj_s).max(0.0);
                let max_end = if vod_duration_sec > 0.0 { vod_duration_sec } else { c.end_sec + 25.0 };
                let raw_end = (c.end_sec + adj_e + 3.0).min(max_end);
                let end = raw_end.max(start + 15.0).min(start + 70.0).min(max_end);
                let cat = c.stage_a_category.clone().unwrap_or_else(|| "fun".to_string());
                let hook = b.map(|x| x.hook.clone()).unwrap_or_default();
                let title = b
                    .map(|x| x.title.clone())
                    .or_else(|| c.stage_a_reason.clone())
                    .unwrap_or_else(|| format!("Clip #{}", c.rank_index));
                let caption = b.map(|x| x.caption.clone()).unwrap_or_default();
                let tags_json = serde_json::to_string(
                    &b.map(|x| x.tags.clone())
                        .unwrap_or_else(|| vec!["twitch".to_string(), "clip".to_string()]),
                )?;

                tx.execute(
                    "INSERT INTO clips
                     (id, project_id, candidate_id, start_sec, end_sec, hook, title, caption, hashtags_json,
                      category, layout_mode, subtitle_style, review_status, render_status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'fill', 'bold_pop', 'pending', 'idle', ?11, ?11)",
                    rusqlite::params![
                        uuid::Uuid::new_v4().to_string(),
                        pid,
                        c.id,
                        start,
                        end,
                        hook,
                        title,
                        caption,
                        tags_json,
                        cat,
                        now
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    })
    .await
}