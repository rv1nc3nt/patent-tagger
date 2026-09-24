//! Persistence for the `tags` table (SPEC section 4.2) and the Tags screen
//! (SPEC section 8): create, edit, archive/restore, parent, statistics.
//! `parent_id` is organisational only: it shapes display and export, never
//! scoring or learning (see docs/DECISIONS.md).

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub definition: String,
    pub parent_id: Option<i64>,
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

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("tag {0} not found")]
    NotFound(i64),
    #[error("tag name must not be empty")]
    EmptyName,
    #[error("tag definition must not be empty")]
    EmptyDefinition,
    #[error("a tag named \"{0}\" already exists (possibly archived)")]
    DuplicateName(String),
    #[error("hotkey must be a single character")]
    InvalidHotkey,
    #[error("hotkey \"{0}\" is reserved by the Review screen")]
    ReservedHotkey(String),
    #[error("hotkey \"{0}\" is already used by tag \"{1}\"")]
    HotkeyInUse(String, String),
    #[error("parent must be an active tag, and not the tag itself or one of its descendants")]
    InvalidParent,
    #[error("archived tags cannot be edited; restore the tag first")]
    Archived,
}

impl From<rusqlite::Error> for TagError {
    fn from(e: rusqlite::Error) -> Self {
        TagError::Storage(StorageError::from(e))
    }
}

/// Keys the Review screen binds itself (SPEC section 8: `J`/`K`, `S`, `/`),
/// which a tag hotkey would otherwise shadow.
pub const RESERVED_HOTKEYS: [&str; 4] = ["j", "k", "s", "/"];

/// The user-editable part of a tag. Surrounding whitespace is trimmed, and
/// an empty colour or hotkey means none.
#[derive(Debug, Clone, Copy)]
pub struct TagFields<'a> {
    pub name: &'a str,
    pub definition: &'a str,
    pub parent_id: Option<i64>,
    pub color: Option<&'a str>,
    pub hotkey: Option<&'a str>,
}

impl<'a> TagFields<'a> {
    fn normalised(self) -> Self {
        let blank_to_none = |v: Option<&'a str>| v.map(str::trim).filter(|v| !v.is_empty());
        TagFields {
            name: self.name.trim(),
            definition: self.definition.trim(),
            parent_id: self.parent_id,
            color: blank_to_none(self.color),
            hotkey: blank_to_none(self.hotkey),
        }
    }
}

/// SPEC section 8: "Tag-schema export and import (JSON)". Just the
/// definitional part of a tag - not learned state (threshold, auto mode),
/// which is model- and history-specific and wouldn't mean anything
/// transplanted into a different installation. `parent` is the parent
/// tag's name, since ids don't carry over between installations.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TagSchema {
    pub name: String,
    pub definition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub color: Option<String>,
    pub hotkey: Option<String>,
}

pub fn export_schema(conn: &Connection) -> Result<Vec<TagSchema>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT t.name, t.definition, p.name, t.color, t.hotkey
         FROM tags t LEFT JOIN tags p ON p.id = t.parent_id AND p.archived = 0
         WHERE t.archived = 0 ORDER BY t.name ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TagSchema {
                name: row.get(0)?,
                definition: row.get(1)?,
                parent: row.get(2)?,
                color: row.get(3)?,
                hotkey: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Whether a tag with this name already exists (import is add-only - see
/// `src-tauri::settings::import_tag_schema` - so existing tags and
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

pub fn find_by_name(conn: &Connection, name: &str) -> Result<Option<TagRow>, StorageError> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM tags WHERE name = ?1"),
        params![name],
        row_to_tag,
    )
    .optional()
    .map_err(StorageError::from)
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SchemaImport {
    /// Ids of the tags created, in schema order.
    pub created: Vec<i64>,
    /// Names of created tags whose hotkey was dropped, because it was
    /// reserved, invalid or already used by an active tag.
    pub hotkeys_dropped: Vec<String>,
    /// Names of created tags left at the top level, because their parent
    /// was not found among active tags or would have made a cycle.
    pub parents_dropped: Vec<String>,
}

/// Add-only schema import (SPEC section 8): entries whose name already
/// exists are skipped, so an existing tag's learned state is never touched.
/// Parents are resolved by name once every new tag exists, so an entry may
/// name a parent that comes later in the file.
pub fn import_schema(
    conn: &Connection,
    schema: &[TagSchema],
    created_at: &str,
) -> Result<SchemaImport, TagError> {
    let mut outcome = SchemaImport::default();
    let mut with_parent = Vec::new();
    for entry in schema {
        if exists_by_name(conn, entry.name.trim())? {
            continue;
        }
        let fields = TagFields {
            name: &entry.name,
            definition: &entry.definition,
            parent_id: None,
            color: entry.color.as_deref(),
            hotkey: entry.hotkey.as_deref(),
        };
        let id = match insert(conn, fields, created_at) {
            Err(
                TagError::InvalidHotkey | TagError::ReservedHotkey(_) | TagError::HotkeyInUse(..),
            ) => {
                outcome.hotkeys_dropped.push(entry.name.trim().to_string());
                insert(
                    conn,
                    TagFields {
                        hotkey: None,
                        ..fields
                    },
                    created_at,
                )?
            }
            other => other?,
        };
        outcome.created.push(id);
        if let Some(parent) = &entry.parent {
            with_parent.push((id, parent.trim()));
        }
    }
    for (id, parent_name) in with_parent {
        let tag = get(conn, id)?.ok_or(TagError::NotFound(id))?;
        let parent = find_by_name(conn, parent_name)?.filter(|p| !p.archived);
        let linked = match parent {
            Some(parent) => {
                let fields = TagFields {
                    name: &tag.name,
                    definition: &tag.definition,
                    parent_id: Some(parent.id),
                    color: tag.color.as_deref(),
                    hotkey: tag.hotkey.as_deref(),
                };
                match update(conn, id, fields, false) {
                    Ok(_) => true,
                    Err(TagError::InvalidParent) => false,
                    Err(e) => return Err(e),
                }
            }
            None => false,
        };
        if !linked {
            outcome.parents_dropped.push(tag.name);
        }
    }
    Ok(outcome)
}

/// Creates a top-level tag. See `insert` for one with a parent.
pub fn create(
    conn: &Connection,
    name: &str,
    definition: &str,
    color: Option<&str>,
    hotkey: Option<&str>,
    created_at: &str,
) -> Result<i64, TagError> {
    insert(
        conn,
        TagFields {
            name,
            definition,
            parent_id: None,
            color,
            hotkey,
        },
        created_at,
    )
}

pub fn insert(conn: &Connection, fields: TagFields, created_at: &str) -> Result<i64, TagError> {
    let fields = fields.normalised();
    validate(conn, None, &fields)?;
    conn.execute(
        "INSERT INTO tags (name, definition, parent_id, color, hotkey, version, archived, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 0, ?6)",
        params![
            fields.name,
            fields.definition,
            fields.parent_id,
            fields.color,
            fields.hotkey,
            created_at
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    pub tag: TagRow,
    /// The name or definition changed, or the version was bumped: the
    /// `"{name}: {definition}"` zero-shot embedding (SPEC 7.2) must be
    /// recomputed for the tag's current version.
    pub needs_embedding: bool,
}

/// Edits a tag. `bump_version` marks a material definition change (SPEC
/// 7.1): labels written under the older version become stale, open to
/// optional re-review, and stay usable for training unless discarded.
pub fn update(
    conn: &Connection,
    id: i64,
    fields: TagFields,
    bump_version: bool,
) -> Result<Updated, TagError> {
    let current = get(conn, id)?.ok_or(TagError::NotFound(id))?;
    if current.archived {
        return Err(TagError::Archived);
    }
    let fields = fields.normalised();
    validate(conn, Some(id), &fields)?;
    conn.execute(
        "UPDATE tags SET name = ?1, definition = ?2, parent_id = ?3, color = ?4, hotkey = ?5,
                version = version + ?6
         WHERE id = ?7",
        params![
            fields.name,
            fields.definition,
            fields.parent_id,
            fields.color,
            fields.hotkey,
            bump_version as i64,
            id
        ],
    )?;
    let tag = get(conn, id)?.ok_or(TagError::NotFound(id))?;
    let needs_embedding =
        bump_version || tag.name != current.name || tag.definition != current.definition;
    Ok(Updated {
        tag,
        needs_embedding,
    })
}

fn validate(conn: &Connection, id: Option<i64>, fields: &TagFields) -> Result<(), TagError> {
    if fields.name.is_empty() {
        return Err(TagError::EmptyName);
    }
    if fields.definition.is_empty() {
        return Err(TagError::EmptyDefinition);
    }
    if let Some(other) = find_by_name(conn, fields.name)? {
        if Some(other.id) != id {
            return Err(TagError::DuplicateName(fields.name.to_string()));
        }
    }
    if let Some(hotkey) = fields.hotkey {
        if hotkey.chars().count() != 1 {
            return Err(TagError::InvalidHotkey);
        }
        let lower = hotkey.to_lowercase();
        if RESERVED_HOTKEYS.contains(&lower.as_str()) {
            return Err(TagError::ReservedHotkey(hotkey.to_string()));
        }
        if let Some(owner) = hotkey_owner(conn, &lower, id)? {
            return Err(TagError::HotkeyInUse(hotkey.to_string(), owner));
        }
    }
    if let Some(parent_id) = fields.parent_id {
        let parent = get(conn, parent_id)?.ok_or(TagError::InvalidParent)?;
        if parent.archived {
            return Err(TagError::InvalidParent);
        }
        // Walk up from the proposed parent: reaching the tag itself means a
        // cycle. The bound guards against a cycle already in the database.
        let mut ancestor = Some(parent);
        let mut steps = 0;
        while let Some(tag) = ancestor {
            if Some(tag.id) == id || steps > 1000 {
                return Err(TagError::InvalidParent);
            }
            steps += 1;
            ancestor = match tag.parent_id {
                Some(pid) => get(conn, pid)?,
                None => None,
            };
        }
    }
    Ok(())
}

/// The active tag (other than `except`) using `hotkey`, case-insensitively.
fn hotkey_owner(
    conn: &Connection,
    lower_hotkey: &str,
    except: Option<i64>,
) -> Result<Option<String>, StorageError> {
    conn.query_row(
        "SELECT name FROM tags
         WHERE archived = 0 AND lower(hotkey) = ?1 AND id IS NOT ?2",
        params![lower_hotkey, except],
        |row| row.get(0),
    )
    .optional()
    .map_err(StorageError::from)
}

const SELECT_COLUMNS: &str = "id, name, definition, parent_id, color, hotkey, version, archived, \
                              threshold, neg_threshold, auto_enabled";

pub fn list_active(conn: &Connection) -> Result<Vec<TagRow>, StorageError> {
    list_where(conn, 0)
}

pub fn list_archived(conn: &Connection) -> Result<Vec<TagRow>, StorageError> {
    list_where(conn, 1)
}

fn list_where(conn: &Connection, archived: i64) -> Result<Vec<TagRow>, StorageError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM tags WHERE archived = ?1 ORDER BY name ASC"
    ))?;
    let rows = stmt
        .query_map(params![archived], row_to_tag)?
        .collect::<Result<Vec<_>, _>>()?;
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

/// Archives a tag. Its children move to the top level, since a parent must
/// be an active tag.
pub fn archive(conn: &Connection, id: i64) -> Result<(), StorageError> {
    conn.execute("UPDATE tags SET archived = 1 WHERE id = ?1", params![id])?;
    conn.execute(
        "UPDATE tags SET parent_id = NULL WHERE parent_id = ?1",
        params![id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Restored {
    pub tag: TagRow,
    /// The hotkey the tag had, when another active tag took it while this
    /// one was archived. The restored tag then has no hotkey.
    pub hotkey_cleared: Option<String>,
}

/// Restores an archived tag. Documents validated while it was archived
/// have no label for it, so they appear in its per-tag review queue.
pub fn unarchive(conn: &Connection, id: i64) -> Result<Restored, TagError> {
    let tag = get(conn, id)?.ok_or(TagError::NotFound(id))?;
    let hotkey_cleared = match &tag.hotkey {
        Some(hotkey) if hotkey_owner(conn, &hotkey.to_lowercase(), Some(id))?.is_some() => {
            Some(hotkey.clone())
        }
        _ => None,
    };
    if hotkey_cleared.is_some() {
        conn.execute("UPDATE tags SET hotkey = NULL WHERE id = ?1", params![id])?;
    }
    conn.execute("UPDATE tags SET archived = 0 WHERE id = ?1", params![id])?;
    let tag = get(conn, id)?.ok_or(TagError::NotFound(id))?;
    Ok(Restored {
        tag,
        hotkey_cleared,
    })
}

pub fn set_threshold(
    conn: &Connection,
    tag_id: i64,
    threshold: Option<f32>,
) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE tags SET threshold = ?1 WHERE id = ?2",
        params![threshold, tag_id],
    )?;
    Ok(())
}

pub fn set_neg_threshold(
    conn: &Connection,
    tag_id: i64,
    neg_threshold: Option<f32>,
) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE tags SET neg_threshold = ?1 WHERE id = ?2",
        params![neg_threshold, tag_id],
    )?;
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

/// SPEC section 8: "Statistics per tag". Precision and recall live on the
/// Metrics screen; these are label counts.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct TagStats {
    pub tag_id: i64,
    pub human_pos: i64,
    pub human_neg: i64,
    pub auto_pos: i64,
    pub auto_neg: i64,
    /// Reviewed documents (validated or auto-completed) with no label for
    /// this tag, e.g. because the tag is newer than their review (SPEC 7.1).
    pub unlabelled: i64,
    /// Human labels written under an older tag version (SPEC 7.1).
    pub stale: i64,
}

pub fn stats(conn: &Connection, tag_id: i64) -> Result<TagStats, TagError> {
    let tag = get(conn, tag_id)?.ok_or(TagError::NotFound(tag_id))?;
    let (human_pos, human_neg, auto_pos, auto_neg, stale) = conn.query_row(
        "SELECT
            coalesce(sum(source = 'human' AND state = 'pos'), 0),
            coalesce(sum(source = 'human' AND state = 'neg'), 0),
            coalesce(sum(source = 'auto' AND state = 'pos'), 0),
            coalesce(sum(source = 'auto' AND state = 'neg'), 0),
            coalesce(sum(source = 'human' AND tag_version < ?2), 0)
         FROM labels WHERE tag_id = ?1",
        params![tag_id, tag.version],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let unlabelled = conn.query_row(
        &format!(
            "SELECT count(*) FROM documents d
             WHERE d.review_state IN {REVIEWED_STATES}
               AND NOT EXISTS (SELECT 1 FROM labels l WHERE l.doc_id = d.id AND l.tag_id = ?1)"
        ),
        params![tag_id],
        |row| row.get(0),
    )?;
    Ok(TagStats {
        tag_id,
        human_pos,
        human_neg,
        auto_pos,
        auto_neg,
        unlabelled,
        stale,
    })
}

/// Review states of documents whose tag set has already been decided,
/// either by a human or by full automation (SPEC 7.6).
pub const REVIEWED_STATES: &str = "('validated', 'auto_completed')";

fn row_to_tag(row: &rusqlite::Row) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: row.get(0)?,
        name: row.get(1)?,
        definition: row.get(2)?,
        parent_id: row.get(3)?,
        color: row.get(4)?,
        hotkey: row.get(5)?,
        version: row.get(6)?,
        archived: row.get::<_, i64>(7)? != 0,
        threshold: row.get(8)?,
        neg_threshold: row.get(9)?,
        auto_enabled: row.get::<_, i64>(10)? != 0,
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
        create(
            &conn,
            "Battery",
            "Relates to batteries",
            Some("#f00"),
            Some("b"),
            NOW,
        )
        .unwrap();
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
        let battery_id = create(
            &conn,
            "Battery",
            "Relates to batteries",
            Some("#f00"),
            Some("b"),
            NOW,
        )
        .unwrap();
        set_threshold(&conn, battery_id, Some(0.8)).unwrap();
        let archived_id = create(&conn, "Old", "No longer used", None, None, NOW).unwrap();
        archive(&conn, archived_id).unwrap();

        let schema = export_schema(&conn).unwrap();
        assert_eq!(
            schema,
            vec![TagSchema {
                name: "Battery".to_string(),
                definition: "Relates to batteries".to_string(),
                parent: None,
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

    fn fields<'a>(name: &'a str, hotkey: Option<&'a str>, parent_id: Option<i64>) -> TagFields<'a> {
        TagFields {
            name,
            definition: "Some definition",
            parent_id,
            color: None,
            hotkey,
        }
    }

    #[test]
    fn insert_trims_fields_and_treats_blank_optionals_as_none() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = insert(
            &conn,
            TagFields {
                name: "  Battery ",
                definition: " About batteries ",
                parent_id: None,
                color: Some(" "),
                hotkey: Some(""),
            },
            NOW,
        )
        .unwrap();
        let tag = get(&conn, id).unwrap().unwrap();
        assert_eq!(tag.name, "Battery");
        assert_eq!(tag.definition, "About batteries");
        assert_eq!(tag.color, None);
        assert_eq!(tag.hotkey, None);
    }

    #[test]
    fn empty_name_or_definition_is_rejected() {
        let conn = storage::open_in_memory().expect("in-memory db");
        assert!(matches!(
            create(&conn, " ", "Def", None, None, NOW),
            Err(TagError::EmptyName)
        ));
        assert!(matches!(
            create(&conn, "Battery", "", None, None, NOW),
            Err(TagError::EmptyDefinition)
        ));
    }

    #[test]
    fn duplicate_names_are_rejected_even_against_archived_tags() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = create(&conn, "Battery", "Def", None, None, NOW).unwrap();
        archive(&conn, id).unwrap();
        assert!(matches!(
            create(&conn, "Battery", "Def", None, None, NOW),
            Err(TagError::DuplicateName(_))
        ));
    }

    #[test]
    fn reserved_and_multi_character_hotkeys_are_rejected() {
        let conn = storage::open_in_memory().expect("in-memory db");
        for key in ["j", "K", "s", "/"] {
            assert!(matches!(
                create(&conn, "Battery", "Def", None, Some(key), NOW),
                Err(TagError::ReservedHotkey(_))
            ));
        }
        assert!(matches!(
            create(&conn, "Battery", "Def", None, Some("ab"), NOW),
            Err(TagError::InvalidHotkey)
        ));
    }

    #[test]
    fn hotkeys_are_unique_among_active_tags_case_insensitively() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let battery = create(&conn, "Battery", "Def", None, Some("b"), NOW).unwrap();
        assert!(matches!(
            create(&conn, "Biology", "Def", None, Some("B"), NOW),
            Err(TagError::HotkeyInUse(_, owner)) if owner == "Battery"
        ));
        // A tag keeps its own hotkey on update.
        update(&conn, battery, fields("Battery", Some("b"), None), false).unwrap();
        // Archived tags release their hotkey.
        archive(&conn, battery).unwrap();
        create(&conn, "Biology", "Def", None, Some("b"), NOW).unwrap();
    }

    #[test]
    fn update_changes_fields_and_reports_when_the_embedding_is_stale() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = insert(&conn, fields("Battery", None, None), NOW).unwrap();

        let colour_only = update(
            &conn,
            id,
            TagFields {
                color: Some("#0f0"),
                ..fields("Battery", Some("x"), None)
            },
            false,
        )
        .unwrap();
        assert!(!colour_only.needs_embedding);
        assert_eq!(colour_only.tag.color.as_deref(), Some("#0f0"));
        assert_eq!(colour_only.tag.hotkey.as_deref(), Some("x"));
        assert_eq!(colour_only.tag.version, 1);

        let renamed = update(&conn, id, fields("Batteries", None, None), false).unwrap();
        assert!(renamed.needs_embedding);
        assert_eq!(renamed.tag.version, 1);

        let bumped = update(&conn, id, fields("Batteries", None, None), true).unwrap();
        assert!(bumped.needs_embedding);
        assert_eq!(bumped.tag.version, 2);
    }

    #[test]
    fn archived_tags_cannot_be_edited() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = create(&conn, "Battery", "Def", None, None, NOW).unwrap();
        archive(&conn, id).unwrap();
        assert!(matches!(
            update(&conn, id, fields("Battery", None, None), false),
            Err(TagError::Archived)
        ));
    }

    #[test]
    fn parent_must_be_active_and_must_not_create_a_cycle() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let energy = create(&conn, "Energy", "Def", None, None, NOW).unwrap();
        let battery = insert(&conn, fields("Battery", None, Some(energy)), NOW).unwrap();
        let cell = insert(&conn, fields("Cell", None, Some(battery)), NOW).unwrap();
        assert_eq!(get(&conn, cell).unwrap().unwrap().parent_id, Some(battery));

        // Self, child and grandchild are all invalid parents.
        for parent in [energy, battery, cell] {
            assert!(matches!(
                update(&conn, energy, fields("Energy", None, Some(parent)), false),
                Err(TagError::InvalidParent)
            ));
        }

        let old = create(&conn, "Old", "Def", None, None, NOW).unwrap();
        archive(&conn, old).unwrap();
        assert!(matches!(
            insert(&conn, fields("New", None, Some(old)), NOW),
            Err(TagError::InvalidParent)
        ));
        assert!(matches!(
            insert(&conn, fields("New", None, Some(9999)), NOW),
            Err(TagError::InvalidParent)
        ));
    }

    #[test]
    fn archiving_a_parent_moves_its_children_to_the_top_level() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let energy = create(&conn, "Energy", "Def", None, None, NOW).unwrap();
        let battery = insert(&conn, fields("Battery", None, Some(energy)), NOW).unwrap();
        archive(&conn, energy).unwrap();
        assert_eq!(get(&conn, battery).unwrap().unwrap().parent_id, None);
    }

    #[test]
    fn unarchive_restores_the_tag_and_clears_a_hotkey_taken_meanwhile() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let battery = create(&conn, "Battery", "Def", None, Some("b"), NOW).unwrap();
        let solar = create(&conn, "Solar", "Def", None, Some("o"), NOW).unwrap();
        archive(&conn, battery).unwrap();
        archive(&conn, solar).unwrap();
        assert_eq!(list_archived(&conn).unwrap().len(), 2);
        create(&conn, "Biology", "Def", None, Some("B"), NOW).unwrap();

        let restored = unarchive(&conn, battery).unwrap();
        assert!(!restored.tag.archived);
        assert_eq!(restored.hotkey_cleared.as_deref(), Some("b"));
        assert_eq!(restored.tag.hotkey, None);

        let restored = unarchive(&conn, solar).unwrap();
        assert_eq!(restored.hotkey_cleared, None);
        assert_eq!(restored.tag.hotkey.as_deref(), Some("o"));
        assert!(list_archived(&conn).unwrap().is_empty());
    }

    #[test]
    fn export_schema_names_the_parent() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let energy = create(&conn, "Energy", "Def", None, None, NOW).unwrap();
        insert(&conn, fields("Battery", None, Some(energy)), NOW).unwrap();
        let schema = export_schema(&conn).unwrap();
        assert_eq!(schema[0].name, "Battery");
        assert_eq!(schema[0].parent.as_deref(), Some("Energy"));
        assert_eq!(schema[1].parent, None);
    }

    #[test]
    fn stats_count_labels_by_source_and_state_plus_unlabelled_and_stale() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = create(&conn, "Battery", "Def", None, None, NOW).unwrap();
        let insert_doc = |key: &str, review_state: &str| -> i64 {
            conn.execute(
                "INSERT INTO documents (pub_key, input_raw, fetch_status, review_state, imported_at)
                 VALUES (?1, ?1, 'fetched', ?2, ?3)",
                params![key, review_state, NOW],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let insert_label = |doc_id: i64, state: &str, source: &str, version: i64| {
            conn.execute(
                "INSERT INTO labels (doc_id, tag_id, state, source, tag_version, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![doc_id, tag_id, state, source, version, NOW],
            )
            .unwrap();
        };
        insert_label(insert_doc("EP1", "validated"), "pos", "human", 1);
        insert_label(insert_doc("EP2", "validated"), "neg", "human", 1);
        insert_label(insert_doc("EP3", "auto_completed"), "pos", "auto", 1);
        insert_label(insert_doc("EP4", "auto_completed"), "neg", "auto", 1);
        insert_doc("EP5", "validated");
        insert_doc("EP6", "auto_completed");
        insert_doc("EP7", "queued");
        update(&conn, tag_id, fields("Battery", None, None), true).unwrap();
        insert_label(insert_doc("EP8", "validated"), "pos", "human", 2);

        assert_eq!(
            stats(&conn, tag_id).unwrap(),
            TagStats {
                tag_id,
                human_pos: 2,
                human_neg: 1,
                auto_pos: 1,
                auto_neg: 1,
                unlabelled: 2,
                stale: 2,
            }
        );
    }

    fn schema_entry(name: &str, parent: Option<&str>, hotkey: Option<&str>) -> TagSchema {
        TagSchema {
            name: name.to_string(),
            definition: "Def".to_string(),
            parent: parent.map(str::to_string),
            color: None,
            hotkey: hotkey.map(str::to_string),
        }
    }

    #[test]
    fn import_schema_links_parents_declared_later_and_skips_existing_names() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let existing = create(&conn, "Existing", "Keep me", None, None, NOW).unwrap();
        let schema = vec![
            schema_entry("Battery", Some("Energy"), None),
            schema_entry("Energy", None, None),
            schema_entry("Existing", None, None),
        ];
        let outcome = import_schema(&conn, &schema, NOW).unwrap();
        assert_eq!(outcome.created.len(), 2);
        assert!(outcome.parents_dropped.is_empty());
        let battery = find_by_name(&conn, "Battery").unwrap().unwrap();
        let energy = find_by_name(&conn, "Energy").unwrap().unwrap();
        assert_eq!(battery.parent_id, Some(energy.id));
        assert_eq!(get(&conn, existing).unwrap().unwrap().definition, "Keep me");
    }

    #[test]
    fn import_schema_drops_conflicting_hotkeys_and_unresolvable_parents() {
        let conn = storage::open_in_memory().expect("in-memory db");
        create(&conn, "Biology", "Def", None, Some("b"), NOW).unwrap();
        let schema = vec![
            schema_entry("Battery", Some("Missing"), Some("B")),
            schema_entry("Skip", None, Some("s")),
            schema_entry("A", Some("C"), None),
            schema_entry("C", Some("A"), None),
        ];
        let outcome = import_schema(&conn, &schema, NOW).unwrap();
        assert_eq!(outcome.created.len(), 4);
        assert_eq!(outcome.hotkeys_dropped, vec!["Battery", "Skip"]);
        // A links to C; C -> A would then be a cycle.
        assert_eq!(outcome.parents_dropped, vec!["Battery", "C"]);
        assert_eq!(
            find_by_name(&conn, "Battery").unwrap().unwrap().hotkey,
            None
        );
    }
}
