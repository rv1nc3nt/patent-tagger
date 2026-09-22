# CLAUDE.md

This repository implements **Patent Tagger**. The full specification is in `docs/SPEC.md`. Read it before any work, and treat it as authoritative.

## Working method

- Work milestone by milestone (SPEC section 10), in order.
- At the start of each milestone, present a short plan (files, crates, open questions) and wait for approval.
- When the specification is ambiguous or seems wrong, ask. Do not make a silent assumption. Record decisions in `docs/DECISIONS.md` (date, question, decision).
- Before coding against EPO OPS, check endpoint paths and header names against the current OPS Reference Guide, and note any deviation from the SPEC in `docs/DECISIONS.md`.
- A milestone is complete only when its acceptance criteria are met and `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.
- Make small, focused commits with conventional messages (`feat:`, `fix:`, `test:`, `docs:`).

## Code rules

- `crates/core` depends on neither Tauri nor the network. The frontend contains no business logic.
- Error handling: `thiserror` in library crates, `anyhow` only in `src-tauri` and `xtask`. No `unwrap()` or `expect()` outside tests, except where an invariant is documented in a comment.
- Logging: `tracing`. Never log OPS credentials or tokens.
- Dependencies:
  - Ask before adding any crate not named in the SPEC.
  - Prefer pure-Rust crates. Anything requiring a C toolchain beyond what `rusqlite/bundled` needs, a DLL, or OpenSSL is refused.
  - The only accepted native dependencies are the system libraries Tauri itself requires on Linux (WebKitGTK 4.1, GTK 3 and their dependencies).
  - Use `reqwest` with `rustls` only.
- Cross-platform:
  - Every feature must work on Windows and Ubuntu.
  - OS-specific code lives only in `src-tauri/src/platform/`, behind `#[cfg(windows)]` / `#[cfg(target_os = "linux")]`.
  - No hard-coded paths or path separators.
  - Tests must pass on both CI jobs.
- Tests never use the network. OPS tests use recorded fixtures in `tests/fixtures/ops/`.
- All SQL goes through the storage module of `crates/core`. Every schema change is a new migration.
- Code, comments and UI text are in English.

## Never commit

- Model weights or tokenizer files (`/models`, git-ignored).
- OPS credentials, `.env` files, or local databases.

## Useful commands

```
cargo xtask fetch-model            # download and verify the embedding model
cargo test --workspace             # all tests (no network)
cargo test -p embed --features parity   # parity test, requires model files
cargo tauri dev                    # development run
cargo tauri build --no-bundle                  # Windows: release .exe
cargo tauri build --bundles deb,appimage       # Linux: .deb and AppImage
patent-tagger import numbers.txt               # headless pipeline (release binary)
```

Live OPS tests (drawings especially) consume the user's quota. Run them only when asked, on at most 10 documents, and record the responses as fixtures so they never need repeating.

On Ubuntu, install the Tauri 2 build prerequisites (WebKitGTK 4.1 development package and related libraries) as listed in the Tauri documentation before the first build.
