//! Versioned project I/O. Relative assets are relative to the project, never CWD.
use crate::song::Song;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static SERIAL: AtomicU64 = AtomicU64::new(0);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u32,
    song: Song,
}
#[derive(Debug)]
pub enum SaveMode {
    CreateNew,
    ReplaceConfirmed(Vec<u8>),
}
pub(crate) fn normal_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        // canonicalize uses extended-length syntax, while normal paths are
        // used by the editor and project files.
        if let Some(text) = path.to_str() {
            if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{unc}"));
            }
            if let Some(local) = text.strip_prefix(r"\\?\") {
                return PathBuf::from(local);
            }
        }
    }
    path
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("Project path is empty".into());
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parent = path.parent().ok_or("Project needs a parent directory")?;
    // Preserve Windows 8.3 and symlink spellings when resolving relative
    // references, while still verifying that the directory exists.
    #[cfg(windows)]
    let parent = {
        let metadata = std::fs::metadata(parent).map_err(|e| format!("Project directory: {e}"))?;
        if !metadata.is_dir() {
            return Err("Project directory is not a directory".into());
        }
        std::path::absolute(parent).map_err(|e| format!("Project directory: {e}"))?
    };
    #[cfg(not(windows))]
    let parent = normal_path(
        parent
            .canonicalize()
            .map_err(|e| format!("Project directory: {e}"))?,
    );
    Ok(parent.join(path.file_name().ok_or("Project needs a filename")?))
}
/// Bound reads and reject symlinks for project files (asset references may resolve normally).
pub fn existing_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Project must be a regular file, not a symlink".into());
    }
    let mut data = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("Project exceeds 8 MiB limit".into());
    }
    Ok(data)
}
fn assets(song: &mut Song, base: &Path, loading: bool) -> Result<(), String> {
    for (id, instrument) in &mut song.instruments {
        if let Some(path) = &instrument.sample {
            if !loading && !path.is_absolute() {
                return Err(format!(
                    "Instrument {}: in-memory sample path must be absolute",
                    id.0
                ));
            }
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                base.join(path)
            };
            let file = File::open(&resolved).map_err(|e| {
                format!(
                    "Instrument {} missing/unreadable sample {}: {e}",
                    id.0,
                    resolved.display()
                )
            })?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err(format!(
                    "Instrument {} sample is not a file: {}",
                    id.0,
                    resolved.display()
                ));
            }
            instrument.sample = Some(if loading {
                resolved
            } else {
                // Compare using the canonical parent spelling. On Windows the
                // project base may have come from canonicalize (long name),
                // while an absolute sample path can retain an 8.3 alias. Do
                // not canonicalize the file itself: it may be a symlink and
                // the project should keep referring to that symlink.
                let comparable = resolved
                    .parent()
                    .and_then(|parent| parent.canonicalize().ok())
                    .and_then(|parent| {
                        resolved
                            .file_name()
                            .map(|name| normal_path(parent).join(name))
                    })
                    .unwrap_or_else(|| resolved.clone());
                let canonical_base = base.canonicalize().map(normal_path).ok();
                resolved
                    .strip_prefix(base)
                    .ok()
                    .or_else(|| {
                        canonical_base
                            .as_ref()
                            .and_then(|base| comparable.strip_prefix(base).ok())
                    })
                    .unwrap_or(&resolved)
                    .to_owned()
            });
        }
    }
    Ok(())
}
pub fn load(path: &Path) -> Result<(Song, PathBuf), String> {
    let path = absolute(path)?;
    let data = existing_bytes(&path)?;
    let mut document: Document =
        serde_json::from_slice(&data).map_err(|e| format!("Malformed project: {e}"))?;
    if document.format != "leviathan-song" || document.version != 1 {
        return Err("Unsupported project format/version".into());
    }
    document.song.validate()?;
    assets(&mut document.song, path.parent().unwrap(), true)?;
    Ok((document.song, path))
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// Writes a complete sibling temporary file, flushes it, then publishes atomically.
/// CreateNew uses a hard link so an unexpectedly existing target is never clobbered.
/// Replacement requires a freshly confirmed byte snapshot; changed targets are rejected.
/// Returns normalized path plus optional post-commit directory-sync warning.
pub fn save(song: &Song, path: &Path, mode: SaveMode) -> Result<(PathBuf, Option<String>), String> {
    song.validate()?;
    let path = absolute(path)?;
    let base = path.parent().unwrap();
    let mut stored = song.clone();
    assets(&mut stored, base, false)?;
    let data = serde_json::to_vec_pretty(&Document {
        format: "leviathan-song".into(),
        version: 1,
        song: stored,
    })
    .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("Project exceeds 8 MiB limit".into());
    }
    let mut allocated = None;
    for _ in 0..100 {
        let temp = base.join(format!(
            ".leviathan-save-{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => {
                allocated = Some((Temp(temp), file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create save temporary file: {e}")),
        }
    }
    let (temp, mut file) = allocated.ok_or("Cannot allocate save temporary file")?;
    file.write_all(&data)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Cannot write project: {e}"))?;
    drop(file);
    match mode {
        SaveMode::CreateNew => fs::hard_link(&temp.0, &path).map_err(|e| {
            format!("Cannot publish new project (existing files need confirmation): {e}")
        })?,
        SaveMode::ReplaceConfirmed(expected) => {
            if existing_bytes(&path)? != expected {
                return Err("Project changed since overwrite confirmation; save cancelled".into());
            }
            fs::rename(&temp.0, &path).map_err(|e| format!("Cannot replace project: {e}"))?;
        }
    }
    drop(temp);
    let warning = File::open(base).and_then(|f| f.sync_all()).err().map(|e| {
        format!("Project committed, but directory durability could not be confirmed: {e}")
    });
    Ok((path, warning))
}
