//! Saved searches by applicant (SPEC 5.6): Tauri commands for the Import
//! screen, and the batch step shared with the CLI's `import --search`.
//! A batch only enqueues `import_document` jobs; fetching them is the
//! normal import worker's job, as for a pasted list.

use crate::db::Db;
use core_lib::rusqlite::Connection;
use core_lib::searches::{self, SavedSearch, SearchFields, SearchHit};
use core_lib::{documents, jobs};
use ops_lib::client::OpsClient;
use ops_lib::search::SearchPage;
use serde::Serialize;
use tauri::State;

/// What one batch did (SPEC 5.6), for the Import screen and the CLI.
#[derive(Debug, Serialize, Default, PartialEq)]
pub struct BatchReport {
    /// The search after the batch (new position, total, imported count).
    pub search: Option<SavedSearch>,
    /// The results read, e.g. 101 to 200.
    pub range: Option<(i64, i64)>,
    pub scanned: usize,
    /// Distinct families among the results read.
    pub families: usize,
    /// Publication chosen for a family already in the database, not imported.
    pub known_families: Vec<String>,
    /// Chosen publications whose `pub_key` is already in the database.
    pub duplicates: Vec<String>,
    /// `pub_key`s enqueued for import.
    pub imported: Vec<String>,
}

/// Reads the next page of `search_id` from OPS and enqueues its new
/// families for import. An exhausted search returns a report with no range.
pub(crate) async fn fetch_next_batch(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    search_id: i64,
    now: &str,
) -> anyhow::Result<BatchReport> {
    let search = {
        let conn = lock(conn_mutex)?;
        searches::get(&conn, search_id)?
            .ok_or_else(|| anyhow::anyhow!("search {search_id} not found"))?
    };
    let Some((begin, end)) = searches::next_range(&search) else {
        return Ok(BatchReport {
            search: Some(search),
            ..Default::default()
        });
    };
    let page = ops_lib::search::search(client, &search.query, begin, end).await?;
    let conn = lock(conn_mutex)?;
    Ok(import_page(&conn, &search, (begin, end), &page, now)?)
}

fn lock(
    conn_mutex: &std::sync::Mutex<Connection>,
) -> anyhow::Result<std::sync::MutexGuard<'_, Connection>> {
    conn_mutex
        .lock()
        .map_err(|_| anyhow::anyhow!("database mutex poisoned"))
}

/// The database side of a batch: one publication per family, families
/// already present skipped, the rest enqueued with their family id, and
/// the search's position advanced.
fn import_page(
    conn: &Connection,
    search: &SavedSearch,
    (begin, end): (i64, i64),
    page: &SearchPage,
    now: &str,
) -> Result<BatchReport, searches::SearchError> {
    let hits: Vec<SearchHit> = page
        .publications
        .iter()
        .map(|p| SearchHit {
            country: p.country.clone(),
            doc_number: p.doc_number.clone(),
            kind: p.kind.clone(),
            family_id: p.family_id.clone(),
            publication_date: p.publication_date.clone(),
            has_english_abstract: p.abstract_text("en").is_some(),
        })
        .collect();
    let chosen = searches::one_per_family(&hits);

    let mut report = BatchReport {
        range: (!hits.is_empty()).then(|| (begin, begin + hits.len() as i64 - 1)),
        scanned: hits.len(),
        families: chosen.len(),
        ..Default::default()
    };
    let mut imported = 0;
    for hit in chosen {
        let pub_key = hit.pub_key();
        if let Some(family_id) = &hit.family_id {
            if documents::family_exists(conn, family_id)? {
                report.known_families.push(pub_key);
                continue;
            }
        }
        match documents::insert_pending(conn, &pub_key, &hit.docdb_id(), now)? {
            Some(doc_id) => {
                if let Some(family_id) = &hit.family_id {
                    documents::set_family_id(conn, doc_id, family_id)?;
                }
                let payload =
                    serde_json::json!({ "doc_id": doc_id, "pub_key": pub_key }).to_string();
                jobs::enqueue(conn, "import_document", &payload, now)?;
                report.imported.push(pub_key);
                imported += 1;
            }
            None => report.duplicates.push(pub_key),
        }
    }

    // Position after the last result OPS could return for this range: an
    // empty or short page means the end of the results.
    let range_end = end.min(page.total_results);
    searches::record_batch(
        conn,
        search.id,
        page.total_results,
        range_end,
        imported,
        now,
    )?;
    report.search = searches::get(conn, search.id)?;
    Ok(report)
}

#[tauri::command]
pub fn list_searches(state: State<Db>) -> Result<Vec<SavedSearch>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    searches::list(&conn).map_err(|e| e.to_string())
}

/// The CQL query `fields` would run, for display while the user types.
#[tauri::command]
pub fn preview_search_query(fields: SearchFields) -> Result<String, String> {
    searches::build_query(&fields).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_search(state: State<Db>, fields: SearchFields) -> Result<SavedSearch, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    searches::create(&conn, &fields, &crate::commands::current_timestamp())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_search(state: State<Db>, search_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    searches::delete(&conn, search_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn restart_search(state: State<Db>, search_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    searches::restart(&conn, search_id).map_err(|e| e.to_string())
}

/// Reads the next 100 results and enqueues their new families. The Import
/// screen then runs `run_import_jobs`, as after a pasted list.
#[tauri::command]
pub async fn fetch_search_batch(
    state: State<'_, Db>,
    search_id: i64,
) -> Result<BatchReport, String> {
    let _lock = crate::lock::PipelineLock::acquire(&state.data_dir).map_err(|e| e.to_string())?;
    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = OpsClient::new(creds.consumer_key, creds.consumer_secret);
    fetch_next_batch(
        &state.conn,
        &client,
        search_id,
        &crate::commands::current_timestamp(),
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::storage;

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn fixture_page() -> SearchPage {
        let xml = std::fs::read_to_string(format!(
            "{}/../tests/fixtures/ops/search_biblio_siemens_healthineers_2021_1-10.xml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture");
        ops_lib::search::parse(&xml).expect("parses")
    }

    fn new_search(conn: &Connection) -> SavedSearch {
        let fields = SearchFields {
            name: "Siemens Healthineers 2021".to_string(),
            applicant: "Siemens Healthineers".to_string(),
            year_from: Some(2021),
            year_to: Some(2021),
            ..Default::default()
        };
        searches::create(conn, &fields, NOW).expect("create")
    }

    #[test]
    fn a_page_enqueues_one_document_per_new_family_and_advances_the_search() {
        let conn = storage::open_in_memory().unwrap();
        let search = new_search(&conn);
        let report = import_page(&conn, &search, (1, 100), &fixture_page(), NOW).unwrap();

        assert_eq!(report.scanned, 10);
        assert_eq!(report.families, 10);
        assert_eq!(report.imported.len(), 10);
        assert_eq!(report.imported[0], "US2021405700");
        assert_eq!(report.range, Some((1, 10)));
        assert_eq!(
            jobs::list_resumable(&conn, "import_document")
                .unwrap()
                .len(),
            10
        );

        let doc = documents::find_by_pub_key(&conn, "US2021405700")
            .unwrap()
            .unwrap();
        assert_eq!(doc.family_id.as_deref(), Some("78826798"));
        assert_eq!(doc.input_raw, "US.2021405700.A1");

        let search = report.search.unwrap();
        assert_eq!(search.total_results, Some(170));
        assert_eq!(search.next_start, 101);
        assert_eq!(search.imported, 10);
    }

    #[test]
    fn families_already_in_the_database_are_skipped() {
        let conn = storage::open_in_memory().unwrap();
        let existing = documents::insert_pending(&conn, "EP9999999", "EP9999999", NOW)
            .unwrap()
            .unwrap();
        documents::set_family_id(&conn, existing, "78826798").unwrap();
        documents::insert_pending(&conn, "US2021407674", "US2021407674", NOW).unwrap();

        let search = new_search(&conn);
        let report = import_page(&conn, &search, (1, 100), &fixture_page(), NOW).unwrap();

        assert_eq!(report.known_families, vec!["US2021405700"]);
        assert_eq!(report.duplicates, vec!["US2021407674"]);
        assert_eq!(report.imported.len(), 8);
        assert_eq!(report.search.unwrap().imported, 8);
    }

    #[test]
    fn an_empty_page_exhausts_the_search() {
        let conn = storage::open_in_memory().unwrap();
        let search = new_search(&conn);
        let empty = SearchPage {
            total_results: 0,
            publications: vec![],
        };
        let report = import_page(&conn, &search, (1, 100), &empty, NOW).unwrap();
        assert_eq!(report.range, None);
        assert!(report.search.unwrap().exhausted);
    }
}
