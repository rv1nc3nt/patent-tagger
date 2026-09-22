//! Fails with a clear message if the model files aren't present (SPEC
//! section 6), instead of a cryptic `include_bytes!` "file not found".

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let model_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/embed is always two levels under the workspace root")
        .join("models")
        .join("bge-small-en-v1.5");

    for file in ["model_f16.safetensors", "tokenizer.json", "config.json"] {
        let path = model_dir.join(file);
        if !path.exists() {
            eprintln!(
                "\nerror: missing model file {path}\n\n\
                 crates/embed embeds the BAAI/bge-small-en-v1.5 weights and \
                 tokenizer at compile time. Run this first:\n\n\
                 \tcargo xtask fetch-model\n",
                path = path.display(),
            );
            std::process::exit(1);
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }

    println!("cargo:rustc-env=MODEL_DIR={}", model_dir.display());
}
