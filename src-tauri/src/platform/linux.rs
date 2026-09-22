use std::path::PathBuf;

/// The directory to check for `portable.flag` in. For a normal binary this
/// is the executable's directory; for an AppImage it's the directory
/// containing the `.AppImage` file itself (`$APPIMAGE`), since the
/// executable Tauri sees is a path inside the read-only squashfs mount.
pub(super) fn portable_check_dir() -> anyhow::Result<PathBuf> {
    if let Ok(appimage_path) = std::env::var("APPIMAGE") {
        if let Some(parent) = std::path::Path::new(&appimage_path).parent() {
            return Ok(parent.to_path_buf());
        }
    }
    exe_dir()
}

fn exe_dir() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("executable path {exe:?} has no parent directory"))
}

/// `$XDG_DATA_HOME/patent-tagger/`, i.e. `~/.local/share/patent-tagger/` by
/// default (SPEC section 4.1).
pub(super) fn default_data_dir() -> anyhow::Result<PathBuf> {
    let base = directories::BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("could not determine the user's data directory"))?;
    Ok(base.data_dir().join("patent-tagger"))
}
