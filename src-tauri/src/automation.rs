//! Applying automatic labels for tags in automatic mode (SPEC 7.5), and,
//! when full automation is enabled, the per-document auto-completion
//! decision, audit sampling and suspension (SPEC 7.6).

use core_lib::full_automation::{self, TagDecision};
use core_lib::rusqlite::Connection;
use core_lib::{audit, documents, embeddings, labels, settings, storage, tags};
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
    for tag in tags::list_active(conn)?
        .into_iter()
        .filter(|t| t.auto_enabled)
    {
        let Some(threshold) = tag.threshold else {
            continue;
        };

        for doc in documents::list_queue(conn)? {
            if labels::has_any_label(conn, doc.id, tag.id)? {
                continue;
            }
            let Some(doc_vector) =
                embeddings::get_document_embedding(conn, doc.id, embedder.model_id())?
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

/// SPEC 7.6: checks `tag_id`'s combined audited precision/recall (over
/// the *shared* last-50-audits window - see docs/DECISIONS.md) and
/// suspends automatic mode if either falls below its target, with at
/// least [`MIN_AUDITED_BEFORE_JUDGING`] audited decisions to judge from.
/// Distinct from [`check_and_disable_if_below_target`] (SPEC 7.5's
/// simpler, precision-only, per-tag safeguard) - both end up flipping the
/// same `auto_enabled` flag, but on different evidence.
pub fn check_and_suspend_for_full_automation(
    conn: &Connection,
    tag_id: i64,
    target_precision: f32,
    target_recall: f32,
) -> Result<bool, storage::StorageError> {
    let Some(tag) = tags::get(conn, tag_id)? else {
        return Ok(false);
    };
    if !tag.auto_enabled {
        return Ok(false);
    }
    let (precision, recall, n) = audit::auto_completion_audit(conn, tag_id)?;
    let should_suspend = n >= MIN_AUDITED_BEFORE_JUDGING
        && (precision.is_some_and(|p| p < target_precision)
            || recall.is_some_and(|r| r < target_recall));
    if should_suspend {
        tags::set_auto_enabled(conn, tag_id, false)?;
    }
    Ok(should_suspend)
}

/// SPEC 7.6's per-document decision, run after [`apply_pending_automatic_labels`]
/// (so tags already decided `pos` there are reused, not re-scored). Only
/// takes effect when full automation is enabled; a no-op summary
/// otherwise, since section 7.6 is entirely gated behind that setting.
/// For every still-queued document where every non-archived tag ends up
/// `Pos` or `Neg` (via an existing label or a freshly-written automatic
/// `neg`, SPEC 7.5: never written outside full automation), the document
/// either becomes `auto_completed` or, if sampled for audit (SPEC 7.6:
/// "a fraction...sampled into the review queue as a full review"), stays
/// queued with every tag already pre-filled as automatic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FullAutomationSummary {
    pub auto_completed: usize,
    pub audited_samples: usize,
}

pub fn apply_full_automation(
    conn: &Connection,
    embedder: &impl Embedder,
    now: &str,
) -> Result<FullAutomationSummary, storage::StorageError> {
    apply_pending_automatic_labels(conn, embedder, now)?;

    let mut summary = FullAutomationSummary::default();
    if !settings::full_automation_enabled(conn)? {
        return Ok(summary);
    }

    let active_tags = tags::list_active(conn)?;
    let audit_rate = settings::audit_rate(conn)?;

    for doc in documents::list_queue(conn)? {
        let mut decisions = Vec::with_capacity(active_tags.len());
        let mut scores: Vec<(i64, Option<f32>, String)> = Vec::with_capacity(active_tags.len());

        for tag in &active_tags {
            if let Some((state, confidence, model_version)) =
                labels::get_label(conn, doc.id, tag.id)?
            {
                let decision = match state {
                    labels::LabelState::Pos => TagDecision::Pos,
                    labels::LabelState::Neg => TagDecision::Neg,
                };
                decisions.push(decision);
                scores.push((tag.id, confidence, model_version.unwrap_or_default()));
                continue;
            }

            if !tag.auto_enabled {
                decisions.push(TagDecision::NotAutomatic);
                scores.push((tag.id, None, String::new()));
                continue;
            }

            let Some(doc_vector) =
                embeddings::get_document_embedding(conn, doc.id, embedder.model_id())?
            else {
                decisions.push(TagDecision::Uncertain);
                scores.push((tag.id, None, String::new()));
                continue;
            };
            let mut neighbours = None;
            let (score, _source, model_version) =
                crate::review::score_one_tag(conn, embedder, tag, &doc_vector, &mut neighbours)?;
            let decision = full_automation::decide_tag(tag, score);
            if decision == TagDecision::Neg {
                if let Some(s) = score {
                    labels::write_automatic_neg_label(
                        conn,
                        doc.id,
                        tag.id,
                        s,
                        &model_version,
                        tag.version,
                        now,
                    )?;
                }
            }
            decisions.push(decision);
            scores.push((tag.id, score, model_version));
        }

        if !full_automation::would_auto_complete(&decisions) {
            continue;
        }

        if full_automation::is_sampled_for_audit(doc.id, audit_rate) {
            summary.audited_samples += 1;
            continue;
        }

        // SPEC 7.4: record every tag's score before this document leaves
        // the review pipeline for good, so it still contributes to the
        // prequential log and the readiness indicator (see
        // docs/DECISIONS.md) even though no human ever validates it.
        for (tag_id, score, model_version) in &scores {
            if let Some(score) = score {
                core_lib::predictions::record(
                    conn,
                    doc.id,
                    *tag_id,
                    model_version,
                    *score,
                    true,
                    now,
                )?;
            }
        }
        documents::mark_auto_completed(conn, doc.id)?;
        summary.auto_completed += 1;

        // SPEC 5.5: the "after tagging" retrieval policies fire on
        // validation *or* auto-completion - `validate_document` triggers
        // this same check for the human path.
        let positive_tag_ids: std::collections::HashSet<i64> = active_tags
            .iter()
            .zip(&decisions)
            .filter(|(_, d)| **d == TagDecision::Pos)
            .map(|(t, _)| t.id)
            .collect();
        crate::commands::enqueue_retrieval_after_tagging(conn, doc.id, &positive_tag_ids, now)?;
    }

    Ok(summary)
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
        let doc = documents::find_by_pub_key(&conn, "EP0000001")
            .unwrap()
            .unwrap();
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
        let doc = documents::find_by_pub_key(&conn, "EP0000001")
            .unwrap()
            .unwrap();
        core_lib::labels::validate_document(
            &conn,
            doc.id,
            std::slice::from_ref(&tag),
            &HashSet::new(),
            NOW,
        )
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
        let checked: HashSet<i64> = if human_confirms {
            [tag.id].into_iter().collect()
        } else {
            HashSet::new()
        };
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

    fn auto_neg_then_human(
        conn: &Connection,
        pub_key: &str,
        tag: &tags::TagRow,
        human_confirms_neg: bool,
    ) {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        labels::write_automatic_neg_label(conn, doc.id, tag.id, 0.05, "model@v1", 1, NOW).unwrap();
        let checked: HashSet<i64> = if human_confirms_neg {
            HashSet::new()
        } else {
            [tag.id].into_iter().collect()
        };
        labels::validate_document(conn, doc.id, std::slice::from_ref(tag), &checked, NOW).unwrap();
    }

    #[test]
    fn suspends_for_full_automation_when_audited_recall_drops_even_if_precision_is_fine() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        // Perfect precision (10/10 confirmed pos), but recall drops: 8
        // confirmed true negatives and 4 overturned (missed positives) ->
        // recall = 10/14 ~= 0.71, below a 0.95 target.
        for i in 0..10 {
            auto_then_human(&conn, &format!("EPTP{i:04}"), &tag, true);
        }
        for i in 0..8 {
            auto_neg_then_human(&conn, &format!("EPTN{i:04}"), &tag, true);
        }
        for i in 0..4 {
            auto_neg_then_human(&conn, &format!("EPFN{i:04}"), &tag, false);
        }

        let suspended = check_and_suspend_for_full_automation(&conn, tag_id, 0.95, 0.95).unwrap();
        assert!(suspended);
        assert!(!tags::get(&conn, tag_id).unwrap().unwrap().auto_enabled);
    }

    #[test]
    fn stays_enabled_for_full_automation_when_both_precision_and_recall_meet_target() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        for i in 0..15 {
            auto_then_human(&conn, &format!("EPTP{i:04}"), &tag, true);
        }
        for i in 0..15 {
            auto_neg_then_human(&conn, &format!("EPTN{i:04}"), &tag, true);
        }

        let suspended = check_and_suspend_for_full_automation(&conn, tag_id, 0.95, 0.95).unwrap();
        assert!(!suspended);
        assert!(tags::get(&conn, tag_id).unwrap().unwrap().auto_enabled);
    }

    struct DirectionEmbedder;
    impl Embedder for DirectionEmbedder {
        fn model_id(&self) -> &str {
            "direction-model@v1"
        }
        fn dim(&self) -> usize {
            2
        }
        fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, embed_lib::EmbedError> {
            unreachable!("this test only scores documents that already have a stored embedding")
        }
    }

    fn fetched_doc_with_embedding(conn: &Connection, pub_key: &str, vector: &[f32]) -> i64 {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        documents::store_fetched(
            conn,
            doc.id,
            &documents::FetchedData {
                title: Some("A gadget".to_string()),
                abstract_text: Some("An abstract".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        embeddings::store_document_embedding(conn, doc.id, "direction-model@v1", vector).unwrap();
        doc.id
    }

    #[test]
    fn apply_full_automation_is_a_noop_when_the_setting_is_off() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();
        embeddings::store_tag_embedding(&conn, tag_id, "direction-model@v1", 1, &[0.0, 1.0])
            .unwrap();

        // Score near -1 for the tag direction, comfortably below any
        // neg_threshold once scored - but full automation is off, so no
        // negative should ever be written and nothing should complete.
        fetched_doc_with_embedding(&conn, "EP0000001", &[0.0, -1.0]);

        let summary = apply_full_automation(&conn, &DirectionEmbedder, NOW).unwrap();
        assert_eq!(summary, FullAutomationSummary::default());

        let doc = documents::find_by_pub_key(&conn, "EP0000001")
            .unwrap()
            .unwrap();
        assert_eq!(doc.review_state, "queued");
    }

    #[test]
    fn apply_full_automation_completes_a_document_decided_on_every_tag() {
        let conn = storage::open_in_memory().expect("in-memory db");
        settings::set(&conn, settings::FULL_AUTOMATION_ENABLED_KEY, "true").unwrap();
        settings::set(&conn, settings::AUDIT_RATE_KEY, "0").unwrap();

        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();
        embeddings::store_tag_embedding(&conn, tag_id, "direction-model@v1", 1, &[0.0, 1.0])
            .unwrap();

        let doc_id = fetched_doc_with_embedding(&conn, "EP0000001", &[0.0, -1.0]);

        let summary = apply_full_automation(&conn, &DirectionEmbedder, NOW).unwrap();
        assert_eq!(summary.auto_completed, 1);
        assert_eq!(summary.audited_samples, 0);

        let doc = documents::find_by_pub_key(&conn, "EP0000001")
            .unwrap()
            .unwrap();
        assert_eq!(doc.review_state, "auto_completed");

        let (state, _, _) = labels::get_label(&conn, doc_id, tag_id).unwrap().unwrap();
        assert_eq!(state, labels::LabelState::Neg);

        // Not counted in tag_metrics (that window pairs predictions with a
        // *human* label specifically), but does feed the readiness
        // indicator, which just replays recorded scores (see
        // docs/DECISIONS.md).
        let readiness = full_automation::auto_completion_readiness(&conn).unwrap();
        assert_eq!(
            readiness.total, 1,
            "the auto-completed document's score should be recorded"
        );
        assert_eq!(readiness.would_auto_complete, 1);
    }

    #[test]
    fn apply_full_automation_enqueues_retrieval_for_an_auto_completed_document() {
        let conn = storage::open_in_memory().expect("in-memory db");
        settings::set(&conn, settings::FULL_AUTOMATION_ENABLED_KEY, "true").unwrap();
        settings::set(&conn, settings::AUDIT_RATE_KEY, "0").unwrap();
        settings::set(
            &conn,
            core_lib::settings::FULLTEXT_POLICY_KEY,
            "after_tagging_all",
        )
        .unwrap();

        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();
        embeddings::store_tag_embedding(&conn, tag_id, "direction-model@v1", 1, &[0.0, 1.0])
            .unwrap();

        let doc_id = fetched_doc_with_embedding(&conn, "EP0000001", &[0.0, 1.0]);

        let summary = apply_full_automation(&conn, &DirectionEmbedder, NOW).unwrap();
        assert_eq!(summary.auto_completed, 1);

        let jobs =
            core_lib::jobs::list_resumable(&conn, crate::retrieval_worker::FULLTEXT_JOB_KIND)
                .unwrap();
        assert_eq!(jobs.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&jobs[0].payload).unwrap();
        assert_eq!(payload["doc_id"].as_i64(), Some(doc_id));
    }

    #[test]
    fn apply_full_automation_routes_a_would_be_completion_to_audit_instead_when_sampled() {
        let conn = storage::open_in_memory().expect("in-memory db");
        settings::set(&conn, settings::FULL_AUTOMATION_ENABLED_KEY, "true").unwrap();
        settings::set(&conn, settings::AUDIT_RATE_KEY, "1").unwrap();

        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();
        embeddings::store_tag_embedding(&conn, tag_id, "direction-model@v1", 1, &[0.0, 1.0])
            .unwrap();

        fetched_doc_with_embedding(&conn, "EP0000001", &[0.0, -1.0]);

        let summary = apply_full_automation(&conn, &DirectionEmbedder, NOW).unwrap();
        assert_eq!(summary.auto_completed, 0);
        assert_eq!(summary.audited_samples, 1);

        let doc = documents::find_by_pub_key(&conn, "EP0000001")
            .unwrap()
            .unwrap();
        assert_eq!(
            doc.review_state, "queued",
            "an audit-sampled document stays in the queue for full review"
        );
    }

    #[test]
    fn apply_full_automation_leaves_a_document_queued_when_a_tag_is_not_automatic() {
        let conn = storage::open_in_memory().expect("in-memory db");
        settings::set(&conn, settings::FULL_AUTOMATION_ENABLED_KEY, "true").unwrap();

        let auto_tag_id =
            tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, auto_tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, auto_tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, auto_tag_id, true).unwrap();
        embeddings::store_tag_embedding(&conn, auto_tag_id, "direction-model@v1", 1, &[0.0, 1.0])
            .unwrap();

        // A second, brand-new tag not yet in automatic mode - SPEC 7.6:
        // "creating a tag stops auto-completion until eligible."
        tags::create(&conn, "Solar", "About solar power", None, None, NOW).unwrap();

        fetched_doc_with_embedding(&conn, "EP0000001", &[0.0, -1.0]);

        let summary = apply_full_automation(&conn, &DirectionEmbedder, NOW).unwrap();
        assert_eq!(summary.auto_completed, 0);

        let doc = documents::find_by_pub_key(&conn, "EP0000001")
            .unwrap()
            .unwrap();
        assert_eq!(doc.review_state, "queued");
    }
}
