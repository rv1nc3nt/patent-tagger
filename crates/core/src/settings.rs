//! Key-value persistence for the `settings` table (SPEC 4.2). The full
//! Settings screen (target precision, target recall, audit rates, retrieval
//! policies, ...) is M7; this just gives M6's threshold calibration
//! somewhere to read/write `target_precision` before that UI exists.

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

pub const TARGET_PRECISION_KEY: &str = "target_precision";
const DEFAULT_TARGET_PRECISION: f32 = 0.95;

/// SPEC 7.5: "the lowest threshold reaching the target precision (default
/// 0.95, configurable)".
pub fn target_precision(conn: &Connection) -> Result<f32, StorageError> {
    Ok(get(conn, TARGET_PRECISION_KEY)?
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TARGET_PRECISION))
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
}
