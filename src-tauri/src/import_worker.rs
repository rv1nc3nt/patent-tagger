//! Drains `import_document` jobs: fetches biblio+abstract from OPS,
//! applies the selection cascade (SPEC 5.3), and persists the result.

use core_lib::rusqlite::Connection;
use core_lib::selection::{self, Candidate};
use core_lib::{documents, jobs, time};
use ops_lib::biblio::{self, Publication};
use ops_lib::client::OpsClient;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ImportPayload {
    doc_id: i64,
    pub_key: String,
}

/// One row of the Import screen's report (SPEC section 8): imported,
/// no English abstract, not found, error - plus any related documents
/// (same application/family, SPEC 5.2) discovered once this one was fetched.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DocumentOutcome {
    pub doc_id: i64,
    pub pub_key: String,
    /// `"fetched" | "no_english_abstract" | "not_found" | "error"`.
    pub status: String,
    pub title: Option<String>,
    pub error: Option<String>,
    pub related_pub_keys: Vec<String>,
}

/// Processes every resumable `import_document` job once, calling
/// `on_progress` after each one (for the Import screen's progress display -
/// SPEC section 8) - so it's usable both from the real app (which emits a
/// Tauri event) and from tests (which can just collect into a `Vec`, or
/// pass `|_| {}` to ignore it). Safe to call repeatedly: jobs left in
/// `running` from an earlier, incomplete pass are picked up again, per
/// SPEC 5.4.
pub async fn run(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    mut on_progress: impl FnMut(&DocumentOutcome),
) -> Vec<DocumentOutcome> {
    let pending = {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        jobs::list_resumable(&conn, "import_document").unwrap_or_default()
    };

    let mut results = Vec::with_capacity(pending.len());

    for job in pending {
        let now = now_iso8601();
        {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let _ = jobs::mark_running(&conn, job.id, &now);
        }

        let payload: ImportPayload = match serde_json::from_str(&job.payload) {
            Ok(p) => p,
            Err(e) => {
                let conn = conn_mutex.lock().expect("db mutex poisoned");
                let message = format!("bad job payload: {e}");
                let _ = jobs::mark_failed(&conn, job.id, &message, &now);
                let outcome = DocumentOutcome {
                    doc_id: -1,
                    pub_key: String::new(),
                    status: "error".to_string(),
                    title: None,
                    error: Some(message),
                    related_pub_keys: vec![],
                };
                on_progress(&outcome);
                results.push(outcome);
                continue;
            }
        };

        let outcome = process_one(conn_mutex, client, &job, &payload, &now).await;
        on_progress(&outcome);
        results.push(outcome);
    }

    results
}

async fn process_one(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    job: &core_lib::jobs::JobRow,
    payload: &ImportPayload,
    now: &str,
) -> DocumentOutcome {
    match fetch_and_select(client, &payload.pub_key).await {
        Ok(data) => {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let title = data.title.clone();
            let status = if data.abstract_text.is_some() {
                "fetched"
            } else {
                "no_english_abstract"
            };
            if let Err(e) = documents::store_fetched(&conn, payload.doc_id, &data) {
                let _ = jobs::mark_failed(&conn, job.id, &e.to_string(), now);
                return DocumentOutcome {
                    doc_id: payload.doc_id,
                    pub_key: payload.pub_key.clone(),
                    status: "error".to_string(),
                    title: None,
                    error: Some(e.to_string()),
                    related_pub_keys: vec![],
                };
            }
            let _ = jobs::mark_done(&conn, job.id, now);
            let related_pub_keys = documents::find_related(&conn, payload.doc_id)
                .unwrap_or_default()
                .into_iter()
                .map(|d| d.pub_key)
                .collect();
            DocumentOutcome {
                doc_id: payload.doc_id,
                pub_key: payload.pub_key.clone(),
                status: status.to_string(),
                title,
                error: None,
                related_pub_keys,
            }
        }
        Err(e) => {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let not_found = matches!(&e, ops_lib::OpsError::Api { status, .. } if *status == 404);
            if not_found {
                let _ = documents::mark_not_found(&conn, payload.doc_id);
            } else {
                let _ = documents::mark_fetch_error(&conn, payload.doc_id, &e.to_string());
            }
            let _ = jobs::mark_failed(&conn, job.id, &e.to_string(), now);
            DocumentOutcome {
                doc_id: payload.doc_id,
                pub_key: payload.pub_key.clone(),
                status: if not_found { "not_found" } else { "error" }.to_string(),
                title: None,
                error: Some(e.to_string()),
                related_pub_keys: vec![],
            }
        }
    }
}

async fn fetch_and_select(
    client: &OpsClient,
    pub_key: &str,
) -> Result<documents::FetchedData, ops_lib::OpsError> {
    let (country, number) = split_pub_key(pub_key);

    let body = client
        .get(&format!("/published-data/publication/docdb/{country}.{number}/biblio,abstract"))
        .await?;
    let mut publications = biblio::parse(&body)?;
    if publications.is_empty() {
        return Err(ops_lib::OpsError::Parse(format!(
            "no exchange-document in response for {pub_key}"
        )));
    }

    let requested = publications.remove(0);
    let mut all_candidates: Vec<Publication> = publications;

    let requested_candidate = to_candidate(&requested);
    let mut sibling_candidates: Vec<Candidate> = all_candidates.iter().map(to_candidate).collect();

    let mut abstract_selected = selection::select_abstract(&requested_candidate, &sibling_candidates);

    if abstract_selected.is_none() {
        // Step 3 of the cascade: no English abstract among the requested
        // publication's own siblings, so widen the search to the family.
        if let Ok(family_body) = client
            .get(&format!("/family/publication/docdb/{country}.{number}/biblio,abstract"))
            .await
        {
            if let Ok(family_publications) = biblio::parse(&family_body) {
                let family_candidates: Vec<Candidate> =
                    family_publications.iter().map(to_candidate).collect();
                abstract_selected = selection::select_abstract(&requested_candidate, &family_candidates);
                all_candidates.extend(family_publications);
                sibling_candidates = family_candidates;
            }
        }
    }

    let title_selected = selection::select_title(&requested_candidate, &sibling_candidates);

    let source_publication = abstract_selected
        .as_ref()
        .and_then(|s| all_candidates.iter().find(|p| p.docdb_id == s.source_docdb_id))
        .unwrap_or(&requested);

    let mut kind_codes: Vec<String> = std::iter::once(requested.kind.clone())
        .chain(all_candidates.iter().map(|p| p.kind.clone()))
        .collect();
    kind_codes.sort();
    kind_codes.dedup();

    Ok(documents::FetchedData {
        title: title_selected.map(|s| s.text),
        abstract_text: abstract_selected.as_ref().map(|s| s.text.clone()),
        abstract_source: abstract_selected.map(|s| s.source_docdb_id),
        applicants: if requested.applicants.is_empty() {
            source_publication.applicants.clone()
        } else {
            requested.applicants.clone()
        },
        publication_date: requested.publication_date.clone(),
        cpc: requested.cpc.clone(),
        ipc: requested.ipc.clone(),
        application_number: requested
            .application_number
            .clone()
            .or_else(|| source_publication.application_number.clone()),
        family_id: requested.family_id.clone().or_else(|| source_publication.family_id.clone()),
        kind_codes,
    })
}

fn to_candidate(p: &Publication) -> Candidate {
    Candidate {
        docdb_id: p.docdb_id.clone(),
        country: p.country.clone(),
        application_number: p.application_number.clone(),
        titles: p.titles.clone(),
        abstracts: p.abstracts.clone(),
    }
}

/// `pub_key` is `{country}{number}` with no separator (SPEC 4.2); country
/// is always the leading alphabetic run (guaranteed by
/// `core_lib::number::ParsedNumber::pub_key`).
fn split_pub_key(pub_key: &str) -> (&str, &str) {
    let split_at = pub_key
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map_or(pub_key.len(), |(i, _)| i);
    pub_key.split_at(split_at)
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    time::format_unix_timestamp(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn splits_pub_key_into_country_and_number() {
        assert_eq!(split_pub_key("EP1234567"), ("EP", "1234567"));
        assert_eq!(split_pub_key("WO2019123456"), ("WO", "2019123456"));
        assert_eq!(split_pub_key("US11000000"), ("US", "11000000"));
    }

    #[test]
    fn to_candidate_carries_the_fields_selection_needs() {
        let mut titles = BTreeMap::new();
        titles.insert("en".to_string(), "A gadget".to_string());
        let publication = Publication {
            docdb_id: "EP.1.A1".to_string(),
            country: "EP".to_string(),
            doc_number: "1".to_string(),
            kind: "A1".to_string(),
            family_id: Some("42".to_string()),
            application_number: Some("EP2020000001".to_string()),
            publication_date: Some("2020-01-01".to_string()),
            titles,
            abstracts: BTreeMap::new(),
            applicants: vec![],
            ipc: vec![],
            cpc: vec![],
        };
        let candidate = to_candidate(&publication);
        assert_eq!(candidate.docdb_id, "EP.1.A1");
        assert_eq!(candidate.application_number.as_deref(), Some("EP2020000001"));
        assert_eq!(candidate.titles.get("en").map(String::as_str), Some("A gadget"));
    }
}
