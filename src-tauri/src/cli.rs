//! Command-line mode (SPEC 7.7): "the same binary accepts subcommands and
//! then runs without opening a window, so that imports can be scheduled."
//! `main.rs` dispatches here when any argv beyond the binary name is
//! present, before Tauri's own window/event loop ever starts.

use clap::{Parser, Subcommand, ValueEnum};
use core_lib::rusqlite::Connection;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "patent-tagger",
    about = "Patent Tagger - headless import/export/status"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Runs the whole import pipeline for a file of publication numbers, or
    /// for the next 100 results of a saved search: fetch, embed, score,
    /// auto-complete where allowed, queue the rest, and retrieve full
    /// text/drawings per policy.
    Import {
        #[arg(required_unless_present = "search", conflicts_with = "search")]
        file: Option<PathBuf>,
        /// Imports the next 100 results of this saved search instead of a
        /// file (see the `search` subcommand).
        #[arg(long)]
        search: Option<String>,
        /// Forces full-text retrieval for every document fetched by this
        /// run, regardless of the configured retrieval policy.
        #[arg(long)]
        fetch_fulltext: bool,
        /// Forces drawings retrieval for every document fetched by this
        /// run, regardless of the configured retrieval policy.
        #[arg(long)]
        fetch_drawings: bool,
    },
    /// Exports documents carrying `--tag` into `--out`.
    Export {
        #[arg(long)]
        tag: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = ExportFormat::Txt)]
        format: ExportFormat,
    },
    /// Manages saved searches by applicant, imported with
    /// `import --search <name>`.
    Search {
        #[command(subcommand)]
        action: SearchAction,
    },
    /// Prints a summary of the current database and job queue state.
    Status,
}

#[derive(Subcommand)]
enum SearchAction {
    /// Saves a new search.
    Add {
        name: String,
        /// Applicant name; separate name variants with `;`.
        #[arg(long)]
        applicant: String,
        /// Publication country, e.g. EP.
        #[arg(long)]
        country: Option<String>,
        /// First publication year.
        #[arg(long)]
        from: Option<i64>,
        /// Last publication year.
        #[arg(long)]
        to: Option<i64>,
    },
    /// Lists saved searches and their progress.
    List,
    /// Deletes a saved search. Documents it imported stay.
    Delete { name: String },
    /// Starts a search over from its first result.
    Restart { name: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum ExportFormat {
    Txt,
    Csv,
    Json,
}

/// Runs the requested subcommand to completion, returning the process
/// exit code (SPEC 7.7: "exits with a non-zero code on failure").
pub fn run(cli: Cli) -> i32 {
    let result = match cli.command {
        Command::Import {
            file,
            search,
            fetch_fulltext,
            fetch_drawings,
        } => {
            let source = match (file, search) {
                (_, Some(name)) => ImportSource::Search(name),
                (Some(file), None) => ImportSource::File(file),
                // clap requires one of the two (`required_unless_present`).
                (None, None) => {
                    eprintln!("error: give a file or --search <name>");
                    return 2;
                }
            };
            run_import(source, fetch_fulltext, fetch_drawings)
        }
        Command::Search { action } => run_search(action),
        Command::Export { tag, out, format } => run_export(&tag, &out, format),
        Command::Status => run_status(),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

fn tokio_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?)
}

enum ImportSource {
    File(PathBuf),
    Search(String),
}

fn run_import(
    source: ImportSource,
    fetch_fulltext: bool,
    fetch_drawings: bool,
) -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let _lock = crate::lock::PipelineLock::acquire(&db.data_dir)?;

    let creds = crate::platform::credentials::load(&db.data_dir)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no OPS credentials saved yet - set them via the GUI's Settings screen first"
        )
    })?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    let embedder = embed_lib::BgeSmallEmbedder::load()?;

    let now = current_timestamp();
    let rt = tokio_runtime()?;
    match source {
        ImportSource::File(file) => {
            let raw_input = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", file.display()))?;
            let report = {
                let conn = db
                    .conn
                    .lock()
                    .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
                crate::commands::import_numbers_into(&conn, &raw_input, &now)?
            };
            println!(
                "imported: {}, duplicates: {}, unparseable: {}, needs review: {}",
                report.imported.len(),
                report.duplicates.len(),
                report.unparseable.len(),
                report.needs_normalisation.len(),
            );
        }
        ImportSource::Search(name) => {
            let search_id = {
                let conn = db
                    .conn
                    .lock()
                    .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
                core_lib::searches::get_by_name(&conn, &name)?
                    .ok_or_else(|| anyhow::anyhow!("no saved search named {name:?}"))?
                    .id
            };
            let report = rt.block_on(crate::search::fetch_next_batch(
                &db.conn, &client, search_id, &now,
            ))?;
            print_batch(&name, &report);
        }
    }

    let outcomes = rt.block_on(crate::import_worker::run(
        &db.conn,
        &client,
        &embedder,
        |_| {},
    ));

    let automation_summary = {
        let conn = db
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
        crate::automation::apply_full_automation(&conn, &embedder, &now)?
    };

    if fetch_fulltext || fetch_drawings {
        let now = current_timestamp();
        let conn = db
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
        for outcome in outcomes.iter().filter(|o| o.status == "fetched") {
            if fetch_fulltext {
                crate::retrieval_worker::enqueue_fulltext(&conn, outcome.doc_id, &now)?;
            }
            if fetch_drawings {
                crate::retrieval_worker::enqueue_drawings(&conn, outcome.doc_id, &now)?;
            }
        }
    }
    // Drains both the "per policy" jobs enqueued by validation/auto-
    // completion above and any forced by --fetch-fulltext/--fetch-drawings.
    rt.block_on(crate::retrieval_worker::run_fulltext(
        &db.conn,
        &client,
        |_| {},
    ));
    rt.block_on(crate::retrieval_worker::run_drawings(
        &db.conn,
        &client,
        &db.data_dir,
        |_| {},
    ));

    let errors = outcomes
        .iter()
        .filter(|o| o.status == "error" || o.status == "not_found")
        .count();
    println!(
        "fetched: {}, auto-completed: {}, audited samples: {}, errors/not found: {errors}",
        outcomes.iter().filter(|o| o.status == "fetched").count(),
        automation_summary.auto_completed,
        automation_summary.audited_samples,
    );

    if errors > 0 {
        anyhow::bail!("{errors} document(s) failed to import");
    }
    Ok(())
}

fn print_batch(name: &str, report: &crate::search::BatchReport) {
    let total = report
        .search
        .as_ref()
        .and_then(|s| s.total_results)
        .unwrap_or(0);
    match report.range {
        Some((begin, end)) => println!(
            "search {name:?}: results {begin}-{end} of {total}, families: {}, \
             already in library: {}, duplicates: {}, imported: {}",
            report.families,
            report.known_families.len(),
            report.duplicates.len(),
            report.imported.len(),
        ),
        None => println!(
            "search {name:?}: no more results ({total} in total, at most {} readable); \
             use `search restart` to start over",
            core_lib::searches::MAX_RESULTS,
        ),
    }
}

fn run_search(action: SearchAction) -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let conn = db
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
    let find = |name: &str| -> anyhow::Result<core_lib::searches::SavedSearch> {
        core_lib::searches::get_by_name(&conn, name)?
            .ok_or_else(|| anyhow::anyhow!("no saved search named {name:?}"))
    };
    match action {
        SearchAction::Add {
            name,
            applicant,
            country,
            from,
            to,
        } => {
            let fields = core_lib::searches::SearchFields {
                name,
                applicant,
                country,
                year_from: from,
                year_to: to,
            };
            let search = core_lib::searches::create(&conn, &fields, &current_timestamp())?;
            println!("saved search {:?}: {}", search.name, search.query);
        }
        SearchAction::List => {
            for s in core_lib::searches::list(&conn)? {
                let progress = match s.total_results {
                    None => "not run yet".to_string(),
                    Some(total) => format!(
                        "read {} of {total}{}",
                        (s.next_start - 1).min(total),
                        if s.exhausted { " (done)" } else { "" }
                    ),
                };
                println!(
                    "{}: {} - {progress}, imported {}",
                    s.name, s.query, s.imported
                );
            }
        }
        SearchAction::Delete { name } => {
            core_lib::searches::delete(&conn, find(&name)?.id)?;
            println!("deleted search {name:?}");
        }
        SearchAction::Restart { name } => {
            core_lib::searches::restart(&conn, find(&name)?.id)?;
            println!("search {name:?} will start over from its first result");
        }
    }
    Ok(())
}

fn run_export(tag_name: &str, out: &std::path::Path, format: ExportFormat) -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let conn = db
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;

    let tag = core_lib::tags::list_active(&conn)?
        .into_iter()
        .find(|t| t.name == tag_name)
        .ok_or_else(|| anyhow::anyhow!("no active tag named {tag_name:?}"))?;

    std::fs::create_dir_all(out)?;

    match format {
        ExportFormat::Txt => {
            let rows = core_lib::library::search(
                &conn,
                &core_lib::library::LibraryFilters {
                    include_tag_ids: vec![tag.id],
                    ..Default::default()
                },
            )?;
            let tag_dir = crate::library::tag_export_dir(&conn, tag.id, out)?;
            let mut exported = 0;
            for row in &rows {
                if crate::library::export_document_folder(&conn, &db.data_dir, row.id, &tag_dir)? {
                    exported += 1;
                }
            }
            println!(
                "exported {exported} document folder(s) to {}",
                tag_dir.display()
            );
        }
        ExportFormat::Csv => {
            let rows = tagged_export_rows(&conn, tag_name)?;
            write_export_file(
                out,
                &format!("{tag_name}.csv"),
                &core_lib::export::to_csv(&rows),
                rows.len(),
            )?;
        }
        ExportFormat::Json => {
            let rows = tagged_export_rows(&conn, tag_name)?;
            write_export_file(
                out,
                &format!("{tag_name}.json"),
                &serde_json::to_string_pretty(&rows)?,
                rows.len(),
            )?;
        }
    }
    Ok(())
}

fn tagged_export_rows(
    conn: &Connection,
    tag_name: &str,
) -> anyhow::Result<Vec<core_lib::export::ExportRow>> {
    Ok(core_lib::export::export_rows(conn)?
        .into_iter()
        .filter(|row| row.tags.iter().any(|t| t == tag_name))
        .collect())
}

fn write_export_file(
    out: &std::path::Path,
    file_name: &str,
    contents: &str,
    count: usize,
) -> anyhow::Result<()> {
    let path = out.join(file_name);
    std::fs::write(&path, contents)?;
    println!("exported {count} document(s) to {}", path.display());
    Ok(())
}

fn run_status() -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let conn = db
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;

    println!("data directory: {}", db.data_dir.display());
    println!(
        "OPS credentials: {}",
        if crate::platform::credentials::load(&db.data_dir)?.is_some() {
            "configured"
        } else {
            "not configured"
        }
    );

    for state in ["queued", "validated", "auto_completed", "skipped"] {
        let count = document_count_by_review_state(&conn, state)?;
        println!("documents ({state}): {count}");
    }
    for kind in [
        "import_document",
        "fulltext_retrieval",
        "drawings_retrieval",
    ] {
        let pending = core_lib::jobs::list_resumable(&conn, kind)?.len();
        let failed = core_lib::jobs::list_failed(&conn, kind)?.len();
        println!("jobs ({kind}): {pending} pending/running, {failed} failed");
    }
    Ok(())
}

fn document_count_by_review_state(conn: &Connection, state: &str) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM documents WHERE review_state = ?1",
        core_lib::rusqlite::params![state],
        |row| row.get(0),
    )?)
}

fn current_timestamp() -> String {
    crate::commands::current_timestamp()
}
