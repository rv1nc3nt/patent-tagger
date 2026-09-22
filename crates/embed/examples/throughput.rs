//! Manual throughput measurement (SPEC section 6: "Measure throughput and
//! report it in the README"). Run with:
//!
//!     cargo run -p patent-embed --release --example throughput

use embed_lib::{BgeSmallEmbedder, Embedder};
use std::time::Instant;

const BATCH_SIZE: usize = 32;
const BATCHES: usize = 10;

fn main() {
    let embedder = BgeSmallEmbedder::load().expect("model should load");

    let text = "Apparatus for manufacturing green bricks. A device for forming green bricks \
                from clay for the brick manufacturing industry, comprising a circulating \
                conveyor carrying mould containers combined to mould container parts, a \
                reservoir for clay arranged above the mould containers.";
    let batch: Vec<String> = std::iter::repeat_n(text.to_string(), BATCH_SIZE).collect();

    // Warm up (first run pays one-time setup costs, e.g. allocator growth).
    embedder.embed(&batch).expect("warm-up embedding should succeed");

    let start = Instant::now();
    for _ in 0..BATCHES {
        embedder.embed(&batch).expect("embedding should succeed");
    }
    let elapsed = start.elapsed();

    let total_texts = BATCH_SIZE * BATCHES;
    let per_text_ms = elapsed.as_secs_f64() * 1000.0 / total_texts as f64;
    let texts_per_sec = total_texts as f64 / elapsed.as_secs_f64();

    println!("model: {}", embedder.model_id());
    println!("batch size: {BATCH_SIZE}, batches: {BATCHES}, total texts: {total_texts}");
    println!("elapsed: {elapsed:?}");
    println!("throughput: {texts_per_sec:.1} texts/sec ({per_text_ms:.2} ms/text)");
}
