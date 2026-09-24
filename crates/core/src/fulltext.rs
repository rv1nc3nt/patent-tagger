//! Persistence for the `fulltext` table (SPEC 4.2/5.5). Selection of
//! *which* publication and language to store is `crate::selection`'s job
//! (pure, network-free); this module only reads/writes the chosen result.

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_FETCHED: &str = "fetched";
pub const STATUS_NOT_AVAILABLE: &str = "not_available";
pub const STATUS_NON_ENGLISH_ONLY: &str = "non_english_only";
pub const STATUS_ERROR: &str = "error";

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct FulltextRow {
    pub doc_id: i64,
    pub description: Option<String>,
    pub claims: Option<String>,
    pub lang: Option<String>,
    pub source: Option<String>,
    pub status: String,
    pub fetched_at: Option<String>,
}

pub fn get(conn: &Connection, doc_id: i64) -> Result<Option<FulltextRow>, StorageError> {
    conn.query_row(
        "SELECT doc_id, description, claims, lang, source, status, fetched_at
         FROM fulltext WHERE doc_id = ?1",
        params![doc_id],
        row_to_fulltext,
    )
    .optional()
    .map_err(StorageError::from)
}

/// One of `description`/`claims` may be `None` even on a `fetched` row -
/// the selected publication might only carry one of the two parts in the
/// chosen language (SPEC 5.5's own EP B1 example: description absent,
/// trilingual claims present).
#[allow(clippy::too_many_arguments)]
pub fn store_fetched(
    conn: &Connection,
    doc_id: i64,
    description: Option<&str>,
    claims: Option<&str>,
    lang: &str,
    source: &str,
    non_english_only: bool,
    fetched_at: &str,
) -> Result<(), StorageError> {
    let status = if non_english_only {
        STATUS_NON_ENGLISH_ONLY
    } else {
        STATUS_FETCHED
    };
    upsert(
        conn,
        doc_id,
        description,
        claims,
        Some(lang),
        Some(source),
        status,
        Some(fetched_at),
    )
}

pub fn store_not_available(
    conn: &Connection,
    doc_id: i64,
    fetched_at: &str,
) -> Result<(), StorageError> {
    upsert(
        conn,
        doc_id,
        None,
        None,
        None,
        None,
        STATUS_NOT_AVAILABLE,
        Some(fetched_at),
    )
}

pub fn store_error(conn: &Connection, doc_id: i64, fetched_at: &str) -> Result<(), StorageError> {
    upsert(
        conn,
        doc_id,
        None,
        None,
        None,
        None,
        STATUS_ERROR,
        Some(fetched_at),
    )
}

#[allow(clippy::too_many_arguments)]
fn upsert(
    conn: &Connection,
    doc_id: i64,
    description: Option<&str>,
    claims: Option<&str>,
    lang: Option<&str>,
    source: Option<&str>,
    status: &str,
    fetched_at: Option<&str>,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO fulltext (doc_id, description, claims, lang, source, status, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (doc_id) DO UPDATE SET
            description = excluded.description, claims = excluded.claims,
            lang = excluded.lang, source = excluded.source,
            status = excluded.status, fetched_at = excluded.fetched_at",
        params![
            doc_id,
            description,
            claims,
            lang,
            source,
            status,
            fetched_at
        ],
    )?;
    Ok(())
}

fn row_to_fulltext(row: &rusqlite::Row) -> rusqlite::Result<FulltextRow> {
    Ok(FulltextRow {
        doc_id: row.get(0)?,
        description: row.get(1)?,
        claims: row.get(2)?,
        lang: row.get(3)?,
        source: row.get(4)?,
        status: row.get(5)?,
        fetched_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, storage};

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn new_doc(conn: &Connection) -> i64 {
        documents::insert_pending(conn, "EP1234567", "EP1234567", NOW)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn absent_row_is_none() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        assert_eq!(get(&conn, doc_id).unwrap(), None);
    }

    #[test]
    fn store_fetched_round_trips_with_one_part_missing() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_fetched(
            &conn,
            doc_id,
            None,
            Some("1. A claim."),
            "EN",
            "EP.1234567.B1",
            false,
            NOW,
        )
        .unwrap();

        let row = get(&conn, doc_id).unwrap().unwrap();
        assert_eq!(row.status, STATUS_FETCHED);
        assert_eq!(row.description, None);
        assert_eq!(row.claims.as_deref(), Some("1. A claim."));
        assert_eq!(row.lang.as_deref(), Some("EN"));
        assert_eq!(row.source.as_deref(), Some("EP.1234567.B1"));
    }

    #[test]
    fn store_fetched_non_english_sets_that_status() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_fetched(
            &conn,
            doc_id,
            Some("[0001] Nur Deutsch."),
            None,
            "DE",
            "EP.1234567.A1",
            true,
            NOW,
        )
        .unwrap();
        assert_eq!(
            get(&conn, doc_id).unwrap().unwrap().status,
            STATUS_NON_ENGLISH_ONLY
        );
    }

    #[test]
    fn store_not_available_clears_any_previous_text() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_fetched(
            &conn,
            doc_id,
            Some("text"),
            None,
            "EN",
            "EP.1234567.A1",
            false,
            NOW,
        )
        .unwrap();
        store_not_available(&conn, doc_id, NOW).unwrap();

        let row = get(&conn, doc_id).unwrap().unwrap();
        assert_eq!(row.status, STATUS_NOT_AVAILABLE);
        assert_eq!(row.description, None);
    }

    #[test]
    fn upsert_updates_rather_than_erroring_on_a_second_write() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_error(&conn, doc_id, NOW).unwrap();
        store_fetched(
            &conn,
            doc_id,
            Some("text"),
            Some("claims"),
            "EN",
            "EP.1234567.A1",
            false,
            NOW,
        )
        .unwrap();
        assert_eq!(get(&conn, doc_id).unwrap().unwrap().status, STATUS_FETCHED);
    }

    #[test]
    fn fulltext_is_searchable_via_fts() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_fetched(
            &conn,
            doc_id,
            Some("[0001] a very particular widget arrangement"),
            Some("1. A widget."),
            "EN",
            "EP.1234567.A1",
            false,
            NOW,
        )
        .unwrap();

        let found: i64 = conn
            .query_row(
                "SELECT rowid FROM fulltext_fts WHERE fulltext_fts MATCH 'widget'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, doc_id);
    }
}
