//! `cargo xtask fetch-model`: downloads BAAI/bge-small-en-v1.5 at a pinned
//! revision, verifies SHA-256 against the hashes below, converts the
//! weights to f16, and writes everything crates/embed embeds into
//! `<repo_root>/models/bge-small-en-v1.5/` (SPEC section 6).

use anyhow::{bail, Context};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

const REVISION: &str = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a";
const BASE_URL: &str = "https://huggingface.co/BAAI/bge-small-en-v1.5/resolve";

/// (filename, expected SHA-256 of the file as downloaded from HF).
/// Recorded from a real download of this exact revision - see
/// docs/DECISIONS.md.
const FILES: &[(&str, &str)] = &[
    (
        "model.safetensors",
        "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
    ),
    (
        "tokenizer.json",
        "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    ),
    (
        "config.json",
        "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
    ),
];

pub fn run(repo_root: &Path) -> anyhow::Result<()> {
    let out_dir = repo_root.join("models").join("bge-small-en-v1.5");
    std::fs::create_dir_all(&out_dir)?;

    let client = reqwest::blocking::Client::new();
    let mut downloaded: BTreeMap<&str, Vec<u8>> = BTreeMap::new();

    for (name, expected_sha256) in FILES {
        eprintln!("Downloading {name}...");
        let url = format!("{BASE_URL}/{REVISION}/{name}");
        let bytes = client
            .get(&url)
            .send()
            .with_context(|| format!("requesting {url}"))?
            .error_for_status()
            .with_context(|| format!("fetching {url}"))?
            .bytes()
            .with_context(|| format!("reading body of {url}"))?
            .to_vec();

        let actual = hex_sha256(&bytes);
        if &actual != expected_sha256 {
            bail!(
                "SHA-256 mismatch for {name}: expected {expected_sha256}, got {actual}. \
                 Refusing to use a file that doesn't match the pinned revision {REVISION}."
            );
        }
        downloaded.insert(name, bytes);
    }

    eprintln!("Converting model.safetensors to f16...");
    let f16_bytes = convert_to_f16(&downloaded["model.safetensors"])?;
    std::fs::write(out_dir.join("model_f16.safetensors"), &f16_bytes)?;
    std::fs::write(
        out_dir.join("tokenizer.json"),
        &downloaded["tokenizer.json"],
    )?;
    std::fs::write(out_dir.join("config.json"), &downloaded["config.json"])?;

    eprintln!(
        "Done. model_f16.safetensors: {:.1} MiB (from {:.1} MiB f32).",
        f16_bytes.len() as f64 / (1024.0 * 1024.0),
        downloaded["model.safetensors"].len() as f64 / (1024.0 * 1024.0),
    );
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Re-encodes every f32 weight tensor as f16, per SPEC section 6
/// ("safetensors converted to f16 at fetch time"). A handful of tensors in
/// this model (e.g. `embeddings.position_ids`) are integer index buffers,
/// not weights - those pass through unchanged, since a real fetch surfaced
/// one (`position_ids`, dtype I64).
fn convert_to_f16(f32_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let source = safetensors::SafeTensors::deserialize(f32_bytes)?;

    // Owns the converted byte buffers so TensorView (which only borrows)
    // stays valid until serialize() runs.
    let mut buffers: Vec<(String, safetensors::Dtype, Vec<usize>, Vec<u8>)> = Vec::new();
    for (name, view) in source.iter() {
        match view.dtype() {
            safetensors::Dtype::F32 => {
                let f16_data: Vec<u8> = view
                    .data()
                    .chunks_exact(4)
                    .flat_map(|chunk| {
                        // Invariant: chunks_exact(4) guarantees each chunk is exactly 4 bytes.
                        let f32_value =
                            f32::from_le_bytes(chunk.try_into().expect("chunk has length 4"));
                        half::f16::from_f32(f32_value).to_le_bytes()
                    })
                    .collect();
                buffers.push((
                    name.to_string(),
                    safetensors::Dtype::F16,
                    view.shape().to_vec(),
                    f16_data,
                ));
            }
            other_dtype => {
                buffers.push((
                    name.to_string(),
                    other_dtype,
                    view.shape().to_vec(),
                    view.data().to_vec(),
                ));
            }
        }
    }

    let views: BTreeMap<String, safetensors::tensor::TensorView> = buffers
        .iter()
        .map(|(name, dtype, shape, data)| {
            // Invariant: for F32->F16 tensors, `data` was built above as
            // exactly `shape.iter().product() * 2` bytes (f16 is 2
            // bytes/element); for passthrough tensors, `data` and `dtype`
            // are unchanged from the already-valid source view. Either way
            // this matches what TensorView::new requires.
            let view = safetensors::tensor::TensorView::new(*dtype, shape.clone(), data)
                .expect("buffer length matches shape/dtype");
            (name.clone(), view)
        })
        .collect();

    Ok(safetensors::serialize(&views, None)?)
}
