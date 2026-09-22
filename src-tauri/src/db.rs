//! Tauri-managed state wrapping the core database connection.

use core_lib::rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Db {
    pub conn: Mutex<Connection>,
    pub data_dir: PathBuf,
}

/// Resolves the data directory (portable mode aware), creates it if
/// missing, and opens/migrates the database inside it.
pub fn open() -> anyhow::Result<Db> {
    let data_dir = crate::platform::resolve_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let conn = core_lib::storage::open(&data_dir.join("patent-tagger.sqlite3"))?;
    Ok(Db {
        conn: Mutex::new(conn),
        data_dir,
    })
}
