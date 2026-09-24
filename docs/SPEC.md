# Patent Tagger — Implementation Specification

## 1. Purpose

A desktop application for Windows and Linux (Ubuntu) that imports patents from a list of patent or publication numbers, retrieves their bibliographic data and English abstract from EPO Open Patent Services (OPS), and lets a user tag them. The application learns from each human decision and suggests tags for new documents. Its goal is to reach fully automatic tagging: tag by tag, once measured precision is sufficient, and eventually for whole documents without human review, apart from uncertain cases and audit samples.

Tagging relies on the abstract. Once a document is tagged, the application also retrieves its full text (description and claims) and, where available, its drawings, and exports each document as a `.txt` file with its drawings as images.

## 2. Hard constraints

- **Language:** Rust for all application logic. The UI frontend is web technology rendered by Tauri; it contains presentation logic only.
- **Framework:** Tauri 2.
- **Platforms:** Windows 10 and 11 (x86_64), and Ubuntu 22.04 LTS and 24.04 LTS (x86_64). CPU only; no GPU required. Both platforms are first-class: same features, same tests, same CI.
- **Portable code:** no platform-specific code outside a small `platform` module in `src-tauri`, which covers data directory, credential store and startup checks. Paths use `PathBuf`, never string concatenation. Text import accepts both CRLF and LF line endings.
- **Distribution, Windows:** one self-contained `.exe`. Nothing may need installing on the target machine, except the Microsoft Edge WebView2 runtime, which is assumed present (it ships with Windows 11 and is deployed to Windows 10 through Windows Update).
  - The MSVC C runtime is linked statically (`+crt-static`).
  - No DLLs are shipped beside the executable.
- **Distribution, Linux:** Tauri renders through the system WebKitGTK (4.1 for Tauri 2), which is a shared system library and cannot be embedded in a single binary. Two deliverables:
  - a `.deb` package, the primary format. It declares its dependencies (WebKitGTK 4.1, GTK 3 and so on), so `apt` installs whatever is missing in one step;
  - an AppImage, the portable single-file format. It bundles its libraries, but it needs FUSE 2, which recent Ubuntu releases do not install by default (package `libfuse2`, or `libfuse2t64` on 24.04). The README documents the workaround of running it with `--appimage-extract-and-run`.
- **Common to both:**
  - No ONNX Runtime: inference uses `candle` (pure Rust).
  - The embedding model weights and tokenizer are embedded in the binary with `include_bytes!`.
  - SQLite is compiled in (`rusqlite`, feature `bundled`).
- **Network:** used only for EPO OPS. Everything else works offline.
- **Scope of text:** tagging uses the English title and abstract only. Full text (description and claims) and drawings are retrieved for storage and export, not for tagging. No PDF processing.
- **Users:** single user per installation; no synchronisation.

## 3. Repository layout

A Cargo workspace:

```
/crates/core     domain types, SQLite storage, learning (no Tauri, no network)
/crates/ops      EPO OPS client (auth, throttling, parsing)
/crates/embed    candle-based embedding model, weights embedded
/src-tauri       Tauri application: commands, background workers, startup, command-line mode
/ui              frontend (Svelte + TypeScript + Vite)
/models          model files fetched by `xtask` (git-ignored)
/xtask           developer tasks: fetch-model, reference-embeddings, release
/tests/fixtures  recorded OPS responses (including full text and drawing pages), reference embeddings
```

`core` must be testable with `cargo test` on any OS without network access or model weights. `embed` exposes a trait so that the model can be swapped later (see section 6).

## 4. Data storage

### 4.1 Location

The default data directory is resolved with the `directories` crate:
- Windows: `%LOCALAPPDATA%\PatentTagger\`;
- Linux: `$XDG_DATA_HOME/patent-tagger/`, i.e. `~/.local/share/patent-tagger/` by default.

**Portable mode:** if a file named `portable.flag` exists next to the executable and that folder is writable, the data directory is `data/` next to the executable. On Linux this applies to the AppImage, where "next to the executable" means next to the `.AppImage` file (environment variable `APPIMAGE`), not inside its read-only mount.

The webview's data folder is set inside the data directory (Tauri window configuration), so that the application writes nowhere else. Logs (`tracing`, rolling files) are also written inside the data directory.

### 4.2 Schema (SQLite)

Use migrations (e.g. `rusqlite_migration`). Enable WAL mode and foreign keys.

```sql
documents(
  id INTEGER PRIMARY KEY,
  pub_key TEXT UNIQUE NOT NULL,        -- country + number, no kind code, e.g. EP1234567
  input_raw TEXT NOT NULL,             -- number as entered by the user
  kind_codes TEXT,                     -- JSON array, e.g. ["A1","B1"]
  application_number TEXT,
  family_id TEXT,                      -- DOCDB family id
  title TEXT,
  abstract TEXT,
  abstract_source TEXT,                -- docdb id of the publication the abstract came from
  applicants TEXT,                     -- JSON array
  publication_date TEXT,               -- ISO 8601, earliest publication
  cpc TEXT, ipc TEXT,                  -- JSON arrays
  fetch_status TEXT NOT NULL,          -- pending | fetched | no_english_abstract | not_found | error
  fetch_error TEXT,
  review_state TEXT NOT NULL,          -- queued | validated | auto_completed | skipped
  imported_at TEXT NOT NULL,
  validated_at TEXT
)
documents_fts  -- FTS5 external-content table over (title, abstract), kept in sync by triggers

fulltext(
  doc_id PRIMARY KEY, description TEXT, claims TEXT,
  lang TEXT,                           -- language of the retrieved text
  source TEXT,                         -- docdb id of the publication the text came from
  status TEXT NOT NULL,                -- pending | fetched | not_available | non_english_only | error
  fetched_at TEXT
)
fulltext_fts   -- FTS5 external-content table over (description, claims)

drawings(
  doc_id, page INTEGER, source TEXT,   -- docdb id of the publication the drawings came from
  path TEXT NOT NULL,                  -- relative to the data directory, e.g. drawings/EP1234567/001.png
  width INTEGER, height INTEGER, fetched_at TEXT,
  PRIMARY KEY (doc_id, page)
)
drawings_status(doc_id PRIMARY KEY, status TEXT NOT NULL, page_count INTEGER, source TEXT, updated_at TEXT)
                                       -- status: pending | fetched | not_available | error

ops_raw(doc_id, endpoint, fetched_at, body BLOB)  -- gzip (flate2, Rust backend); allows re-parsing without spending quota

tags(
  id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, definition TEXT NOT NULL,
  parent_id INTEGER NULL, color TEXT, hotkey TEXT NULL, version INTEGER NOT NULL DEFAULT 1,
  archived INTEGER NOT NULL DEFAULT 0, auto_enabled INTEGER NOT NULL DEFAULT 0,
  threshold REAL NULL,                 -- at or above: confident presence
  neg_threshold REAL NULL,             -- below: confident absence (used by full automation, section 7.6)
  created_at TEXT NOT NULL
)

labels(                                 -- current state; absence of a row = unknown
  doc_id, tag_id, state TEXT NOT NULL,  -- pos | neg
  source TEXT NOT NULL,                 -- human | auto  (auto neg only for auto_completed documents)
  confidence REAL NULL, model_version TEXT NULL, tag_version INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (doc_id, tag_id)
)
label_history(...)                      -- append-only copy of every label change, with timestamp

embeddings(doc_id, model_id TEXT, dim INTEGER, vector BLOB, PRIMARY KEY (doc_id, model_id))
tag_embeddings(tag_id, model_id, tag_version, vector BLOB)

predictions(                            -- scores captured before human validation (prequential log)
  doc_id, tag_id, model_version TEXT, score REAL, suggested INTEGER, created_at TEXT
)
classifiers(tag_id, model_id, weights BLOB, bias REAL, n_pos INTEGER, n_neg INTEGER, trained_at TEXT)

jobs(id, kind TEXT, payload TEXT, state TEXT, attempts INTEGER, last_error TEXT, updated_at TEXT)
settings(key TEXT PRIMARY KEY, value TEXT)
```

Vectors are stored as little-endian `f32` BLOBs, L2-normalised. `model_id` identifies the model and the hash of its weights, e.g. `bge-small-en-v1.5-f16@<sha256-prefix>`.

Drawing images are files under `drawings/<pub_key>/` in the data directory, not BLOBs. The database stores their relative paths. Backup (section 8) therefore covers the database and the `drawings/` folder together, as one `.zip` archive.

## 5. Import and EPO OPS

### 5.1 Input

The user either pastes a list or selects a `.txt` or `.csv` file, one number per entry. Tolerate:
- spaces, dots, slashes and commas inside numbers;
- presence or absence of a kind code;
- forms such as `EP 1 234 567 B1`, `EP1234567`, `US 2020/0123456 A1`, `US11000000B2`, `WO2019/123456`, `WO 2019123456 A1`.

Lines that cannot be parsed are reported, not silently dropped. Duplicates within the input, or already in the database, are reported and skipped.

### 5.2 Normalisation

1. A local parser handles common EP, WO and US forms and extracts the country code, number and optional kind code.
2. Anything the parser cannot normalise confidently goes through the OPS number-service (conversion from `original` to `docdb` format).

The document key `pub_key` is country code plus number without kind code, so that EP A1 and EP B1 of the same number form one document.

**Related documents:** when an imported document shares an application number or DOCDB family with an existing document, the import report flags it and offers to copy that document's human labels as suggestions (never as validated labels).

### 5.3 Abstract selection

Follow this order:

1. The English abstract (`lang="en"`) of the requested publication.
2. An English abstract from another publication of the same application (e.g. the A1 when a B1 was requested).
3. An English abstract from a DOCDB family member, preferring WO, then US, GB, EP, then any other English source.
4. Otherwise set `fetch_status = no_english_abstract`. The UI lets the user paste an abstract manually, stored with `abstract_source = manual`.

Always record `abstract_source`. Use the English title where available, falling back in the same order.

Also store: applicants, earliest publication date, CPC and IPC classifications, application number, DOCDB family id.

### 5.4 OPS client (`crates/ops`)

- **API:** OPS v3.2, base `https://ops.epo.org/3.2/rest-services/`.
- **Authentication:** OAuth2 client credentials at `https://ops.epo.org/3.2/auth/accesstoken`, with HTTP Basic auth using the consumer key and secret. Cache the token and renew it before expiry (about 20 minutes) and on an invalid-token response.
- **Format:** request XML and parse with `roxmltree`. Do not rely on OPS's XML-to-JSON conversion.
- **HTTP:** `reqwest` with `rustls` (no OpenSSL, no native-tls).
- **Throttling:**
  - Parse the throttling-control response header (overall service state plus a colour per service category) and the quota headers.
  - Adapt the request rate: normal on green, slow down on yellow, pause on red, stop and report on black or on quota exhaustion.
  - At most 2 concurrent requests.
- **Resilience:** exponential backoff with jitter on 5xx and network errors, with a maximum attempt count. Imports run as persistent `jobs`, so an interrupted import resumes at the next launch.
- **Credentials:** entered in Settings and stored in the operating system's credential store via the `keyring` crate, never in SQLite or logs:
  - Windows: Credential Manager;
  - Linux: the Secret Service API (GNOME Keyring on Ubuntu). Use the crate's pure-Rust cryptography option rather than OpenSSL.
  - If no credential store is available (for example a minimal desktop without a keyring daemon), fall back to a file in the data directory with permissions `0600`, and show a warning in Settings.
  
  Provide a "Test connection" button.
- **Reference:** check exact endpoint paths, constituents and header names against the current OPS Reference Guide before coding. Record real responses as fixtures (anonymisation is not needed; the data is public).
- **Tests:** use recorded fixtures only. Tests never call the network. Parser tests must cover: EP A1 with English abstract; EP B1 without abstract; EP A1 in French or German with an English family member; US grant; US pre-grant publication; WO; not found. Full-text and image tests are listed in section 5.5.

### 5.5 Full text and drawings

**When to retrieve.** A setting defines the retrieval policy, separately for full text and for drawings:
- never;
- on demand (button in the document view);
- after tagging, when the document is validated or auto-completed;
- after tagging, only for documents carrying selected tags.

Defaults: full text after tagging for all documents; drawings on demand. Drawing pages count against the images quota category and are much heavier than text. Retrieval runs as persistent `jobs` with the same throttling as imports, at lower priority than abstract imports.

**Full text.**
- Use the OPS full-text inquiry to see which parts exist for a publication, then retrieve the description and claims constituents.
- Full-text coverage in OPS depends on the publishing authority and period. The code must not assume availability: it relies on the inquiry result.
- Selection order:
  1. the English text of the publication whose abstract was used;
  2. another publication of the same application (for an EP B1, take the English claims, since B1 claims appear in three languages);
  3. an English-language family member (WO, then US, GB, EP);
  4. otherwise, if only non-English text exists, store it with `status = non_english_only` and its language code;
  5. if no text exists at all, set `status = not_available`.
- Always record `source`, the publication the text came from, so that export and UI can state it.
- Conversion to plain text:
  - Keep paragraph numbers from the description as `[0001]`, and claim numbers.
  - Keep headings on their own lines.
  - Replace tables, chemical formulae and mathematical formulae that are not plain text with placeholders such as `[Table 1 not reproduced]`.
  - Decode entities, normalise whitespace, and output UTF-8.
- Full text is searchable in the Library (FTS5) but is **not** used for embeddings or tagging.

**Drawings.**
- Use the OPS images inquiry to find the `Drawing` document instance and its page count, then retrieve each page. Prefer the drawings of the publication used for the full text, else any publication of the same application, else an English family member (drawings are language-neutral, so any family member with identical drawings is acceptable, but record the source).
- Pages usually arrive as TIFF, often CCITT Group 4 (fax) compressed. Convert each page to a 1-bit or greyscale PNG for storage and display, since webviews do not display TIFF.
  - Use a pure-Rust decoder: check whether the `tiff` crate decodes Group 4 in the version used; otherwise use the `fax` crate for decoding and the `png` crate for encoding.
  - Apply the rotation stored in the page, if any, so that landscape sheets display correctly.
- Also retrieve the `FirstPageClipping` (the representative drawing on the front page) when available. It serves as the thumbnail in lists.
- If no drawings exist, set the status to `not_available`. This is normal: some patents have no drawings.

**Tests (fixtures):**
- EP A1 with full text in English;
- EP B1 with trilingual claims;
- EP in French with an English WO family member;
- a publication without full text;
- drawing inquiry and a Group 4 TIFF page, with the decoded PNG compared against a reference image;
- a publication without drawings.

## 6. Embedding model (`crates/embed`)

- **Model:** `BAAI/bge-small-en-v1.5` (MIT licence), BERT architecture via `candle-transformers`, CPU backend.
- **Weights:** `safetensors` converted to f16 at fetch time, loaded from memory (`VarBuilder::from_buffered_safetensors`). Computation in f32.
- **Tokenizer:** `tokenizer.json` embedded and loaded with `Tokenizer::from_bytes` (`tokenizers` crate). Prefer a pure-Rust regex backend over `onig` if the feature set allows; verify.
- **Input text:** `"{title}. {abstract}"`, truncated at 512 tokens. BGE query instructions are not used, since all inputs are passages.
- **Pooling:** CLS token, then L2 normalisation (as specified for BGE).
- **Build:** `cargo xtask fetch-model` downloads the files from Hugging Face at a pinned revision, verifies SHA-256 hashes stored in the repository, and converts the weights to f16. `build.rs` in `crates/embed` fails with a clear message if the files are missing. Weights are never committed.
- **Parity test:** `cargo xtask reference-embeddings` is a development-only Python script using `sentence-transformers`. It produces reference vectors for about 10 fixed texts, which are committed. A test (behind a feature flag, run when the weights are present) requires cosine similarity ≥ 0.999 with the Rust output.
- **Execution:** embedding runs on a dedicated worker thread with batching, and reports progress to the UI. Measure throughput and report it in the README.
- **Swappability:** trait `Embedder { fn model_id(&self) -> &str; fn dim(&self) -> usize; fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>; }`. When the active model changes, re-embed all documents as a background job; the old vectors remain until the job completes.

## 7. Learning (`crates/core::learn`)

All learning code is deterministic given a seed, and is unit-tested on synthetic data (clustered vectors with known tags).

### 7.1 Labelling semantics

- **Validating a document** means that every non-archived tag has been considered. Checked tags become `pos`, unchecked tags become `neg`, both with `source = human`.
- **A tag created after a document was validated** has no label for that document (unknown). The Tags screen offers a "review this tag against existing documents" queue sorted by descending score, where the user confirms `pos`/`neg` for that tag only.
- **Incrementing a tag's `version`** (after a material definition change) marks older labels for optional re-review. They remain usable for training unless the user discards them.
- **Training data:** only `source = human` labels are used. Automatic labels are never used for training.

### 7.2 Scorers

Each scorer produces a score in [0, 1] per (document, tag).

**Zero-shot.** The tag embedding is the embedding of `"{name}: {definition}"`. The score is the cosine similarity mapped through a fixed monotonic function. It is used only while a tag has fewer than 3 positives; its suggestions are shown as weak and are never pre-checked.

**k-NN.** Take the k = 10 nearest validated documents by cosine similarity (brute force; sufficient up to about 100,000 documents). For tag *t*, keep only the neighbours with a human label for *t*. The score is the similarity-weighted fraction of positives among them, with negative similarities clipped to 0. If no neighbour has a label for *t*, the score is undefined.

**Logistic regression, one per tag.**
- Trained on human `pos`/`neg` labels, with L2 regularisation (configurable λ, default 1e-3) and balanced class weights.
- Full-batch optimisation implemented in plain Rust over `Vec<f32>`; no heavy linear-algebra dependency is needed at 384 dimensions.
- Trained only once the tag has ≥ 5 positives and ≥ 5 negatives.
- Retrained in the background after every 10 validations, or on demand.
- Weights are persisted in `classifiers`.

### 7.3 Blending

The blend weight is w = n_pos / (n_pos + 20), and the score is w · LR + (1 − w) · kNN.
- If LR is not available, use kNN.
- If kNN is undefined, use LR.
- If neither is available, use zero-shot.

`model_version` is a string identifying the embedding model, the classifier training timestamps and the blending parameters.

### 7.4 Prequential evaluation

When a document is validated, first write its current scores for every tag to `predictions`, then apply the labels and update the learners. Scores are therefore always measured before learning from the document.

For each tag, over a rolling window of the last 300 validated documents, compute:
- precision and recall at the current threshold;
- the precision–recall curve;
- support (number of positives and of evaluated documents).

### 7.5 Thresholds and automatic tagging

- **Suggestion threshold:** a tag is pre-checked when its score ≥ the tag's threshold, or ≥ 0.5 if not yet calibrated.
- **Calibrated threshold:** the lowest threshold reaching the target precision (default 0.95, configurable) on prequential data.
- **Eligibility for automatic mode:** ≥ 30 positives, ≥ 150 evaluated documents, and the target precision reached at a recall the user can see.
- **Enabling:** the user enables automatic mode per tag explicitly; it is never enabled by default. A tag in automatic mode writes `pos` labels with `source = auto`, the confidence, and the `model_version`, for scores ≥ threshold. Outside full automation (section 7.6), the system never writes automatic negatives, and the document still goes to review for the remaining tags.
- **Audit:** a configurable fraction (default 10 %) of automatically tagged documents is routed to the review queue. If audited precision over the last 50 audited documents falls below the target, automatic mode for that tag is disabled and the user is notified.

### 7.6 Full automation

Full automation is the end state: documents are tagged completely without review, and only uncertain cases reach a human.

**Confident absence.** Each tag also has a `neg_threshold`: the highest score such that, on prequential data, the positives scoring below it do not exceed 1 − target recall (default target recall 0.95, configurable). A score below `neg_threshold` means "confidently absent"; a score between `neg_threshold` and `threshold` is uncertain.

**Per-document decision.** After scoring, a document is **auto-completed** when, for every non-archived tag:
- the tag is in automatic mode, and
- its score is either ≥ `threshold` (written as `pos`, `source = auto`) or < `neg_threshold` (written as `neg`, `source = auto`).

The document then gets `review_state = auto_completed` and does not enter the review queue.

Otherwise the document enters the queue in **focused review**:
- confident tags are pre-filled and marked as automatic;
- uncertain tags, and tags not in automatic mode, are highlighted;
- the user only needs to decide those, then validates.

On validation, all labels become `source = human`, because the user has seen the whole list.

**Safeguards.**
- **Audit:** a fraction (default 5 %) of auto-completed documents is sampled into the review queue as a full review. Disagreements are logged per tag. If audited precision falls below the target precision, or audited recall below the target recall, over the last 50 audits for a tag, automatic mode for that tag is suspended and the user is notified. Suspending one tag sends future documents to focused review, not full review.
- **New tags:** creating a tag stops auto-completion until that tag becomes eligible. Documents in the meantime go to focused review for the new tag only.
- **Readiness indicator:** the Metrics screen shows the share of the last 300 documents that would have been auto-completed with the current settings. This lets the user see how close the system is to full automation before enabling it.
- **Global switch:** full automation is a single setting, off by default. It can be turned on only when at least one tag is in automatic mode.

**Training.** Automatic labels, positive or negative, are never used for training. Audited and focused-review documents provide the continuing human signal.

### 7.7 Command-line mode

The same binary accepts subcommands and then runs without opening a window, so that imports can be scheduled (Windows Task Scheduler, `cron` or systemd timers):

```
patent-tagger import <file> [--fetch-fulltext] [--fetch-drawings]
patent-tagger export --tag <name> --out <folder> [--format txt|csv|json]
patent-tagger status
```

- `import` runs the whole pipeline: fetch, embed, score, auto-complete where allowed, queue the rest, and retrieve full text and drawings per policy. It then prints a summary (imported, auto-completed, queued, errors) and exits with a non-zero code on failure.
- On Windows the release binary uses the GUI subsystem, so command-line mode must attach to the parent console (`AttachConsole(ATTACH_PARENT_PROCESS)` via `windows-sys`) to print output.
- A lock file in the data directory prevents two import pipelines from running at once, whether from the GUI or the command line. The database uses WAL mode with a busy timeout, so the GUI stays usable while a scheduled import runs.

## 8. User interface

The application is keyboard-first. The UI calls Rust exclusively through typed Tauri commands and receives progress through Tauri events. No business logic lives in the frontend.

**Import**
- Paste box and file picker; import-job progress.
- Report with sections: imported, duplicates, related documents (same application or family), not found, no English abstract, errors.
- Retry button for failed entries.

**Review (main screen), in three panes:**
- **Left:** the queue. Ordering options: import order, or most uncertain first (scores closest to their thresholds). Audit samples and focused-review documents are marked as such.
- **Centre:** publication number, title, abstract, applicants, date, CPC; a link that opens the publication in Espacenet in the default browser. Tabs for Description, Claims and Drawings when retrieved, each showing its source publication. The Drawings tab has thumbnails, a zoomable page view and a "retrieve now" button when the policy is on demand.
- **Right:** every tag with its score bar. Suggested tags are pre-checked; zero-shot suggestions are visually distinct. In focused review, confident automatic tags are shown as filled, and uncertain tags are highlighted.
- **Shortcuts:**
  - `J`/`K` for next and previous document;
  - each tag's hotkey toggles it;
  - `Enter` validates and moves on;
  - `S` skips;
  - `/` focuses the tag filter.

**Library**
- Full-text search over abstracts and, optionally, descriptions and claims (FTS5), combined with filters: tags (including/excluding), label source (human or auto), review state (validated, auto-completed), full-text and drawings availability. Also "similar to this document" (embedding search).
- Table view with drawing thumbnails; document detail with label history.
- Bulk action: retrieve full text or drawings for the current selection.

**Tags**
- Create, edit, archive, restore (unarchive); definition, parent, colour, hotkey.
- Statistics per tag; review queue for new tags; automatic-mode toggle with eligibility status.

**Metrics**
- Per-tag table: precision, recall, support, `threshold`, `neg_threshold`, eligibility, audit results.
- Precision–recall curve per tag.
- Full-automation readiness indicator (section 7.6) and counts of auto-completed, audited and focused-review documents.
- OPS quota usage, as reported by the response headers.

**Settings**
- OPS credentials and connection test.
- Target precision, target recall, audit rates, full-automation switch.
- Retrieval policies for full text and drawings (section 5.5).
- Data folder (read-only display).
- Backup of the database and `drawings/` folder to a single `.zip` archive (SQLite online backup API for the database), and restore.
- Tag-schema export and import (JSON).

**Export**
- CSV (publication number, title, tags, sources), full JSON, and one `.txt` list of publication numbers per tag.
- **Document export:** for a selection (or a tag filter), one folder per active tag, holding one folder per document carrying that tag. A document with several tags is copied into each of their folders; a selected document with no tag goes into `_untagged/`. Tag folder names are made safe for Windows and Linux, and kept unique:

  ```
  Batteries/
    EP1234567/
      EP1234567.txt
      drawings/001.png, 002.png, …
  _untagged/
    EP7654321/…
  ```

  The `.txt` file is UTF-8 with LF line endings, laid out as follows:

  ```
  Publication: EP1234567 (A1, B1)
  Title: …
  Applicants: …
  Publication date: …
  CPC: …
  Tags: tag A; tag B
  Sources: abstract EP.1234567.A1; full text EP.1234567.B1; drawings EP.1234567.A1

  ===== ABSTRACT =====
  …
  ===== DESCRIPTION =====
  [0001] …
  ===== CLAIMS =====
  1. …
  ===== DRAWINGS =====
  drawings/001.png
  …
  ```

  Missing parts are stated explicitly (e.g. `Full text not available in OPS`), never omitted silently.

The UI follows the system light/dark setting. English only. Use the official Tauri plugins for cross-platform behaviour: `tauri-plugin-dialog` for file pickers and message boxes, and `tauri-plugin-opener` to open Espacenet links in the default browser.

## 9. Build and packaging

**Common**
- Release profile: `lto = "fat"`, `codegen-units = 1`, `opt-level = 3` (not `"z"`, to keep inference fast), `panic = "abort"`, `strip = true`.
- `cargo xtask release` builds the deliverables for the host platform.

**Windows**
- `.cargo/config.toml` sets `-C target-feature=+crt-static` for `x86_64-pc-windows-msvc` only.
- Build with `cargo tauri build --no-bundle`. The deliverable is the `.exe` alone; no installer is required.
- Startup: if WebView2 is missing or window creation fails, show a native message box explaining the requirement, instead of failing silently. This check must run before any webview is created.
- `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`.
- Code signing is out of scope, but the release task leaves a hook for `signtool`.

**Linux**
- Build on Ubuntu 22.04, the oldest supported release, so that the binary's glibc requirement is satisfied on both 22.04 and 24.04.
- Build with `cargo tauri build --bundles deb,appimage`.
- The `.deb` declares its runtime dependencies. The deliverables include a `.desktop` entry and an icon.
- A missing WebKitGTK prevents the binary from loading at all, so no in-app message is possible. The `.deb` dependencies cover this, and the README lists the `apt` command for the bare binary and the AppImage.

**CI (GitHub Actions)**
- Two jobs, `windows-latest` and `ubuntu-22.04`, each running `fetch-model`, the tests and the release build, and uploading the deliverables as artifacts.
- The Ubuntu job first installs the Tauri build prerequisites listed in the Tauri 2 documentation (WebKitGTK 4.1 development package and related libraries). Take the exact package list from that documentation, not from memory.
- Dependency checks:
  - Windows: `dumpbin /dependents` shows only system DLLs (no `vcruntime*.dll`, no `onnxruntime.dll`);
  - Linux: `objdump -p` or `ldd` shows only glibc, GTK/WebKitGTK and their usual system dependencies (no `onnxruntime`, no OpenSSL unless Tauri itself requires it).
- A smoke test installs the `.deb` in a clean `ubuntu:24.04` container and checks that the dependencies resolve.

## 10. Milestones and acceptance criteria

Implement in order. Each milestone ends with passing tests, `cargo fmt`, and `cargo clippy -- -D warnings`.

| # | Milestone | Acceptance |
|---|---|---|
| M0 | Workspace, Tauri window, CI producing Windows and Linux builds | The `.exe` starts on a clean Windows 10/11 VM; the `.deb` installs and starts on clean Ubuntu 22.04 and 24.04 VMs; the AppImage starts on Ubuntu 24.04 with `libfuse2t64`; dependency checks pass |
| M1 | Schema, migrations, data directory, portable mode, number parser | Unit tests for every input form in section 5.1 |
| M2 | OPS client, import jobs, Import screen | Fixture tests for every case in section 5.4; a live import of 20 numbers works with real credentials |
| M3 | Embedding crate with embedded weights | Parity test ≥ 0.999; throughput reported |
| M4 | Review screen, labels, zero-shot and k-NN suggestions | Full keyboard workflow; labels and history persisted |
| M5 | Logistic regression, blending, prequential log, Metrics screen | Synthetic-data tests show LR outperforming k-NN once data is sufficient; metrics match a hand-computed example |
| M6 | Thresholds, per-tag automatic mode, audit sampling | Automatic mode unavailable before eligibility; automatic disabling tested |
| M7 | Library search, similarity search, CSV/JSON export, backup/restore, tag-schema import/export, settings | Round-trip tests for backup and exports |
| M8 | Full text and drawings: retrieval policies, jobs, text conversion, TIFF→PNG, document tabs, document export | Fixture tests of section 5.5; exported `.txt` matches a reference file; a live retrieval for 10 documents works |
| M9 | Full automation and command-line mode | `neg_threshold` calibration tested on synthetic data; auto-completion, focused review and audit suspension tested; scheduled `import` works headless on both platforms |
| M10 | Polish and packaging | Each deliverable < 150 MB (AppImage < 250 MB); cold start < 3 s on a mid-range laptop on both platforms; README with build and installation instructions for both platforms |

## 11. Out of scope

macOS; Linux distributions other than Ubuntu LTS (they may work, but are not tested); ARM builds; multi-user operation or synchronisation; PDF processing; OCR of drawings; machine translation of non-English full text; use of full text or drawings for tagging (a possible later extension, e.g. embedding the first claim); non-English abstracts beyond the fallback in section 5.3; GPU inference; cloud services other than EPO OPS; OPS searches (CQL) and legal-status retrieval (possible later extensions).
