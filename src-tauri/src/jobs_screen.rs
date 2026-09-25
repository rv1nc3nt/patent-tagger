//! Tauri commands for the Jobs screen: the state of the persistent job
//! queue (SPEC 4.2 `jobs`, 5.4, 5.5), with retry and cancel actions.

use crate::db::Db;
use crate::pipeline::Pipeline;
use crate::retrieval_worker::{DRAWINGS_JOB_KIND, FULLTEXT_JOB_KIND};
use core_lib::jobs::{self, JobCounts, JobDetail};
use serde::Serialize;
use tauri::State;

const IMPORT_JOB_KIND: &str = "import_document";
const KINDS: [&str; 3] = [IMPORT_JOB_KIND, FULLTEXT_JOB_KIND, DRAWINGS_JOB_KIND];
/// Failed jobs listed on the screen; the counts cover all of them.
const FAILED_LIST_LIMIT: i64 = 200;

#[derive(Debug, Serialize)]
pub struct JobOverview {
    /// One row per job kind: import, full text, drawings.
    pub counts: Vec<JobCounts>,
    /// Pending plus running jobs of every kind, for the tab label.
    pub active_total: i64,
    /// Failed jobs of every kind (the list below is capped).
    pub failed_total: i64,
    pub running: Vec<JobDetail>,
    pub failed: Vec<JobDetail>,
    pub import_running: bool,
    pub retrieval_running: bool,
}

#[tauri::command(async)]
pub fn job_overview(state: State<Db>, pipeline: State<Pipeline>) -> Result<JobOverview, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let counts = jobs::counts(&conn, &KINDS).map_err(|e| e.to_string())?;
    let active_total = counts.iter().map(|c| c.pending + c.running).sum();
    let failed_total = counts.iter().map(|c| c.failed).sum();
    Ok(JobOverview {
        counts,
        active_total,
        failed_total,
        running: jobs::list_detailed(&conn, jobs::STATE_RUNNING, FAILED_LIST_LIMIT)
            .map_err(|e| e.to_string())?,
        failed: jobs::list_detailed(&conn, jobs::STATE_FAILED, FAILED_LIST_LIMIT)
            .map_err(|e| e.to_string())?,
        import_running: pipeline.import_running(),
        retrieval_running: pipeline.retrieval_running(),
    })
}

/// Starts the background run that processes jobs of `kind`.
fn start_run(app: &tauri::AppHandle, kind: &str) {
    if kind == IMPORT_JOB_KIND {
        crate::pipeline::spawn_import(app);
    } else {
        crate::pipeline::spawn_retrieval(app);
    }
}

/// Puts one failed job back in the queue and starts processing it.
#[tauri::command(async)]
pub fn retry_job(app: tauri::AppHandle, state: State<Db>, job_id: i64) -> Result<(), String> {
    let kind = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        let kind = jobs::kind_of(&conn, job_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("job {job_id} not found"))?;
        jobs::retry(&conn, job_id, &crate::commands::current_timestamp())
            .map_err(|e| e.to_string())?;
        kind
    };
    start_run(&app, &kind);
    Ok(())
}

/// Puts every failed job of `kind` back in the queue and starts processing.
/// Returns how many.
#[tauri::command(async)]
pub fn retry_failed_jobs(
    app: tauri::AppHandle,
    state: State<Db>,
    kind: String,
) -> Result<usize, String> {
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown job kind: {kind}"));
    }
    let n = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        jobs::retry_failed(&conn, &kind, &crate::commands::current_timestamp())
            .map_err(|e| e.to_string())?
    };
    start_run(&app, &kind);
    Ok(n)
}

/// Removes the pending retrievals of `kind`; their documents stay "not
/// retrieved". Pending imports cannot be cancelled: their documents would
/// stay half-imported. Returns how many.
#[tauri::command(async)]
pub fn cancel_pending_jobs(state: State<Db>, kind: String) -> Result<usize, String> {
    if kind != FULLTEXT_JOB_KIND && kind != DRAWINGS_JOB_KIND {
        return Err(format!("pending {kind} jobs cannot be cancelled"));
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    jobs::cancel_pending(&conn, &kind).map_err(|e| e.to_string())
}
