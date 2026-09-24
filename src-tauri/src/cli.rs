//! Command-line mode (SPEC 7.7): "the same binary accepts subcommands and
//! then runs without opening a window, so that imports can be scheduled."
//! `main.rs` dispatches here when any argv beyond the binary name is
//! present, before Tauri's own window/event loop ever starts.

use clap::{Parser, Subcommand, ValueEnum};
use core_lib::rusqlite::Connection;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "patent-tagger", about = "Patent Tagger - headless import/export/status")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Runs the whole import pipeline for a file of publication numbers:
    /// fetch, embed, score, auto-complete where allowed, queue the rest,
    /// and retrieve full text/drawings per policy.
    Import {
        file: PathBuf,
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
    /// Prints a summary of the current database and job queue state.
    Status,
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
        Command::Import { file, fetch_fulltext, fetch_drawings } => run_import(&file, fetch_fulltext, fetch_drawings),
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
    Ok(tokio::runtime::Builder::new_current_thread().enable_time().build()?)
}

fn run_import(file: &std::path::Path, fetch_fulltext: bool, fetch_drawings: bool) -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let _lock = crate::lock::PipelineLock::acquire(&db.data_dir)?;

    let raw_input = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", file.display()))?;

    let creds = crate::platform::credentials::load(&db.data_dir)?
        .ok_or_else(|| anyhow::anyhow!("no OPS credentials saved yet - set them via the GUI's Settings screen first"))?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    let embedder = embed_lib::BgeSmallEmbedder::load()?;

    let now = current_timestamp();
    let report = {
        let conn = db.conn.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
        crate::commands::import_numbers_into(&conn, &raw_input, &now)?
    };

    let rt = tokio_runtime()?;
    let outcomes = rt.block_on(crate::import_worker::run(&db.conn, &client, &embedder, |_| {}));

    let automation_summary = {
        let conn = db.conn.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
        crate::automation::apply_full_automation(&conn, &embedder, &now)?
    };

    if fetch_fulltext || fetch_drawings {
        let now = current_timestamp();
        let conn = db.conn.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
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
    rt.block_on(crate::retrieval_worker::run_fulltext(&db.conn, &client, |_| {}));
    rt.block_on(crate::retrieval_worker::run_drawings(&db.conn, &client, &db.data_dir, |_| {}));

    let errors = outcomes.iter().filter(|o| o.status == "error" || o.status == "not_found").count();
    println!(
        "imported: {}, duplicates: {}, unparseable: {}, needs review: {}",
        report.imported.len(),
        report.duplicates.len(),
        report.unparseable.len(),
        report.needs_normalisation.len(),
    );
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

fn run_export(tag_name: &str, out: &std::path::Path, format: ExportFormat) -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let conn = db.conn.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;

    let tag = core_lib::tags::list_active(&conn)?
        .into_iter()
        .find(|t| t.name == tag_name)
        .ok_or_else(|| anyhow::anyhow!("no active tag named {tag_name:?}"))?;

    std::fs::create_dir_all(out)?;

    match format {
        ExportFormat::Txt => {
            let rows = core_lib::library::search(
                &conn,
                &core_lib::library::LibraryFilters { include_tag_ids: vec![tag.id], ..Default::default() },
            )?;
            let mut exported = 0;
            for row in &rows {
                if crate::library::export_document_folder(&conn, &db.data_dir, row.id, out)? {
                    exported += 1;
                }
            }
            println!("exported {exported} document folder(s) to {}", out.display());
        }
        ExportFormat::Csv => {
            let rows = tagged_export_rows(&conn, tag_name)?;
            write_export_file(out, &format!("{tag_name}.csv"), &core_lib::export::to_csv(&rows), rows.len())?;
        }
        ExportFormat::Json => {
            let rows = tagged_export_rows(&conn, tag_name)?;
            write_export_file(out, &format!("{tag_name}.json"), &serde_json::to_string_pretty(&rows)?, rows.len())?;
        }
    }
    Ok(())
}

fn tagged_export_rows(conn: &Connection, tag_name: &str) -> anyhow::Result<Vec<core_lib::export::ExportRow>> {
    Ok(core_lib::export::export_rows(conn)?
        .into_iter()
        .filter(|row| row.tags.iter().any(|t| t == tag_name))
        .collect())
}

fn write_export_file(out: &std::path::Path, file_name: &str, contents: &str, count: usize) -> anyhow::Result<()> {
    let path = out.join(file_name);
    std::fs::write(&path, contents)?;
    println!("exported {count} document(s) to {}", path.display());
    Ok(())
}

fn run_status() -> anyhow::Result<()> {
    let db = crate::db::open()?;
    let conn = db.conn.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;

    println!("data directory: {}", db.data_dir.display());
    println!(
        "OPS credentials: {}",
        if crate::platform::credentials::load(&db.data_dir)?.is_some() { "configured" } else { "not configured" }
    );

    for state in ["queued", "validated", "auto_completed", "skipped"] {
        let count = document_count_by_review_state(&conn, state)?;
        println!("documents ({state}): {count}");
    }
    for kind in ["import_document", "fulltext_retrieval", "drawings_retrieval"] {
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
