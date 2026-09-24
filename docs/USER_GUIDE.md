# Patent Tagger — User Guide

Patent Tagger imports patents from a list of publication numbers, fetches their bibliographic data and English abstract from EPO Open Patent Services (OPS), and helps you tag them. It learns from every document you validate and suggests tags for new ones. Tag by tag, once its measured precision is good enough, it can take over completely.

The application has six tabs: **Import**, **Review**, **Tags**, **Metrics**, **Library** and **Settings**. A typical first session goes: enter OPS credentials in Settings, create a few tags, import numbers, then review.

## 1. First steps

### OPS credentials

You need a free EPO OPS account. Register at the EPO developer portal and create an app to get a *consumer key* and a *consumer secret*.

In **Settings → EPO OPS**, open *EPO OPS credentials*, enter both values, click **Save**, then **Test connection**. The credentials go to your operating system's credential store (Windows Credential Manager, or GNOME Keyring on Ubuntu), never to the database or the logs. If no credential store is available, they are kept in a file readable only by you in the data folder.

### Where your data lives

**Settings → Data folder** shows where the database, drawings and logs are stored:

- Windows: `%LOCALAPPDATA%\PatentTagger\`
- Ubuntu: `~/.local/share/patent-tagger/`

**Portable mode:** put an empty file named `portable.flag` next to the executable (next to the `.AppImage` file on Linux) and the application keeps everything in a `data/` folder beside it instead. The folder must be writable.

## 2. Tags

Create your tags before you start reviewing. Suggestions are only as good as the definitions you give them.

In the **Tags** tab, click **New tag** and fill in:

- **Name** and **Definition**: both required. Before any examples exist, suggestions come from comparing each document with the text "name: definition", so write the definition as a short, concrete description of what qualifies.
- **Parent**: optional, for organising tags into a tree. It only affects display and has no effect on suggestions or learning.
- **Colour**: optional.
- **Hotkey**: one character that toggles the tag on the Review screen. `J`, `K`, `S` and `/` are reserved, and two active tags can't share a hotkey.

Select a tag in the list to edit it. The **Labels** table shows how many documents have it as human or automatic positive and negative, how many reviewed documents have no label for it (*unknown*), and how many labels were made under an older definition (*stale*).

### Changing a definition

When you edit a definition, the **Material change: bump to version N** box is ticked for you. Leave it ticked if the meaning changed, and untick it for a mere rewording. After a bump, existing labels stay in use for learning but count as stale, so you can re-check them. **Discard stale labels…** stops using them altogether: those documents become unknown for the tag. The change history is kept either way.

### Reviewing a tag against existing documents

A tag created after you have already validated documents has no label for them. **Review against existing documents** opens a queue of the unknown and stale documents for that tag alone, highest score first. Answer **Yes** (`Y`) or **No** (`N`) for each one; `J`/`K` move without deciding, and `Esc` goes back to the tag list. This labels only that one tag and doesn't send the document back to the main Review queue.

### Archiving

**Archive** hides a tag from review and suggestions. Archived tags are listed under *Archived* at the bottom of the list, and **Restore** brings one back. Documents validated while the tag was archived are unknown for it, and they appear in its review queue.

## 3. Importing documents

In the **Import** tab, paste publication numbers (one per line) or click **Choose file…** to load a `.txt` or `.csv` file, then click **Import**. Common forms are accepted, with or without kind codes, spaces or punctuation:

```
EP 1 234 567 B1
EP1234567
US 2020/0123456 A1
US11000000B2
WO2019/123456
WO 2019123456 A1
```

The A1 and B1 of the same number are one document. The application fetches each document in the background, and the report then lists:

| Section | Meaning |
|---|---|
| Imported | Fetched, and in the Review queue. |
| Duplicates | Already in your input or in the database; skipped. |
| Related documents | Existing documents from the same application or patent family. |
| No English abstract | No English abstract was found for the publication, its application or its family. These documents aren't queued for review. |
| Needs manual number lookup | The number looks valid, but its format couldn't be interpreted with certainty. Check it and import it in a standard form. |
| Could not parse | Not recognised as a publication number. |
| Not found / errors | OPS doesn't know the number, or the request failed. **Retry** tries again. |

When the requested publication has no English abstract, the application takes one from another publication of the same application, then from a family member (WO, then US, GB, EP). The source is recorded.

OPS limits how fast and how much you can download. The application slows down or pauses when OPS asks it to. An import interrupted by closing the application resumes at the next launch.

## 4. Reviewing

The **Review** tab is the main screen and is built for the keyboard.

- **Left: the queue.** *Import order*, or *Most uncertain first*, which puts documents whose scores are closest to a decision at the top.
- **Centre: the document.** Publication number, title, applicants, date, CPC, and **Open in Espacenet**. The **Description**, **Claims** and **Drawings** tabs appear once those have been retrieved, each with the publication it came from. Click a drawing to enlarge it.
- **Right: the tags.** Each tag has a score bar. Tags the application is confident about are already ticked. Faint entries are weak guesses based only on the tag definition; they are never ticked for you.

Tick the tags that apply and validate. Validating means *every* tag has been considered: ticked tags are recorded as present, unticked ones as absent.

| Key | Action |
|---|---|
| `J` / `K` | Next / previous document |
| tag hotkey | Toggle that tag |
| `Enter` | Validate and move on |
| `S` | Skip (leave undecided) |
| `/` | Filter the tag list |

Each shortcut also has a button in the toolbar.

### How suggestions improve

At first, suggestions only compare the document with each tag's definition. As you validate, the application uses your most similar past decisions, and once a tag has at least 5 positive and 5 negative examples it trains a dedicated model for it. The models retrain in the background every 10 validations; **Metrics → Retrain now** forces it. Only your own decisions are used for learning, never automatic ones.

## 5. Automation

Automation is opt-in at two levels. Nothing is decided without you until you switch it on.

### Automatic mode per tag

Every score is recorded before you validate a document, so the application can measure how accurate its suggestions really were. From these measurements it derives, for each tag, the lowest score that reaches your **target precision** (Settings, default 0.95).

A tag becomes **eligible** for automatic mode once it has at least 30 positives, 150 evaluated documents, and reaches the target precision. Its status is shown on the Tags and Metrics screens. Enable it with **Enable automatic mode** on the Tags screen.

A tag in automatic mode is ticked automatically when its score is high enough, and marked **auto** in the tag pane. The document still comes to you for the other tags. A share of these documents is sent to you for checking; if the checked precision drops below the target, automatic mode for the tag is switched off and you are told.

### Full automation

With **Settings → Full automation** on (it needs at least one tag in automatic mode), a new document skips review entirely when *every* active tag can be decided confidently, present or absent. Such documents are *auto-completed*.

Everything else goes to the queue in **focused review**: confident tags are already filled and marked **auto**, and the uncertain ones are highlighted. You only need to decide those, then validate. Validating counts as your decision on every tag.

Safeguards:

- **Audit rate** (default 5 %): this share of auto-completed documents comes back to you as a full review. If checked precision or recall falls below its target for a tag, that tag's automatic mode is suspended.
- **Target recall** (default 0.95) sets how sure the application must be before it records a tag as absent.
- A **new tag** stops auto-completion until it becomes eligible itself. Until then, documents come to you in focused review for that tag only.

## 6. Metrics

The **Metrics** tab shows how close you are to full automation:

- **Readiness**: the share of recent documents that *would* have been auto-completed with the current settings, plus counts of auto-completed, audited and focused-review documents.
- **Per-tag table**:
  - *Support*: positives out of evaluated documents.
  - *Precision* and *Recall*: measured over the last 300 validated documents.
  - *Threshold*: the score at or above which the tag counts as present.
  - *Neg. threshold*: the score below which it counts as absent.
  - *Audit*: results of the checks on automatic decisions.
  - *Automatic mode*: eligibility and status.
  - *PR curve*: the trade-off between precision and recall.

"Not calibrated" means there isn't enough data yet; a score of 0.5 is used meanwhile.

## 7. Full text and drawings

Tagging uses only the title and abstract. Once a document is tagged, the application can also retrieve its full text (description and claims) and its drawings, for reading and export. **Settings → Retrieval policies** sets when, separately for each:

- **Never**
- **On demand**: use **Retrieve full text now** or the Drawings tab's **Retrieve now** in the Review screen, or the Library's bulk buttons.
- **After tagging, for all documents**
- **After tagging, for selected tags**

The defaults are full text after tagging, and drawings on demand. Drawings are much heavier and use a separate OPS quota.

Full text is taken in English where it exists, from the same publication, its application or its family. If only another language exists, it is stored and labelled with its language code. Some publications have no full text or no drawings in OPS; this is shown rather than treated as an error.

## 8. Library and export

The **Library** tab lists every document. Search titles and abstracts, filter by label source, review state, and full-text or drawings availability, and click **+** or **−** on a tag to require or exclude it. **similar** lists the ten most similar documents.

Exports:

- **Export CSV** and **Export JSON**: every fetched document, with its publication number, title, tags and source publication.
- **list** (next to a tag): a `.txt` file with the publication numbers carrying that tag.
- **Export selected…**: one folder per ticked document, holding a `.txt` file with the bibliographic data, tags, sources, abstract, description and claims, plus the drawings as PNG files. Missing parts are stated in the file.

Tick documents to **Retrieve full text for selected** or **Retrieve drawings for selected**.

## 9. Backup and sharing tags

In **Settings**:

- **Back up now…** writes the database and all drawings to one `.zip` file. **Restore from backup…** replaces your data with a backup's; restart the application afterwards.
- **Export tags…** saves your active tags' definitions (names, definitions, parents, colours, hotkeys) as JSON. **Import tags…** adds the tags from such a file. Thresholds, automatic mode and labels are not part of the file; imported tags start from scratch.

### Tag file format

A tag file is a JSON array with one object per tag. You can write one by hand to set up a tag scheme in one go:

```json
[
  {
    "name": "Energy storage",
    "definition": "Devices or methods that store electrical, chemical or thermal energy for later use.",
    "color": "#2e7d32",
    "hotkey": "e"
  },
  {
    "name": "Battery",
    "definition": "Electrochemical cells and battery packs, including their electrodes, electrolytes and management systems.",
    "parent": "Energy storage",
    "color": "#1565c0",
    "hotkey": "b"
  },
  {
    "name": "Supercapacitor",
    "definition": "Electric double-layer or pseudo-capacitors used for energy storage.",
    "parent": "Energy storage"
  }
]
```

| Field | Required | Content |
|---|---|---|
| `name` | yes | The tag name. |
| `definition` | yes | What qualifies for the tag. It drives the first suggestions, so make it concrete. |
| `parent` | no | The *name* of the parent tag. It can be a tag defined later in the same file or one that already exists. |
| `color` | no | A CSS colour, e.g. `"#1565c0"`. |
| `hotkey` | no | A single character for the Review screen. |

Rules applied on import:

- A tag whose name already exists, including an archived one, is skipped, so existing tags and what they have learned are never changed.
- A hotkey that is reserved (`J`, `K`, `S`, `/`), longer than one character, or already used by an active tag is dropped. The tag is still created without it.
- A parent that doesn't exist or would create a cycle is ignored, and the tag is placed at the top level.
- Both cases are listed in the message shown after the import.
- `name` and `definition` must not be empty. An entry with an empty `name` or `definition` stops the import with an error. Tags earlier in the file will already have been created.

## 10. Command-line mode

The same executable runs without a window when given a command, so imports can be scheduled with Windows Task Scheduler, `cron` or a systemd timer:

```
patent-tagger import numbers.txt [--fetch-fulltext] [--fetch-drawings]
patent-tagger export --tag "<tag name>" --out <folder> [--format txt|csv|json]
patent-tagger status
```

- `import` runs the whole pipeline: fetch, suggest, auto-complete where allowed, queue the rest, and retrieve full text and drawings according to your policies. `--fetch-fulltext` and `--fetch-drawings` retrieve them for every imported document regardless of policy. It prints a summary and exits with a non-zero code if any document failed.
- `export` writes the documents carrying a tag: one folder per document (`txt`, the default), or a single CSV or JSON file.
- `status` prints the data folder, document counts per review state, and pending or failed jobs.

The application can stay open during a scheduled import: you can keep reviewing. Only one import or retrieval runs at a time, so starting another from the window meanwhile reports "pipeline already running".

## 11. Known limitations

- A document without an English abstract can't be reviewed yet: the application has no way to enter an abstract by hand.
- OPS quota usage isn't shown on the Metrics screen.
- In the review queue, audit samples aren't marked as such.
