//! Turn-taking between the GUI's pipelines (SPEC 7.7): imports, search
//! batches and retrievals share OPS and the job queue, so only one of them
//! processes a job at a time. Instead of failing while another runs, each
//! waits for a turn. A turn covers one job, so a "retrieve now" click gets
//! in after the current document even during a long import. Waiters are
//! served in arrival order (`tokio::sync::Mutex` is fair). A turn also
//! holds the `import.lock` file lock, so a command-line import and the GUI
//! never run a job at the same time.

use crate::db::Db;
use crate::lock::{PipelineLock, WAIT_POLL};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

pub struct Pipeline {
    turns: tokio::sync::Mutex<()>,
    data_dir: PathBuf,
    /// A background retrieval run is going (see [`spawn_retrieval`]).
    retrieval_running: AtomicBool,
    /// A background import run is going (see [`spawn_import`]).
    import_running: AtomicBool,
}

/// One job's worth of exclusive access. Released on drop.
pub struct Turn<'a> {
    _guard: tokio::sync::MutexGuard<'a, ()>,
    _file: PipelineLock,
}

impl Pipeline {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            turns: tokio::sync::Mutex::new(()),
            data_dir,
            retrieval_running: AtomicBool::new(false),
            import_running: AtomicBool::new(false),
        }
    }

    pub fn retrieval_running(&self) -> bool {
        self.retrieval_running.load(Ordering::SeqCst)
    }

    pub fn import_running(&self) -> bool {
        self.import_running.load(Ordering::SeqCst)
    }

    /// Waits for the other GUI pipelines, then for a command-line import
    /// holding the file lock.
    pub async fn turn(&self) -> anyhow::Result<Turn<'_>> {
        let guard = self.turns.lock().await;
        loop {
            if let Some(file) = PipelineLock::try_acquire(&self.data_dir)? {
                return Ok(Turn {
                    _guard: guard,
                    _file: file,
                });
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }
}

/// Retrieves pending full text and drawings in the background (SPEC 5.5:
/// "after tagging" policies), until none is left. Called after imports,
/// validations and at startup. When a run is already going it does nothing:
/// that run picks up newly queued jobs before it ends.
pub fn spawn_retrieval(app: &AppHandle) {
    if app
        .state::<Pipeline>()
        .retrieval_running
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let db = app.state::<Db>();
        let pipeline = app.state::<Pipeline>();
        match crate::platform::credentials::load(&db.data_dir) {
            Ok(Some(creds)) => {
                let client =
                    ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
                loop {
                    let fulltext = crate::retrieval_worker::run_fulltext(
                        &db.conn,
                        &client,
                        Some(&pipeline),
                        None,
                        |_| {},
                    )
                    .await;
                    let drawings = crate::retrieval_worker::run_drawings(
                        &db.conn,
                        &client,
                        &db.data_dir,
                        Some(&pipeline),
                        None,
                        |_| {},
                    )
                    .await;
                    match (fulltext, drawings) {
                        (Ok(0), Ok(0)) => break,
                        (Ok(_), Ok(_)) => {}
                        (Err(e), _) | (_, Err(e)) => {
                            log::warn!("background retrieval stopped: {e:#}");
                            break;
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => log::warn!("background retrieval: loading OPS credentials: {e:#}"),
        }
        pipeline.retrieval_running.store(false, Ordering::SeqCst);
    });
}

/// Runs pending import jobs in the background, e.g. after the Jobs
/// screen's retry, then automation and retrieval as after an import from
/// the Import screen. Progress goes to the same `import-progress` event.
/// Does nothing when a background import run is already going.
pub fn spawn_import(app: &AppHandle) {
    if app
        .state::<Pipeline>()
        .import_running
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let db = app.state::<Db>();
        let pipeline = app.state::<Pipeline>();
        let model = app.state::<crate::model::Model>();
        match crate::commands::ops_client(&db) {
            Ok(client) => {
                let result = crate::import_worker::run(
                    &db.conn,
                    &client,
                    &model.0,
                    Some(&pipeline),
                    |outcome| {
                        let _ = app.emit("import-progress", outcome);
                    },
                )
                .await;
                if let Err(e) = result {
                    log::warn!("background import stopped: {e:#}");
                }
                match db.conn.lock() {
                    Ok(conn) => {
                        let now = crate::commands::current_timestamp();
                        if let Err(e) =
                            crate::automation::apply_full_automation(&conn, &model.0, &now)
                        {
                            log::warn!("automation after background import: {e}");
                        }
                    }
                    Err(_) => log::warn!("database mutex poisoned"),
                }
                spawn_retrieval(&app);
            }
            Err(e) => log::warn!("background import: {e}"),
        }
        pipeline.import_running.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn a_turn_waits_for_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = Pipeline::new(dir.path().to_path_buf());
        runtime().block_on(async {
            let first = pipeline.turn().await.unwrap();
            let second = tokio::time::timeout(Duration::from_millis(100), pipeline.turn()).await;
            assert!(
                second.is_err(),
                "the second turn waits while the first is held"
            );
            drop(first);
            assert!(pipeline.turn().await.is_ok());
        });
    }

    #[test]
    fn a_turn_waits_for_a_command_line_import_holding_the_file_lock() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = Pipeline::new(dir.path().to_path_buf());
        let cli = PipelineLock::try_acquire(dir.path()).unwrap().unwrap();
        runtime().block_on(async {
            let waiting = tokio::time::timeout(Duration::from_millis(600), pipeline.turn()).await;
            assert!(waiting.is_err(), "no turn while the file lock is held");
            drop(cli);
            assert!(pipeline.turn().await.is_ok());
        });
    }
}
