//! Persistence for the generic `jobs` table (SPEC section 4.2). Import runs
//! as persistent jobs so an interrupted import resumes at the next launch
//! (section 5.4); job-specific data lives in `payload` as JSON so this
//! stays a generic queue with no schema change per job kind.

use crate::storage::StorageError;
use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq)]
pub struct JobRow {
    pub id: i64,
    pub kind: String,
    pub payload: String,
    pub state: String,
    pub attempts: i64,
    pub last_error: Option<String>,
}

pub const STATE_PENDING: &str = "pending";
pub const STATE_RUNNING: &str = "running";
pub const STATE_DONE: &str = "done";
pub const STATE_FAILED: &str = "failed";

pub fn enqueue(
    conn: &Connection,
    kind: &str,
    payload: &str,
    updated_at: &str,
) -> Result<i64, StorageError> {
    conn.execute(
        "INSERT INTO jobs (kind, payload, state, attempts, updated_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![kind, payload, STATE_PENDING, updated_at],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Pending or running jobs of `kind`, oldest first — running ones are
/// included so a job left running by an interrupted previous launch (SPEC
/// 5.4) is picked back up rather than silently stuck.
pub fn list_resumable(conn: &Connection, kind: &str) -> Result<Vec<JobRow>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload, state, attempts, last_error FROM jobs
         WHERE kind = ?1 AND state IN (?2, ?3)
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![kind, STATE_PENDING, STATE_RUNNING], row_to_job)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn mark_running(conn: &Connection, id: i64, updated_at: &str) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE jobs SET state = ?1, attempts = attempts + 1, updated_at = ?2 WHERE id = ?3",
        params![STATE_RUNNING, updated_at, id],
    )?;
    Ok(())
}

pub fn mark_done(conn: &Connection, id: i64, updated_at: &str) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE jobs SET state = ?1, last_error = NULL, updated_at = ?2 WHERE id = ?3",
        params![STATE_DONE, updated_at, id],
    )?;
    Ok(())
}

pub fn mark_failed(
    conn: &Connection,
    id: i64,
    error: &str,
    updated_at: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE jobs SET state = ?1, last_error = ?2, updated_at = ?3 WHERE id = ?4",
        params![STATE_FAILED, error, updated_at, id],
    )?;
    Ok(())
}

/// Resets a failed job back to pending, for the Import screen's retry
/// button (SPEC section 8).
pub fn retry(conn: &Connection, id: i64, updated_at: &str) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE jobs SET state = ?1, last_error = NULL, updated_at = ?2 WHERE id = ?3 AND state = ?4",
        params![STATE_PENDING, updated_at, id, STATE_FAILED],
    )?;
    Ok(())
}

fn row_to_job(row: &rusqlite::Row) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: row.get(0)?,
        kind: row.get(1)?,
        payload: row.get(2)?,
        state: row.get(3)?,
        attempts: row.get(4)?,
        last_error: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    const NOW: &str = "2026-01-01T00:00:00Z";

    #[test]
    fn enqueued_jobs_are_resumable() {
        let conn = storage::open_in_memory().expect("in-memory db");
        enqueue(&conn, "import_document", r#"{"raw":"EP1234567"}"#, NOW).unwrap();
        let jobs = list_resumable(&conn, "import_document").unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, STATE_PENDING);
        assert_eq!(jobs[0].attempts, 0);
    }

    #[test]
    fn running_jobs_are_still_resumable_after_an_interrupted_launch() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = enqueue(&conn, "import_document", "{}", NOW).unwrap();
        mark_running(&conn, id, NOW).unwrap();

        let jobs = list_resumable(&conn, "import_document").unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, STATE_RUNNING);
        assert_eq!(jobs[0].attempts, 1);
    }

    #[test]
    fn done_jobs_are_not_resumable() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = enqueue(&conn, "import_document", "{}", NOW).unwrap();
        mark_running(&conn, id, NOW).unwrap();
        mark_done(&conn, id, NOW).unwrap();

        assert!(list_resumable(&conn, "import_document").unwrap().is_empty());
    }

    #[test]
    fn retry_moves_a_failed_job_back_to_pending() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = enqueue(&conn, "import_document", "{}", NOW).unwrap();
        mark_running(&conn, id, NOW).unwrap();
        mark_failed(&conn, id, "network error", NOW).unwrap();

        retry(&conn, id, NOW).unwrap();

        let jobs = list_resumable(&conn, "import_document").unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].state, STATE_PENDING);
        assert_eq!(jobs[0].last_error, None);
    }

    #[test]
    fn retry_does_nothing_to_a_job_that_is_not_failed() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let id = enqueue(&conn, "import_document", "{}", NOW).unwrap();
        retry(&conn, id, NOW).unwrap();
        assert_eq!(list_resumable(&conn, "import_document").unwrap()[0].state, STATE_PENDING);
    }
}
