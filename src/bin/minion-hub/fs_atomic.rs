//! Atomic file writes with caller-provided validation.

use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub(crate) fn write_atomic_string<F>(
    path: &Path,
    contents: &str,
    mode: u32,
    validate: F,
) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = temp_path_for(path);
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("failed to create {}", tmp.display()))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    drop(file);

    validate(&tmp)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn write_atomic_string_if_changed<F>(
    path: &Path,
    contents: &str,
    mode: u32,
    validate: F,
) -> Result<bool>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if path.exists() && fs::read_to_string(path)? == contents {
        return Ok(false);
    }
    write_atomic_string(path, contents, mode, validate)?;
    Ok(true)
}

pub(crate) fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("minion-hub");
    path.with_file_name(format!(".{}.tmp.{}", file_name, std::process::id()))
}
