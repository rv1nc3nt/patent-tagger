//! Retraining orchestration (SPEC 7.2): "retrained in the background after
//! every 10 validations, or on demand." Only the trigger cadence lives
//! here; the actual training math is `core_lib::classifier::train`.

use core_lib::rusqlite::Connection;
use core_lib::{classifier, embeddings, predictions, settings, storage, tags};
use embed_lib::Embedder;

const RETRAIN_EVERY_N_VALIDATIONS: i64 = 10;

/// Retrains every active tag with enough data (SPEC 7.2: >= 5 positives
/// and >= 5 negatives) and recalibrates every active tag's threshold (SPEC
/// 7.5) - both unconditional here; callers decide when to call this (every
/// 10 validations, or on demand). Threshold recalibration doesn't need a
/// trained classifier (it only needs prequential data), so it runs for
/// every active tag regardless of whether LR training succeeded.
pub fn retrain_eligible_tags(
    conn: &Connection,
    embedder: &impl Embedder,
    now: &str,
) -> Result<usize, storage::StorageError> {
    let target_precision = settings::target_precision(conn)?;
    let target_recall = settings::target_recall(conn)?;
    let mut retrained = 0;
    for tag in tags::list_active(conn)? {
        let samples = embeddings::training_samples_for_tag(conn, tag.id, embedder.model_id())?;
        if let Some(trained) = classifier::train(&samples, classifier::default_l2_lambda()) {
            classifier::store(conn, tag.id, embedder.model_id(), &trained, now)?;
            retrained += 1;
        }

        let calibrated = predictions::calibrate_threshold(conn, tag.id, target_precision)?;
        tags::set_threshold(conn, tag.id, calibrated)?;

        // SPEC 7.6: neg_threshold calibration, same cadence as the
        // ordinary threshold - both come from the same prequential window.
        let neg_calibrated = predictions::calibrate_neg_threshold(conn, tag.id, target_recall)?;
        tags::set_neg_threshold(conn, tag.id, neg_calibrated)?;
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
    use core_lib::{documents, labels, predictions};
    use std::collections::HashSet;

    struct FakeEmbedder;
    impl Embedder for FakeEmbedder {
        fn model_id(&self) -> &str {
            "fake-model@v1"
        }
        fn dim(&self) -> usize {
            2
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, embed_lib::EmbedError> {
            Ok(texts.iter().map(|_| vec![0.0, 0.0]).collect())
        }
    }

    #[test]
    fn retrain_recalibrates_thresholds_for_every_active_tag() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(
            &conn,
            "Battery",
            "About batteries",
            None,
            None,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        assert_eq!(
            tag.threshold, None,
            "no threshold before any prequential data exists"
        );

        // Cleanly separated scores so a threshold reaching the default
        // 0.95 target precision is reachable.
        for i in 0..5 {
            let pub_key = format!("EPPOS{i:04}");
            documents::insert_pending(&conn, &pub_key, &pub_key, "2026-01-01T00:00:00Z").unwrap();
            let doc = documents::find_by_pub_key(&conn, &pub_key)
                .unwrap()
                .unwrap();
            predictions::record(
                &conn,
                doc.id,
                tag_id,
                "fake-model@v1",
                0.9,
                true,
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
            labels::validate_document(
                &conn,
                doc.id,
                std::slice::from_ref(&tag),
                &[tag_id].into_iter().collect(),
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        }

        retrain_eligible_tags(&conn, &FakeEmbedder, "2026-01-02T00:00:00Z").unwrap();

        let recalibrated = tags::get(&conn, tag_id).unwrap().unwrap();
        assert!(
            recalibrated.threshold.is_some(),
            "threshold should now be calibrated"
        );
    }

    #[test]
    fn scheduled_retrain_point_is_every_tenth_validation() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(
            &conn,
            "Battery",
            "About batteries",
            None,
            None,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        for i in 0..9 {
            documents::insert_pending(
                &conn,
                &format!("EP{i:07}"),
                &format!("EP{i:07}"),
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
            let doc = documents::find_by_pub_key(&conn, &format!("EP{i:07}"))
                .unwrap()
                .unwrap();
            labels::validate_document(
                &conn,
                doc.id,
                std::slice::from_ref(&tag),
                &HashSet::new(),
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
            assert!(
                !is_scheduled_retrain_point(&conn).unwrap(),
                "should not trigger before the 10th"
            );
        }

        documents::insert_pending(&conn, "EP9999999", "EP9999999", "2026-01-01T00:00:00Z").unwrap();
        let tenth = documents::find_by_pub_key(&conn, "EP9999999")
            .unwrap()
            .unwrap();
        labels::validate_document(
            &conn,
            tenth.id,
            std::slice::from_ref(&tag),
            &HashSet::new(),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        assert!(
            is_scheduled_retrain_point(&conn).unwrap(),
            "the 10th validation should trigger"
        );
    }
}
