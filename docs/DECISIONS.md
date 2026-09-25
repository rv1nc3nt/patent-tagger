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
## 2026-09-22 — M5 prequential window, threshold, and PR curve

**Question:** Section 7.4 says "a rolling window of the last 300 validated documents" per tag, without saying whether that's 300 documents globally or 300 per tag, and doesn't fix a threshold for the headline precision/recall figures (only the PR curve is asked for explicitly) or say how densely to sample the curve.

**Decision:** Per tag: the last 300 `predictions` rows for that `tag_id` (each corresponds to a validation event where the document carried that tag), joined to the matching human label. Headline precision/recall use threshold 0.5, matching section 7.5's own fallback ("or >= 0.5 if not yet calibrated") since no calibrated per-tag threshold exists until M6. The PR curve samples 21 fixed thresholds (0.00, 0.05, ..., 1.00) rather than deriving thresholds from observed score values, for simplicity and determinism.

## 2026-09-22 — M5 blending supersedes M4's zero-shot cutoff

**Decision:** `scoring::blend_scores` (section 7.3's actual fallback chain: blend when both LR and k-NN exist, else whichever one does, else zero-shot) replaces M4's simpler "zero-shot only below 3 positives, k-NN otherwise" rule in `review::score_document`. The two are consistent: zero-shot is still only ever passed into `blend_scores` as `Some` when `n_pos < ZERO_SHOT_POSITIVE_CEILING` (per 7.2), so the fallback chain naturally reduces to the old behaviour in that region and correctly returns no score at all when a tag is past that ceiling but neither LR (needs ≥5/≥5) nor k-NN is available yet.

## 2026-09-22 — Found and fixed a real bug in the synthetic-data test's own PRNG

While building the M5 acceptance test (LR outperforming k-NN on clustered synthetic data, per section 7's intro), the deterministic pseudo-random hash used to generate noise dimensions had `(h >> 40) as u32 as f32 / u32::MAX as f32` — `h >> 40` only leaves 24 significant bits, but dividing by `u32::MAX` assumes a 32-bit range, so every generated value landed within about 1/256th of `-1.0`, silently defeating the "noise" entirely (both scorers scored 100% regardless of sample size, which was itself the tell). Fixed to `h >> 32`. Verified by printing raw hash values at intermediate steps before and after the fix.

## 2026-09-22 — M4 scoring formulas and scope

**Question:** Section 7.2 says the zero-shot score is "cosine similarity mapped through a fixed monotonic function" without naming the function.

**Decision:** `(cosine + 1) / 2`, clamped to `[0, 1]` — the simplest monotonic map from cosine similarity's `[-1, 1]` range onto a `[0, 1]` score. `crates/core::scoring::zero_shot_score`.

**Question:** M4's milestone bullet is "Review screen, labels, zero-shot and k-NN suggestions" with no LR/blending (that's M5). What score does the Review screen show when a tag has ≥3 positives (past zero-shot's cutoff) but k-NN is also undefined (fewer than 10 validated documents exist yet, or none of the k nearest have a label for that tag)?

**Decision:** No score at all — the tag shows unchecked with no score bar, which is the correct degenerate case rather than something to paper over with a fallback SPEC doesn't ask for. Once ≥3 positives, zero-shot stops being used entirely (per SPEC's own wording: "used only while a tag has fewer than 3 positives"); it isn't a fallback for k-NN.

**Question:** Which documents appear in the M4 review queue?

**Decision:** Only `review_state = "queued"` documents with `fetch_status = "fetched"` (i.e., they have both a title and abstract, and therefore an embedding). Documents with `no_english_abstract` need a manually-pasted abstract before they can be embedded/tagged at all (SPEC 5.3 step 4) — that manual-paste UI isn't part of M4's stated scope, so those documents simply don't appear in the queue yet rather than appearing un-tagged.

## 2026-09-22 — M3 acceptance verified: parity test passes, throughput measured

**Result:** `cargo test -p patent-embed --features parity` passes: all 10 committed reference cases (real `sentence-transformers` output for `BAAI/bge-small-en-v1.5` at the pinned revision) match the Rust/candle implementation at cosine similarity ≥ 0.999. Confirmed the model's own `1_Pooling/config.json` specifies `pooling_mode_cls_token: true` (matching SPEC section 6 and our implementation), not mean pooling.

Generating the reference vectors needed a fix: `sentence-transformers`/`torch` auto-detected a GPU in this sandbox that its installed build couldn't actually use (`CUDA error: no kernel image is available for execution on the device`). Fixed by passing `device="cpu"` explicitly to `SentenceTransformer(...)` in `xtask/reference_embeddings.py` — also the more correct choice regardless, since SPEC section 2 is CPU-only end to end and candle's side (`Device::Cpu`) never used the GPU either.

Throughput measured (release build, this sandbox, CPU, batch size 32): **25.0 texts/sec, 40.0 ms/text**. Recorded in the README per SPEC section 6.

## 2026-09-22 — `onig` (C) turns out to be unavoidable via `candle-core`

**Question:** Section 6 says "Prefer a pure-Rust regex backend over `onig` if the feature set allows; verify" for the `tokenizers` crate, and section 2's hard constraint refuses any C toolchain requirement beyond what `rusqlite/bundled` needs.

**Investigation:** `tokenizers` 0.21+ does default to `onig` (C, via `onig_sys`) and `esaxx_fast` (C++, via `esaxx-rs/cpp`) — both avoidable with `default-features = false` on a *direct* dependency. But `candle-core` 0.11.0 (a dependency of `candle-transformers`, which section 6 explicitly names for the BERT architecture) declares, unconditionally on non-wasm targets: `tokenizers = { version = "0.22.0", features = ["onig"], default-features = false }`. Cargo features are additive across the whole dependency graph — a crate three levels away requesting a feature turns it on globally for that shared dependency, and nothing in a downstream crate's own `Cargo.toml` can subtract it. So depending on `candle-transformers` at all pulls in `onig`, regardless of what `crates/embed` itself requests.

**Decision:** Accept `onig` as an unavoidable consequence of using `candle-transformers` (the SPEC's own named choice for BERT), per section 6's explicit hedge ("if the feature set allows" — it doesn't). Verified this stays within the spirit of "no C toolchain beyond what `rusqlite/bundled` needs": `onig_sys` compiles bundled C source via the `cc` crate (same tier as `rusqlite/bundled`'s SQLite amalgamation) rather than requiring a system `liboniguruma` via `pkg-config` or `bindgen`/`libclang` — confirmed by a clean `cargo build -p patent-embed` in this sandbox with no extra system packages installed. `esaxx-rs` (a separate, smaller native dependency of `tokenizers`) resolved to its pure-Rust path, not its C++ (`cpp` feature) one — confirmed via `cargo tree -e features -i esaxx-rs`, no `esaxx-rs feature "cpp"` edge present.

`crates/embed`'s own `tokenizers` dependency is pinned to `0.22` (matching `candle-core`'s requirement) with `default-features = false`, so at least nothing *extra* gets requested on top of what `candle-core` already forces.

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

## 2026-09-22 — M2 live 20-document import: acceptance verified

**Question:** M2's stated acceptance criterion is "a live import of 20 numbers works with real credentials."

**Decision/result:** Verified via a temporary, non-committed test (real OPS calls, deleted immediately after, per CLAUDE.md's rule that tests never use the network) importing 20 real-or-plausible numbers end to end: parse → dedupe → enqueue → OPS fetch → selection cascade → persist. Result: 15 `fetched` (with real titles/abstracts), 4 correctly `no_english_abstract`, 1 correctly `error` (a genuinely nonexistent number, 404). No crashes, no hangs, no incorrect status. Also confirmed `pub_key` correctly treats requesting e.g. an EP number with no kind code as "all local variants" (OPS returns every kind for that number in one call), which handles cascade step 2 ("another publication of the same application") without an extra request in the common case.

## 2026-09-22 — M2 credentials scope

**Question:** M2's acceptance needs real OPS credentials (live 20-document import, fixture recording), but the full Settings screen is M7.

**Decision:** Build a minimal credentials path now — `keyring`-backed storage, Tauri commands, and a bare form with a "Test connection" button — not the full Settings screen. Expand at M7. Credentials for fixture-recording/live-testing are supplied via a local, gitignored `.env` (never pasted into chat).

- `src-tauri::platform`: `resolve_data_dir()` is OS-gated (`#[cfg(windows)]` / `#[cfg(target_os = "linux")]`) per section 2, delegating to `windows.rs`/`linux.rs` for the actual base-directory and portable-mode-directory lookup (Linux additionally checks `$APPIMAGE`). The portability check itself (`portable.flag` present + directory writable) is a pure function of a `&Path`, so it's unit-tested with real temp directories instead of mocking the OS calls.

## 2026-09-23 — M8 fulltext/images endpoint shapes and fixtures verified

**Question:** Section 5.5 needs the fulltext-inquiry/description/claims and images-inquiry endpoint shapes verified against the live host, plus fixture tests for: EP A1 English, EP B1 trilingual claims, EP FR/DE-only with an English WO family member, a publication with no full text, a publication with drawings, one without, and a not-found case.

**Decision/result:** Verified against the live host with real credentials:
- `/published-data/publication/{format}/{number}/fulltext` (inquiry) lists `<ftxt:fulltext-instance lang=".." desc="description|claims">` — code must not assume availability, matching SPEC's own wording.
- `/published-data/publication/{format}/{number}/description` and `.../claims` return the text directly; paragraph/claim numbers (`[0001]`, `1.`) are already literal text inside `<p>`/`<claim-text>`, not separate elements — no numbering needs to be synthesized.
- `/published-data/publication/{format}/{number}/images` (inquiry) lists `<document-instance desc="FullDocument|Drawing|FirstPageClipping" link=".." number-of-pages="N">`. A publication with no drawings simply omits the `Drawing` instance — not an error.
- A specific drawing page is fetched via the `X-OPS-Range` **HTTP header** (not a query parameter), set to the 1-based page number — confirmed by the response echoing back `x-ops-range: 1`. Not documented anywhere found without testing.

Recorded 6 of 7 fixture cases as real data (EP1000000 A1/B1, a genuine 404, a real drawings/no-drawings pair, a real DE-only fulltext EP0100001). Could not find a real "EP FR/DE-only with an English WO family member" (the one real DE-only patent found, EP0100001, has a family member US4626301 with no full text in OPS) — synthesized this one case, reusing M2's existing synthetic EP2500000/WO2012000001 publication numbers for narrative consistency, clearly labeled `synthetic_*`.

The non-text-content placeholder logic in `fulltext::parse_description`/`parse_claims` (tables/maths/chemistry → `"[... not reproduced]"`) is implemented per SPEC 5.5 but **unverified against any real example** — none of the recorded fixtures happen to contain a table or formula. Flagged in the module's own doc comment; revisit if a real case surfaces.

## 2026-09-23 — `tiff` crate decodes Group 4 CCITT natively; no separate `fax` crate needed

**Question:** Section 6 flags uncertainty over whether the `tiff` crate decodes Group 4 (CCITT fax) compression, the format OPS actually serves for drawing pages, or whether a separate `fax` crate would be needed as a fallback.

**Decision:** Confirmed empirically: `tiff` 0.11.3 pulls in a `fax`/`fax34` sub-crate itself (visible via `cargo tree`) and decodes `CompressionMethod::Fax4` out of the box — no extra dependency needed. Verified end-to-end against a real fetched drawing page (`ep1000000_a1_drawing_page1_group4.tiff`, 2479×3508 raw): decoded, converted to PNG, and confirmed correct via an independent Python/Pillow decode of the same file.

## 2026-09-23 — TIFF page rotation: apply the `Orientation` tag, output landscape

**Question:** Section 5.5 says to "apply the rotation stored in the page, if any, so that landscape sheets display correctly," without naming the mechanism. The `tiff` crate does not apply the `Orientation` tag (274) automatically.

**Investigation/bug caught:** The real fixture page's raw TIFF storage is portrait (2479 wide × 3508 tall) with `Orientation = 6` ("rotate 90° CW to display correctly"), correctly displaying as landscape (3508×2479) once rotated — confirmed against Pillow, which *does* apply TIFF orientation automatically on open (`Image.open(...).size` already returns the rotated landscape dimensions). The reference PNG fixture originally committed for the parity test had been generated from the raw, *unrotated* decode (portrait, sideways "FIG.1" heading) — a mistake caught by the Rust implementation (which correctly applies the rotation) disagreeing with that fixture on image dimensions (3508×2479 vs 2479×3508). Regenerated the reference fixture from Pillow's orientation-corrected open, confirmed visually (right-side-up, landscape) before overwriting.

**Decision:** `images::tiff_page_to_png` reads tag 274 manually (`Decoder::get_tag_u32`, defaulting to `1`/no rotation when absent) and applies the corresponding one of the 8 EXIF/TIFF orientation transforms (rotation and/or mirroring) to the decoded pixel buffer before encoding to PNG, swapping width/height for the 4 transforms that involve a 90°/270° turn. Output is always 8-bit greyscale PNG regardless of the source's bit depth, since SPEC allows either "1-bit or greyscale" and a single output path avoids re-packing 1-bit rows after a rotation changes row byte-alignment.

## 2026-09-23 — M8 full-text/drawings selection cascade: one language per document, not per part

**Question:** SPEC 4.2's `fulltext` table has a single `lang`/`source`/`status` per document, but a real publication can offer description and claims in *different* language sets (confirmed live: `ep0100001` has description only in German; a B1 grant can have claims in three languages while its description has none at all). SPEC 5.5's cascade ("the English text of the publication...another publication of the same application...an English family member...otherwise non-English...otherwise not_available") reads as if selecting one language for "the full text" as a whole - how does that square with per-part availability differing?

**Decision:** The cascade selects one *publication* (and one language) for the whole document, using the union of description+claims languages available at each candidate to decide the cascade winner (`selection::FulltextCandidate.langs`), then fetches whichever of description/claims that winning publication actually has *in that language*, leaving the other `NULL` if the winning publication lacks it in that language - never mixing two different languages within one document's `fulltext` row. This matches the single-lang schema exactly and still honours the "for an EP B1, take the English claims" example (a B1 with no English description but English claims wins the cascade via its claims language, and only claims gets populated).

The non-English fallback (step 4, `non_english_only`) reruns the identical cascade (own publication, then same-application, then family by WO/US/GB/EP/other priority) but accepting *any* language instead of requiring English, picking the alphabetically-first language at whichever candidate wins - SPEC doesn't specify a tie-break among non-English languages, and this is the simplest deterministic choice. Implemented as `selection::select_fulltext`/`select_drawings` in `crates/core/src/selection.rs`.

## 2026-09-23 — M8 export `.txt`: explicit wording per status, non-English text is labelled

**Decision:** Extended `export::document_txt` (SPEC 8's per-document export) with the `Sources:` line (missing from the M7 stub) and real DESCRIPTION/CLAIMS/DRAWINGS sections once `DocumentExportFulltext`/`DocumentExportDrawings` are `Some`. Wording is deliberately specific per state so nothing reads as silently blank ("Missing parts are stated explicitly"): `None` (never attempted) says "Description/Claims/Drawings not retrieved."; `status = not_available` says "Full text not available in OPS." / "This publication has no drawings."; `status = error` says "...retrieval failed."; a `non_english_only` fetch prefixes the text with `[Non-English text, language: DE]` so a reader isn't confused why English claims say something else. Not in SPEC's own template example (which only shows the fully-available case) but consistent with its explicit-missing-parts rule.

## 2026-09-23 — M8 retrieval worker: eager candidate gathering, `FirstPageClipping` stored at page 0

**Decision:** `src-tauri::retrieval_worker` re-fetches the same-application siblings (`/published-data/publication/docdb/{country}.{number}/biblio,abstract`, exactly what `import_worker` called at import time) and family (`/family/publication/docdb/.../biblio,abstract`) rather than storing every candidate's docdb id at import time (SPEC's schema has no such table). To bound the extra OPS calls this costs, the family lookup is skipped entirely whenever the requested publication or one of its siblings already has English text/drawings - the common case for most patents - and the final decision reuses `core_lib::selection::select_fulltext`/`select_drawings` unchanged once candidates are gathered, rather than re-implementing the cascade priority inline.

The `FirstPageClipping` instance (SPEC 5.5: "serves as the thumbnail in lists") is stored in the `drawings` table at `page = 0` (`retrieval_worker::THUMBNAIL_PAGE`), outside the 1-based real page range, since the schema has no separate thumbnail column.

**M8 acceptance verified:** a live, temporary (deleted before commit), non-network-committed test retrieved full text and drawings for 10 real documents (EP1000000, EP2000000, EP1500000, US5000000/6000000/7000000/8000000/9000000/10000000, WO2019123456) - zero job failures; drawings fetched for all 10 (1-24 pages each, TIFF→PNG conversion and file writes all succeeded); full text fetched (English) for the 3 EP/WO publications that had it via OPS, correctly `not_available` for the 7 US ones (matching known OPS full-text coverage limits for older US grants).

## 2026-09-23 — M9 full-automation audit: one shared window for precision and recall

**Question:** SPEC 7.6's safeguard reads "if audited precision falls below the target precision, or audited recall below the target recall, over the last 50 audits for a tag, automatic mode...is suspended" - one window, two metrics, but precision only comes from audited `pos` decisions (TP/FP) and recall needs `neg` decisions too (FN), so what exactly is "the last 50 audits"?

**Decision:** `audit::auto_completion_audit` takes the most recent 50 automatic decisions for the tag *of either state* (`pos` or `neg`) that a human has since confirmed or overturned, then computes precision from the `pos` subset and recall from the combination of that subset's TPs and the `neg` subset's FNs - one shared 50-item window, not 50 of each. This is a *different* function from the pre-existing `audited_precision` (SPEC 7.5's simpler, `pos`-only, 50-item-window safeguard for plain per-tag automatic mode, kept unchanged and still used by `src-tauri::automation::check_and_disable_if_below_target`) - the two safeguards are explicitly distinct in SPEC (7.5 vs. 7.6) and only happen to share their `label_history`-pairing trick.

No new "was this document audit-sampled" tracking was needed: SPEC 7.6 itself says validating converts every label to `source = human` "because the user has seen the whole list" - true for a focused review just as much as an audit-sampled full review - so both naturally produce the same auto-then-human `label_history` pairs this function already looks for.

## 2026-09-23 — M9 audit sampling: deterministic hash, not `rand`

**Decision:** `full_automation::is_sampled_for_audit(doc_id, rate)` uses a cheap multiplicative hash of `doc_id` mapped to `[0, 1)` rather than true randomness. `crates/core` has no network/IO dependencies and no existing randomness source (`crates/ops` depends on `rand` for backoff jitter, but pulling that into `core` for one boolean coin-flip isn't worth a new cross-crate dependency); a hash also makes the 5% sample rate deterministic and directly testable (same document always samples the same way at a fixed rate), which matches section 7's own principle that "all learning code is deterministic given a seed."

## 2026-09-23 — M9 readiness indicator replays recorded scores, doesn't re-score live

**Decision:** `full_automation::auto_completion_readiness` (SPEC 7.6: "the share of the last 300 documents that would have been auto-completed with the current settings") reuses each document's most recently recorded score per tag from `predictions` (already written at every validation per SPEC 7.4) rather than re-embedding/re-scoring documents live. This keeps the function pure core logic with no `Embedder` dependency, and is exactly what the indicator needs: a replay of recorded history against *current* tag thresholds, not a fresh scoring pass. "The last 300 documents" is read as the 300 most recently scored (by each document's latest prediction row), not specifically "validated" - so it also picks up documents scored by a future auto-completion pass once M9 ships (see the next entry).

## 2026-09-23 — M9 Metrics screen: "audited/focused-review" counts derived, no new schema

**Question:** SPEC 7.6 wants "counts of auto-completed, audited and focused-review documents" on the Metrics screen, but nothing in the schema flags "this queued document was audit-sampled" versus "this one is in focused review because some tags are uncertain" versus "this one hasn't been touched by automation at all."

**Decision:** Derived purely from existing data, no schema change: `full_automation::automation_state_counts` counts `auto_completed` (`review_state`), then classifies every other `queued`+`fetched` document by how many active tags already carry a `source = auto` label - all of them (`audited_or_complete`: this is what an audit sample looks like, since sampling writes the full decided set before routing to review instead of skipping it) or some but not all (`focused_review`). A plain queued document untouched by automation falls into neither bucket, which is correct - it isn't part of the automation picture at all yet.

## 2026-09-23 — Known gap: OPS quota usage not shown on the Metrics screen

SPEC 8 lists "OPS quota usage, as reported by the response headers" as part of the Metrics screen, alongside the full-automation readiness/counts M9 just built. `OpsClient` currently parses and acts on the *throttling* header (`X-Throttling-Control`, section 5.4's colour-coded rate limiting) but never captures or exposes the separate quota-usage headers (`X-IndividualQuotaPerHour-Used`, `X-RegisteredQuotaPerWeek-Used`, mentioned in the M2 endpoint-verification entry above) anywhere queryable by the UI. Left out of M9's scope (not part of its stated acceptance criteria) - flagged here rather than silently dropped; picking it up means threading quota headers from `OpsClient` through to a small persisted/queryable state (they're per-key-account figures the server reports, not something the app computes itself).

## 2026-09-24 — M9 command-line mode: `clap` and a direct `tokio` dependency approved

**Question:** SPEC 7.7's subcommands need argument parsing, and the CLI has to drive the async import/retrieval workers without Tauri's event loop. Neither `clap` nor `tokio` is named in the SPEC.

**Decision (user-approved):** `clap` 4 (derive) parses `import`/`export`/`status`. It is pure Rust and gives `--help`, errors and `--format` value checking. `tokio` (`rt`, `time`, `sync`) is a direct dependency of `src-tauri` for a current-thread runtime. It was already in the tree through Tauri and reqwest. `windows-sys` (named in SPEC 7.7) is a Windows-only dependency for `AttachConsole(ATTACH_PARENT_PROCESS)`.

## 2026-09-24 — M9 command-line mode: dispatch, lock and busy timeout

**Decisions:**
- `main.rs` enters CLI mode whenever the process has any argument beyond the binary name. A normal launch with no arguments opens the GUI.
- `--fetch-fulltext`/`--fetch-drawings` force retrieval for every document fetched by that run, on top of the configured policy. The run then drains all pending retrieval jobs, both the forced ones and those enqueued by policy.
- The lock (`src-tauri/src/lock.rs`) is an exclusive OS file lock on `import.lock` in the data directory, taken with `std::fs::File::try_lock` (stable since Rust 1.89; `flock` on Linux, `LockFileEx` on Windows). This is portable and needs no new crate or platform code. The OS releases the lock when the holding process exits, even after a crash or `kill -9`. A leftover `import.lock` file never blocks a later run, so no manual cleanup is needed. (An earlier version used `create_new` plus delete-on-drop, which left a stale lock after a crash.) Every GUI command that drains the job queue takes the same lock: `run_import_jobs`, `run_retrieval_jobs`, `retrieve_*_now` and the Library's bulk retrieval. This stops the GUI and a scheduled import from processing the same pending jobs twice. While a scheduled import runs, those GUI commands return a "pipeline already running" error. Reading, reviewing and labelling are not blocked.
- `storage::open` sets a 5 s SQLite busy timeout in addition to WAL mode, so the GUI and a CLI process wait for each other's short write transactions instead of failing with `SQLITE_BUSY`.
- **Not yet verified on Windows:** `AttachConsole` is written against the documented API, but no Windows target was available. Headless mode was smoke-tested on Linux only, with no display set: `status`, the export error paths, lock contention and missing credentials. The Windows CI job and a real Task Scheduler run still need to confirm M9's "headless on both platforms" criterion.

## 2026-09-24 — Review screen: a button for every keyboard action

**Decision (user request):** The Review screen has Previous (K), Next (J), Skip (S) and Validate (Enter) buttons, each with its key in the tooltip. Tag hotkeys already had checkboxes, and `/` already had a visible filter box. Enter always validates, even when a toolbar button has focus. Shortcuts keep working after a tag checkbox is clicked.

## 2026-09-24 — Tags screen: scope and semantics (user-approved)

**Question:** SPEC 8 lists a Tags screen ("create, edit, archive; definition, parent, colour, hotkey; statistics per tag; review queue for new tags; automatic-mode toggle"), but until now tag management was a small panel on the Review screen. The user asked for a dedicated tab implementing the full SPEC.

**Decisions:**
- **Material change:** the edit form has a "Material change: bump version" checkbox. It is ticked by default when the definition text changes, and the user can override it. Any change to the name or definition, and any version bump, re-embeds the tag for its current version (zero-shot looks the embedding up by `tag_version`).
- **Stale labels** are human labels whose `tag_version` is older than the tag's. They stay usable for training (SPEC 7.1). "Discard stale labels" deletes those rows from `labels`, so the documents become unknown for the tag and reappear in its review queue. `label_history` keeps them. No migration was needed, since `label_history.state` stays `pos`/`neg`.
- **Parent** is organisational only. It gives the tree display (Tags screen, Review tag pane) and a `parent` field (by name) in the tag-schema export. It has no effect on scoring, labels or learning. A parent must be an active tag and must not create a cycle. Archiving a tag moves its children to the top level. Schema import resolves parents by name after creating all new tags. It leaves a tag at the top level when its parent is missing or would form a cycle.
- **Hotkeys** must be one character, unique among active tags (case-insensitive), and not `J`, `K`, `S` or `/`. Schema import and restore drop a conflicting hotkey and report it, instead of failing.
- **Per-tag review queue:** covers reviewed documents (`validated` or `auto_completed`) that are either unknown for the tag or have a stale human label. Sorted by descending score, unscored last. Confirming yes/no writes one human label under the current version and leaves `review_state` alone. A "yes" goes through the after-tagging retrieval policy.
- **Prequential log:** when a document was unknown for the tag, its score is recorded in `predictions` before the label is written, as validation does. For a stale label no prediction is recorded, because the document was training data for that tag and its score is not out-of-sample.
- **Automatic-mode toggle** moved from the Metrics screen to the Tags screen. Metrics shows eligibility read-only.
- **Unarchive** is not in the SPEC. At the user's request it was added to SPEC 8 and implemented: the tag becomes active again, and documents validated while it was archived are unknown for it, so they appear in its review queue.
- When a human label replaces an automatic one, it clears the automatic label's `confidence` and `model_version`.

## 2026-09-24 — A missing tag embedding is computed when the tag is scored

**Question:** A tag's zero-shot embedding is computed right after the tag is saved (create, edit, tag-schema import). If that step failed, the tag was saved without an embedding and never got a zero-shot score, and the command reported an error for a save that had succeeded (the Tags form then stayed in "new" mode, so saving again failed with a duplicate name).

**Decision:** `review::score_one_tag` computes and stores the embedding when none exists for the tag's current version and the active model. This also covers a future change of embedding model. If the embedder fails there, the tag has no zero-shot score for that pass, as before, and the next scoring retries. Embedding right after a save is kept as the fast path, but its failure no longer fails the save.

## 2026-09-24 — Document export grouped by tag folders (user-approved)

**Question:** The user asked for the document export to create one folder per tag and copy each document into its tags' folders. SPEC 8 had one flat folder per document.

**Decisions:**
- **Scope:** "Export selected…" in the Library now writes `<dest>/<tag>/<pub_key>/…`. There is no separate whole-library button.
- **Flat tag folders:** parents are organisational only, so a child tag's folder is not nested inside its parent's.
- **Several tags:** the document folder is copied into each tag's folder. Only active tags with a positive label (any source) count, as for the `Tags:` line.
- **No tag:** selected documents without an active positive tag go into `_untagged/`.
- **CLI:** `export --tag X --out D --format txt` writes into `D/<X folder>/`, the same layout as the GUI. CSV and JSON exports are unchanged.
- **Folder names:** `export::sanitize_folder_name` turns Windows-forbidden characters and control characters into `_`, trims trailing dots and spaces, and suffixes reserved device names (`CON`, `COM1`, …) with `_`. Names are unique case-insensitively, also against `_untagged`. On a clash the lower tag id keeps the plain name and later ones get ` (2)`, ` (3)`, and so on.

## 2026-09-24 — Import by applicant through saved OPS searches (user-approved)

**Question:** The user asked to import patents by applicant instead of by number, in batches of 100 on demand. SPEC 11 had OPS searches (CQL) out of scope.

**Decisions (SPEC 5.6, 7.7, 8 and 11 updated):**
- **Where:** the Import screen and the command line (`search add|list|delete|restart`, `import --search <name>`). Searches are saved by name, with their position, so each batch and each scheduled run continues where the last one stopped.
- **Fields:** applicant, required, with name variants separated by `;` and combined with `or`. Country and a range of publication years are optional. Every field only narrows the query, which matters because of the 2,000-result cap.
- **Batch:** exactly one page of 100 results (one search request) per "fetch next", importing whatever is new in it. It does not keep reading until 100 new documents are found.
- **Family:** the DOCDB family id that OPS returns with each result. It costs no extra request, unlike the INPADOC extended family (any shared priority). A family already in the database, whether imported from a search or from a list, is skipped.
- **Member kept:** within a page, a family's earliest publication with an English abstract, or its earliest publication when none has one. `search/biblio` returns abstracts and dates with the results, so choosing needs no extra request.
- **Pipeline:** the chosen publications are imported like a pasted list, with the family id stored at insert so the next page already sees the family. Their biblio is fetched again by the normal import job. Reusing the search response would save those requests, but would need a second code path through the selection cascade (SPEC 5.3), so it is left as a possible optimisation.
- **Start over:** an exhausted search can be restarted from result 1, to pick up new publications. Families already imported are skipped again.
- **Verified against the live host** (2026-09-25, 3 calls, recorded in `tests/fixtures/ops/search_biblio_*.xml`):
  - `GET /published-data/search/biblio?q=<CQL>&Range=<begin>-<end>` works, with the range as a query parameter.
  - The response carries `ops:biblio-search/@total-result-count`, and each `exchange-document` carries `@family-id`, the docdb publication date and its abstracts. The existing `biblio::parse` reads it unchanged.
  - `pd within "2021 2021"` and `pd>=2023` are both accepted. OPS echoes the query normalised (`pd >= 2023`).
  - Asking beyond result 2,000 returns `400 CLIENT.InvalidQuery`. `searches::next_range` never goes past 2,000.
  - Results come newest first. New publications therefore push older ones back: a later batch re-reads some results (skipped as known families) and misses only the newest ones, which "Start over" picks up.
  - A page costs about 25 KB of quota per result, so about 2.5 MB per 100-result batch.
- **Not verified:** the response to a query with no match. It is assumed to be the `404 SERVER.EntityNotFound` fault seen for publications, and `ops::search::search` treats a 404 as an empty page.

