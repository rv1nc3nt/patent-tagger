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

**Decision:** `fr.famillevincent.patenttagger`, based on the project owner's domain. Change if a different one is wanted before the first real release (the identifier should not change after distributing builds, since OS-level app identity depends on it).

## 2026-09-22 — `fetch-model` left out of CI for now

**Question:** Section 9 describes each CI job as running `fetch-model`, the tests, and the release build — but `cargo xtask fetch-model` doesn't exist yet (it's M3 work); a real invocation would fail every CI run before M3.

**Decision:** `ci.yml` runs fmt/clippy/test and the release build now, with a `TODO(M3)` comment marking where the `fetch-model` step goes in once `crates/embed` and `xtask` implement it.

## 2026-09-22 — Webview data folder deferred to M1

**Question:** Section 4.1 says the webview's data folder should live inside the application data directory, but that directory's resolution (via the `directories` crate, with portable-mode detection) is M1 work, not yet implemented.

**Decision:** `tauri.conf.json` uses Tauri's default webview data location for now. Wiring it to the resolved data directory happens in M1 once `crates/core` can compute that path.
