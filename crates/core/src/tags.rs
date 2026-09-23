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
    /// SPEC 7.5: the calibrated pre-check/auto-decision threshold, once
    /// calibration has found one; `None` falls back to 0.5 everywhere it's
    /// used.
    pub threshold: Option<f32>,
    /// SPEC 7.6: the calibrated "confident absence" threshold - a score
    /// below this means confidently absent. `None` until calibrated;
    /// full automation can never decide this tag's negative side until it
    /// exists (see `full_automation::decide_tag`).
    pub neg_threshold: Option<f32>,
    /// SPEC 7.5: "the user enables automatic mode per tag explicitly; it is
    /// never enabled by default."
    pub auto_enabled: bool,
}

/// SPEC section 8: "Tag-schema export and import (JSON)". Just the
/// definitional part of a tag - not learned state (threshold, auto mode),
/// which is model- and history-specific and wouldn't mean anything
/// transplanted into a different installation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TagSchema {
    pub name: String,
    pub definition: String,
    pub color: Option<String>,
    pub hotkey: Option<String>,
}

pub fn export_schema(conn: &Connection) -> Result<Vec<TagSchema>, StorageError> {
    let mut stmt = conn.prepare("SELECT name, definition, color, hotkey FROM tags WHERE archived = 0 ORDER BY name ASC")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TagSchema {
                name: row.get(0)?,
                definition: row.get(1)?,
                color: row.get(2)?,
                hotkey: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Whether a tag with this name already exists (import is add-only - see
/// `src-tauri::commands::import_tag_schema` - so existing tags and
/// whatever they've already learned are never touched).
pub fn exists_by_name(conn: &Connection, name: &str) -> Result<bool, StorageError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE name = ?1)",
        params![name],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n != 0)
    .map_err(StorageError::from)
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

const SELECT_COLUMNS: &str =
    "id, name, definition, color, hotkey, version, archived, threshold, neg_threshold, auto_enabled";

pub fn list_active(conn: &Connection) -> Result<Vec<TagRow>, StorageError> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM tags WHERE archived = 0 ORDER BY name ASC"))?;
    let rows = stmt.query_map([], row_to_tag)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<TagRow>, StorageError> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM tags WHERE id = ?1"),
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

pub fn set_threshold(conn: &Connection, tag_id: i64, threshold: Option<f32>) -> Result<(), StorageError> {
    conn.execute("UPDATE tags SET threshold = ?1 WHERE id = ?2", params![threshold, tag_id])?;
    Ok(())
}

pub fn set_neg_threshold(conn: &Connection, tag_id: i64, neg_threshold: Option<f32>) -> Result<(), StorageError> {
    conn.execute("UPDATE tags SET neg_threshold = ?1 WHERE id = ?2", params![neg_threshold, tag_id])?;
    Ok(())
}

/// Unconditional - callers (the `enable_automatic_mode` command) are
/// responsible for checking eligibility first (SPEC 7.5: "Automatic mode
/// unavailable before eligibility").
pub fn set_auto_enabled(conn: &Connection, tag_id: i64, enabled: bool) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE tags SET auto_enabled = ?1 WHERE id = ?2",
        params![enabled as i64, tag_id],
    )?;
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
        threshold: row.get(7)?,
        neg_threshold: row.get(8)?,
        auto_enabled: row.get::<_, i64>(9)? != 0,
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

    #[test]
    fn new_tags_have_no_threshold_and_are_not_auto_enabled() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = create(&conn, "Battery", "Relates to batteries", None, None, NOW).unwrap();
        let tag = get(&conn, id).unwrap().unwrap();
        assert_eq!(tag.threshold, None);
        assert_eq!(tag.neg_threshold, None);
        assert!(!tag.auto_enabled);
    }

    #[test]
    fn set_threshold_and_set_auto_enabled_round_trip() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = create(&conn, "Battery", "Relates to batteries", None, None, NOW).unwrap();

        set_threshold(&conn, id, Some(0.73)).unwrap();
        set_neg_threshold(&conn, id, Some(0.12)).unwrap();
        set_auto_enabled(&conn, id, true).unwrap();

        let tag = get(&conn, id).unwrap().unwrap();
        assert_eq!(tag.threshold, Some(0.73));
        assert_eq!(tag.neg_threshold, Some(0.12));
        assert!(tag.auto_enabled);
    }

    #[test]
    fn export_schema_excludes_archived_tags_and_learned_state() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let battery_id = create(&conn, "Battery", "Relates to batteries", Some("#f00"), Some("b"), NOW).unwrap();
        set_threshold(&conn, battery_id, Some(0.8)).unwrap();
        let archived_id = create(&conn, "Old", "No longer used", None, None, NOW).unwrap();
        archive(&conn, archived_id).unwrap();

        let schema = export_schema(&conn).unwrap();
        assert_eq!(
            schema,
            vec![TagSchema {
                name: "Battery".to_string(),
                definition: "Relates to batteries".to_string(),
                color: Some("#f00".to_string()),
                hotkey: Some("b".to_string()),
            }]
        );
    }

    #[test]
    fn exists_by_name_reflects_current_tags() {
        let conn = storage::open_in_memory().expect("in-memory db");
        assert!(!exists_by_name(&conn, "Battery").unwrap());
        create(&conn, "Battery", "Relates to batteries", None, None, NOW).unwrap();
        assert!(exists_by_name(&conn, "Battery").unwrap());
    }
}
