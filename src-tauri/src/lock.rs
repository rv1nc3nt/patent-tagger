//! SPEC 7.7: "a lock file in the data directory prevents two import
//! pipelines from running at once, whether from the GUI or the command
//! line."

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

const LOCK_FILE_NAME: &str = "import.lock";

/// Holds an exclusive OS lock on `import.lock` for as long as it lives.
pub struct PipelineLock {
    // Never read: the lock is released when this handle is closed on drop.
    _file: File,
}

impl PipelineLock {
    /// Takes an exclusive OS file lock (`flock` on Linux, `LockFileEx` on
    /// Windows, via `std::fs::File::try_lock`) on the lock file, failing
    /// immediately if another pipeline holds it. The OS releases the lock
    /// when the holding process exits, even if it crashes. The file itself
    /// is left in place, so a leftover `import.lock` never blocks a later
    /// run. Two acquisitions within the same process also conflict,
    /// because each opens its own handle.
    pub fn acquire(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("opening lock file {path:?}: {e}"))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(anyhow::anyhow!(
                "another import or retrieval pipeline is already running (lock file {path:?} is held)"
            )),
            Err(TryLockError::Error(e)) => Err(anyhow::anyhow!("locking {path:?}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquiring_twice_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = PipelineLock::acquire(dir.path()).expect("first acquire should succeed");
        assert!(
            PipelineLock::acquire(dir.path()).is_err(),
            "a second acquire should fail while the first is held"
        );

        drop(first);
        assert!(
            PipelineLock::acquire(dir.path()).is_ok(),
            "acquiring again after the first is dropped should succeed"
        );
    }

    #[test]
    fn a_leftover_lock_file_from_a_crashed_run_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        // What a crashed process leaves behind: the file, but no OS lock.
        std::fs::write(dir.path().join(LOCK_FILE_NAME), b"").unwrap();
        assert!(PipelineLock::acquire(dir.path()).is_ok());
    }
}
