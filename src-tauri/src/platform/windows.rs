use std::path::PathBuf;

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
