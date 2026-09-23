//! Full automation (SPEC 7.6): the per-document auto-completion decision,
//! audit sampling, and the Metrics screen's readiness indicator. Pure
//! decision logic here; scoring a document, persisting labels and moving
//! it out of the review queue are `src-tauri`'s job (it owns the
//! `Embedder`, which this network/Tauri-free crate deliberately doesn't
//! depend on).

use crate::storage::StorageError;
use crate::tags::TagRow;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagDecision {
    /// Score >= `threshold`.
    Pos,
    /// Score < `neg_threshold`.
    Neg,
    /// A score exists but falls between `neg_threshold` and `threshold`
    /// (or one/both aren't calibrated yet), or no score exists at all.
    Uncertain,
    /// The tag isn't in automatic mode - never contributes to
    /// auto-completion (SPEC 7.5: "outside full automation, the system
    /// never writes automatic negatives"; SPEC 7.6: "creating a tag stops
    /// auto-completion until eligible").
    NotAutomatic,
}

/// SPEC 7.6's per-tag half of the per-document decision.
pub fn decide_tag(tag: &TagRow, score: Option<f32>) -> TagDecision {
    if !tag.auto_enabled {
        return TagDecision::NotAutomatic;
    }
    let Some(score) = score else {
        return TagDecision::Uncertain;
    };
    if tag.threshold.is_some_and(|t| score >= t) {
        TagDecision::Pos
    } else if tag.neg_threshold.is_some_and(|nt| score < nt) {
        TagDecision::Neg
    } else {
        TagDecision::Uncertain
    }
}

/// SPEC 7.6: "a document is auto-completed when, for every non-archived
/// tag: the tag is in automatic mode, and its score is either >=
/// threshold...or < neg_threshold." `decisions` must cover every active
/// tag - an empty list (no active tags at all) is never auto-completed,
/// since there is nothing to have confidently decided.
pub fn would_auto_complete(decisions: &[TagDecision]) -> bool {
    !decisions.is_empty() && decisions.iter().all(|d| matches!(d, TagDecision::Pos | TagDecision::Neg))
}

/// SPEC 7.6: "a fraction (default 5%) of auto-completed documents is
/// sampled into the review queue as a full review." A cheap
/// multiplicative hash of `doc_id` mapped to `[0, 1)`, rather than true
/// randomness: the same document always samples the same way (given a
/// fixed `rate`), which keeps this deterministic and testable, and avoids
/// adding a `rand` dependency to this crate purely for one boolean coin
/// flip (`crates/ops` already depends on `rand` for jitter, but `crates/
/// core` deliberately has no network/IO dependencies to keep minimal).
pub fn is_sampled_for_audit(doc_id: i64, rate: f32) -> bool {
    let h = (doc_id as u64).wrapping_mul(2_654_435_761).wrapping_add(0x9E37_79B9_7F4A_7C15);
    let h = (h ^ (h >> 15)).wrapping_mul(0x2545_F491_4F6C_DD1D);
    let unit = (h >> 32) as u32 as f32 / u32::MAX as f32;
    unit < rate
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ReadinessResult {
    pub would_auto_complete: i64,
    pub total: i64,
}

/// SPEC 7.6: "the Metrics screen shows the share of the last 300 documents
/// that would have been auto-completed with the current settings." Reuses
/// each document's most recently recorded score per tag from `predictions`
/// (already written at every validation, SPEC 7.4) rather than re-scoring
/// documents live, so this needs no `Embedder` and stays purely a replay
/// of recorded history against *current* tag settings.
pub fn auto_completion_readiness(conn: &Connection) -> Result<ReadinessResult, StorageError> {
    let active_tags = crate::tags::list_active(conn)?;

    let mut doc_stmt = conn.prepare(
        "SELECT doc_id FROM (SELECT doc_id, MAX(id) AS last_id FROM predictions GROUP BY doc_id)
         ORDER BY last_id DESC LIMIT 300",
    )?;
    let doc_ids: Vec<i64> = doc_stmt.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;

    let mut score_stmt = conn.prepare(
        "SELECT score FROM predictions WHERE doc_id = ?1 AND tag_id = ?2 ORDER BY id DESC LIMIT 1",
    )?;

    let mut would_auto_complete_count = 0;
    for doc_id in &doc_ids {
        let mut decisions = Vec::with_capacity(active_tags.len());
        for tag in &active_tags {
            let score: Option<f64> = score_stmt
                .query_row(params![doc_id, tag.id], |row| row.get(0))
                .optional()?;
            decisions.push(decide_tag(tag, score.map(|s| s as f32)));
        }
        if would_auto_complete(&decisions) {
            would_auto_complete_count += 1;
        }
    }

    Ok(ReadinessResult { would_auto_complete: would_auto_complete_count, total: doc_ids.len() as i64 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, predictions, storage, tags};

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn tag_with(threshold: Option<f32>, neg_threshold: Option<f32>, auto_enabled: bool) -> TagRow {
        TagRow {
            id: 1,
            name: "Battery".to_string(),
            definition: "About batteries".to_string(),
            color: None,
            hotkey: None,
            version: 1,
            archived: false,
            threshold,
            neg_threshold,
            auto_enabled,
        }
    }

    #[test]
    fn decide_tag_not_automatic_when_auto_mode_is_off() {
        let tag = tag_with(Some(0.8), Some(0.2), false);
        assert_eq!(decide_tag(&tag, Some(0.9)), TagDecision::NotAutomatic);
    }

    #[test]
    fn decide_tag_pos_at_or_above_threshold() {
        let tag = tag_with(Some(0.8), Some(0.2), true);
        assert_eq!(decide_tag(&tag, Some(0.8)), TagDecision::Pos);
        assert_eq!(decide_tag(&tag, Some(0.95)), TagDecision::Pos);
    }

    #[test]
    fn decide_tag_neg_below_neg_threshold() {
        let tag = tag_with(Some(0.8), Some(0.2), true);
        assert_eq!(decide_tag(&tag, Some(0.1)), TagDecision::Neg);
    }

    #[test]
    fn decide_tag_uncertain_in_the_gap_or_without_a_score() {
        let tag = tag_with(Some(0.8), Some(0.2), true);
        assert_eq!(decide_tag(&tag, Some(0.5)), TagDecision::Uncertain);
        assert_eq!(decide_tag(&tag, None), TagDecision::Uncertain);

        // Not yet calibrated at all.
        let uncalibrated = tag_with(None, None, true);
        assert_eq!(decide_tag(&uncalibrated, Some(0.99)), TagDecision::Uncertain);
    }

    #[test]
    fn would_auto_complete_requires_every_tag_decided_and_none_uncertain() {
        assert!(would_auto_complete(&[TagDecision::Pos, TagDecision::Neg]));
        assert!(!would_auto_complete(&[TagDecision::Pos, TagDecision::Uncertain]));
        assert!(!would_auto_complete(&[TagDecision::Pos, TagDecision::NotAutomatic]));
        assert!(!would_auto_complete(&[]), "no tags at all is never auto-completed");
    }

    #[test]
    fn audit_sampling_is_deterministic_and_roughly_matches_the_rate() {
        assert_eq!(is_sampled_for_audit(42, 0.1), is_sampled_for_audit(42, 0.1));
        let sampled = (0..10_000).filter(|&id| is_sampled_for_audit(id, 0.05)).count();
        let fraction = sampled as f32 / 10_000.0;
        assert!((fraction - 0.05).abs() < 0.01, "expected roughly 5%, got {:.3}", fraction);
    }

    #[test]
    fn audit_sampling_never_selects_at_rate_zero_or_always_at_rate_one() {
        assert!((0..1000).all(|id| !is_sampled_for_audit(id, 0.0)));
        assert!((0..1000).all(|id| is_sampled_for_audit(id, 1.0)));
    }

    fn doc_with_score(conn: &Connection, pub_key: &str, tag_id: i64, score: f32) -> i64 {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        predictions::record(conn, doc.id, tag_id, "model@v1", score, score >= 0.5, NOW).unwrap();
        doc.id
    }

    #[test]
    fn readiness_counts_documents_whose_recorded_scores_would_now_auto_complete() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        doc_with_score(&conn, "EPPOS0001", tag_id, 0.9); // would auto-complete (pos)
        doc_with_score(&conn, "EPNEG0001", tag_id, 0.1); // would auto-complete (neg)
        doc_with_score(&conn, "EPUNC0001", tag_id, 0.5); // uncertain - would not

        let readiness = auto_completion_readiness(&conn).unwrap();
        assert_eq!(readiness.total, 3);
        assert_eq!(readiness.would_auto_complete, 2);
    }

    #[test]
    fn readiness_reflects_current_settings_not_settings_at_scoring_time() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        // No threshold/auto_enabled set yet when the score was recorded.
        doc_with_score(&conn, "EPPOS0001", tag_id, 0.9);

        let before = auto_completion_readiness(&conn).unwrap();
        assert_eq!(before.would_auto_complete, 0, "the tag isn't in automatic mode yet");

        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        let after = auto_completion_readiness(&conn).unwrap();
        assert_eq!(after.would_auto_complete, 1, "replaying the same recorded score against the new settings");
    }

    #[test]
    fn readiness_window_keeps_only_the_last_300_documents() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        tags::set_threshold(&conn, tag_id, Some(0.8)).unwrap();
        tags::set_neg_threshold(&conn, tag_id, Some(0.2)).unwrap();
        tags::set_auto_enabled(&conn, tag_id, true).unwrap();

        for i in 0..305 {
            doc_with_score(&conn, &format!("EP{i:07}"), tag_id, 0.9);
        }

        let readiness = auto_completion_readiness(&conn).unwrap();
        assert_eq!(readiness.total, 300);
        assert_eq!(readiness.would_auto_complete, 300);
    }
}
