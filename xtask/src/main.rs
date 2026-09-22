use anyhow::bail;
use std::env;

fn main() -> anyhow::Result<()> {
    let task = env::args().nth(1);
    match task.as_deref() {
        Some("fetch-model") => bail!("xtask fetch-model: not yet implemented (milestone M3)"),
        Some("reference-embeddings") => {
            bail!("xtask reference-embeddings: not yet implemented (milestone M3)")
        }
        Some("release") => bail!("xtask release: not yet implemented (milestone M10)"),
        Some(other) => bail!("unknown xtask: {other}"),
        None => bail!("usage: cargo xtask <fetch-model|reference-embeddings|release>"),
    }
}
