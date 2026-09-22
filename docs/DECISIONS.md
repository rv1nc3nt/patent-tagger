# Decisions

Record of decisions made where the specification was ambiguous or silent, per `CLAUDE.md`'s working method.

## 2026-09-22 — Repository layout

**Question:** `CLAUDE.md` referenced `docs/SPEC.md` and `docs/DECISIONS.md`, but the spec lived at `specifications.md` in the repo root and no `docs/` folder existed.

**Decision:** Moved `specifications.md` → `docs/SPEC.md`, renamed `Claude.md` → `CLAUDE.md`, created this file. No content changes to the spec.

## 2026-09-22 — License

**Question:** SPEC and CLAUDE.md are silent on license.

**Decision:** `0BSD` (BSD Zero Clause License).

## 2026-09-22 — Toolchain pin

**Question:** SPEC and CLAUDE.md are silent on Rust edition and toolchain version.

**Decision:** Rust 2021 edition, latest stable toolchain at time of scaffolding, pinned via `rust-toolchain.toml` so `windows-latest` and `ubuntu-22.04` CI runners (and local dev) resolve to the same compiler.

## 2026-09-22 — Git remote / CI deferral

**Question:** Section 9 specifies GitHub Actions CI on `windows-latest` and `ubuntu-22.04`, which requires a GitHub remote.

**Decision:** No remote exists yet. Git is initialized locally. `.github/workflows/ci.yml` is written and committed now so it's ready the moment a remote exists, but it can't run or be verified until then.

## 2026-09-22 — Crate/package naming

**Question:** Section 3 names the workspace directories `crates/core`, `crates/ops`, `crates/embed`, but `core` collides with the standard library crate if used as a Cargo package name.

**Decision:** Directories stay as named in the spec; Cargo package names get a `patent-` prefix (`patent-core`, `patent-ops`, `patent-embed`), with library crate names `core_lib`, `ops_lib`, `embed_lib`. The Tauri app package is `patent-tagger`, matching the CLI binary name used throughout section 7.7.

## 2026-09-22 — Tauri app identifier

**Question:** `cargo tauri init` requires a reverse-domain bundle identifier; the spec doesn't state one.

**Decision:** `eu.rvincent.patenttagger`, based on the project owner's domain. Change if a different one is wanted before the first real release (the identifier should not change after distributing builds, since OS-level app identity depends on it).

## 2026-09-22 — `fetch-model` left out of CI for now

**Question:** Section 9 describes each CI job as running `fetch-model`, the tests, and the release build — but `cargo xtask fetch-model` doesn't exist yet (it's M3 work); a real invocation would fail every CI run before M3.

**Decision:** `ci.yml` runs fmt/clippy/test and the release build now, with a `TODO(M3)` comment marking where the `fetch-model` step goes in once `crates/embed` and `xtask` implement it.

## 2026-09-22 — Webview data folder wiring deferred past M1

**Question:** Section 4.1 says the webview's data folder should live inside the application data directory. `src-tauri::platform::resolve_data_dir()` (M1) now computes that path, but the config-based `dataDirectory` in `tauri.conf.json` only accepts a path relative to Tauri's own `appDataDir()`, not an arbitrary absolute path (see `tauri-utils` `WindowConfig::data_directory` doc comment) — an absolute path needs the window to be created programmatically via `WebviewWindowBuilder::data_directory` instead of the declarative config.

**Decision:** Not wired yet; `tauri.conf.json` still uses Tauri's default webview data location. This needs a window-creation change I can't verify without a display, and there's no consumer of the data directory yet (no DB is opened against it). Deferred to whichever milestone first opens the database against a real path — expected to be M2 (Import), alongside switching the declarative window to a programmatic one.

## 2026-09-22 — M1 schema/parser/platform implementation notes

- `crates/core::storage`: one migration (`schema.sql`) creating the full section 4.2 schema, run through `rusqlite_migration`. FTS5 external-content tables (`documents_fts`, `fulltext_fts`) kept in sync via `INSERT`/`UPDATE`/`DELETE` triggers, tested directly (insert a document, update its title/abstract, confirm an FTS `MATCH` finds it).
- `crates/core::number`: no `regex` dependency added (not named in SPEC) — the parser is hand-written char/byte scanning over a compact (separator-stripped) string, splitting a trailing `letter + digits` kind code from the numeric prefix. Unknown country codes (anything other than EP/US/WO) return `ParseOutcome::NeedsNormalisation` rather than failing, per the earlier parser-fallback-contract decision.
- `src-tauri::platform`: `resolve_data_dir()` is OS-gated (`#[cfg(windows)]` / `#[cfg(target_os = "linux")]`) per section 2, delegating to `windows.rs`/`linux.rs` for the actual base-directory and portable-mode-directory lookup (Linux additionally checks `$APPIMAGE`). The portability check itself (`portable.flag` present + directory writable) is a pure function of a `&Path`, so it's unit-tested with real temp directories instead of mocking the OS calls.
