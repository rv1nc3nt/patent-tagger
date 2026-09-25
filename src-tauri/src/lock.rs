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
    /// Windows, via `std::fs::File::try_lock`) on the lock file, or returns
    /// `Ok(None)` at once while another pipeline holds it. The OS releases
    /// the lock when the holding process exits, even if it crashes. The
    /// file itself is left in place, so a leftover `import.lock` never
    /// blocks a later run. Two acquisitions within the same process also
    /// conflict, because each opens its own handle.
    pub fn try_acquire(data_dir: &Path) -> anyhow::Result<Option<Self>> {
        let path = data_dir.join(LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("opening lock file {path:?}: {e}"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(anyhow::anyhow!("locking {path:?}: {e}")),
        }
    }

    /// Waits until the lock is free, for the command line: a scheduled
    /// import waits for the GUI's current work instead of failing.
    /// `on_wait` is called once, if the lock is not free at first.
    pub fn acquire_blocking(data_dir: &Path, on_wait: impl FnOnce()) -> anyhow::Result<Self> {
        let mut on_wait = Some(on_wait);
        loop {
            if let Some(lock) = Self::try_acquire(data_dir)? {
                return Ok(lock);
            }
            if let Some(f) = on_wait.take() {
                f();
            }
            std::thread::sleep(WAIT_POLL);
        }
    }
}

/// How often a waiting pipeline checks the lock again.
pub const WAIT_POLL: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use super::*;

    fn held(dir: &Path) -> PipelineLock {
        PipelineLock::try_acquire(dir)
            .expect("lock file opens")
            .expect("lock is free")
    }

    #[test]
    fn acquiring_twice_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = held(dir.path());
        assert!(
            PipelineLock::try_acquire(dir.path()).unwrap().is_none(),
            "a second acquire should find the lock held"
        );

        drop(first);
        assert!(
            PipelineLock::try_acquire(dir.path()).unwrap().is_some(),
            "acquiring again after the first is dropped should succeed"
        );
    }

    #[test]
    fn a_leftover_lock_file_from_a_crashed_run_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        // What a crashed process leaves behind: the file, but no OS lock.
        std::fs::write(dir.path().join(LOCK_FILE_NAME), b"").unwrap();
        assert!(PipelineLock::try_acquire(dir.path()).unwrap().is_some());
    }

    #[test]
    fn acquire_blocking_waits_until_the_holder_releases() {
        let dir = tempfile::tempdir().unwrap();
        let first = held(dir.path());
        let path = dir.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let lock = PipelineLock::acquire_blocking(&path, || tx.send(()).unwrap());
            lock.is_ok()
        });
        rx.recv().expect("the waiter reports that it waits");
        drop(first);
        assert!(waiter.join().unwrap());
    }
}
