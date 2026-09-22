//! Retraining orchestration (SPEC 7.2): "retrained in the background after
//! every 10 validations, or on demand." Only the trigger cadence lives
//! here; the actual training math is `core_lib::classifier::train`.

use core_lib::rusqlite::Connection;
use core_lib::{classifier, embeddings, storage, tags};
use embed_lib::Embedder;

const RETRAIN_EVERY_N_VALIDATIONS: i64 = 10;

/// Retrains every active tag with enough data (SPEC 7.2: >= 5 positives
/// and >= 5 negatives), unconditionally - callers decide when to call
/// this (every 10 validations, or on demand).
pub fn retrain_eligible_tags(
    conn: &Connection,
    embedder: &impl Embedder,
    now: &str,
) -> Result<usize, storage::StorageError> {
    let mut retrained = 0;
    for tag in tags::list_active(conn)? {
        let samples = embeddings::training_samples_for_tag(conn, tag.id, embedder.model_id())?;
        if let Some(trained) = classifier::train(&samples, classifier::default_l2_lambda()) {
            classifier::store(conn, tag.id, embedder.model_id(), &trained, now)?;
            retrained += 1;
        }
    }
    Ok(retrained)
}

/// True every [`RETRAIN_EVERY_N_VALIDATIONS`]-th validated document.
pub fn is_scheduled_retrain_point(conn: &Connection) -> Result<bool, storage::StorageError> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM documents WHERE review_state = 'validated'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0 && count % RETRAIN_EVERY_N_VALIDATIONS == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::{documents, labels};
    use std::collections::HashSet;

    #[test]
    fn scheduled_retrain_point_is_every_tenth_validation() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, "2026-01-01T00:00:00Z").unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        for i in 0..9 {
            documents::insert_pending(&conn, &format!("EP{i:07}"), &format!("EP{i:07}"), "2026-01-01T00:00:00Z").unwrap();
            let doc = documents::find_by_pub_key(&conn, &format!("EP{i:07}")).unwrap().unwrap();
            labels::validate_document(&conn, doc.id, std::slice::from_ref(&tag), &HashSet::new(), "2026-01-01T00:00:00Z").unwrap();
            assert!(!is_scheduled_retrain_point(&conn).unwrap(), "should not trigger before the 10th");
        }

        documents::insert_pending(&conn, "EP9999999", "EP9999999", "2026-01-01T00:00:00Z").unwrap();
        let tenth = documents::find_by_pub_key(&conn, "EP9999999").unwrap().unwrap();
        labels::validate_document(&conn, tenth.id, std::slice::from_ref(&tag), &HashSet::new(), "2026-01-01T00:00:00Z").unwrap();
        assert!(is_scheduled_retrain_point(&conn).unwrap(), "the 10th validation should trigger");
    }
}
