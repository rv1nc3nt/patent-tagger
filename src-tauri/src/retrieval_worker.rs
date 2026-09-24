//! Drains `fulltext_retrieval`/`drawings_retrieval` jobs (SPEC 5.5):
//! rebuilds the same-application/family candidate set OPS provides
//! (re-fetching biblio the way `import_worker` did at import time, since
//! `documents` only stores the final merged result, not every sibling's
//! docdb id), applies `core_lib::selection`'s cascade, fetches the winning
//! publication's text/pages, and persists. Runs at lower priority than
//! imports (SPEC 5.4) - callers drain `import_worker::run` first.

use core_lib::rusqlite::Connection;
use core_lib::selection::{
    self, DrawingsCandidate, DrawingsSelected, FulltextCandidate, FulltextSelected,
};
use core_lib::{documents, drawings, fulltext, jobs, storage::StorageError, time};
use ops_lib::biblio::{self, Publication};
use ops_lib::client::OpsClient;
use ops_lib::images::DocumentInstance;
use ops_lib::{fulltext as ops_fulltext, images as ops_images, OpsError};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

pub const FULLTEXT_JOB_KIND: &str = "fulltext_retrieval";
pub const DRAWINGS_JOB_KIND: &str = "drawings_retrieval";
/// The page number `drawings::insert_page` uses for the `FirstPageClipping`
/// instance (SPEC 5.5: "serves as the thumbnail in lists") - outside the
/// 1-based real page range, so it never collides with an actual page.
pub const THUMBNAIL_PAGE: i64 = 0;

#[derive(Debug, Deserialize)]
struct RetrievalPayload {
    doc_id: i64,
}

pub fn enqueue_fulltext(conn: &Connection, doc_id: i64, now: &str) -> Result<(), StorageError> {
    let payload = serde_json::json!({ "doc_id": doc_id }).to_string();
    jobs::enqueue(conn, FULLTEXT_JOB_KIND, &payload, now)?;
    Ok(())
}

pub fn enqueue_drawings(conn: &Connection, doc_id: i64, now: &str) -> Result<(), StorageError> {
    let payload = serde_json::json!({ "doc_id": doc_id }).to_string();
    jobs::enqueue(conn, DRAWINGS_JOB_KIND, &payload, now)?;
    Ok(())
}

pub async fn run_fulltext(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    mut on_progress: impl FnMut(i64),
) {
    let pending = {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        jobs::list_resumable(&conn, FULLTEXT_JOB_KIND).unwrap_or_default()
    };
    for job in pending {
        let now = now_iso8601();
        {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let _ = jobs::mark_running(&conn, job.id, &now);
        }
        let Ok(payload) = serde_json::from_str::<RetrievalPayload>(&job.payload) else {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let _ = jobs::mark_failed(&conn, job.id, "bad job payload", &now);
            continue;
        };

        let result = process_fulltext(conn_mutex, client, payload.doc_id, &now).await;
        {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            match result {
                Ok(()) => {
                    let _ = jobs::mark_done(&conn, job.id, &now);
                }
                Err(e) => {
                    let _ = jobs::mark_failed(&conn, job.id, &e.to_string(), &now);
                    let _ = fulltext::store_error(&conn, payload.doc_id, &now);
                }
            }
        }
        on_progress(payload.doc_id);
    }
}

pub async fn run_drawings(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    data_dir: &Path,
    mut on_progress: impl FnMut(i64),
) {
    let pending = {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        jobs::list_resumable(&conn, DRAWINGS_JOB_KIND).unwrap_or_default()
    };
    for job in pending {
        let now = now_iso8601();
        {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let _ = jobs::mark_running(&conn, job.id, &now);
        }
        let Ok(payload) = serde_json::from_str::<RetrievalPayload>(&job.payload) else {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            let _ = jobs::mark_failed(&conn, job.id, "bad job payload", &now);
            continue;
        };

        let result = process_drawings(conn_mutex, client, data_dir, payload.doc_id, &now).await;
        {
            let conn = conn_mutex.lock().expect("db mutex poisoned");
            match result {
                Ok(()) => {
                    let _ = jobs::mark_done(&conn, job.id, &now);
                }
                Err(e) => {
                    let _ = jobs::mark_failed(&conn, job.id, &e.to_string(), &now);
                    let _ = drawings::store_error(&conn, payload.doc_id, &now);
                }
            }
        }
        on_progress(payload.doc_id);
    }
}

struct DocContext {
    pub_key: String,
    application_number: Option<String>,
    abstract_source: Option<String>,
}

fn load_context(
    conn_mutex: &std::sync::Mutex<Connection>,
    doc_id: i64,
) -> Result<DocContext, OpsError> {
    let conn = conn_mutex.lock().expect("db mutex poisoned");
    let detail = documents::get_full(&conn, doc_id)
        .map_err(|e| OpsError::Parse(e.to_string()))?
        .ok_or_else(|| OpsError::Parse(format!("document {doc_id} not found")))?;
    Ok(DocContext {
        pub_key: detail.pub_key,
        application_number: detail.application_number,
        abstract_source: detail.abstract_source,
    })
}

async fn process_fulltext(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    doc_id: i64,
    now: &str,
) -> Result<(), OpsError> {
    let ctx = load_context(conn_mutex, doc_id)?;
    let requested_docdb = ctx
        .abstract_source
        .clone()
        .unwrap_or_else(|| ctx.pub_key.clone());

    let requested_langs = fetch_fulltext_langs(client, &requested_docdb).await?;
    let requested_candidate = FulltextCandidate {
        docdb_id: requested_docdb.clone(),
        country: country_of(&requested_docdb),
        application_number: ctx.application_number.clone(),
        langs: requested_langs,
    };

    let mut candidates: Vec<FulltextCandidate> = Vec::new();
    if !has_english(&requested_candidate.langs) {
        let (country, number) = crate::import_worker::split_pub_key(&ctx.pub_key);
        let siblings = fetch_siblings(client, country, number).await?;
        for p in siblings.iter().filter(|p| p.docdb_id != requested_docdb) {
            let langs = fetch_fulltext_langs(client, &p.docdb_id).await?;
            candidates.push(FulltextCandidate {
                docdb_id: p.docdb_id.clone(),
                country: p.country.clone(),
                application_number: p.application_number.clone(),
                langs,
            });
        }

        // Family is only consulted when nothing found so far has English -
        // avoids the extra family-lookup and per-member inquiry calls in
        // the common case where the primary publication (or a sibling)
        // already carries full text (see docs/DECISIONS.md).
        if !candidates.iter().any(|c| has_english(&c.langs)) {
            if let Ok(family) = fetch_family(client, country, number).await {
                let mut seen: std::collections::HashSet<String> =
                    candidates.iter().map(|c| c.docdb_id.clone()).collect();
                seen.insert(requested_docdb.clone());
                let new_family_members: Vec<Publication> = family
                    .into_iter()
                    .filter(|p| !seen.contains(&p.docdb_id))
                    .collect();
                for p in &new_family_members {
                    seen.insert(p.docdb_id.clone());
                    let langs = fetch_fulltext_langs(client, &p.docdb_id).await?;
                    candidates.push(FulltextCandidate {
                        docdb_id: p.docdb_id.clone(),
                        country: p.country.clone(),
                        application_number: p.application_number.clone(),
                        langs,
                    });
                }
            }
        }
    }

    let Some(selected) = selection::select_fulltext(&requested_candidate, &candidates) else {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        fulltext::store_not_available(&conn, doc_id, now).map_err(storage_err)?;
        return Ok(());
    };

    finish_fulltext(conn_mutex, client, doc_id, &selected, now).await
}

async fn finish_fulltext(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    doc_id: i64,
    selected: &FulltextSelected,
    now: &str,
) -> Result<(), OpsError> {
    let lang = selected.lang.to_lowercase();
    let description = fetch_part(
        client,
        &selected.source_docdb_id,
        "description",
        &lang,
        ops_fulltext::parse_description,
    )
    .await?;
    let claims = fetch_part(
        client,
        &selected.source_docdb_id,
        "claims",
        &lang,
        ops_fulltext::parse_claims,
    )
    .await?;

    let conn = conn_mutex.lock().expect("db mutex poisoned");
    fulltext::store_fetched(
        &conn,
        doc_id,
        description.as_deref(),
        claims.as_deref(),
        &selected.lang,
        &selected.source_docdb_id,
        selected.non_english_only,
        now,
    )
    .map_err(storage_err)
}

async fn fetch_part(
    client: &OpsClient,
    docdb_id: &str,
    part: &str,
    lang: &str,
    parse: impl Fn(&str, &str) -> Result<Option<(String, String)>, OpsError>,
) -> Result<Option<String>, OpsError> {
    match client
        .get(&format!(
            "/published-data/publication/docdb/{docdb_id}/{part}"
        ))
        .await
    {
        Ok(body) => Ok(parse(&body, lang)?.map(|(_, text)| text)),
        Err(OpsError::Api { status: 404, .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

async fn fetch_fulltext_langs(
    client: &OpsClient,
    docdb_id: &str,
) -> Result<BTreeSet<String>, OpsError> {
    match client
        .get(&format!(
            "/published-data/publication/docdb/{docdb_id}/fulltext"
        ))
        .await
    {
        Ok(body) => Ok(ops_fulltext::parse_inquiry(&body)?
            .into_iter()
            .map(|i| i.lang)
            .collect()),
        Err(OpsError::Api { status: 404, .. }) => Ok(BTreeSet::new()),
        Err(e) => Err(e),
    }
}

fn has_english(langs: &BTreeSet<String>) -> bool {
    langs.iter().any(|l| l.eq_ignore_ascii_case("en"))
}

async fn process_drawings(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    data_dir: &Path,
    doc_id: i64,
    now: &str,
) -> Result<(), OpsError> {
    let ctx = load_context(conn_mutex, doc_id)?;

    // SPEC 5.5: "prefer the drawings of the publication used for the full
    // text" - fall back to the abstract's source when full text was never
    // retrieved (e.g. the fulltext policy is "never").
    let requested_docdb = {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        fulltext::get(&conn, doc_id)
            .ok()
            .flatten()
            .and_then(|f| f.source)
    }
    .or_else(|| ctx.abstract_source.clone())
    .unwrap_or_else(|| ctx.pub_key.clone());

    let requested_instances = fetch_images_inquiry(client, &requested_docdb).await?;
    let requested_candidate = DrawingsCandidate {
        docdb_id: requested_docdb.clone(),
        country: country_of(&requested_docdb),
        application_number: ctx.application_number.clone(),
        has_drawings: ops_images::find_drawing(&requested_instances).is_some(),
    };

    let mut candidates: Vec<DrawingsCandidate> = Vec::new();
    let mut instances_by_docdb: Vec<(String, Vec<DocumentInstance>)> =
        vec![(requested_docdb.clone(), requested_instances)];

    if !requested_candidate.has_drawings {
        let (country, number) = crate::import_worker::split_pub_key(&ctx.pub_key);
        let siblings = fetch_siblings(client, country, number).await?;
        for p in siblings.iter().filter(|p| p.docdb_id != requested_docdb) {
            let instances = fetch_images_inquiry(client, &p.docdb_id).await?;
            candidates.push(DrawingsCandidate {
                docdb_id: p.docdb_id.clone(),
                country: p.country.clone(),
                application_number: p.application_number.clone(),
                has_drawings: ops_images::find_drawing(&instances).is_some(),
            });
            instances_by_docdb.push((p.docdb_id.clone(), instances));
        }

        if !candidates.iter().any(|c| c.has_drawings) {
            if let Ok(family) = fetch_family(client, country, number).await {
                let mut seen: std::collections::HashSet<String> =
                    candidates.iter().map(|c| c.docdb_id.clone()).collect();
                seen.insert(requested_docdb.clone());
                let new_family_members: Vec<Publication> = family
                    .into_iter()
                    .filter(|p| !seen.contains(&p.docdb_id))
                    .collect();
                for p in &new_family_members {
                    seen.insert(p.docdb_id.clone());
                    let instances = fetch_images_inquiry(client, &p.docdb_id).await?;
                    candidates.push(DrawingsCandidate {
                        docdb_id: p.docdb_id.clone(),
                        country: p.country.clone(),
                        application_number: p.application_number.clone(),
                        has_drawings: ops_images::find_drawing(&instances).is_some(),
                    });
                    instances_by_docdb.push((p.docdb_id.clone(), instances));
                }
            }
        }
    }

    let Some(DrawingsSelected { source_docdb_id }) =
        selection::select_drawings(&requested_candidate, &candidates)
    else {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        drawings::store_not_available(&conn, doc_id, now).map_err(storage_err)?;
        return Ok(());
    };

    let instances = instances_by_docdb
        .into_iter()
        .find(|(docdb, _)| *docdb == source_docdb_id)
        .map(|(_, instances)| instances)
        .unwrap_or_default();
    let Some(drawing) = ops_images::find_drawing(&instances) else {
        let conn = conn_mutex.lock().expect("db mutex poisoned");
        drawings::store_not_available(&conn, doc_id, now).map_err(storage_err)?;
        return Ok(());
    };

    let pages_dir = data_dir.join("drawings").join(&ctx.pub_key);
    std::fs::create_dir_all(&pages_dir)
        .map_err(|e| OpsError::Parse(format!("creating drawings directory: {e}")))?;

    for page in 1..=drawing.number_of_pages {
        fetch_and_store_page(
            conn_mutex,
            client,
            &pages_dir,
            &ctx.pub_key,
            doc_id,
            &source_docdb_id,
            &drawing.link,
            page as i64,
            now,
        )
        .await?;
    }

    // SPEC 5.5: "also retrieve the FirstPageClipping...it serves as the
    // thumbnail in lists" - stored at THUMBNAIL_PAGE, not part of the
    // 1-based page range.
    if let Some(clipping) = ops_images::find_first_page_clipping(&instances) {
        fetch_and_store_page(
            conn_mutex,
            client,
            &pages_dir,
            &ctx.pub_key,
            doc_id,
            &source_docdb_id,
            &clipping.link,
            THUMBNAIL_PAGE,
            now,
        )
        .await?;
    }

    let conn = conn_mutex.lock().expect("db mutex poisoned");
    drawings::store_fetched_status(
        &conn,
        doc_id,
        drawing.number_of_pages as i64,
        &source_docdb_id,
        now,
    )
    .map_err(storage_err)
}

#[allow(clippy::too_many_arguments)]
async fn fetch_and_store_page(
    conn_mutex: &std::sync::Mutex<Connection>,
    client: &OpsClient,
    pages_dir: &Path,
    pub_key: &str,
    doc_id: i64,
    source_docdb_id: &str,
    link: &str,
    page: i64,
    now: &str,
) -> Result<(), OpsError> {
    let tiff_bytes = client
        .get_bytes(&format!("/{link}"), ("X-OPS-Range", &page.to_string()))
        .await?;
    let (width, height, png_bytes) = ops_images::tiff_page_to_png(&tiff_bytes)
        .map_err(|e| OpsError::Parse(format!("converting drawing page {page}: {e}")))?;

    let file_name = if page == THUMBNAIL_PAGE {
        "thumbnail.png".to_string()
    } else {
        format!("{page:03}.png")
    };
    let relative_path = format!("drawings/{pub_key}/{file_name}");
    std::fs::write(pages_dir.join(&file_name), &png_bytes)
        .map_err(|e| OpsError::Parse(format!("writing drawing page {page}: {e}")))?;

    let conn = conn_mutex.lock().expect("db mutex poisoned");
    drawings::insert_page(
        &conn,
        &drawings::NewDrawingPage {
            doc_id,
            page,
            source: source_docdb_id,
            path: &relative_path,
            width: width as i64,
            height: height as i64,
            fetched_at: now,
        },
    )
    .map_err(storage_err)
}

async fn fetch_images_inquiry(
    client: &OpsClient,
    docdb_id: &str,
) -> Result<Vec<DocumentInstance>, OpsError> {
    match client
        .get(&format!(
            "/published-data/publication/docdb/{docdb_id}/images"
        ))
        .await
    {
        Ok(body) => ops_images::parse_images_inquiry(&body),
        Err(OpsError::Api { status: 404, .. }) => Ok(vec![]),
        Err(e) => Err(e),
    }
}

/// Re-fetches the same "biblio,abstract" call `import_worker` made at
/// import time - SPEC 5.5's "another publication of the same application"
/// is, in practice, every kind-code variant OPS returns for this
/// publication number (see `import_worker::fetch_and_select`).
async fn fetch_siblings(
    client: &OpsClient,
    country: &str,
    number: &str,
) -> Result<Vec<Publication>, OpsError> {
    let body = client
        .get(&format!(
            "/published-data/publication/docdb/{country}.{number}/biblio,abstract"
        ))
        .await?;
    biblio::parse(&body)
}

async fn fetch_family(
    client: &OpsClient,
    country: &str,
    number: &str,
) -> Result<Vec<Publication>, OpsError> {
    let body = client
        .get(&format!(
            "/family/publication/docdb/{country}.{number}/biblio,abstract"
        ))
        .await?;
    biblio::parse(&body)
}

/// A docdb id's country is its leading `{country}.` segment, e.g. `"EP"`
/// from `"EP.1000000.A1"`.
fn country_of(docdb_id: &str) -> String {
    docdb_id.split('.').next().unwrap_or_default().to_string()
}

fn storage_err(e: StorageError) -> OpsError {
    OpsError::Parse(e.to_string())
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

    #[test]
    fn country_of_reads_the_leading_docdb_segment() {
        assert_eq!(country_of("EP.1000000.A1"), "EP");
        assert_eq!(country_of("WO.2012000001.A1"), "WO");
    }

    #[test]
    fn has_english_is_case_insensitive() {
        assert!(has_english(&BTreeSet::from(["EN".to_string()])));
        assert!(!has_english(&BTreeSet::from([
            "DE".to_string(),
            "FR".to_string()
        ])));
        assert!(!has_english(&BTreeSet::new()));
    }
}
