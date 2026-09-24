//! SPEC 7.7: "a lock file in the data directory prevents two import
//! pipelines from running at once, whether from the GUI or the command
//! line."

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

const LOCK_FILE_NAME: &str = "import.lock";

pub struct PipelineLock {
    path: PathBuf,
}

impl PipelineLock {
    /// Atomically creates the lock file, failing if it already exists -
    /// a simple, portable "poor man's lock" rather than a true OS advisory
    /// lock (which would need a platform-specific API or a new
    /// dependency). A lock left behind by a process that crashed instead
    /// of exiting normally (so [`Drop`] never ran) has to be removed
    /// manually before another pipeline can run - an accepted simplicity
    /// trade-off given how rarely that should happen in practice.
    pub fn acquire(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(LOCK_FILE_NAME);
        OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
            anyhow::anyhow!(
                "another import or retrieval pipeline appears to already be running \
                 (lock file {path:?} already exists - delete it if a previous run crashed \
                 without cleaning up): {e}"
            )
        })?;
        Ok(Self { path })
    }
}

impl Drop for PipelineLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquiring_twice_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = PipelineLock::acquire(dir.path()).expect("first acquire should succeed");
        assert!(PipelineLock::acquire(dir.path()).is_err(), "a second acquire should fail while the first is held");

        drop(first);
        assert!(PipelineLock::acquire(dir.path()).is_ok(), "acquiring again after the first is dropped should succeed");
    }

    #[test]
    fn drop_removes_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILE_NAME);
        let lock = PipelineLock::acquire(dir.path()).unwrap();
        assert!(lock_path.exists());
        drop(lock);
        assert!(!lock_path.exists());
    }
}
