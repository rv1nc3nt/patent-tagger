//! Prequential evaluation (SPEC section 7.4): scores are recorded *before*
//! the corresponding human label is learned from, so precision/recall
//! measure genuine out-of-sample performance rather than fit-to-training.

use crate::storage::StorageError;
use rusqlite::{params, Connection};

pub fn record(
    conn: &Connection,
    doc_id: i64,
    tag_id: i64,
    model_version: &str,
    score: f32,
    suggested: bool,
    created_at: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO predictions (doc_id, tag_id, model_version, score, suggested, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            doc_id,
            tag_id,
            model_version,
            score,
            suggested as i64,
            created_at
        ],
    )?;
    Ok(())
}

/// SPEC 7.4: "a rolling window of the last 300 validated documents" -
/// interpreted per-tag, as the last 300 predictions recorded for that tag
/// (each one corresponds to a document being validated while carrying that
/// tag), since different tags can have wildly different validation counts.
const WINDOW_SIZE: usize = 300;
/// SPEC 7.4 doesn't fix a threshold for the headline precision/recall
/// figures; 0.5 matches section 7.5's own fallback ("or >= 0.5 if not yet
/// calibrated") - no calibrated per-tag threshold exists until M6.
const DEFAULT_THRESHOLD: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PrPoint {
    pub threshold: f32,
    pub precision: Option<f32>,
    pub recall: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TagMetrics {
    pub tag_id: i64,
    pub support_total: i64,
    pub support_pos: i64,
    pub precision: Option<f32>,
    pub recall: Option<f32>,
    pub pr_curve: Vec<PrPoint>,
}

struct ScoredLabel {
    score: f32,
    is_pos: bool,
}

fn windowed_scored_labels(
    conn: &Connection,
    tag_id: i64,
) -> Result<Vec<ScoredLabel>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT p.score, l.state FROM predictions p
         JOIN labels l ON l.doc_id = p.doc_id AND l.tag_id = p.tag_id AND l.source = 'human'
         WHERE p.tag_id = ?1
         ORDER BY p.id DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![tag_id, WINDOW_SIZE as i64], |row| {
            let score: f64 = row.get(0)?;
            let state: String = row.get(1)?;
            Ok(ScoredLabel {
                score: score as f32,
                is_pos: state == "pos",
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn precision_recall_at(items: &[ScoredLabel], threshold: f32) -> (Option<f32>, Option<f32>) {
    let mut tp = 0u32;
    let mut fp = 0u32;
    let mut fn_ = 0u32;
    for item in items {
        let predicted_pos = item.score >= threshold;
        match (predicted_pos, item.is_pos) {
            (true, true) => tp += 1,
            (true, false) => fp += 1,
            (false, true) => fn_ += 1,
            (false, false) => {}
        }
    }
    let precision = if tp + fp > 0 {
        Some(tp as f32 / (tp + fp) as f32)
    } else {
        None
    };
    let recall = if tp + fn_ > 0 {
        Some(tp as f32 / (tp + fn_) as f32)
    } else {
        None
    };
    (precision, recall)
}

/// SPEC 7.5: eligibility for automatic mode requires at least this many
/// positives and this many evaluated documents (in the same rolling
/// window as the rest of prequential evaluation).
pub const MIN_POSITIVES_FOR_AUTO: i64 = 30;
pub const MIN_EVALUATED_FOR_AUTO: i64 = 150;

/// The lowest threshold (searched on a 0.01 grid) reaching `target_precision`
/// on the window, or `None` if no threshold does (SPEC 7.5).
fn calibrate_from_items(items: &[ScoredLabel], target_precision: f32) -> Option<f32> {
    (0..=100).map(|step| step as f32 * 0.01).find(|&threshold| {
        precision_recall_at(items, threshold)
            .0
            .is_some_and(|p| p >= target_precision)
    })
}

pub fn calibrate_threshold(
    conn: &Connection,
    tag_id: i64,
    target_precision: f32,
) -> Result<Option<f32>, StorageError> {
    let items = windowed_scored_labels(conn, tag_id)?;
    Ok(calibrate_from_items(&items, target_precision))
}

/// SPEC 7.6: "the highest score such that, on prequential data, the
/// positives scoring below it do not exceed 1 - target recall" - restated
/// via `precision_recall_at`'s own recall (fraction of actual positives at
/// or above a threshold): the highest threshold whose recall still meets
/// `target_recall`, since recall only falls (or stays flat) as the
/// threshold rises. `None` when the window has no actual positives at all
/// (recall is undefined then - nothing to calibrate against) or when even
/// threshold 0.0 doesn't reach `target_recall`.
fn calibrate_neg_threshold_from_items(items: &[ScoredLabel], target_recall: f32) -> Option<f32> {
    (0..=100)
        .rev()
        .map(|step| step as f32 * 0.01)
        .find(|&threshold| {
            precision_recall_at(items, threshold)
                .1
                .is_some_and(|r| r >= target_recall)
        })
}

pub fn calibrate_neg_threshold(
    conn: &Connection,
    tag_id: i64,
    target_recall: f32,
) -> Result<Option<f32>, StorageError> {
    let items = windowed_scored_labels(conn, tag_id)?;
    Ok(calibrate_neg_threshold_from_items(&items, target_recall))
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Eligibility {
    pub eligible: bool,
    pub n_pos: i64,
    pub n_evaluated: i64,
    pub calibrated_threshold: Option<f32>,
}

pub fn tag_eligibility(
    conn: &Connection,
    tag_id: i64,
    target_precision: f32,
) -> Result<Eligibility, StorageError> {
    let items = windowed_scored_labels(conn, tag_id)?;
    let n_evaluated = items.len() as i64;
    let n_pos = items.iter().filter(|i| i.is_pos).count() as i64;
    let calibrated_threshold = calibrate_from_items(&items, target_precision);
    let eligible = n_pos >= MIN_POSITIVES_FOR_AUTO
        && n_evaluated >= MIN_EVALUATED_FOR_AUTO
        && calibrated_threshold.is_some();
    Ok(Eligibility {
        eligible,
        n_pos,
        n_evaluated,
        calibrated_threshold,
    })
}

pub fn tag_metrics(conn: &Connection, tag_id: i64) -> Result<TagMetrics, StorageError> {
    let items = windowed_scored_labels(conn, tag_id)?;
    let support_total = items.len() as i64;
    let support_pos = items.iter().filter(|i| i.is_pos).count() as i64;

    let (precision, recall) = precision_recall_at(&items, DEFAULT_THRESHOLD);

    let mut pr_curve = Vec::new();
    let mut step = 0;
    while step <= 20 {
        let threshold = step as f32 * 0.05;
        let (p, r) = precision_recall_at(&items, threshold);
        pr_curve.push(PrPoint {
            threshold,
            precision: p,
            recall: r,
        });
        step += 1;
    }

    Ok(TagMetrics {
        tag_id,
        support_total,
        support_pos,
        precision,
        recall,
        pr_curve,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, labels, storage, tags};
    use std::collections::HashSet;

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn validated_doc_with_prediction(
        conn: &Connection,
        pub_key: &str,
        tag: &tags::TagRow,
        score: f32,
        is_pos: bool,
    ) -> i64 {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        record(
            conn,
            doc.id,
            tag.id,
            "model@v1",
            score,
            score >= DEFAULT_THRESHOLD,
            NOW,
        )
        .unwrap();
        let checked: HashSet<i64> = if is_pos {
            [tag.id].into_iter().collect()
        } else {
            HashSet::new()
        };
        labels::validate_document(conn, doc.id, std::slice::from_ref(tag), &checked, NOW).unwrap();
        doc.id
    }

    #[test]
    fn hand_computed_precision_recall_example() {
        // A deliberately simple, hand-verifiable confusion matrix at the
        // default 0.5 threshold:
        //   score 0.9, actually pos -> TP
        //   score 0.8, actually neg -> FP
        //   score 0.3, actually pos -> FN
        //   score 0.1, actually neg -> TN
        // precision = TP/(TP+FP) = 1/2 = 0.5
        // recall    = TP/(TP+FN) = 1/2 = 0.5
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        validated_doc_with_prediction(&conn, "EP0000001", &tag, 0.9, true);
        validated_doc_with_prediction(&conn, "EP0000002", &tag, 0.8, false);
        validated_doc_with_prediction(&conn, "EP0000003", &tag, 0.3, true);
        validated_doc_with_prediction(&conn, "EP0000004", &tag, 0.1, false);

        let metrics = tag_metrics(&conn, tag_id).unwrap();
        assert_eq!(metrics.support_total, 4);
        assert_eq!(metrics.support_pos, 2);
        assert_eq!(metrics.precision, Some(0.5));
        assert_eq!(metrics.recall, Some(0.5));
    }

    #[test]
    fn pr_curve_has_21_points_from_0_to_1() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        validated_doc_with_prediction(&conn, "EP0000001", &tag, 0.5, true);

        let metrics = tag_metrics(&conn, tag_id).unwrap();
        assert_eq!(metrics.pr_curve.len(), 21);
        assert!((metrics.pr_curve.first().unwrap().threshold - 0.0).abs() < 1e-6);
        assert!((metrics.pr_curve.last().unwrap().threshold - 1.0).abs() < 1e-6);
    }

    #[test]
    fn precision_and_recall_are_none_when_never_predicted_positive() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        validated_doc_with_prediction(&conn, "EP0000001", &tag, 0.1, false);

        let metrics = tag_metrics(&conn, tag_id).unwrap();
        assert_eq!(
            metrics.precision, None,
            "no positive predictions means precision is undefined"
        );
    }

    #[test]
    fn window_keeps_only_the_most_recent_300_predictions() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        for i in 0..305 {
            validated_doc_with_prediction(&conn, &format!("EP{i:07}"), &tag, 0.9, true);
        }

        let metrics = tag_metrics(&conn, tag_id).unwrap();
        assert_eq!(metrics.support_total, 300);
    }

    #[test]
    fn calibrate_threshold_finds_the_lowest_threshold_reaching_target_precision() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        for i in 0..9 {
            validated_doc_with_prediction(&conn, &format!("EPPOS{i:04}"), &tag, 0.8, true);
        }
        // A false positive at 0.7 and another at 0.2: including the 0.2 one
        // drops precision to 9/11 ~= 0.818 (below 0.9); excluding just the
        // 0.2 one (threshold > 0.2) gives 9/10 = 0.9 (meets target).
        validated_doc_with_prediction(&conn, "EPFP0700", &tag, 0.7, false);
        validated_doc_with_prediction(&conn, "EPFP0200", &tag, 0.2, false);

        let threshold = calibrate_threshold(&conn, tag_id, 0.9).unwrap().unwrap();
        assert!(
            (threshold - 0.21).abs() < 1e-6,
            "expected 0.21, got {threshold}"
        );

        let (precision, _) =
            precision_recall_at(&windowed_scored_labels(&conn, tag_id).unwrap(), threshold);
        assert_eq!(precision, Some(0.9));
    }

    #[test]
    fn calibrate_threshold_is_none_when_target_is_unreachable() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        // 1 true positive, 1 false positive at the same score - precision
        // can never exceed 0.5 no matter the threshold.
        validated_doc_with_prediction(&conn, "EPPOS0001", &tag, 0.9, true);
        validated_doc_with_prediction(&conn, "EPFP00001", &tag, 0.9, false);

        assert_eq!(calibrate_threshold(&conn, tag_id, 0.95).unwrap(), None);
    }

    #[test]
    fn calibrate_neg_threshold_finds_the_highest_threshold_reaching_target_recall() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        // 18 positives at 0.9, 2 positives at 0.15 (a false negative risk
        // at low thresholds), plus negatives spread out. At neg_threshold
        // 0.15, both low-scoring positives are still counted (score >=
        // threshold), so recall = 1.0; at 0.16, they'd be missed (recall =
        // 18/20 = 0.9, below a 0.95 target) - so 0.15 is the highest
        // threshold that still reaches target recall.
        for i in 0..18 {
            validated_doc_with_prediction(&conn, &format!("EPPOS{i:04}"), &tag, 0.9, true);
        }
        for i in 0..2 {
            validated_doc_with_prediction(&conn, &format!("EPLOWPOS{i:04}"), &tag, 0.15, true);
        }
        for i in 0..10 {
            validated_doc_with_prediction(&conn, &format!("EPNEG{i:04}"), &tag, 0.05, false);
        }

        let neg_threshold = calibrate_neg_threshold(&conn, tag_id, 0.95)
            .unwrap()
            .unwrap();
        assert!(
            (neg_threshold - 0.15).abs() < 1e-6,
            "expected 0.15, got {neg_threshold}"
        );

        let (_, recall) = precision_recall_at(
            &windowed_scored_labels(&conn, tag_id).unwrap(),
            neg_threshold,
        );
        assert_eq!(recall, Some(1.0));
    }

    #[test]
    fn calibrate_neg_threshold_is_none_without_any_positives() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        validated_doc_with_prediction(&conn, "EPNEG0001", &tag, 0.1, false);

        assert_eq!(calibrate_neg_threshold(&conn, tag_id, 0.95).unwrap(), None);
    }

    #[test]
    fn eligibility_requires_positives_evaluated_count_and_a_reachable_threshold() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        // Only 10 positives, 20 evaluated total - below both minimums.
        for i in 0..10 {
            validated_doc_with_prediction(&conn, &format!("EPPOS{i:04}"), &tag, 0.9, true);
        }
        for i in 0..10 {
            validated_doc_with_prediction(&conn, &format!("EPNEG{i:04}"), &tag, 0.1, false);
        }
        let not_yet = tag_eligibility(&conn, tag_id, 0.95).unwrap();
        assert!(!not_yet.eligible);
        assert_eq!(not_yet.n_pos, 10);
        assert_eq!(not_yet.n_evaluated, 20);

        // Cleanly separated scores, so target precision is always
        // reachable once there's enough data: 30 positives, 120
        // negatives = 150 evaluated, both minimums met.
        for i in 10..30 {
            validated_doc_with_prediction(&conn, &format!("EPPOS{i:04}"), &tag, 0.9, true);
        }
        for i in 10..120 {
            validated_doc_with_prediction(&conn, &format!("EPNEG{i:04}"), &tag, 0.1, false);
        }
        let now_eligible = tag_eligibility(&conn, tag_id, 0.95).unwrap();
        assert!(now_eligible.eligible);
        assert_eq!(now_eligible.n_pos, 30);
        assert_eq!(now_eligible.n_evaluated, 150);
        assert!(now_eligible.calibrated_threshold.is_some());
    }
}
