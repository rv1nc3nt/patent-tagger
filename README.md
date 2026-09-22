# Patent Tagger

Desktop application for importing, tagging and exporting patents via EPO Open Patent Services. See `docs/SPEC.md` for the full specification.

## Development

```
cargo xtask fetch-model                        # once: download + convert the embedding model
cargo tauri dev
```

Requires Node.js (frontend, `ui/`) and, on Linux, the Tauri 2 build prerequisites (WebKitGTK 4.1, GTK 3 and related development packages — see `.github/workflows/ci.yml` for the exact list).

### Embedding model (`crates/embed`)

`cargo xtask fetch-model` downloads `BAAI/bge-small-en-v1.5` at a pinned revision, verifies it against the SHA-256 hashes recorded in `xtask/src/fetch_model.rs`, converts the weights to f16, and writes everything to `models/bge-small-en-v1.5/` (git-ignored). `crates/embed`'s `build.rs` fails with a clear message if this hasn't been run.

Throughput (release build, CPU, batch size 32, this development machine): **~25 texts/sec (~40 ms/text)**. Measure it yourself with:

```
cargo run -p patent-embed --release --example throughput
```

The parity test (`cargo test -p patent-embed --features parity`) compares the Rust implementation against real `sentence-transformers` output, committed in `crates/embed/tests/reference_embeddings.json`. Regenerating those reference vectors (only needed if the pinned model revision changes) requires a Python environment:

```
python3 -m venv .venv-reference-embeddings
source .venv-reference-embeddings/bin/activate
pip install sentence-transformers
cargo xtask reference-embeddings
```

## Build

```
cargo tauri build --no-bundle                  # Windows: release .exe
cargo tauri build --bundles deb,appimage       # Linux: .deb and AppImage
```

The AppImage needs FUSE 2 to run (`libfuse2`, or `libfuse2t64` on Ubuntu 24.04); otherwise run it with `--appimage-extract-and-run`.
