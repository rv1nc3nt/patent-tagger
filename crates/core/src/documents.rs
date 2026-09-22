//! Persistence for the `documents` table (SPEC section 4.2). Timestamps are
//! passed in rather than read from the system clock, so this stays
//! deterministic and testable without mocking time.

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRow {
    pub id: i64,
    pub pub_key: String,
    pub input_raw: String,
    pub application_number: Option<String>,
    pub family_id: Option<String>,
    pub fetch_status: String,
    pub review_state: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct QueueEntry {
    pub id: i64,
    pub pub_key: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct DocumentDetail {
    pub id: i64,
    pub pub_key: String,
    pub title: Option<String>,
    pub abstract_text: Option<String>,
    pub applicants: Vec<String>,
    pub publication_date: Option<String>,
    pub cpc: Vec<String>,
    pub ipc: Vec<String>,
    pub kind_codes: Vec<String>,
    pub application_number: Option<String>,
    pub family_id: Option<String>,
}

/// Documents ready for review (SPEC section 8): fetched (title+abstract,
/// and therefore an embedding) and not yet validated/skipped, in import
/// order (M4's only ordering option - see docs/DECISIONS.md).
pub fn list_queue(conn: &Connection) -> Result<Vec<QueueEntry>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, pub_key, title FROM documents
         WHERE fetch_status = 'fetched' AND review_state = 'queued'
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(QueueEntry {
                id: row.get(0)?,
                pub_key: row.get(1)?,
                title: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_full(conn: &Connection, doc_id: i64) -> Result<Option<DocumentDetail>, StorageError> {
    conn.query_row(
        "SELECT id, pub_key, title, \"abstract\", applicants, publication_date, cpc, ipc,
                kind_codes, application_number, family_id
         FROM documents WHERE id = ?1",
        params![doc_id],
        |row| {
            let json_array = |s: Option<String>| -> Vec<String> {
                s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
            };
            Ok(DocumentDetail {
                id: row.get(0)?,
                pub_key: row.get(1)?,
                title: row.get(2)?,
                abstract_text: row.get(3)?,
                applicants: json_array(row.get(4)?),
                publication_date: row.get(5)?,
                cpc: json_array(row.get(6)?),
                ipc: json_array(row.get(7)?),
                kind_codes: json_array(row.get(8)?),
                application_number: row.get(9)?,
                family_id: row.get(10)?,
            })
        },
    )
    .optional()
    .map_err(StorageError::from)
}

/// Inserts a new document in `fetch_status = "pending"`, `review_state =
/// "queued"`. Returns `Ok(None)` instead of erroring when `pub_key` already
/// exists, since the import flow reports duplicates rather than failing
/// (SPEC section 5.1).
pub fn insert_pending(
    conn: &Connection,
    pub_key: &str,
    input_raw: &str,
    imported_at: &str,
) -> Result<Option<i64>, StorageError> {
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO documents (pub_key, input_raw, fetch_status, review_state, imported_at)
         VALUES (?1, ?2, 'pending', 'queued', ?3)",
        params![pub_key, input_raw, imported_at],
    )?;
    Ok(if inserted == 0 {
        None
    } else {
        Some(conn.last_insert_rowid())
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FetchedData {
    pub title: Option<String>,
    pub abstract_text: Option<String>,
    pub abstract_source: Option<String>,
    pub applicants: Vec<String>,
    pub publication_date: Option<String>,
    pub cpc: Vec<String>,
    pub ipc: Vec<String>,
    pub application_number: Option<String>,
    pub family_id: Option<String>,
    pub kind_codes: Vec<String>,
}

/// Persists OPS data fetched for a document (SPEC section 5.3), setting
/// `fetch_status` to `"fetched"` when an English abstract was found, or
/// `"no_english_abstract"` when `data.abstract_text` is `None`.
pub fn store_fetched(conn: &Connection, doc_id: i64, data: &FetchedData) -> Result<(), StorageError> {
    let fetch_status = if data.abstract_text.is_some() {
        "fetched"
    } else {
        "no_english_abstract"
    };
    conn.execute(
        "UPDATE documents SET
            title = ?1, \"abstract\" = ?2, abstract_source = ?3, applicants = ?4,
            publication_date = ?5, cpc = ?6, ipc = ?7, application_number = ?8,
            family_id = ?9, kind_codes = ?10, fetch_status = ?11, fetch_error = NULL
         WHERE id = ?12",
        params![
            data.title,
            data.abstract_text,
            data.abstract_source,
            serde_json::to_string(&data.applicants).unwrap_or_default(),
            data.publication_date,
            serde_json::to_string(&data.cpc).unwrap_or_default(),
            serde_json::to_string(&data.ipc).unwrap_or_default(),
            data.application_number,
            data.family_id,
            serde_json::to_string(&data.kind_codes).unwrap_or_default(),
            fetch_status,
            doc_id,
        ],
    )?;
    Ok(())
}

pub fn mark_fetch_error(conn: &Connection, doc_id: i64, error: &str) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE documents SET fetch_status = 'error', fetch_error = ?1 WHERE id = ?2",
        params![error, doc_id],
    )?;
    Ok(())
}

/// SPEC section 4.2's `fetch_status` distinguishes `not_found` (OPS has no
/// such publication) from `error` (something went wrong trying to find
/// out), so the Import report can show a "not found" section rather than
/// lumping every failure under "errors".
pub fn mark_not_found(conn: &Connection, doc_id: i64) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE documents SET fetch_status = 'not_found', fetch_error = NULL WHERE id = ?1",
        params![doc_id],
    )?;
    Ok(())
}

pub fn find_by_pub_key(
    conn: &Connection,
    pub_key: &str,
) -> Result<Option<DocumentRow>, StorageError> {
    conn.query_row(
        "SELECT id, pub_key, input_raw, application_number, family_id, fetch_status, review_state
         FROM documents WHERE pub_key = ?1",
        params![pub_key],
        row_to_document,
    )
    .optional()
    .map_err(StorageError::from)
}

/// Other documents sharing `application_number` or `family_id` with (and
/// not equal to) `doc_id` — the "related documents" case in SPEC section
/// 5.2, which the import report flags for suggested (not validated) labels.
pub fn find_related(conn: &Connection, doc_id: i64) -> Result<Vec<DocumentRow>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT d.id, d.pub_key, d.input_raw, d.application_number, d.family_id,
                d.fetch_status, d.review_state
         FROM documents d, documents self
         WHERE self.id = ?1 AND d.id != self.id
           AND ((self.application_number IS NOT NULL AND d.application_number = self.application_number)
             OR (self.family_id IS NOT NULL AND d.family_id = self.family_id))",
    )?;
    let rows = stmt
        .query_map(params![doc_id], row_to_document)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_document(row: &rusqlite::Row) -> rusqlite::Result<DocumentRow> {
    Ok(DocumentRow {
        id: row.get(0)?,
        pub_key: row.get(1)?,
        input_raw: row.get(2)?,
        application_number: row.get(3)?,
        family_id: row.get(4)?,
        fetch_status: row.get(5)?,
        review_state: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    #[test]
    fn queue_only_includes_fetched_and_queued_documents_in_import_order() {
        let conn = storage::open_in_memory().expect("in-memory db");
        insert_pending(&conn, "EP1111111", "EP1111111", "2026-01-01T00:00:00Z").unwrap();
        insert_pending(&conn, "EP2222222", "EP2222222", "2026-01-01T00:00:00Z").unwrap();
        insert_pending(&conn, "EP3333333", "EP3333333", "2026-01-01T00:00:00Z").unwrap();
        let fetched_and_queued = find_by_pub_key(&conn, "EP1111111").unwrap().unwrap();
        let fetched_but_validated = find_by_pub_key(&conn, "EP2222222").unwrap().unwrap();
        let queued_but_not_fetched = find_by_pub_key(&conn, "EP3333333").unwrap().unwrap();

        store_fetched(
            &conn,
            fetched_and_queued.id,
            &FetchedData {
                title: Some("A".to_string()),
                abstract_text: Some("Abstract A".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        store_fetched(
            &conn,
            fetched_but_validated.id,
            &FetchedData {
                title: Some("B".to_string()),
                abstract_text: Some("Abstract B".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE documents SET review_state = 'validated' WHERE id = ?1",
            params![fetched_but_validated.id],
        )
        .unwrap();
        let _ = queued_but_not_fetched;

        let queue = list_queue(&conn).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].pub_key, "EP1111111");
        assert_eq!(queue[0].title.as_deref(), Some("A"));
    }

    #[test]
    fn get_full_parses_json_array_columns() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z")
            .unwrap()
            .unwrap();
        store_fetched(
            &conn,
            doc_id,
            &FetchedData {
                title: Some("A gadget".to_string()),
                abstract_text: Some("An abstract".to_string()),
                applicants: vec!["ACME".to_string(), "Foo Corp".to_string()],
                cpc: vec!["B28B1/29".to_string()],
                ipc: vec!["B28B1/29".to_string()],
                kind_codes: vec!["A1".to_string(), "B1".to_string()],
                ..Default::default()
            },
        )
        .unwrap();

        let detail = get_full(&conn, doc_id).unwrap().unwrap();
        assert_eq!(detail.title.as_deref(), Some("A gadget"));
        assert_eq!(detail.applicants, vec!["ACME", "Foo Corp"]);
        assert_eq!(detail.kind_codes, vec!["A1", "B1"]);
    }

    #[test]
    fn inserting_the_same_pub_key_twice_reports_a_duplicate() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let first = insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z")
            .expect("insert should succeed");
        assert!(first.is_some());

        let second = insert_pending(&conn, "EP1234567", "EP 1 234 567", "2026-01-01T00:00:01Z")
            .expect("insert should succeed");
        assert_eq!(second, None, "a duplicate pub_key should be reported, not inserted");
    }

    #[test]
    fn store_fetched_sets_status_fetched_when_abstract_present() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z")
            .unwrap()
            .unwrap();

        store_fetched(
            &conn,
            doc_id,
            &FetchedData {
                title: Some("A gadget".to_string()),
                abstract_text: Some("An abstract".to_string()),
                abstract_source: Some("EP.1234567.A1".to_string()),
                applicants: vec!["ACME".to_string()],
                publication_date: Some("2020-01-01".to_string()),
                cpc: vec!["B28B1/29".to_string()],
                ipc: vec!["B28B1/29".to_string()],
                application_number: Some("EP20200000001".to_string()),
                family_id: Some("12345".to_string()),
                kind_codes: vec!["A1".to_string()],
            },
        )
        .unwrap();

        let doc = find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        assert_eq!(doc.fetch_status, "fetched");
        assert_eq!(doc.application_number.as_deref(), Some("EP20200000001"));
        assert_eq!(doc.family_id.as_deref(), Some("12345"));
    }

    #[test]
    fn store_fetched_sets_status_no_english_abstract_when_missing() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z")
            .unwrap()
            .unwrap();

        store_fetched(&conn, doc_id, &FetchedData::default()).unwrap();

        let doc = find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        assert_eq!(doc.fetch_status, "no_english_abstract");
    }

    #[test]
    fn mark_fetch_error_records_the_error_and_status() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z")
            .unwrap()
            .unwrap();

        mark_fetch_error(&conn, doc_id, "network timeout").unwrap();

        let doc = find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        assert_eq!(doc.fetch_status, "error");
    }

    #[test]
    fn mark_not_found_is_distinct_from_a_generic_error() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = insert_pending(&conn, "EP9999999", "EP9999999", "2026-01-01T00:00:00Z")
            .unwrap()
            .unwrap();

        mark_not_found(&conn, doc_id).unwrap();

        let doc = find_by_pub_key(&conn, "EP9999999").unwrap().unwrap();
        assert_eq!(doc.fetch_status, "not_found");
    }

    #[test]
    fn find_by_pub_key_returns_none_when_absent() {
        let conn = storage::open_in_memory().expect("in-memory db");
        assert_eq!(find_by_pub_key(&conn, "EP1234567").unwrap(), None);
    }

    #[test]
    fn related_documents_share_application_number_or_family_id() {
        let conn = storage::open_in_memory().expect("in-memory db");
        insert_pending(&conn, "EP1234567", "EP1234567", "2026-01-01T00:00:00Z").unwrap();
        insert_pending(&conn, "EP7654321", "EP7654321", "2026-01-01T00:00:00Z").unwrap();
        insert_pending(&conn, "US1111111", "US1111111", "2026-01-01T00:00:00Z").unwrap();
        conn.execute(
            "UPDATE documents SET application_number = 'EP2020000001' WHERE pub_key IN ('EP1234567', 'EP7654321')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE documents SET family_id = 'FAM1' WHERE pub_key = 'US1111111'",
            [],
        )
        .unwrap();

        let doc = find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        let related = find_related(&conn, doc.id).unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].pub_key, "EP7654321");
    }
}
