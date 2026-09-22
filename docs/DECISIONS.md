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
## 2026-09-22 — OPS v3.2 endpoint paths verified for M2

**Question:** CLAUDE.md requires checking exact OPS endpoint paths/headers against the current Reference Guide before coding.

**Decision:** Confirmed against the live host (`ops.epo.org`) with unauthenticated probe requests — a wrong path 404s, a recognized-but-unauthorized path 403s with `AnonymousQuotaPerDay`/Fair Use rejection reasons, which lets path shape be verified without spending real quota:
- Base URL `https://ops.epo.org/3.2/rest-services/`, matching SPEC section 5.4.
- OAuth2 token endpoint `POST https://ops.epo.org/3.2/auth/accesstoken`, HTTP Basic auth, form body `grant_type=client_credentials`. Error body: `<error><code>401</code><message>ClientId is Invalid</message></error>`.
- Published data: `/published-data/{publication|application|priority}/{docdb|epodoc|original}/{number}/{constituents}`, constituents comma-joined (verified `biblio,abstract` as one call).
- Number service: `/number-service/{reference-type}/{format}/{number}/{target-format}`.
- Family: `/family/publication/{format}/{number}/{constituents}`.
- Images/fulltext: `/published-data/publication/{format}/{number}/images` and `.../fulltext`.
- Throttling header `X-Throttling-Control`, format `idle (retrieval=green:200, search=yellow:20, inpadoc=red:30, images=green:200, other=green:1000)` — overall state (idle/busy/overloaded) then per-category `colour:limit`. No SPEC deviation found; exact quota-header names (vs. throttling) to be confirmed against a real authenticated response once credentials are available.

## 2026-09-22 — OPS fixtures: 5 real, 3 synthetic

**Question:** Section 5.4 requires fixture tests for EP A1 w/ English abstract, EP B1 w/o abstract, EP A1 FR/DE w/ English family member, US grant, US pre-grant, WO, not found. SPEC section 11 puts OPS search (CQL) out of scope, so there's no way to look up specific documents matching an exact edge case — only direct-by-number lookup.

**Decision:** Recorded 5 real live responses (with the user's real credentials, against `tests/fixtures/ops/`): EP1000000 A1 (the EPO's own "1 millionth patent" milestone, trilingual, English abstract), EP1000000 B1 (same family — turned out to also carry an English abstract, so it doesn't demonstrate the "no abstract" case despite being real), its family listing (5 members: EP A1/B1, AT T1, NL C2, US A), US5960411 (Amazon's "1-Click" patent, pre-2001 grant, kind `A`), US6285999 B1 (Google's PageRank patent), WO2019123456 A1 (found to be real by chance), and a genuine 404 fault (`EP.9999999.Z9`).

Could not locate real examples of "EP B1 with literally no abstract" or "EP A1 with only FR/DE, no English anywhere, with an English family member" without search — built 3 clearly-labeled synthetic fixtures (`synthetic_*.xml`, XML-commented as such) instead, hand-edited to match the verified real schema exactly (same element/attribute names, same structure) rather than guessed.

**Also learned along the way:**
- OPS error bodies are not always `<error>...</error>` — a 404 comes back as `<fault xmlns="http://ops.epo.org"><code>SERVER.EntityNotFound</code><message>No results found</message></fault>`. `error::from_error_body`'s namespace-agnostic `has_tag_name("message")` search happens to handle both shapes without changes (verified against the real fault body).
- `exchange-document` carries `country`/`doc-number`/`kind`/`family-id` as attributes directly, redundant with (and simpler than) the nested `publication-reference/document-id[@document-id-type='docdb']`.
- `invention-title` has no separate "title" wrapper — one element per language, direct child of `bibliographic-data`.
- `abstract` is a sibling of `bibliographic-data` inside `exchange-document`, not nested under it.
- The real EP1000000 A1 abstract's first paragraph starts with a `[0001]` numbering prefix, which is normally a description-only convention — abstract text is stored as-is (not stripped), since SPEC section 5.5 (full text) is the only place paragraph-number handling is specified, not abstracts.
- roxmltree's `Node::has_tag_name("foo")` compares local name only, ignoring the document's namespace — confirmed empirically against both the `http://www.epo.org/exchange`-namespaced biblio documents and the `http://ops.epo.org`-namespaced fault document.

## 2026-09-22 — M2 credentials scope

**Question:** M2's acceptance needs real OPS credentials (live 20-document import, fixture recording), but the full Settings screen is M7.

**Decision:** Build a minimal credentials path now — `keyring`-backed storage, Tauri commands, and a bare form with a "Test connection" button — not the full Settings screen. Expand at M7. Credentials for fixture-recording/live-testing are supplied via a local, gitignored `.env` (never pasted into chat).

- `src-tauri::platform`: `resolve_data_dir()` is OS-gated (`#[cfg(windows)]` / `#[cfg(target_os = "linux")]`) per section 2, delegating to `windows.rs`/`linux.rs` for the actual base-directory and portable-mode-directory lookup (Linux additionally checks `$APPIMAGE`). The portability check itself (`portable.flag` present + directory writable) is a pure function of a `&Path`, so it's unit-tested with real temp directories instead of mocking the OS calls.
