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
