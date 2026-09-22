use anyhow::{bail, Context};
use std::env;
use std::path::PathBuf;

mod fetch_model;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/ always has a parent directory (the workspace root)")
        .to_path_buf()
}

/// Development-only: shells out to a Python script using
/// sentence-transformers (SPEC section 6). Not needed for a normal build;
/// only for regenerating crates/embed/tests/reference_embeddings.json when
/// the pinned model revision changes. Requires a Python environment with
/// sentence-transformers installed - see the script's own docstring.
fn run_reference_embeddings(repo_root: &std::path::Path) -> anyhow::Result<()> {
    let script = repo_root.join("xtask").join("reference_embeddings.py");
    let status = std::process::Command::new("python3")
        .arg(&script)
        .current_dir(repo_root)
        .status()
        .with_context(|| format!("running {}", script.display()))?;
    if !status.success() {
        bail!("reference_embeddings.py exited with {status}");
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let task = env::args().nth(1);
    match task.as_deref() {
        Some("fetch-model") => fetch_model::run(&repo_root()),
        Some("reference-embeddings") => run_reference_embeddings(&repo_root()),
        Some("release") => bail!("xtask release: not yet implemented (milestone M10)"),
        Some(other) => bail!("unknown xtask: {other}"),
        None => bail!("usage: cargo xtask <fetch-model|reference-embeddings|release>"),
    }
}
