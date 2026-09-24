//! SPEC section 7 intro: "unit-tested on synthetic data (clustered vectors
//! with known tags)". This test builds vectors where the real signal lives
//! in a handful of dimensions and the rest is noise (comparable to how a
//! real 384-dim sentence embedding carries a tag's signal in only part of
//! its dimensions) - exactly the setting where logistic regression, which
//! learns to weight the informative dimensions, should outperform k-NN,
//! which treats every dimension equally via raw cosine similarity. SPEC
//! section 10's M5 acceptance criterion is this outperformance "once data
//! is sufficient" - demonstrated here at n=100/class vs. n=10/class.

use core_lib::labels::LabelState;
use core_lib::{classifier, documents, labels, predictions, scoring, storage, tags};
use std::collections::{HashMap, HashSet};

const DIM: usize = 384;
const SIGNAL_DIMS: usize = 4;

/// Deterministic pseudo-random value in `[-1, 1]` - a cheap multiplicative
/// hash, not `rand`, so this stays a plain-Rust, no-new-dependency test.
fn pseudo_noise(seed: u64, dim: usize) -> f32 {
    let h = seed
        .wrapping_mul(2_654_435_761)
        .wrapping_add((dim as u64).wrapping_mul(40_503));
    let h = (h ^ (h >> 15)).wrapping_mul(0x2545F4914F6CDD1D);
    ((h >> 32) as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn make_sample(seed: u64, is_positive: bool) -> Vec<f32> {
    let signal = if is_positive { 1.0 } else { -1.0 };
    let mut v = vec![0.0f32; DIM];
    for (d, value) in v.iter_mut().enumerate().take(SIGNAL_DIMS) {
        *value = signal + pseudo_noise(seed, d) * 0.3;
    }
    for (d, value) in v.iter_mut().enumerate().skip(SIGNAL_DIMS) {
        *value = pseudo_noise(seed, d) * 2.0;
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn dataset(n_per_class: usize, seed_offset: u64) -> Vec<(Vec<f32>, bool)> {
    (0..n_per_class)
        .flat_map(|i| {
            let seed = seed_offset + i as u64;
            [
                (make_sample(seed * 2, true), true),
                (make_sample(seed * 2 + 1, false), false),
            ]
        })
        .collect()
}

fn knn_accuracy(train: &[(Vec<f32>, bool)], test: &[(Vec<f32>, bool)]) -> f32 {
    let neighbours: Vec<(i64, Vec<f32>)> = train
        .iter()
        .enumerate()
        .map(|(i, (v, _))| (i as i64, v.clone()))
        .collect();
    let labels: HashMap<i64, LabelState> = train
        .iter()
        .enumerate()
        .map(|(i, (_, y))| (i as i64, if *y { LabelState::Pos } else { LabelState::Neg }))
        .collect();

    let correct = test
        .iter()
        .filter(|(x, y)| {
            let score = scoring::knn_score(x, &neighbours, &labels).unwrap_or(0.5);
            (score >= 0.5) == *y
        })
        .count();
    correct as f32 / test.len() as f32
}

fn lr_accuracy(train: &[(Vec<f32>, bool)], test: &[(Vec<f32>, bool)]) -> f32 {
    let clf = classifier::train(train, classifier::default_l2_lambda()).expect("should train");
    let correct = test
        .iter()
        .filter(|(x, y)| (clf.predict(x) >= 0.5) == *y)
        .count();
    correct as f32 / test.len() as f32
}

#[test]
fn logistic_regression_outperforms_knn_once_data_is_sufficient() {
    let test_set = dataset(100, 100_000); // held-out, disjoint seed range

    // With very little training data, neither scorer has enough signal to
    // reliably beat chance - both should be mediocre, not a real basis for
    // comparison.
    let small_train = dataset(classifier::MIN_POSITIVES, 0);
    let small_knn = knn_accuracy(&small_train, &test_set);
    let small_lr = lr_accuracy(&small_train, &test_set);
    println!("n=5/class: knn={small_knn:.2}, lr={small_lr:.2}");

    // With substantially more training data, LR should learn to weight the
    // signal dimensions and clearly outperform k-NN, which stays confused
    // by the noise dimensions regardless of how much data it has (it
    // still only ever looks at the nearest 10 by raw cosine similarity).
    let large_train = dataset(100, 10_000);
    let large_knn = knn_accuracy(&large_train, &test_set);
    let large_lr = lr_accuracy(&large_train, &test_set);
    println!("n=100/class: knn={large_knn:.2}, lr={large_lr:.2}");

    assert!(
        large_lr > large_knn + 0.1,
        "expected LR to clearly outperform k-NN with sufficient data: lr={large_lr:.2}, knn={large_knn:.2}"
    );
    assert!(
        large_lr > 0.9,
        "LR should be quite accurate with 100 samples/class: {large_lr:.2}"
    );
}

/// SPEC 10's M9 acceptance criterion: "neg_threshold calibration tested on
/// synthetic data" (SPEC 7.6's "confident absence" threshold). Trains an
/// LR classifier on clustered synthetic data, scores a calibration set,
/// records those scores/labels through the real `predictions`/`labels`
/// tables (exactly as `retrain_eligible_tags` would), calibrates a
/// `neg_threshold` for a target recall, then verifies on a disjoint
/// held-out set that scoring below that threshold really does miss no
/// more than the tolerated share of true positives.
#[test]
fn neg_threshold_calibration_achieves_target_recall_on_held_out_data() {
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

    let train = dataset(100, 10_000);
    let clf = classifier::train(&train, classifier::default_l2_lambda()).expect("should train");

    // The calibration set becomes this tag's prequential window.
    let calibration = dataset(100, 20_000);
    for (i, (x, is_pos)) in calibration.iter().enumerate() {
        let pub_key = format!("EP{i:07}");
        documents::insert_pending(&conn, &pub_key, &pub_key, "2026-01-01T00:00:00Z").unwrap();
        let doc = documents::find_by_pub_key(&conn, &pub_key)
            .unwrap()
            .unwrap();
        let score = clf.predict(x);
        predictions::record(
            &conn,
            doc.id,
            tag_id,
            "model@v1",
            score,
            score >= 0.5,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let checked: HashSet<i64> = if *is_pos {
            [tag_id].into_iter().collect()
        } else {
            HashSet::new()
        };
        labels::validate_document(
            &conn,
            doc.id,
            std::slice::from_ref(&tag),
            &checked,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
    }

    let target_recall = 0.95;
    let neg_threshold = predictions::calibrate_neg_threshold(&conn, tag_id, target_recall)
        .unwrap()
        .expect("should find a calibratable neg_threshold on well-separated clustered data");

    let held_out = dataset(200, 30_000);
    let positives: Vec<&Vec<f32>> = held_out
        .iter()
        .filter(|(_, is_pos)| *is_pos)
        .map(|(x, _)| x)
        .collect();
    let missed = positives
        .iter()
        .filter(|x| clf.predict(x) < neg_threshold)
        .count();
    let held_out_recall = 1.0 - (missed as f32 / positives.len() as f32);

    println!("neg_threshold={neg_threshold:.2}, held-out recall={held_out_recall:.3}");
    assert!(
        held_out_recall >= target_recall - 0.05,
        "expected held-out recall close to the {target_recall} target, got {held_out_recall:.3}"
    );
}
