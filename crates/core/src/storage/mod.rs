//! SQLite storage (SPEC section 4). Schema and migrations only; the
//! higher-level repository functions (documents, tags, labels, ...) land in
//! later milestones alongside the features that need them.

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use std::sync::LazyLock;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

static MIGRATIONS: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(vec![
        M::up(include_str!("schema.sql")),
        M::up(include_str!("002_saved_searches.sql")),
    ])
});

/// Opens (creating if needed) the database at `path`, enables WAL mode and
/// foreign keys, and migrates it to the latest schema.
pub fn open(path: &std::path::Path) -> Result<Connection, StorageError> {
    let mut conn = Connection::open(path)?;
    apply_pragmas(&conn)?;
    MIGRATIONS.to_latest(&mut conn)?;
    Ok(conn)
}

/// Opens an in-memory database, for tests. Foreign keys are enabled; WAL
/// mode does not apply to `:memory:` databases.
pub fn open_in_memory() -> Result<Connection, StorageError> {
    let mut conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    MIGRATIONS.to_latest(&mut conn)?;
    Ok(conn)
}

/// How long a connection waits on another process's write lock.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn apply_pragmas(conn: &Connection) -> Result<(), StorageError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // SPEC 7.7: a scheduled command-line import and the GUI can hold the
    // same database file open at once; wait for the other's write
    // transaction instead of failing immediately with SQLITE_BUSY.
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("valid query against sqlite_master");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query_map succeeds")
            .collect::<Result<_, _>>()
            .expect("all rows are valid strings")
    }

    #[test]
    fn migrations_are_internally_consistent() {
        MIGRATIONS
            .validate()
            .expect("every migration should be valid");
    }

    #[test]
    fn migration_creates_every_table_from_section_4_2() {
        let conn = open_in_memory().expect("in-memory database should open and migrate");
        let tables = table_names(&conn);
        for expected in [
            "documents",
            "fulltext",
            "drawings",
            "drawings_status",
            "ops_raw",
            "tags",
            "labels",
            "label_history",
            "embeddings",
            "tag_embeddings",
            "predictions",
            "classifiers",
            "jobs",
            "settings",
            "saved_searches",
        ] {
            assert!(
                tables.iter().any(|t| t == expected),
                "expected table {expected} to exist, found: {tables:?}"
            );
        }
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let conn = open_in_memory().expect("in-memory database should open and migrate");
        let result = conn.execute(
            "INSERT INTO fulltext (doc_id, status) VALUES (999, 'pending')",
            [],
        );
        assert!(
            result.is_err(),
            "inserting a fulltext row for a non-existent document should violate the foreign key"
        );
    }

    #[test]
    fn documents_fts_stays_in_sync_with_documents() {
        let conn = open_in_memory().expect("in-memory database should open and migrate");
        conn.execute(
            "INSERT INTO documents (pub_key, input_raw, fetch_status, review_state, imported_at)
             VALUES ('EP1234567', 'EP1234567', 'fetched', 'queued', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("insert should succeed");
        conn.execute(
            "UPDATE documents SET title = 'A gadget', \"abstract\" = 'An abstract about gadgets'
             WHERE pub_key = 'EP1234567'",
            [],
        )
        .expect("update should succeed");

        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM documents_fts WHERE documents_fts MATCH 'gadgets'",
                [],
                |row| row.get(0),
            )
            .expect("fts query should succeed");
        assert_eq!(hits, 1);
    }

    #[test]
    fn open_creates_a_file_backed_database_with_wal_mode() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let path = dir.path().join("patent-tagger.sqlite3");

        let conn = open(&path).expect("file-backed database should open and migrate");
        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("journal_mode pragma should be readable");
        assert_eq!(journal_mode.to_lowercase(), "wal");
        assert!(path.exists());
    }
}
