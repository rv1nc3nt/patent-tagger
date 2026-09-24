//! Tauri commands for the Import and Review screens (SPEC sections 5.1, 8).

use crate::db::Db;
use crate::model::Model;
use core_lib::rusqlite::Connection;
use core_lib::tags::TagRow;
use core_lib::{documents, jobs, labels, number, tags};
use embed_lib::Embedder;
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

#[derive(Debug, Serialize, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub duplicates: Vec<String>,
    pub needs_normalisation: Vec<String>,
    pub unparseable: Vec<String>,
}

#[tauri::command]
pub fn import_numbers(state: State<Db>, raw_input: String) -> Result<ImportReport, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    import_numbers_into(&conn, &raw_input, &current_timestamp()).map_err(|e| e.to_string())
}

/// Minimal credentials entry point for M2 (SPEC section 5.4); the full
/// Settings screen (target precision, retrieval policies, etc.) is M7.
#[tauri::command]
pub fn save_ops_credentials(
    state: State<Db>,
    consumer_key: String,
    consumer_secret: String,
) -> Result<(), String> {
    crate::platform::credentials::save(
        &state.data_dir,
        &crate::platform::credentials::OpsCredentials {
            consumer_key,
            consumer_secret,
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_ops_connection(state: State<'_, Db>) -> Result<String, String> {
    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    // A lightweight, well-known lookup just to prove the credentials work
    // end-to-end (auth + a real data call), not to fetch anything useful.
    client
        .get("/published-data/publication/docdb/EP.1000000.A1/biblio")
        .await
        .map(|_| "connected".to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_import_jobs(
    app: tauri::AppHandle,
    state: State<'_, Db>,
    model: State<'_, Model>,
) -> Result<Vec<crate::import_worker::DocumentOutcome>, String> {
    use tauri::Emitter;

    // SPEC 7.7: refuse to run alongside a scheduled command-line import.
    // Every command that drains the job queue takes the same lock.
    let _lock = crate::lock::PipelineLock::acquire(&state.data_dir).map_err(|e| e.to_string())?;
    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    let outcomes = crate::import_worker::run(&state.conn, &client, &model.0, |outcome| {
        let _ = app.emit("import-progress", outcome);
    })
    .await;

    // Newly fetched documents may already qualify for an automatic
    // decision under a tag that's already in automatic mode (SPEC 7.5),
    // or even full auto-completion if enabled (SPEC 7.6).
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        crate::automation::apply_full_automation(&conn, &model.0, &current_timestamp())
            .map_err(|e| e.to_string())?;
    }
    Ok(outcomes)
}

/// Moves the failed job for `doc_id` back to pending (SPEC section 8's
/// retry button). Does not itself re-fetch - call `run_import_jobs` again
/// afterward.
#[tauri::command]
pub fn retry_document(state: State<Db>, doc_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now = current_timestamp();
    let failed = jobs::list_failed(&conn, "import_document").map_err(|e| e.to_string())?;
    let job = failed
        .into_iter()
        .find(|j| job_payload_doc_id(&j.payload) == Some(doc_id))
        .ok_or_else(|| format!("no failed import job found for document {doc_id}"))?;
    jobs::retry(&conn, job.id, &now).map_err(|e| e.to_string())
}

fn job_payload_doc_id(payload: &str) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()?
        .get("doc_id")?
        .as_i64()
}

/// Parses pasted/uploaded input (one number per line, SPEC 5.1), skips
/// duplicates already in the database, and enqueues an `import_document`
/// job per new document. Fetching from OPS happens in the background
/// worker that drains those jobs, not here. Kept free of Tauri types so
/// it's directly unit-testable against an in-memory database, and
/// `pub(crate)` so the CLI's `import` subcommand (`cli.rs`) can reuse it.
pub(crate) fn import_numbers_into(
    conn: &Connection,
    raw_input: &str,
    now: &str,
) -> Result<ImportReport, core_lib::storage::StorageError> {
    let mut report = ImportReport::default();

    for line in raw_input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match number::parse(line) {
            number::ParseOutcome::Parsed(parsed) => {
                let pub_key = parsed.pub_key();
                match documents::insert_pending(conn, &pub_key, line, now)? {
                    Some(doc_id) => {
                        let payload =
                            serde_json::json!({ "doc_id": doc_id, "pub_key": pub_key }).to_string();
                        jobs::enqueue(conn, "import_document", &payload, now)?;
                        report.imported.push(pub_key);
                    }
                    None => report.duplicates.push(line.to_string()),
                }
            }
            number::ParseOutcome::NeedsNormalisation(raw) => report.needs_normalisation.push(raw),
            number::ParseOutcome::Unparseable(raw) => report.unparseable.push(raw),
        }
    }

    Ok(report)
}

#[tauri::command]
pub fn list_tags(state: State<Db>) -> Result<Vec<TagRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::list_active(&conn).map_err(|e| e.to_string())
}

/// Creates a tag and computes its `"{name}: {definition}"` embedding
/// immediately (SPEC 7.2), so zero-shot scoring never has to fall back to
/// computing it lazily during review.
#[tauri::command]
pub fn create_tag(
    state: State<Db>,
    model: State<Model>,
    name: String,
    definition: String,
    color: Option<String>,
    hotkey: Option<String>,
) -> Result<TagRow, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now = current_timestamp();
    let id = tags::create(&conn, &name, &definition, color.as_deref(), hotkey.as_deref(), &now)
        .map_err(|e| e.to_string())?;

    let text = format!("{name}: {definition}");
    let vector = model
        .0
        .embed(&[text])
        .map_err(|e| e.to_string())?
        .pop()
        .ok_or("embedder returned no vector")?;
    core_lib::embeddings::store_tag_embedding(&conn, id, model.0.model_id(), 1, &vector)
        .map_err(|e| e.to_string())?;

    tags::get(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "tag vanished immediately after creation".to_string())
}

#[tauri::command]
pub fn archive_tag(state: State<Db>, tag_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::archive(&conn, tag_id).map_err(|e| e.to_string())
}

/// SPEC 7.5: "Automatic mode unavailable before eligibility" - refuses
/// unless the tag currently meets the eligibility bar, in which case its
/// threshold is set to the just-calibrated value and auto mode turns on.
#[tauri::command]
pub fn enable_automatic_mode(state: State<Db>, model: State<Model>, tag_id: i64) -> Result<TagRow, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let target_precision = core_lib::settings::target_precision(&conn).map_err(|e| e.to_string())?;
    let eligibility =
        core_lib::predictions::tag_eligibility(&conn, tag_id, target_precision).map_err(|e| e.to_string())?;
    if !eligibility.eligible {
        return Err(format!(
            "tag {tag_id} is not yet eligible for automatic mode (n_pos={}, n_evaluated={}, target precision {} {})",
            eligibility.n_pos,
            eligibility.n_evaluated,
            target_precision,
            if eligibility.calibrated_threshold.is_some() { "reachable" } else { "not reachable" },
        ));
    }
    tags::set_threshold(&conn, tag_id, eligibility.calibrated_threshold).map_err(|e| e.to_string())?;
    tags::set_auto_enabled(&conn, tag_id, true).map_err(|e| e.to_string())?;

    crate::automation::apply_full_automation(&conn, &model.0, &current_timestamp())
        .map_err(|e| e.to_string())?;

    tags::get(&conn, tag_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("tag {tag_id} not found"))
}

#[tauri::command]
pub fn disable_automatic_mode(state: State<Db>, tag_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::set_auto_enabled(&conn, tag_id, false).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn tag_eligibility(state: State<Db>, tag_id: i64) -> Result<core_lib::predictions::Eligibility, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let target_precision = core_lib::settings::target_precision(&conn).map_err(|e| e.to_string())?;
    core_lib::predictions::tag_eligibility(&conn, tag_id, target_precision).map_err(|e| e.to_string())
}

/// `ordering`: `"import"` (default, `documents::list_queue`'s own order)
/// or `"uncertain"` (SPEC section 8's "most uncertain first").
#[tauri::command]
pub fn review_queue(
    state: State<Db>,
    model: State<Model>,
    ordering: Option<String>,
) -> Result<Vec<documents::QueueEntry>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match ordering.as_deref() {
        Some("uncertain") => crate::review::queue_ordered_by_uncertainty(&conn, &model.0).map_err(|e| e.to_string()),
        _ => documents::list_queue(&conn).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub fn document_detail(
    state: State<Db>,
    model: State<Model>,
    doc_id: i64,
) -> Result<Option<crate::review::DocumentView>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    crate::review::document_view(&conn, &model.0, doc_id).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ValidateResult {
    /// Names of tags whose automatic mode was just disabled (SPEC 7.5's
    /// audit safeguard) because of this validation - the frontend shows
    /// this as a notice ("the user is notified").
    pub auto_disabled_tags: Vec<String>,
}

/// Validates a document: every active tag gets a human `pos`/`neg` label
/// (SPEC 7.1), and the document leaves the review queue.
///
/// SPEC 7.4: scores are recorded to `predictions` *before* the labels are
/// applied, so the prequential log measures genuine out-of-sample
/// performance rather than what the model looks like after learning from
/// this very document.
#[tauri::command]
pub fn validate_document(
    state: State<Db>,
    model: State<Model>,
    doc_id: i64,
    checked_tag_ids: Vec<i64>,
) -> Result<ValidateResult, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now = current_timestamp();
    let active_tags = tags::list_active(&conn).map_err(|e| e.to_string())?;

    // Tags with an existing automatic decision on this document - once the
    // human's decision lands in label_history right below it, these are
    // the ones that just became "audited" (SPEC 7.5).
    let previously_auto = labels::auto_labelled_tag_ids(&conn, doc_id).map_err(|e| e.to_string())?;

    let scores = crate::review::score_document(&conn, &model.0, &active_tags, doc_id).map_err(|e| e.to_string())?;
    for tag_score in &scores {
        if let Some(score) = tag_score.score {
            core_lib::predictions::record(
                &conn,
                doc_id,
                tag_score.tag_id,
                &tag_score.model_version,
                score,
                tag_score.suggested,
                &now,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let checked: HashSet<i64> = checked_tag_ids.into_iter().collect();
    labels::validate_document(&conn, doc_id, &active_tags, &checked, &now).map_err(|e| e.to_string())?;
    enqueue_retrieval_after_tagging(&conn, doc_id, &checked, &now).map_err(|e| e.to_string())?;

    let target_precision = core_lib::settings::target_precision(&conn).map_err(|e| e.to_string())?;
    let target_recall = core_lib::settings::target_recall(&conn).map_err(|e| e.to_string())?;
    let mut auto_disabled_tags = Vec::new();
    for tag in &active_tags {
        if !previously_auto.contains(&tag.id) {
            continue;
        }
        let disabled = crate::automation::check_and_disable_if_below_target(&conn, tag.id, target_precision)
            .map_err(|e| e.to_string())?;
        // Suspension (SPEC 7.6) only matters for a tag the first check
        // didn't already disable - both flip the same `auto_enabled` flag.
        let suspended = !disabled
            && crate::automation::check_and_suspend_for_full_automation(
                &conn,
                tag.id,
                target_precision,
                target_recall,
            )
            .map_err(|e| e.to_string())?;
        if disabled || suspended {
            auto_disabled_tags.push(tag.name.clone());
        }
    }

    if crate::retrain::is_scheduled_retrain_point(&conn).map_err(|e| e.to_string())? {
        crate::retrain::retrain_eligible_tags(&conn, &model.0, &now).map_err(|e| e.to_string())?;
        crate::automation::apply_full_automation(&conn, &model.0, &now).map_err(|e| e.to_string())?;
    }

    Ok(ValidateResult { auto_disabled_tags })
}

/// SPEC 5.5's "after tagging" retrieval policies, checked once a document
/// is validated. Skips enqueueing when retrieval already succeeded, so
/// re-validating a document (e.g. after a tag's definition changed) never
/// re-does finished work.
/// `pub(crate)` so `automation::apply_full_automation` can trigger the same
/// policy check when a document becomes `auto_completed` (SPEC 5.5: "after
/// tagging, when the document is validated *or auto-completed*").
pub(crate) fn enqueue_retrieval_after_tagging(
    conn: &Connection,
    doc_id: i64,
    checked_tag_ids: &HashSet<i64>,
    now: &str,
) -> Result<(), core_lib::storage::StorageError> {
    use core_lib::retrieval_policy::should_retrieve_after_tagging;
    let document_tag_ids: Vec<i64> = checked_tag_ids.iter().copied().collect();

    let fulltext_policy = core_lib::settings::fulltext_policy(conn)?;
    let fulltext_policy_tag_ids = core_lib::settings::fulltext_policy_tag_ids(conn)?;
    let fulltext_already_done = core_lib::fulltext::get(conn, doc_id)?
        .is_some_and(|f| matches!(f.status.as_str(), "fetched" | "non_english_only"));
    if !fulltext_already_done
        && should_retrieve_after_tagging(&fulltext_policy, &fulltext_policy_tag_ids, &document_tag_ids)
    {
        crate::retrieval_worker::enqueue_fulltext(conn, doc_id, now)?;
    }

    let drawings_policy = core_lib::settings::drawings_policy(conn)?;
    let drawings_policy_tag_ids = core_lib::settings::drawings_policy_tag_ids(conn)?;
    let drawings_already_done = core_lib::drawings::get_status(conn, doc_id)?
        .is_some_and(|d| d.status == "fetched");
    if !drawings_already_done
        && should_retrieve_after_tagging(&drawings_policy, &drawings_policy_tag_ids, &document_tag_ids)
    {
        crate::retrieval_worker::enqueue_drawings(conn, doc_id, now)?;
    }
    Ok(())
}

/// Drains any pending `fulltext_retrieval`/`drawings_retrieval` jobs (SPEC
/// 5.5) - both the ones enqueued by `enqueue_retrieval_after_tagging` and
/// any left over from an interrupted previous run. Lower priority than
/// imports (SPEC 5.4): callers run `run_import_jobs` first.
#[tauri::command]
pub async fn run_retrieval_jobs(state: State<'_, Db>) -> Result<(), String> {
    let _lock = crate::lock::PipelineLock::acquire(&state.data_dir).map_err(|e| e.to_string())?;
    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    crate::retrieval_worker::run_fulltext(&state.conn, &client, |_| {}).await;
    crate::retrieval_worker::run_drawings(&state.conn, &client, &state.data_dir, |_| {}).await;
    Ok(())
}

/// The Description/Claims tabs' and Drawings tab's "retrieve now" button
/// (SPEC section 8), for the "on demand" retrieval policy.
#[tauri::command]
pub async fn retrieve_fulltext_now(state: State<'_, Db>, doc_id: i64) -> Result<(), String> {
    let _lock = crate::lock::PipelineLock::acquire(&state.data_dir).map_err(|e| e.to_string())?;
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        crate::retrieval_worker::enqueue_fulltext(&conn, doc_id, &current_timestamp())
            .map_err(|e| e.to_string())?;
    }
    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    crate::retrieval_worker::run_fulltext(&state.conn, &client, |_| {}).await;
    Ok(())
}

#[tauri::command]
pub async fn retrieve_drawings_now(state: State<'_, Db>, doc_id: i64) -> Result<(), String> {
    let _lock = crate::lock::PipelineLock::acquire(&state.data_dir).map_err(|e| e.to_string())?;
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        crate::retrieval_worker::enqueue_drawings(&conn, doc_id, &current_timestamp())
            .map_err(|e| e.to_string())?;
    }
    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    crate::retrieval_worker::run_drawings(&state.conn, &client, &state.data_dir, |_| {}).await;
    Ok(())
}

/// The Drawings tab's page images (SPEC section 8): `page` is 1-based for
/// a real page, or `0` for the `FirstPageClipping` thumbnail.
#[tauri::command]
pub fn read_drawing_page(state: State<Db>, doc_id: i64, page: i64) -> Result<Vec<u8>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    crate::review::read_drawing_page(&conn, &state.data_dir, doc_id, page).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn skip_document(state: State<Db>, doc_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    labels::skip_document(&conn, doc_id).map_err(|e| e.to_string())
}

/// On-demand retraining (SPEC 7.2: "or on demand"), in addition to the
/// automatic every-10-validations trigger in `validate_document`.
#[tauri::command]
pub fn retrain_now(state: State<Db>, model: State<Model>) -> Result<usize, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    crate::retrain::retrain_eligible_tags(&conn, &model.0, &current_timestamp()).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct TagMetricsRow {
    pub name: String,
    #[serde(flatten)]
    pub metrics: core_lib::predictions::TagMetrics,
    /// SPEC 7.5's simpler, `pos`-only automatic-mode safeguard metric.
    pub audited_precision: Option<f32>,
    pub audited_n: i64,
    /// SPEC 7.6's combined precision+recall full-automation safeguard,
    /// from the same shared audit window (see docs/DECISIONS.md).
    pub full_automation_precision: Option<f32>,
    pub full_automation_recall: Option<f32>,
    pub full_automation_n: i64,
}

#[tauri::command]
pub fn tag_metrics(state: State<Db>) -> Result<Vec<TagMetricsRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::list_active(&conn)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|tag| {
            let metrics = core_lib::predictions::tag_metrics(&conn, tag.id).map_err(|e| e.to_string())?;
            let (audited_precision, audited_n) =
                core_lib::audit::audited_precision(&conn, tag.id).map_err(|e| e.to_string())?;
            let (full_automation_precision, full_automation_recall, full_automation_n) =
                core_lib::audit::auto_completion_audit(&conn, tag.id).map_err(|e| e.to_string())?;
            Ok(TagMetricsRow {
                name: tag.name,
                metrics,
                audited_precision,
                audited_n,
                full_automation_precision,
                full_automation_recall,
                full_automation_n,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct FullAutomationSummaryView {
    #[serde(flatten)]
    pub readiness: core_lib::full_automation::ReadinessResult,
    #[serde(flatten)]
    pub counts: core_lib::full_automation::AutomationStateCounts,
}

/// SPEC 7.6: "the Metrics screen shows the share of the last 300
/// documents that would have been auto-completed" plus "counts of
/// auto-completed, audited and focused-review documents."
#[tauri::command]
pub fn full_automation_summary(state: State<Db>) -> Result<FullAutomationSummaryView, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let readiness = core_lib::full_automation::auto_completion_readiness(&conn).map_err(|e| e.to_string())?;
    let counts = core_lib::full_automation::automation_state_counts(&conn).map_err(|e| e.to_string())?;
    Ok(FullAutomationSummaryView { readiness, counts })
}

pub(crate) fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    core_lib::time::format_unix_timestamp(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::storage;

    const NOW: &str = "2026-01-01T00:00:00Z";

    #[test]
    fn sorts_input_lines_into_the_right_report_sections() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let input = "EP 1 234 567 B1\nGB2345678A\nnot a number\n";
        let report = import_numbers_into(&conn, input, NOW).expect("should succeed");

        assert_eq!(report.imported, vec!["EP1234567"]);
        assert_eq!(report.needs_normalisation, vec!["GB2345678A"]);
        assert_eq!(report.unparseable, vec!["not a number"]);
        assert!(report.duplicates.is_empty());
    }

    #[test]
    fn duplicate_within_the_same_input_is_reported_not_imported_twice() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let input = "EP1234567\nEP 1 234 567\n";
        let report = import_numbers_into(&conn, input, NOW).expect("should succeed");

        assert_eq!(report.imported, vec!["EP1234567"]);
        assert_eq!(report.duplicates, vec!["EP 1 234 567"]);
    }

    #[test]
    fn already_imported_document_is_reported_as_duplicate_on_a_second_import() {
        let conn = storage::open_in_memory().expect("in-memory db");
        import_numbers_into(&conn, "EP1234567", NOW).unwrap();
        let report = import_numbers_into(&conn, "EP1234567", NOW).expect("should succeed");

        assert!(report.imported.is_empty());
        assert_eq!(report.duplicates, vec!["EP1234567"]);
    }

    #[test]
    fn each_imported_document_gets_an_import_job() {
        let conn = storage::open_in_memory().expect("in-memory db");
        import_numbers_into(&conn, "EP1234567\nUS11000000B2", NOW).unwrap();

        let queued = jobs::list_resumable(&conn, "import_document").unwrap();
        assert_eq!(queued.len(), 2);
    }

    #[test]
    fn blank_lines_and_crlf_are_tolerated() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let report = import_numbers_into(&conn, "EP1234567\r\n\r\nUS11000000B2\r\n", NOW)
            .expect("should succeed");
        assert_eq!(report.imported, vec!["EP1234567", "US11000000"]);
    }
}
