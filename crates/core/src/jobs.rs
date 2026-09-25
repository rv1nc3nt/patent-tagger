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

/// Failed jobs of `kind`, oldest first — for the Import screen's retry
/// button (SPEC section 8).
pub fn list_failed(conn: &Connection, kind: &str) -> Result<Vec<JobRow>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload, state, attempts, last_error FROM jobs
         WHERE kind = ?1 AND state = ?2
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![kind, STATE_FAILED], row_to_job)?
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

/// How many jobs of one kind are in each state that still matters, for the
/// Jobs screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct JobCounts {
    pub kind: String,
    pub pending: i64,
    pub running: i64,
    pub failed: i64,
}

/// Counts for each of `kinds`, in that order (zero counts included).
pub fn counts(conn: &Connection, kinds: &[&str]) -> Result<Vec<JobCounts>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT state, count(*) FROM jobs WHERE kind = ?1 AND state IN (?2, ?3, ?4) GROUP BY state",
    )?;
    kinds
        .iter()
        .map(|kind| {
            let mut counts = JobCounts {
                kind: kind.to_string(),
                ..Default::default()
            };
            let rows = stmt.query_map(
                params![kind, STATE_PENDING, STATE_RUNNING, STATE_FAILED],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?;
            for row in rows {
                let (state, n) = row?;
                match state.as_str() {
                    STATE_PENDING => counts.pending = n,
                    STATE_RUNNING => counts.running = n,
                    _ => counts.failed = n,
                }
            }
            Ok(counts)
        })
        .collect()
}

/// A job with the document it concerns, for the Jobs screen.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JobDetail {
    pub id: i64,
    pub kind: String,
    pub state: String,
    pub doc_id: Option<i64>,
    pub pub_key: Option<String>,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// Jobs in `state`, most recently updated first, at most `limit`. The
/// document comes from the payload's `doc_id`, which every job kind has.
pub fn list_detailed(
    conn: &Connection,
    state: &str,
    limit: i64,
) -> Result<Vec<JobDetail>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT j.id, j.kind, j.state, d.id, d.pub_key, j.attempts, j.last_error, j.updated_at
         FROM jobs j
         LEFT JOIN documents d ON d.id = json_extract(j.payload, '$.doc_id')
         WHERE j.state = ?1
         ORDER BY j.updated_at DESC, j.id DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![state, limit], |row| {
            Ok(JobDetail {
                id: row.get(0)?,
                kind: row.get(1)?,
                state: row.get(2)?,
                doc_id: row.get(3)?,
                pub_key: row.get(4)?,
                attempts: row.get(5)?,
                last_error: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The job's kind, or `None` when it doesn't exist.
pub fn kind_of(conn: &Connection, id: i64) -> Result<Option<String>, StorageError> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row("SELECT kind FROM jobs WHERE id = ?1", params![id], |row| {
            row.get(0)
        })
        .optional()?)
}

/// Resets every failed job of `kind` to pending. Returns how many.
pub fn retry_failed(
    conn: &Connection,
    kind: &str,
    updated_at: &str,
) -> Result<usize, StorageError> {
    Ok(conn.execute(
        "UPDATE jobs SET state = ?1, last_error = NULL, updated_at = ?2 WHERE kind = ?3 AND state = ?4",
        params![STATE_PENDING, updated_at, kind, STATE_FAILED],
    )?)
}

/// Deletes the pending (not running) jobs of `kind`. Returns how many.
pub fn cancel_pending(conn: &Connection, kind: &str) -> Result<usize, StorageError> {
    Ok(conn.execute(
        "DELETE FROM jobs WHERE kind = ?1 AND state = ?2",
        params![kind, STATE_PENDING],
    )?)
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
    fn list_failed_returns_only_failed_jobs() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let ok_id = enqueue(&conn, "import_document", "{}", NOW).unwrap();
        let failed_id = enqueue(&conn, "import_document", "{}", NOW).unwrap();
        mark_running(&conn, ok_id, NOW).unwrap();
        mark_done(&conn, ok_id, NOW).unwrap();
        mark_running(&conn, failed_id, NOW).unwrap();
        mark_failed(&conn, failed_id, "boom", NOW).unwrap();

        let failed = list_failed(&conn, "import_document").unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].id, failed_id);
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
        assert_eq!(
            list_resumable(&conn, "import_document").unwrap()[0].state,
            STATE_PENDING
        );
    }

    #[test]
    fn counts_include_every_requested_kind() {
        let conn = storage::open_in_memory().unwrap();
        enqueue(&conn, "a", "{}", NOW).unwrap();
        let running = enqueue(&conn, "a", "{}", NOW).unwrap();
        mark_running(&conn, running, NOW).unwrap();
        let failed = enqueue(&conn, "a", "{}", NOW).unwrap();
        mark_failed(&conn, failed, "boom", NOW).unwrap();
        let done = enqueue(&conn, "a", "{}", NOW).unwrap();
        mark_done(&conn, done, NOW).unwrap();

        let counts = counts(&conn, &["a", "b"]).unwrap();
        assert_eq!(
            counts,
            vec![
                JobCounts {
                    kind: "a".to_string(),
                    pending: 1,
                    running: 1,
                    failed: 1
                },
                JobCounts {
                    kind: "b".to_string(),
                    ..Default::default()
                },
            ]
        );
    }

    #[test]
    fn list_detailed_names_the_document_of_each_job() {
        let conn = storage::open_in_memory().unwrap();
        let doc_id = crate::documents::insert_pending(&conn, "EP1234567", "EP1234567", NOW)
            .unwrap()
            .unwrap();
        let id = enqueue(&conn, "a", &format!(r#"{{"doc_id":{doc_id}}}"#), NOW).unwrap();
        mark_failed(&conn, id, "boom", NOW).unwrap();
        let orphan = enqueue(&conn, "a", "{}", NOW).unwrap();
        mark_failed(&conn, orphan, "bad", NOW).unwrap();

        let failed = list_detailed(&conn, STATE_FAILED, 10).unwrap();
        assert_eq!(failed.len(), 2);
        let with_doc = failed.iter().find(|j| j.id == id).unwrap();
        assert_eq!(with_doc.pub_key.as_deref(), Some("EP1234567"));
        assert_eq!(with_doc.last_error.as_deref(), Some("boom"));
        assert_eq!(
            failed.iter().find(|j| j.id == orphan).unwrap().pub_key,
            None
        );
    }

    #[test]
    fn retry_failed_and_cancel_pending_touch_only_their_kind_and_state() {
        let conn = storage::open_in_memory().unwrap();
        let failed = enqueue(&conn, "a", "{}", NOW).unwrap();
        mark_failed(&conn, failed, "boom", NOW).unwrap();
        let other = enqueue(&conn, "b", "{}", NOW).unwrap();
        mark_failed(&conn, other, "boom", NOW).unwrap();
        let running = enqueue(&conn, "a", "{}", NOW).unwrap();
        mark_running(&conn, running, NOW).unwrap();

        assert_eq!(retry_failed(&conn, "a", NOW).unwrap(), 1);
        assert_eq!(list_failed(&conn, "b").unwrap().len(), 1);
        // The retried job is pending again; the running one is left alone.
        assert_eq!(cancel_pending(&conn, "a").unwrap(), 1);
        let left = list_resumable(&conn, "a").unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, running);
        assert_eq!(kind_of(&conn, running).unwrap().as_deref(), Some("a"));
        assert_eq!(kind_of(&conn, failed).unwrap(), None);
    }
}
