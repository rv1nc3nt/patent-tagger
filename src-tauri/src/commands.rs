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

    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    Ok(crate::import_worker::run(&state.conn, &client, &model.0, |outcome| {
        let _ = app.emit("import-progress", outcome);
    })
    .await)
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
/// worker that drains those jobs, not here. Kept free of Tauri types so it's
/// directly unit-testable against an in-memory database.
fn import_numbers_into(
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

#[tauri::command]
pub fn review_queue(state: State<Db>) -> Result<Vec<documents::QueueEntry>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    documents::list_queue(&conn).map_err(|e| e.to_string())
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

/// Validates a document: every active tag gets a human `pos`/`neg` label
/// (SPEC 7.1), and the document leaves the review queue.
#[tauri::command]
pub fn validate_document(state: State<Db>, doc_id: i64, checked_tag_ids: Vec<i64>) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let active_tags = tags::list_active(&conn).map_err(|e| e.to_string())?;
    let checked: HashSet<i64> = checked_tag_ids.into_iter().collect();
    labels::validate_document(&conn, doc_id, &active_tags, &checked, &current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn skip_document(state: State<Db>, doc_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    labels::skip_document(&conn, doc_id).map_err(|e| e.to_string())
}

fn current_timestamp() -> String {
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
