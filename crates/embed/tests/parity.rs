//! SPEC section 6: cosine similarity >= 0.999 against reference vectors
//! from the real sentence-transformers implementation. Behind the
//! `parity` feature (run with `cargo test -p patent-embed --features parity`)
//! since it needs `tests/reference_embeddings.json`, produced by
//! `cargo xtask reference-embeddings` (a development-only Python script).

#![cfg(feature = "parity")]

use embed_lib::{BgeSmallEmbedder, Embedder};
use serde::Deserialize;

#[derive(Deserialize)]
struct ReferenceFile {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    text: String,
    embedding: Vec<f32>,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
fn matches_sentence_transformers_reference_embeddings() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/reference_embeddings.json"
    ))
    .expect(
        "tests/reference_embeddings.json is missing - run `cargo xtask reference-embeddings` first",
    );
    let reference: ReferenceFile = serde_json::from_str(&raw).expect("valid reference JSON");
    assert!(!reference.cases.is_empty());

    let embedder = BgeSmallEmbedder::load().expect("model should load");

    for case in &reference.cases {
        let ours = embedder
            .embed(std::slice::from_ref(&case.text))
            .expect("embedding should succeed")
            .remove(0);
        assert_eq!(ours.len(), case.embedding.len());

        let similarity = cosine_similarity(&ours, &case.embedding);
        assert!(
            similarity >= 0.999,
            "cosine similarity {similarity} < 0.999 for text: {:?}",
            case.text
        );
    }
}
