//! Persistence for the `tags` table (SPEC section 4.2). Minimal create/
//! list/archive for M4; the full Tags screen (statistics, per-tag review
//! queue for newly created tags, automatic-mode toggle) is later.

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub definition: String,
    pub color: Option<String>,
    pub hotkey: Option<String>,
    pub version: i64,
    pub archived: bool,
}

pub fn create(
    conn: &Connection,
    name: &str,
    definition: &str,
    color: Option<&str>,
    hotkey: Option<&str>,
    created_at: &str,
) -> Result<i64, StorageError> {
    conn.execute(
        "INSERT INTO tags (name, definition, color, hotkey, version, archived, created_at)
         VALUES (?1, ?2, ?3, ?4, 1, 0, ?5)",
        params![name, definition, color, hotkey, created_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_active(conn: &Connection) -> Result<Vec<TagRow>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, definition, color, hotkey, version, archived
         FROM tags WHERE archived = 0 ORDER BY name ASC",
    )?;
    let rows = stmt.query_map([], row_to_tag)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<TagRow>, StorageError> {
    conn.query_row(
        "SELECT id, name, definition, color, hotkey, version, archived FROM tags WHERE id = ?1",
        params![id],
        row_to_tag,
    )
    .optional()
    .map_err(StorageError::from)
}

pub fn archive(conn: &Connection, id: i64) -> Result<(), StorageError> {
    conn.execute("UPDATE tags SET archived = 1 WHERE id = ?1", params![id])?;
    Ok(())
}

/// Positive human-labelled documents for `tag_id` (SPEC 7.2: zero-shot is
/// only used below 3 positives).
pub fn count_human_positives(conn: &Connection, tag_id: i64) -> Result<i64, StorageError> {
    conn.query_row(
        "SELECT count(*) FROM labels WHERE tag_id = ?1 AND state = 'pos' AND source = 'human'",
        params![tag_id],
        |row| row.get(0),
    )
    .map_err(StorageError::from)
}

fn row_to_tag(row: &rusqlite::Row) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: row.get(0)?,
        name: row.get(1)?,
        definition: row.get(2)?,
        color: row.get(3)?,
        hotkey: row.get(4)?,
        version: row.get(5)?,
        archived: row.get::<_, i64>(6)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    const NOW: &str = "2026-01-01T00:00:00Z";

    #[test]
    fn create_and_list_active() {
        let conn = storage::open_in_memory().expect("in-memory db");
        create(&conn, "Battery", "Relates to batteries", Some("#f00"), Some("b"), NOW).unwrap();
        let tags = list_active(&conn).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "Battery");
        assert_eq!(tags[0].version, 1);
        assert!(!tags[0].archived);
    }

    #[test]
    fn archived_tags_are_excluded_from_list_active() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = create(&conn, "Battery", "Relates to batteries", None, None, NOW).unwrap();
        archive(&conn, id).unwrap();
        assert!(list_active(&conn).unwrap().is_empty());
        assert!(get(&conn, id).unwrap().unwrap().archived);
    }

    #[test]
    fn count_human_positives_ignores_negatives_and_auto_labels() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = create(&conn, "Battery", "Relates to batteries", None, None, NOW).unwrap();
        conn.execute(
            "INSERT INTO documents (pub_key, input_raw, fetch_status, review_state, imported_at)
             VALUES ('EP1', 'EP1', 'fetched', 'validated', ?1)",
            params![NOW],
        )
        .unwrap();
        let doc_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO labels (doc_id, tag_id, state, source, tag_version, created_at)
             VALUES (?1, ?2, 'pos', 'human', 1, ?3)",
            params![doc_id, tag_id, NOW],
        )
        .unwrap();

        assert_eq!(count_human_positives(&conn, tag_id).unwrap(), 1);
    }
}
