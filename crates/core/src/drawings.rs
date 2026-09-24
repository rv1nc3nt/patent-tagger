//! Persistence for the `drawings` and `drawings_status` tables (SPEC
//! 4.2/5.5). Image files themselves live under `drawings/<pub_key>/` in the
//! data directory (writing them is `src-tauri`'s job, per SPEC 4.2); this
//! module only stores their relative paths and metadata.

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_FETCHED: &str = "fetched";
pub const STATUS_NOT_AVAILABLE: &str = "not_available";
pub const STATUS_ERROR: &str = "error";

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DrawingPage {
    pub doc_id: i64,
    pub page: i64,
    pub source: Option<String>,
    /// Relative to the data directory, e.g. `drawings/EP1234567/001.png`.
    pub path: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fetched_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct DrawingsStatusRow {
    pub doc_id: i64,
    pub status: String,
    pub page_count: Option<i64>,
    pub source: Option<String>,
    pub updated_at: Option<String>,
}

pub fn get_status(
    conn: &Connection,
    doc_id: i64,
) -> Result<Option<DrawingsStatusRow>, StorageError> {
    conn.query_row(
        "SELECT doc_id, status, page_count, source, updated_at FROM drawings_status WHERE doc_id = ?1",
        params![doc_id],
        row_to_status,
    )
    .optional()
    .map_err(StorageError::from)
}

pub fn list_pages(conn: &Connection, doc_id: i64) -> Result<Vec<DrawingPage>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT doc_id, page, source, path, width, height, fetched_at
         FROM drawings WHERE doc_id = ?1 ORDER BY page ASC",
    )?;
    let rows = stmt
        .query_map(params![doc_id], row_to_page)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A page to insert with [`insert_page`]; the borrowed counterpart of
/// [`DrawingPage`].
#[derive(Debug, Clone, Copy)]
pub struct NewDrawingPage<'a> {
    pub doc_id: i64,
    pub page: i64,
    pub source: &'a str,
    /// Relative to the data directory, e.g. `drawings/EP1234567/001.png`.
    pub path: &'a str,
    pub width: i64,
    pub height: i64,
    pub fetched_at: &'a str,
}

pub fn insert_page(conn: &Connection, new: &NewDrawingPage) -> Result<(), StorageError> {
    let NewDrawingPage {
        doc_id,
        page,
        source,
        path,
        width,
        height,
        fetched_at,
    } = *new;
    conn.execute(
        "INSERT INTO drawings (doc_id, page, source, path, width, height, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (doc_id, page) DO UPDATE SET
            source = excluded.source, path = excluded.path,
            width = excluded.width, height = excluded.height, fetched_at = excluded.fetched_at",
        params![doc_id, page, source, path, width, height, fetched_at],
    )?;
    Ok(())
}

pub fn store_fetched_status(
    conn: &Connection,
    doc_id: i64,
    page_count: i64,
    source: &str,
    updated_at: &str,
) -> Result<(), StorageError> {
    upsert_status(
        conn,
        doc_id,
        STATUS_FETCHED,
        Some(page_count),
        Some(source),
        updated_at,
    )
}

pub fn store_not_available(
    conn: &Connection,
    doc_id: i64,
    updated_at: &str,
) -> Result<(), StorageError> {
    upsert_status(conn, doc_id, STATUS_NOT_AVAILABLE, None, None, updated_at)
}

pub fn store_error(conn: &Connection, doc_id: i64, updated_at: &str) -> Result<(), StorageError> {
    upsert_status(conn, doc_id, STATUS_ERROR, None, None, updated_at)
}

fn upsert_status(
    conn: &Connection,
    doc_id: i64,
    status: &str,
    page_count: Option<i64>,
    source: Option<&str>,
    updated_at: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO drawings_status (doc_id, status, page_count, source, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (doc_id) DO UPDATE SET
            status = excluded.status, page_count = excluded.page_count,
            source = excluded.source, updated_at = excluded.updated_at",
        params![doc_id, status, page_count, source, updated_at],
    )?;
    Ok(())
}

fn row_to_status(row: &rusqlite::Row) -> rusqlite::Result<DrawingsStatusRow> {
    Ok(DrawingsStatusRow {
        doc_id: row.get(0)?,
        status: row.get(1)?,
        page_count: row.get(2)?,
        source: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn row_to_page(row: &rusqlite::Row) -> rusqlite::Result<DrawingPage> {
    Ok(DrawingPage {
        doc_id: row.get(0)?,
        page: row.get(1)?,
        source: row.get(2)?,
        path: row.get(3)?,
        width: row.get(4)?,
        height: row.get(5)?,
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
    fn absent_status_is_none() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        assert_eq!(get_status(&conn, doc_id).unwrap(), None);
        assert!(list_pages(&conn, doc_id).unwrap().is_empty());
    }

    #[test]
    fn fetched_status_and_pages_round_trip_in_page_order() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_fetched_status(&conn, doc_id, 2, "EP.1234567.A1", NOW).unwrap();
        insert_page(
            &conn,
            &NewDrawingPage {
                doc_id,
                page: 2,
                source: "EP.1234567.A1",
                path: "drawings/EP1234567/002.png",
                width: 3508,
                height: 2479,
                fetched_at: NOW,
            },
        )
        .unwrap();
        insert_page(
            &conn,
            &NewDrawingPage {
                doc_id,
                page: 1,
                source: "EP.1234567.A1",
                path: "drawings/EP1234567/001.png",
                width: 3508,
                height: 2479,
                fetched_at: NOW,
            },
        )
        .unwrap();

        let status = get_status(&conn, doc_id).unwrap().unwrap();
        assert_eq!(status.status, STATUS_FETCHED);
        assert_eq!(status.page_count, Some(2));

        let pages = list_pages(&conn, doc_id).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].page, 1,
            "pages should come back in page order regardless of insert order"
        );
        assert_eq!(pages[1].page, 2);
    }

    #[test]
    fn not_available_is_normal_and_has_no_page_count() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_not_available(&conn, doc_id, NOW).unwrap();
        let status = get_status(&conn, doc_id).unwrap().unwrap();
        assert_eq!(status.status, STATUS_NOT_AVAILABLE);
        assert_eq!(status.page_count, None);
    }

    #[test]
    fn upsert_status_updates_rather_than_erroring() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        store_error(&conn, doc_id, NOW).unwrap();
        store_fetched_status(&conn, doc_id, 1, "EP.1234567.A1", NOW).unwrap();
        assert_eq!(
            get_status(&conn, doc_id).unwrap().unwrap().status,
            STATUS_FETCHED
        );
    }

    #[test]
    fn insert_page_upserts_on_the_same_page_number() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let doc_id = new_doc(&conn);
        insert_page(
            &conn,
            &NewDrawingPage {
                doc_id,
                page: 1,
                source: "EP.1234567.A1",
                path: "drawings/EP1234567/001.png",
                width: 100,
                height: 100,
                fetched_at: NOW,
            },
        )
        .unwrap();
        insert_page(
            &conn,
            &NewDrawingPage {
                doc_id,
                page: 1,
                source: "EP.1234567.A1",
                path: "drawings/EP1234567/001.png",
                width: 3508,
                height: 2479,
                fetched_at: NOW,
            },
        )
        .unwrap();

        let pages = list_pages(&conn, doc_id).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].width, Some(3508));
    }
}
