//! OS-specific code (SPEC section 2): data directory resolution, portable
//! mode, credential store (M2) and startup checks (M0/M10). This is the only
//! module allowed to branch on target OS.

pub mod credentials;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

use std::path::{Path, PathBuf};

/// Resolves the application's data directory, honouring portable mode
/// (SPEC section 4.1): a `portable.flag` file next to the executable (or,
/// on Linux, next to the AppImage) selects `<that dir>/data` instead of the
/// OS default, provided that directory is writable.
pub fn resolve_data_dir() -> anyhow::Result<PathBuf> {
    let portable_check_dir = portable_check_dir()?;
    if is_portable(&portable_check_dir) {
        return Ok(portable_check_dir.join("data"));
    }
    default_data_dir()
}

#[cfg(windows)]
fn portable_check_dir() -> anyhow::Result<PathBuf> {
    windows::exe_dir()
}

#[cfg(windows)]
fn default_data_dir() -> anyhow::Result<PathBuf> {
    windows::default_data_dir()
}

#[cfg(target_os = "linux")]
fn portable_check_dir() -> anyhow::Result<PathBuf> {
    linux::portable_check_dir()
}

#[cfg(target_os = "linux")]
fn default_data_dir() -> anyhow::Result<PathBuf> {
    linux::default_data_dir()
}

/// SPEC 7.7: "on Windows the release binary uses the GUI subsystem, so
/// command-line mode must attach to the parent console...to print
/// output." A no-op on Linux, where the release binary is a normal
/// console-attached process already. Written against the documented
/// `AttachConsole` API but unverified on a real Windows build (no
/// Windows target available in this sandbox - see docs/DECISIONS.md).
#[cfg(windows)]
pub fn attach_parent_console() {
    windows::attach_parent_console();
}

#[cfg(not(windows))]
pub fn attach_parent_console() {}

/// True when `dir` contains a `portable.flag` file and is itself writable.
/// Pure with respect to its argument, so it's testable without touching the
/// real executable path or environment.
fn is_portable(dir: &Path) -> bool {
    dir.join("portable.flag").is_file() && is_writable(dir)
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".patent-tagger-write-test");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_portable_without_flag_file() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        assert!(!is_portable(dir.path()));
    }

    #[test]
    fn portable_when_flag_file_present_and_dir_writable() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        std::fs::write(dir.path().join("portable.flag"), "").expect("write should succeed");
        assert!(is_portable(dir.path()));
    }

    #[test]
    #[cfg(unix)]
    fn not_portable_when_dir_is_read_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        std::fs::write(dir.path().join("portable.flag"), "").expect("write should succeed");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555))
            .expect("chmod should succeed");

        let portable = is_portable(dir.path());

        // Restore write permission so the tempdir cleans itself up.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
            .expect("chmod should succeed");

        assert!(!portable, "a read-only directory must not be treated as portable");
    }
}
