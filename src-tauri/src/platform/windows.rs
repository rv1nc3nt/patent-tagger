use std::path::PathBuf;

/// SPEC 7.7. `AttachConsole(ATTACH_PARENT_PROCESS)` fails harmlessly (and
/// is ignored here) when there is no parent console to attach to, e.g.
/// the app was double-clicked rather than launched from a terminal -
/// `import`/`export`/`status` were never going to print anywhere useful
/// in that case either.
pub(super) fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// The executable's own directory, checked for `portable.flag`.
pub(super) fn exe_dir() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("executable path {exe:?} has no parent directory"))
}

/// `%LOCALAPPDATA%\PatentTagger\` (SPEC section 4.1).
pub(super) fn default_data_dir() -> anyhow::Result<PathBuf> {
    let base = directories::BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("could not determine the user's data directory"))?;
    Ok(base.data_local_dir().join("PatentTagger"))
}
