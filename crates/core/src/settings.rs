//! Key-value persistence for the `settings` table (SPEC 4.2, section 8's
//! Settings screen).

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, StorageError> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
        .optional()
        .map_err(StorageError::from)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn get_f32(conn: &Connection, key: &str, default: f32) -> Result<f32, StorageError> {
    Ok(get(conn, key)?.and_then(|s| s.parse().ok()).unwrap_or(default))
}

fn get_bool(conn: &Connection, key: &str, default: bool) -> Result<bool, StorageError> {
    Ok(get(conn, key)?.map(|s| s == "true").unwrap_or(default))
}

pub const TARGET_PRECISION_KEY: &str = "target_precision";
const DEFAULT_TARGET_PRECISION: f32 = 0.95;

/// SPEC 7.5: "the lowest threshold reaching the target precision (default
/// 0.95, configurable)".
pub fn target_precision(conn: &Connection) -> Result<f32, StorageError> {
    get_f32(conn, TARGET_PRECISION_KEY, DEFAULT_TARGET_PRECISION)
}

pub const TARGET_RECALL_KEY: &str = "target_recall";
const DEFAULT_TARGET_RECALL: f32 = 0.95;

/// SPEC 7.6: the target recall used for `neg_threshold` calibration
/// ("confident absence"). Stored from M7 (section 8's Settings screen);
/// not consumed until M9.
pub fn target_recall(conn: &Connection) -> Result<f32, StorageError> {
    get_f32(conn, TARGET_RECALL_KEY, DEFAULT_TARGET_RECALL)
}

pub const AUDIT_RATE_KEY: &str = "audit_rate";
/// SPEC 7.6: "a fraction (default 5%) of auto-completed documents is
/// sampled." M6's own per-tag safeguard never actually consumed this
/// setting (it audits every automatic decision that happens to reach
/// review anyway, not a sampled fraction - see docs/DECISIONS.md), so M9's
/// full-automation sampling is this setting's first real consumer; its
/// default follows 7.6's number rather than 7.5's different one (10%),
/// which was only ever a placeholder.
const DEFAULT_AUDIT_RATE: f32 = 0.05;

/// The fraction of auto-completed documents sampled into the review queue
/// as a full review (SPEC 7.6).
pub fn audit_rate(conn: &Connection) -> Result<f32, StorageError> {
    get_f32(conn, AUDIT_RATE_KEY, DEFAULT_AUDIT_RATE)
}

pub const FULL_AUTOMATION_ENABLED_KEY: &str = "full_automation_enabled";

/// SPEC 7.6: "a single setting, off by default. It can be turned on only
/// when at least one tag is in automatic mode" - that precondition is the
/// caller's job to enforce (see `commands::update_settings`); has no
/// behavioural effect until M9 builds full automation itself.
pub fn full_automation_enabled(conn: &Connection) -> Result<bool, StorageError> {
    get_bool(conn, FULL_AUTOMATION_ENABLED_KEY, false)
}

pub const FULLTEXT_POLICY_KEY: &str = "fulltext_retrieval_policy";
pub const DRAWINGS_POLICY_KEY: &str = "drawings_retrieval_policy";
const DEFAULT_FULLTEXT_POLICY: &str = "after_tagging_all";
const DEFAULT_DRAWINGS_POLICY: &str = "on_demand";

/// SPEC 5.5: `"never" | "on_demand" | "after_tagging_all" |
/// "after_tagging_selected_tags"`. Stored from M7; not consumed until M8
/// builds the retrieval pipeline that reads it.
pub fn fulltext_policy(conn: &Connection) -> Result<String, StorageError> {
    Ok(get(conn, FULLTEXT_POLICY_KEY)?.unwrap_or_else(|| DEFAULT_FULLTEXT_POLICY.to_string()))
}

pub fn drawings_policy(conn: &Connection) -> Result<String, StorageError> {
    Ok(get(conn, DRAWINGS_POLICY_KEY)?.unwrap_or_else(|| DEFAULT_DRAWINGS_POLICY.to_string()))
}

pub const FULLTEXT_POLICY_TAG_IDS_KEY: &str = "fulltext_policy_tag_ids";
pub const DRAWINGS_POLICY_TAG_IDS_KEY: &str = "drawings_policy_tag_ids";

/// The tag ids consulted by the `after_tagging_selected_tags` policy
/// (SPEC 5.5). Stored as JSON since `settings` is a plain key-value table.
fn get_tag_ids(conn: &Connection, key: &str) -> Result<Vec<i64>, StorageError> {
    Ok(get(conn, key)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

fn set_tag_ids(conn: &Connection, key: &str, tag_ids: &[i64]) -> Result<(), StorageError> {
    set(conn, key, &serde_json::to_string(tag_ids).unwrap_or_default())
}

pub fn fulltext_policy_tag_ids(conn: &Connection) -> Result<Vec<i64>, StorageError> {
    get_tag_ids(conn, FULLTEXT_POLICY_TAG_IDS_KEY)
}

pub fn set_fulltext_policy_tag_ids(conn: &Connection, tag_ids: &[i64]) -> Result<(), StorageError> {
    set_tag_ids(conn, FULLTEXT_POLICY_TAG_IDS_KEY, tag_ids)
}

pub fn drawings_policy_tag_ids(conn: &Connection) -> Result<Vec<i64>, StorageError> {
    get_tag_ids(conn, DRAWINGS_POLICY_TAG_IDS_KEY)
}

pub fn set_drawings_policy_tag_ids(conn: &Connection, tag_ids: &[i64]) -> Result<(), StorageError> {
    set_tag_ids(conn, DRAWINGS_POLICY_TAG_IDS_KEY, tag_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    #[test]
    fn target_precision_defaults_to_0_95_when_unset() {
        let conn = storage::open_in_memory().expect("in-memory db");
        assert_eq!(target_precision(&conn).unwrap(), 0.95);
    }

    #[test]
    fn set_and_get_round_trip() {
        let conn = storage::open_in_memory().expect("in-memory db");
        set(&conn, TARGET_PRECISION_KEY, "0.9").unwrap();
        assert_eq!(target_precision(&conn).unwrap(), 0.9);
    }

    #[test]
    fn set_twice_updates_rather_than_erroring() {
        let conn = storage::open_in_memory().expect("in-memory db");
        set(&conn, TARGET_PRECISION_KEY, "0.9").unwrap();
        set(&conn, TARGET_PRECISION_KEY, "0.99").unwrap();
        assert_eq!(target_precision(&conn).unwrap(), 0.99);
    }

    #[test]
    fn every_setting_has_the_documented_default() {
        let conn = storage::open_in_memory().expect("in-memory db");
        assert_eq!(target_recall(&conn).unwrap(), 0.95);
        assert_eq!(audit_rate(&conn).unwrap(), 0.05);
        assert!(!full_automation_enabled(&conn).unwrap());
        assert_eq!(fulltext_policy(&conn).unwrap(), "after_tagging_all");
        assert_eq!(drawings_policy(&conn).unwrap(), "on_demand");
    }

    #[test]
    fn bool_and_policy_settings_round_trip() {
        let conn = storage::open_in_memory().expect("in-memory db");
        set(&conn, FULL_AUTOMATION_ENABLED_KEY, "true").unwrap();
        assert!(full_automation_enabled(&conn).unwrap());

        set(&conn, FULLTEXT_POLICY_KEY, "never").unwrap();
        assert_eq!(fulltext_policy(&conn).unwrap(), "never");
    }

    #[test]
    fn policy_tag_ids_default_empty_and_round_trip() {
        let conn = storage::open_in_memory().expect("in-memory db");
        assert_eq!(fulltext_policy_tag_ids(&conn).unwrap(), Vec::<i64>::new());

        set_fulltext_policy_tag_ids(&conn, &[1, 2, 3]).unwrap();
        assert_eq!(fulltext_policy_tag_ids(&conn).unwrap(), vec![1, 2, 3]);
        assert_eq!(drawings_policy_tag_ids(&conn).unwrap(), Vec::<i64>::new());

        set_drawings_policy_tag_ids(&conn, &[4]).unwrap();
        assert_eq!(drawings_policy_tag_ids(&conn).unwrap(), vec![4]);
        assert_eq!(fulltext_policy_tag_ids(&conn).unwrap(), vec![1, 2, 3], "the two policies' tag lists are independent");
    }
}
