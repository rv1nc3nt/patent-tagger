//! Per-tag logistic regression (SPEC section 7.2): full-batch gradient
//! descent in plain Rust over `Vec<f32>`, L2-regularised, with balanced
//! class weights. Deterministic (zero-initialised weights, fixed iteration
//! count) - no seed needed.

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

/// SPEC 7.2: trained only once a tag has at least this many positives and
/// negatives.
pub const MIN_POSITIVES: usize = 5;
pub const MIN_NEGATIVES: usize = 5;

const ITERATIONS: usize = 500;
const LEARNING_RATE: f32 = 0.5;
const DEFAULT_L2_LAMBDA: f32 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
pub struct Classifier {
    pub weights: Vec<f32>,
    pub bias: f32,
    pub n_pos: i64,
    pub n_neg: i64,
    /// When this classifier was trained - `None` fresh out of [`train`]
    /// (which doesn't know the wall-clock time), `Some` once loaded back
    /// from storage. Part of SPEC 7.3's `model_version` string.
    pub trained_at: Option<String>,
}

impl Classifier {
    pub fn predict(&self, x: &[f32]) -> f32 {
        sigmoid(dot(&self.weights, x) + self.bias)
    }
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Trains a classifier on `(embedding, is_positive)` pairs. Returns `None`
/// if there are fewer than [`MIN_POSITIVES`]/[`MIN_NEGATIVES`] of either
/// class (SPEC 7.2).
pub fn train(samples: &[(Vec<f32>, bool)], l2_lambda: f32) -> Option<Classifier> {
    let n_pos = samples.iter().filter(|(_, y)| *y).count();
    let n_neg = samples.len() - n_pos;
    if n_pos < MIN_POSITIVES || n_neg < MIN_NEGATIVES {
        return None;
    }
    let dim = samples[0].0.len();

    // Balanced class weights: each class contributes equally to the total
    // loss regardless of how imbalanced the sample is.
    let weight_pos = samples.len() as f32 / (2.0 * n_pos as f32);
    let weight_neg = samples.len() as f32 / (2.0 * n_neg as f32);

    let mut weights = vec![0.0f32; dim];
    let mut bias = 0.0f32;

    for _ in 0..ITERATIONS {
        let mut grad_w = vec![0.0f32; dim];
        let mut grad_b = 0.0f32;

        for (x, y) in samples {
            let target = if *y { 1.0 } else { 0.0 };
            let sample_weight = if *y { weight_pos } else { weight_neg };
            let prediction = sigmoid(dot(&weights, x) + bias);
            let error = (prediction - target) * sample_weight;
            for (g, xi) in grad_w.iter_mut().zip(x) {
                *g += error * xi;
            }
            grad_b += error;
        }

        let n = samples.len() as f32;
        for (w, g) in weights.iter_mut().zip(&grad_w) {
            let reg = 2.0 * l2_lambda * *w;
            *w -= LEARNING_RATE * (g / n + reg);
        }
        bias -= LEARNING_RATE * (grad_b / n);
    }

    Some(Classifier {
        weights,
        bias,
        n_pos: n_pos as i64,
        n_neg: n_neg as i64,
        trained_at: None,
    })
}

pub fn default_l2_lambda() -> f32 {
    DEFAULT_L2_LAMBDA
}

fn to_bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("chunks_exact(4)")))
        .collect()
}

pub fn store(
    conn: &Connection,
    tag_id: i64,
    model_id: &str,
    classifier: &Classifier,
    trained_at: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO classifiers (tag_id, model_id, weights, bias, n_pos, n_neg, trained_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (tag_id) DO UPDATE SET
            model_id = excluded.model_id, weights = excluded.weights, bias = excluded.bias,
            n_pos = excluded.n_pos, n_neg = excluded.n_neg, trained_at = excluded.trained_at",
        params![
            tag_id,
            model_id,
            to_bytes(&classifier.weights),
            classifier.bias,
            classifier.n_pos,
            classifier.n_neg,
            trained_at,
        ],
    )?;
    Ok(())
}

pub fn load(conn: &Connection, tag_id: i64, model_id: &str) -> Result<Option<Classifier>, StorageError> {
    conn.query_row(
        "SELECT weights, bias, n_pos, n_neg, trained_at FROM classifiers WHERE tag_id = ?1 AND model_id = ?2",
        params![tag_id, model_id],
        |row| {
            let weights: Vec<u8> = row.get(0)?;
            Ok(Classifier {
                weights: from_bytes(&weights),
                bias: row.get(1)?,
                n_pos: row.get(2)?,
                n_neg: row.get(3)?,
                trained_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(StorageError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two well-separated 2D clusters (padded to a few dimensions to
    /// exercise the general-`dim` code path), deterministic and simple
    /// enough to hand-verify convergence.
    fn clustered_samples(n_per_class: usize) -> Vec<(Vec<f32>, bool)> {
        let mut samples = Vec::new();
        for i in 0..n_per_class {
            let offset = (i as f32) * 0.01;
            samples.push((vec![5.0 + offset, 5.0, 0.0], true));
            samples.push((vec![-5.0 - offset, -5.0, 0.0], false));
        }
        samples
    }

    #[test]
    fn training_requires_minimum_positives_and_negatives() {
        let mut samples = clustered_samples(3);
        samples.truncate(4); // 2 positives, 2 negatives - below the minimum.
        assert!(train(&samples, default_l2_lambda()).is_none());
    }

    #[test]
    fn trained_classifier_separates_well_clustered_data() {
        let samples = clustered_samples(20);
        let classifier = train(&samples, default_l2_lambda()).expect("should train");

        for (x, y) in &samples {
            let prediction = classifier.predict(x);
            if *y {
                assert!(prediction > 0.9, "positive sample scored {prediction}");
            } else {
                assert!(prediction < 0.1, "negative sample scored {prediction}");
            }
        }
    }

    #[test]
    fn store_and_load_round_trips() {
        let conn = crate::storage::open_in_memory().expect("in-memory db");
        conn.execute(
            "INSERT INTO tags (name, definition, created_at) VALUES ('Foo', 'A foo tag', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let tag_id = conn.last_insert_rowid();

        let samples = clustered_samples(10);
        let classifier = train(&samples, default_l2_lambda()).unwrap();
        store(&conn, tag_id, "model@abc", &classifier, "2026-01-01T00:00:00Z").unwrap();

        let loaded = load(&conn, tag_id, "model@abc").unwrap().unwrap();
        assert_eq!(loaded.weights, classifier.weights);
        assert_eq!(loaded.bias, classifier.bias);
        assert_eq!(loaded.n_pos, 10);
        assert_eq!(loaded.n_neg, 10);
        assert_eq!(loaded.trained_at.as_deref(), Some("2026-01-01T00:00:00Z"));

        assert_eq!(load(&conn, tag_id, "other-model").unwrap(), None);
    }
}
