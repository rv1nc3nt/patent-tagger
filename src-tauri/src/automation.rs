//! Applying automatic labels for tags in automatic mode (SPEC 7.5). Not a
//! full-document auto-complete (that's section 7.6 / M9) - a document with
//! some tags auto-decided still goes through the normal review queue for
//! whatever's left, since M6 has no way to skip review at all yet.

use core_lib::rusqlite::Connection;
use core_lib::{audit, documents, embeddings, labels, storage, tags};
use embed_lib::Embedder;

/// SPEC 7.5's audit safeguard needs at least this many audited decisions
/// before judging precision - "over the last 50" implies a real sample,
/// and disabling automation off a single unlucky case would be noisy
/// rather than a genuine safeguard.
const MIN_AUDITED_BEFORE_JUDGING: i64 = 10;

/// Checks `tag_id`'s audited precision (SPEC 7.5) and disables automatic
/// mode if it's fallen below `target_precision`, with at least
/// [`MIN_AUDITED_BEFORE_JUDGING`] audited decisions to judge from. Returns
/// `true` when it just disabled the tag (for the caller to notify the user
/// - SPEC 7.5: "the user is notified").
pub fn check_and_disable_if_below_target(
    conn: &Connection,
    tag_id: i64,
    target_precision: f32,
) -> Result<bool, storage::StorageError> {
    let Some(tag) = tags::get(conn, tag_id)? else {
        return Ok(false);
    };
    if !tag.auto_enabled {
        return Ok(false);
    }
    let (precision, n) = audit::audited_precision(conn, tag_id)?;
    let should_disable =
        n >= MIN_AUDITED_BEFORE_JUDGING && precision.is_some_and(|p| p < target_precision);
    if should_disable {
        tags::set_auto_enabled(conn, tag_id, false)?;
    }
    Ok(should_disable)
}

/// For every tag in automatic mode with a calibrated threshold, scores
/// every still-queued document that doesn't already have a label for that
/// tag, and writes an automatic `pos` label (SPEC 7.5) for the ones at or
/// above threshold. Safe to call repeatedly (idempotent: a document once
/// labelled, auto or human, is never rescored for that tag).
pub fn apply_pending_automatic_labels(
    conn: &Connection,
    embedder: &impl Embedder,
    now: &str,
) -> Result<usize, storage::StorageError> {
    let mut applied = 0;
    for tag in tags::list_active(conn)?.into_iter().filter(|t| t.auto_enabled) {
        let Some(threshold) = tag.threshold else { continue };

        for doc in documents::list_queue(conn)? {
            if labels::has_any_label(conn, doc.id, tag.id)? {
                continue;
            }
            let Some(doc_vector) = embeddings::get_document_embedding(conn, doc.id, embedder.model_id())?
            else {
                continue;
            };

            let mut neighbours = None;
            let (score, _source, model_version) =
                crate::review::score_one_tag(conn, embedder, &tag, &doc_vector, &mut neighbours)?;

            if let Some(score) = score {
                if score >= threshold
                    && labels::write_automatic_label(
                        conn,
                        doc.id,
                        tag.id,
                        score,
                        &model_version,
                        tag.version,
                        now,
                    )?
                {
                    applied += 1;
                }
            }
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::predictions;
    use std::collections::HashSet;

    const NOW: &str = "2026-01-01T00:00:00Z";

    struct FakeEmbedder;
    impl Embedder for FakeEmbedder {
        fn model_id(&self) -> &str {
            "fake-model@v1"
        }
        fn dim(&self) -> usize {
            2
        }
        fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, embed_lib::EmbedError> {
            unreachable!("this test only exercises the queue-scan/threshold-gate path")
        }
    }

    #[test]
    fn does_not_apply_when_not_auto_enabled_or_no_threshold() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let embedder = FakeEmbedder;
        tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        documents::insert_pending(&conn, "EP0000001", "EP0000001", NOW).unwrap();

        let applied = apply_pending_automatic_labels(&conn, &embedder, NOW).unwrap();
        assert_eq!(applied, 0);
    }

    #[test]
    fn applies_automatic_label_once_enabled_with_a_threshold() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let embedder = FakeEmbedder;
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.6)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        documents::insert_pending(&conn, "EP0000001", "EP0000001", NOW).unwrap();
        let doc = documents::find_by_pub_key(&conn, "EP0000001").unwrap().unwrap();
        core_lib::documents::store_fetched(
            &conn,
            doc.id,
            &core_lib::documents::FetchedData {
                title: Some("A gadget".to_string()),
                abstract_text: Some("An abstract".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        embeddings::store_document_embedding(&conn, doc.id, "fake-model@v1", &[1.0, 0.0]).unwrap();

        // A validated neighbour with a human pos label close enough (via
        // k-NN, since <5/5 rules out LR and >=3 positives rules out
        // zero-shot only if n_pos>=3 - here n_pos will be 0 initially, so
        // zero-shot applies; give the tag its own embedding too so
        // zero-shot has something to compare against.
        embeddings::store_tag_embedding(&conn, tag_id, "fake-model@v1", 1, &[1.0, 0.0]).unwrap();

        let applied = apply_pending_automatic_labels(&conn, &embedder, NOW).unwrap();
        assert_eq!(applied, 1);

        let state: String = conn
            .query_row(
                "SELECT state FROM labels WHERE doc_id = ?1 AND tag_id = ?2",
                core_lib::rusqlite::params![doc.id, tag_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "pos");

        // Re-running must not double-apply or error (the doc now has a
        // label, so it's skipped).
        let applied_again = apply_pending_automatic_labels(&conn, &embedder, NOW).unwrap();
        assert_eq!(applied_again, 0);
    }

    #[test]
    fn does_not_reapply_to_a_document_with_an_existing_human_label() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let embedder = FakeEmbedder;
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.1)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        documents::insert_pending(&conn, "EP0000001", "EP0000001", NOW).unwrap();
        let doc = documents::find_by_pub_key(&conn, "EP0000001").unwrap().unwrap();
        core_lib::labels::validate_document(&conn, doc.id, std::slice::from_ref(&tag), &HashSet::new(), NOW)
            .unwrap();

        // Not even scored (no embedding stored), but more importantly
        // `has_any_label` should already skip it before that matters.
        let applied = apply_pending_automatic_labels(&conn, &embedder, NOW).unwrap();
        assert_eq!(applied, 0);
        let _ = predictions::tag_metrics(&conn, tag_id); // sanity: doesn't panic
    }

    fn auto_then_human(conn: &Connection, pub_key: &str, tag: &tags::TagRow, human_confirms: bool) {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        labels::write_automatic_label(conn, doc.id, tag.id, 0.9, "model@v1", 1, NOW).unwrap();
        let checked: HashSet<i64> = if human_confirms { [tag.id].into_iter().collect() } else { HashSet::new() };
        labels::validate_document(conn, doc.id, std::slice::from_ref(tag), &checked, NOW).unwrap();
    }

    #[test]
    fn stays_enabled_below_the_minimum_audited_sample_size() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        // 3 overturned auto decisions - 0% precision, but below the
        // minimum sample size, so this must not trip the safeguard yet.
        for i in 0..3 {
            auto_then_human(&conn, &format!("EP{i:07}"), &tag, false);
        }

        let disabled = check_and_disable_if_below_target(&conn, tag_id, 0.95).unwrap();
        assert!(!disabled);
        assert!(tags::get(&conn, tag_id).unwrap().unwrap().auto_enabled);
    }

    #[test]
    fn disables_once_audited_precision_drops_below_target_with_enough_samples() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        // 8 confirmed, 2 overturned = 80% audited precision, below a 95% target.
        for i in 0..8 {
            auto_then_human(&conn, &format!("EPOK{i:04}"), &tag, true);
        }
        for i in 0..2 {
            auto_then_human(&conn, &format!("EPBAD{i:04}"), &tag, false);
        }

        let disabled = check_and_disable_if_below_target(&conn, tag_id, 0.95).unwrap();
        assert!(disabled);
        assert!(!tags::get(&conn, tag_id).unwrap().unwrap().auto_enabled);
    }

    #[test]
    fn stays_enabled_when_audited_precision_meets_target() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        for i in 0..10 {
            auto_then_human(&conn, &format!("EP{i:07}"), &tag, true);
        }

        let disabled = check_and_disable_if_below_target(&conn, tag_id, 0.95).unwrap();
        assert!(!disabled);
        assert!(tags::get(&conn, tag_id).unwrap().unwrap().auto_enabled);
    }
}
