//! Versioned project I/O. Relative assets are relative to the project, never CWD.
use crate::song::Song;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "legacy_project.rs"]
mod legacy_project;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REGISTERED_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
const PROJECT_DIALECT: &str = "recovered-meters-v1";
static SERIAL: AtomicU64 = AtomicU64::new(0);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u32,
    #[serde(default)]
    dialect: Option<String>,
    song: Song,
}
#[derive(Serialize)]
struct DocumentRef<'a> {
    format: &'static str,
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    dialect: Option<&'static str>,
    song: &'a Song,
}
struct BoundedWriter<W> {
    inner: W,
    written: usize,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > MAX_BYTES - self.written as u64 {
            return Err(std::io::Error::other("Project exceeds 8 MiB encoded limit"));
        }
        let written = self.inner.write(bytes)?;
        self.written += written;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
fn encode_document(song: &Song, writer: impl Write) -> Result<usize, String> {
    let mut bounded = BoundedWriter {
        inner: writer,
        written: 0,
    };
    serde_json::to_writer_pretty(
        &mut bounded,
        &DocumentRef {
            format: "leviathan-song",
            version: 1,
            dialect: legacy_project::needs_dialect_marker(song).then_some(PROJECT_DIALECT),
            song,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(bounded.written)
}
/// Pure, bounded count of the exact pretty project wrapper with current paths.
/// No asset IO, model validation, output allocation, or save-readiness promise.
/// Normal absolute paths conservatively bound storage after prefix stripping;
/// Save still validates assets and rechecks its actual normalized encoding.
pub fn check_serialized_size(song: &Song) -> Result<usize, String> {
    encode_document(song, std::io::sink())
}
/// Exact persisted-document equality, including signed zero and lexical paths.
/// A structural difference returns false without encoding. Otherwise both exact
/// pretty wrappers are bounded by the existing 8 MiB limit. This is pure: no
/// model validation, path normalization, asset IO or save-readiness claim.
/// Equal-looking oversized/unserializable values return an error, never a
/// guessed no-op. Callers must decide failure policy before changing history.
pub fn same_serialized_song(left: &Song, right: &Song) -> Result<bool, String> {
    if left != right {
        return Ok(false);
    }
    let mut encoded = Vec::new();
    encode_document(left, &mut encoded)?;
    let mut comparison = ComparingWriter {
        expected: &encoded,
        written: 0,
        equal: true,
    };
    encode_document(right, &mut comparison)?;
    Ok(comparison.equal && comparison.written == encoded.len())
}

struct ComparingWriter<'a> {
    expected: &'a [u8],
    written: usize,
    equal: bool,
}
impl Write for ComparingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .written
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("Document comparison length overflow"))?;
        self.equal &= self.expected.get(self.written..end) == Some(bytes);
        self.written = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[derive(Debug)]
pub enum SaveMode {
    CreateNew,
    ReplaceConfirmed(Vec<u8>),
}
pub(crate) fn absolute(path: &Path) -> Result<PathBuf, String> {
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
    let parent = path
        .parent()
        .ok_or("Project needs a parent directory")?
        .canonicalize()
        .map_err(|e| format!("Project directory: {e}"))?;
    let canonical = parent.join(path.file_name().ok_or("Project needs a filename")?);
    Ok(normalize_canonical_path(canonical))
}

/// Windows `canonicalize` returns extended-length (`\\?\`) paths even for
/// ordinary short DOS/UNC paths. Keep those paths in their ordinary spelling
/// unless a component needs verbatim semantics or the result exceeds Win32's
/// legacy path limit. Safe, ordinary-length `\\?\C:\...` inputs are
/// equivalent to their normal DOS spelling and are normalized too; Windows
/// itself may return that form for paths created by `temp_dir()`.
/// This keeps model identity and UI-visible paths stable while preserving
/// paths that genuinely need the extended-length namespace.
pub(crate) fn normalize_canonical_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::path::{Component, Prefix};

        if path.as_os_str().encode_wide().count() >= 260 {
            return path;
        }
        let components: Vec<_> = path.components().collect();
        if components.iter().any(|component| {
            let Component::Normal(name) = component else {
                return false;
            };
            let display = name.to_string_lossy();
            let last = display.chars().last();
            last.is_some_and(|ch| ch == ' ' || ch == '.') || reserved_windows_device_name(&display)
        }) {
            return path;
        }

        let Some(Component::Prefix(prefix)) = components.first() else {
            return path;
        };
        let mut output = match prefix.kind() {
            Prefix::VerbatimDisk(disk) => PathBuf::from(format!("{}:", char::from(disk))),
            Prefix::VerbatimUNC(server, share) => {
                let mut unc = PathBuf::from(r"\\");
                unc.push(server);
                unc.push(share);
                unc
            }
            _ => return path,
        };
        for component in components.iter().skip(1) {
            output.push(component.as_os_str());
        }
        output
    }
    #[cfg(not(windows))]
    {
        path
    }
}

#[cfg(windows)]
fn reserved_windows_device_name(component: &str) -> bool {
    let normalized = component.trim_end_matches([' ', '.']);
    let stem = normalized
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.']);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || upper
        .strip_prefix("COM")
        .is_some_and(is_reserved_device_digit)
        || upper
            .strip_prefix("LPT")
            .is_some_and(is_reserved_device_digit)
}

#[cfg(windows)]
fn is_reserved_device_digit(suffix: &str) -> bool {
    matches!(
        suffix,
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
    )
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::normalize_canonical_path;
    use std::path::PathBuf;

    #[test]
    fn long_targets_keep_the_prefix_even_when_the_parent_is_short() {
        let path = PathBuf::from(format!(
            r"\\?\C:\{}\{}.json",
            "segment".repeat(28),
            "f".repeat(80)
        ));
        assert!(path.parent().unwrap().to_string_lossy().len() < 260);
        assert!(path.to_string_lossy().len() >= 260);
        let normalized = normalize_canonical_path(path.clone());
        assert_eq!(normalized, path);
    }

    #[test]
    fn reserved_dos_device_spelling_keeps_verbatim_prefix() {
        let path = PathBuf::from(r"\\?\C:\safe\CON .txt");
        assert_eq!(normalize_canonical_path(path.clone()), path);
    }

    #[test]
    fn short_verbatim_disk_paths_normalize_to_dos_paths() {
        let path = PathBuf::from(r"\\?\C:\safe\song.json");
        assert_eq!(
            normalize_canonical_path(path),
            PathBuf::from(r"C:\safe\song.json")
        );
    }
}
/// Bound reads and reject symlinks for project files (asset references may resolve normally).
pub fn existing_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Project must be a regular file, not a symlink".into());
    }
    if metadata.len() > MAX_BYTES {
        return Err("Project exceeds 8 MiB limit".into());
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
fn resolve_asset(
    path: &Path,
    base: &Path,
    loading: bool,
    label: &str,
    policy: &mut impl FnMut(&Path) -> Result<(), String>,
) -> Result<(PathBuf, u64), String> {
    if !loading && !path.is_absolute() {
        return Err(format!("{label}: in-memory sample path must be absolute"));
    }
    let resolved = if path.is_absolute() {
        normalize_canonical_path(path.to_owned())
    } else {
        normalize_canonical_path(base.join(path))
    };
    policy(&resolved).map_err(|e| format!("{label} sample {} denied: {e}", resolved.display()))?;
    // Opening a FIFO can block the synchronous project controller forever.
    // Asset symlinks are allowed, but their resolved target must be a file.
    let unreadable = |e| {
        format!(
            "{label} missing/unreadable sample {}: {e}",
            resolved.display()
        )
    };
    let metadata = fs::metadata(&resolved).map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(format!(
            "{label} sample is not a file: {}",
            resolved.display()
        ));
    }
    let file = File::open(&resolved).map_err(unreadable)?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err(format!(
            "{label} sample is not a file: {}",
            resolved.display()
        ));
    }
    Ok((resolved, metadata.len()))
}
fn stored_asset_path(resolved: PathBuf, base: &Path, loading: bool) -> PathBuf {
    if loading {
        resolved
    } else {
        resolved.strip_prefix(base).unwrap_or(&resolved).to_owned()
    }
}
fn assets(
    song: &mut Song,
    base: &Path,
    loading: bool,
    rewrite_paths: bool,
    mut policy: impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    for (id, instrument) in &mut song.instruments {
        if let Some(path) = &instrument.sample {
            let (resolved, _) = resolve_asset(
                path,
                base,
                loading,
                &format!("Instrument {}", id.0),
                &mut policy,
            )?;
            if rewrite_paths {
                instrument.sample = Some(stored_asset_path(resolved, base, loading));
            }
        }
    }
    let mut total_source_bytes = 0u64;
    let mut sources = Vec::with_capacity(song.samples.len());
    for (id, sample) in &song.samples {
        let (resolved, bytes) = resolve_asset(
            &sample.path,
            base,
            loading,
            &format!("Source {}", id.0),
            &mut policy,
        )?;
        if bytes > crate::sample_asset::MAX_ENCODED_BYTES as u64 {
            return Err(format!("Source {} exceeds the 64 MiB encoded budget", id.0));
        }
        total_source_bytes = total_source_bytes.saturating_add(bytes);
        if total_source_bytes > MAX_REGISTERED_SOURCE_BYTES {
            return Err("Registered sources exceed the 256 MiB aggregate encoded budget; remove unused sources or use smaller files".into());
        }
        sources.push((*id, resolved));
    }
    // Complete bounded path/size preflight before hashing any registered source.
    for (id, resolved) in sources {
        let sample = song.samples.get_mut(&id).unwrap();
        // Registered identity must remain true even for currently unused sources.
        // Import/engine decoding separately verifies native audio metadata.
        crate::sample_asset::verify_identity(sample, &resolved)
            .map_err(|e| format!("Source {}: {e}", id.0))?;
        if rewrite_paths {
            sample.path = stored_asset_path(resolved, base, loading);
        }
    }
    Ok(())
}
/// Off-thread, non-publishing preflight for adding references to an existing
/// document. Reuses save's all-source policy: absolute readable legacy paths,
/// registered identity hashes, 64 MiB/file and 256 MiB aggregate encoded bounds.
/// Legacy samples are not decoded or hashed. Captured absolute-path pretty JSON
/// is checked conservatively; this is not an arbitrary future Save As guarantee.
/// No project/asset is created or repaired, and caller paths/content are unchanged.
/// Authorization precedes every asset metadata/open; filesystem TOCTOU remains.
pub fn check_registration_sources(
    song: &Song,
    authorize: impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    check_serialized_size(song)?;
    song.validate()?;
    let mut inspected = song.clone();
    // Save-mode resolution rejects relative in-memory paths, so this empty base
    // is never used to infer a CWD or rewrite an inspected path.
    assets(&mut inspected, Path::new(""), false, false, authorize)
}
pub fn load(path: &Path) -> Result<(Song, PathBuf), String> {
    load_with_asset_policy(path, |_| Ok(()))
}
/// Parse and validate once, authorizing each legacy sample and registered source before metadata/read access.
/// The caller must separately authorize the project file. Sample paths are absolute,
/// but can contain `..` or symlink components; the policy owns containment checks.
pub fn load_with_asset_policy(
    path: &Path,
    policy: impl Fn(&Path) -> Result<(), String>,
) -> Result<(Song, PathBuf), String> {
    let path = absolute(path)?;
    let data = existing_bytes(&path)?;
    legacy_project::preflight_effect_semantics(&data)?;
    let mut document: Document = match serde_json::from_slice(&data) {
        Ok(document) => document,
        Err(native_error) => match legacy_project::migrate_alpha2(&data)? {
            Some(song) => Document {
                format: "leviathan-song".into(),
                version: 1,
                dialect: None,
                song,
            },
            None => return Err(format!("Malformed project: {native_error}")),
        },
    };
    if document.format != "leviathan-song" || document.version != 1 {
        return Err("Unsupported project format/version".into());
    }
    if document
        .dialect
        .as_deref()
        .is_some_and(|dialect| dialect != PROJECT_DIALECT)
    {
        return Err("Unsupported project dialect".into());
    }
    document.song.validate()?;
    assets(
        &mut document.song,
        path.parent().unwrap(),
        true,
        true,
        policy,
    )?;
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
    assets(&mut stored, base, false, true, |_| Ok(()))?;
    let mut data = Vec::new();
    encode_document(&stored, &mut data)?;
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
