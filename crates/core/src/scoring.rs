//! Zero-shot and k-NN scorers (SPEC section 7.2). Pure functions over
//! already-loaded vectors/labels - no DB access here, so the scoring math
//! itself is directly unit-testable; `crates/core::labels`/`::embeddings`
//! supply the inputs.

use crate::labels::LabelState;
use std::collections::HashMap;

const K_NEIGHBOURS: usize = 10;
/// SPEC 7.2: zero-shot is used only below this many positives for a tag.
pub const ZERO_SHOT_POSITIVE_CEILING: i64 = 3;

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Cosine similarity of the document embedding to the tag's `"{name}:
/// {definition}"` embedding, mapped through a fixed monotonic function.
/// SPEC doesn't specify which function; `(cosine + 1) / 2` is the simplest
/// one that's monotonic and maps `[-1, 1]` onto `[0, 1]` (see
/// docs/DECISIONS.md).
pub fn zero_shot_score(doc_vector: &[f32], tag_vector: &[f32]) -> f32 {
    ((cosine_similarity(doc_vector, tag_vector) + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// Combines whichever of LR/k-NN/zero-shot are available (SPEC 7.3):
/// `w = n_pos / (n_pos + 20)`, `score = w*LR + (1-w)*kNN` when both exist;
/// otherwise whichever single one exists; otherwise zero-shot, if it's
/// even in play (it only is below [`ZERO_SHOT_POSITIVE_CEILING`] positives,
/// per SPEC 7.2 - `zero_shot` should already be `None` above that, which
/// callers get by only passing `Some` when `n_pos < ZERO_SHOT_POSITIVE_CEILING`).
/// `None` when nothing is available at all.
pub fn blend_scores(
    lr: Option<f32>,
    knn: Option<f32>,
    zero_shot: Option<f32>,
    n_pos: i64,
) -> Option<(f32, &'static str)> {
    match (lr, knn) {
        (Some(lr), Some(knn)) => {
            let w = n_pos as f32 / (n_pos as f32 + 20.0);
            Some((w * lr + (1.0 - w) * knn, "blend"))
        }
        (Some(lr), None) => Some((lr, "lr")),
        (None, Some(knn)) => Some((knn, "knn")),
        (None, None) => zero_shot.map(|s| (s, "zero_shot")),
    }
}

/// SPEC 7.2: take the k nearest validated documents by cosine similarity,
/// keep only the ones with a human label for this tag, and return the
/// similarity-weighted fraction of positives (negative similarities
/// clipped to 0). `None` when no neighbour has a label for this tag, or
/// when every labelled neighbour has non-positive similarity (so the
/// weighted average would be 0/0).
pub fn knn_score(
    doc_vector: &[f32],
    // (doc_id, embedding) for every validated document with an embedding.
    validated_neighbours: &[(i64, Vec<f32>)],
    // doc_id -> this tag's human label for that document, if any.
    labels_for_tag: &HashMap<i64, LabelState>,
) -> Option<f32> {
    let mut by_similarity: Vec<(i64, f32)> = validated_neighbours
        .iter()
        .map(|(id, vec)| (*id, cosine_similarity(doc_vector, vec)))
        .collect();
    by_similarity.sort_by(|a, b| b.1.total_cmp(&a.1));
    by_similarity.truncate(K_NEIGHBOURS);

    let mut weighted_positive = 0.0f32;
    let mut weight_total = 0.0f32;
    let mut any_labelled = false;

    for (doc_id, similarity) in by_similarity {
        let Some(label) = labels_for_tag.get(&doc_id) else {
            continue;
        };
        any_labelled = true;
        let weight = similarity.max(0.0);
        weight_total += weight;
        if *label == LabelState::Pos {
            weighted_positive += weight;
        }
    }

    if !any_labelled || weight_total == 0.0 {
        return None;
    }
    Some(weighted_positive / weight_total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_weights_lr_more_heavily_as_positives_grow() {
        let (low_n, _) = blend_scores(Some(1.0), Some(0.0), None, 0).unwrap();
        let (mid_n, _) = blend_scores(Some(1.0), Some(0.0), None, 20).unwrap();
        let (high_n, _) = blend_scores(Some(1.0), Some(0.0), None, 1000).unwrap();
        assert!(low_n < mid_n);
        assert!(mid_n < high_n);
        assert!(
            (low_n - 0.0).abs() < 1e-6,
            "at n_pos=0, w=0, so score should be all k-NN"
        );
        assert!((mid_n - 0.5).abs() < 1e-6, "at n_pos=20, w=0.5 exactly");
        assert!(
            high_n > 0.9,
            "at n_pos=1000, w should be close to 1 (mostly LR)"
        );
    }

    #[test]
    fn blend_falls_back_to_whichever_single_scorer_is_available() {
        assert_eq!(blend_scores(Some(0.7), None, None, 10), Some((0.7, "lr")));
        assert_eq!(blend_scores(None, Some(0.3), None, 10), Some((0.3, "knn")));
    }

    #[test]
    fn blend_falls_back_to_zero_shot_only_when_nothing_else_is_available() {
        assert_eq!(
            blend_scores(None, None, Some(0.6), 1),
            Some((0.6, "zero_shot"))
        );
        assert_eq!(blend_scores(None, None, None, 1), None);
    }

    #[test]
    fn zero_shot_maps_identical_vectors_to_one() {
        let v = vec![0.6, 0.8];
        assert!((zero_shot_score(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zero_shot_maps_opposite_vectors_to_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((zero_shot_score(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn zero_shot_maps_orthogonal_vectors_to_half() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((zero_shot_score(&a, &b) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn knn_is_undefined_when_no_neighbour_has_the_tag_label() {
        let doc = vec![1.0, 0.0];
        let neighbours = vec![(1, vec![1.0, 0.0]), (2, vec![0.9, 0.1])];
        let labels = HashMap::new();
        assert_eq!(knn_score(&doc, &neighbours, &labels), None);
    }

    #[test]
    fn knn_is_similarity_weighted() {
        let doc = vec![1.0, 0.0];
        // doc_1 is an exact match (similarity 1.0, positive), doc_2 is
        // orthogonal (similarity 0.0, negative) - the orthogonal one
        // should barely affect the score, but must not be excluded from
        // "any_labelled" bookkeeping either.
        let neighbours = vec![(1, vec![1.0, 0.0]), (2, vec![0.0, 1.0])];
        let mut labels = HashMap::new();
        labels.insert(1, LabelState::Pos);
        labels.insert(2, LabelState::Neg);

        let score = knn_score(&doc, &neighbours, &labels).unwrap();
        assert!(
            (score - 1.0).abs() < 1e-6,
            "the only-weighted neighbour is fully positive"
        );
    }

    #[test]
    fn knn_clips_negative_similarity_to_zero_weight() {
        let doc = vec![1.0, 0.0];
        // doc_1: similarity -1.0 (opposite direction), labelled positive.
        // With clipping, its weight is 0, so it should not contribute -
        // and since it is the only labelled neighbour, the total weight
        // is 0 and the score is undefined.
        let neighbours = vec![(1, vec![-1.0, 0.0])];
        let mut labels = HashMap::new();
        labels.insert(1, LabelState::Pos);

        assert_eq!(knn_score(&doc, &neighbours, &labels), None);
    }

    #[test]
    fn knn_only_considers_the_k_nearest_neighbours() {
        let doc = vec![1.0, 0.0];
        // 11 neighbours: the 10 nearest are all negative, the single
        // farthest (least similar) is positive and should be excluded.
        let mut neighbours: Vec<(i64, Vec<f32>)> = (0..10)
            .map(|i| (i, vec![1.0 - i as f32 * 0.01, 0.01]))
            .collect();
        neighbours.push((99, vec![-1.0, 0.0]));
        let mut labels = HashMap::new();
        for i in 0..10 {
            labels.insert(i, LabelState::Neg);
        }
        labels.insert(99, LabelState::Pos);

        // All 10 nearest are negative with positive similarity, so the
        // score should be a well-defined 0.0, not None and not pulled
        // toward the excluded 11th (farthest) positive neighbour.
        let score = knn_score(&doc, &neighbours, &labels).unwrap();
        assert!((score - 0.0).abs() < 1e-6);
    }
}
