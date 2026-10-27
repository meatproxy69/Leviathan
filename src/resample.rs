//! Bounded isolated-occurrence printing through the existing Engine.
//! Preparation is pure. Printing publishes a NEW external WAV; document history
//! only registers/removes references and never deletes a published artifact.
use crate::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine},
    RATE, TAIL,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata, OpenOptions},
    io::BufWriter,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "isolated-occurrence-pre-master-resample-v1";
pub const MAX_VOICE_FRAMES: u64 = 1 << 28;
/// Publication currently requires Unix stable file identity; other platforms fail closed.
pub const PUBLICATION_SUPPORTED: bool = cfg!(unix);
/// Conservative encoded-header allowance, and space for the importer's guard.
pub const MAX_PRINTED_FRAMES: u64 = ((sample_asset::MAX_ENCODED_BYTES - 4096) / 8) as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TailPolicy {
    MusicalOnly,
    ReleaseTail,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResampleConfig {
    pub version: u32,
    pub occurrence: u32,
    pub name: String,
    pub root_note: u8,
    pub tail: TailPolicy,
    pub output: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResampleSummary {
    pub algorithm: String,
    pub occurrence: u32,
    pub pattern: PatternId,
    pub bpm: f64,
    pub lines_per_beat: u16,
    pub musical_frames: u64,
    pub tail_frames: u64,
    pub printed_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub starts: u64,
    pub voice_frames: u64,
    pub output: PathBuf,
    pub sample: SampleId,
    pub region: RegionId,
    pub instrument: InstrumentId,
    pub source_samples: Vec<SampleId>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResampleSource {
    pub sample: SampleId,
    pub asset: SampleAsset,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResampleReceipt {
    pub config: ResampleConfig,
    pub summary: ResampleSummary,
    pub base_sha256: String,
    pub sha256: String,
    pub encoded_bytes: u64,
    pub post_master_peak: f32,
    pub asset_peak: f32,
    pub post_master_energy: f64,
    pub source_identities: Vec<ResampleSource>,
}
#[derive(Debug, Clone)]
pub struct ResamplePlan {
    id: String,
    base: Song,
    isolated: Song,
    config: ResampleConfig,
    summary: ResampleSummary,
}
impl ResamplePlan {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn config(&self) -> &ResampleConfig {
        &self.config
    }
    pub fn summary(&self) -> &ResampleSummary {
        &self.summary
    }
}
#[derive(Debug, Clone)]
pub struct ResamplePreview {
    id: String,
    base: Song,
    candidate: Song,
    config: ResampleConfig,
    asset: SampleAsset,
    receipt: ResampleReceipt,
    parent: ParentIdentity,
}
impl ResamplePreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &ResampleConfig {
        &self.config
    }
    pub fn summary(&self) -> &ResampleSummary {
        &self.receipt.summary
    }
    pub fn asset(&self) -> &SampleAsset {
        &self.asset
    }
    pub fn receipt(&self) -> &ResampleReceipt {
        &self.receipt
    }
    /// Pure registration only. Caller also guards its editor generation and
    /// reauthorizes/verifies the published file off the realtime/UI thread.
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base {
            return Err("Resample result is stale; published WAV remains on disk".into());
        }
        Ok(self.candidate.clone())
    }
}
/// Paths are declared paths, not silently canonicalized replacements. Adapters
/// may enforce stricter root/symlink policy. Called before any selected decode,
/// initially for output, immediately before publication, and on verification.
#[derive(Debug, Clone, Copy)]
pub enum Access<'a> {
    ReadSource(&'a Path),
    PublishOutput(&'a Path),
}
fn digest(value: &impl Serialize) -> Result<String, String> {
    Ok(sample_asset::content_sha256(
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
    ))
}
fn next_id(last: Option<u64>, label: &str) -> Result<u64, String> {
    last.unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| format!("No fresh {label} ID remains"))
}
fn check_output_syntax(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        || path.file_name().is_none()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(
            "Resample output must be an absolute .wav path without . or .. components".into(),
        );
    }
    Ok(())
}
fn check_budget(starts: u64, frames: u64) -> Result<u64, String> {
    if frames == 0
        || frames > MAX_PRINTED_FRAMES
        || frames
            .checked_add(1)
            .and_then(|n| n.checked_mul(2))
            .is_none_or(|n| n > sample_asset::MAX_SAMPLE_VALUES as u64)
    {
        return Err(format!("Resample exceeds conservative printed-frame limit {MAX_PRINTED_FRAMES}; shorten the occurrence or omit the release tail"));
    }
    let work = starts
        .checked_mul(frames)
        .ok_or("Resample voice-frame work overflow")?;
    if work > MAX_VOICE_FRAMES {
        return Err(format!("Resample needs {work} voice-frames, exceeding {MAX_VOICE_FRAMES}; shorten or split the occurrence"));
    }
    Ok(work)
}
fn candidate(plan: &ResamplePlan, asset: SampleAsset) -> Result<Song, String> {
    let mut song = plan.base.clone();
    song.samples.insert(plan.summary.sample, asset);
    song.regions.insert(
        plan.summary.region,
        SampleRegion {
            name: plan.config.name.clone(),
            sample: plan.summary.sample,
            start_frame: 0,
            end_frame: plan.summary.printed_frames,
            root_note: plan.config.root_note,
            amp_envelope: None,
            forward_loop: None,
        },
    );
    song.instruments.insert(
        plan.summary.instrument,
        Instrument {
            name: plan.config.name.clone(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: Some(plan.summary.region),
        },
    );
    song.validate()?;
    crate::project::check_serialized_size(&song)?;
    Ok(song)
}
/// Pure: no pathname reads, source hashing/decoding, file creation, or history.
pub fn prepare(song: &Song, config: ResampleConfig) -> Result<ResamplePlan, String> {
    crate::project::check_serialized_size(song)?;
    crate::generation::check_work(song, false).map_err(|e| format!("Resample preflight: {e}"))?;
    song.validate()?;
    if config.version != VERSION {
        return Err("Unsupported resample version; expected 1".into());
    }
    if config.name.trim().is_empty() || config.name.len() > 256 || config.root_note > 127 {
        return Err(
            "Resample name must be nonblank and at most 256 bytes; root note must be 0..127".into(),
        );
    }
    check_output_syntax(&config.output)?;
    let occurrence = song
        .order
        .get(config.occurrence as usize)
        .ok_or("Unknown resample occurrence")?
        .clone();
    let mut isolated = song.clone();
    isolated.order = vec![occurrence.clone()];
    let manifest = song_render::evaluate_model(&isolated)
        .map_err(|e| format!("Isolated cold occurrence: {e}"))?;
    if manifest.master_gain != 0.5 || manifest.sample_rate != RATE || RATE != 48000 {
        return Err("Resample v1 requires the fixed 48 kHz / 0.5 master Engine".into());
    }
    // Only registered, bounded, identity-verified sources are allowed. Never
    // silently convert legacy playback (different DSP) into region playback.
    if manifest
        .assets
        .keys()
        .any(|id| !manifest.regions.contains_key(id))
    {
        return Err(
            "Resample uses a legacy sample path; import/register a region and assign it first"
                .into(),
        );
    }
    let sources: BTreeSet<_> = manifest
        .regions
        .values()
        .chain(manifest.pad_voices.values().map(|p| &p.region))
        .chain(manifest.multisample_voices.values().map(|m| &m.region))
        .map(|r| r.sample)
        .collect();
    for id in &sources {
        if !song.samples[id].path.is_absolute() {
            return Err("Used registered resample source path must be absolute; open the project or import the source first".into());
        }
    }
    let tail_frames = match config.tail {
        TailPolicy::MusicalOnly => 0,
        TailPolicy::ReleaseTail => TAIL,
    };
    let printed_frames = manifest
        .musical_frames
        .checked_add(tail_frames)
        .ok_or("Resample duration overflow")?;
    let starts = manifest
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .count() as u64;
    let voice_frames = check_budget(starts, printed_frames)?;
    let summary = ResampleSummary {
        algorithm: ALGORITHM.into(), occurrence: config.occurrence, pattern: occurrence.pattern,
        bpm: song.bpm, lines_per_beat: song.lines_per_beat,
        musical_frames: manifest.musical_frames, tail_frames, printed_frames,
        sample_rate: RATE, channels: 2, starts, voice_frames, output: config.output.clone(),
        sample: SampleId(next_id(song.samples.keys().next_back().map(|i| i.0), "sample")?),
        region: RegionId(next_id(song.regions.keys().next_back().map(|i| i.0), "region")?),
        instrument: InstrumentId(next_id(song.instruments.keys().next_back().map(|i| i.0), "instrument")?),
        source_samples: sources.into_iter().collect(),
        warnings: vec![
            "Isolated cold occurrence, all tracks: not an arrangement crop. No preceding voices or column memory; phrase instances and drum round-robin counters restart. Aliases/mutes are retained, timing restarts at frame 0.".into(),
            "Pre-master float32 WAV = Engine output × 2 under fixed master 0.5, not normalization. Values may exceed 1 and clip in external consumers. The printed audio already includes the current track mix. Native-root/unity/center/no-options replay with neutral effective track mix preserves output only while held through the entire printed region, without early OFF/replacement/SongEnd. A printed release tail is already baked; stopping at the old shorter end releases it again.".into(),
            "MusicalOnly truncates at the musical boundary and may click. ReleaseTail includes exactly 500 ms (24000 frames); the whole new region includes that tail. No inferred BPM, markers, trimming or fades.".into(),
            "Before Register, an off-thread all-source project preflight checks readability/policy, registered identities and the 256 MiB aggregate encoded budget. Unused missing/changed sources can block registration even after a successful print; the WAV remains.".into(),
            "Print creates a durable external WAV. Cancel, stale registration, Undo and closing the preview never delete a published artifact; Redo only restores references.".into(),
            "Publication v1 requires Unix stable file identity; other platforms fail closed before decode/temp creation. Parent/path rechecks do not sandbox malicious concurrent filesystem replacement.".into(),
            "Only used registered regions are supported; legacy sample paths must be imported and assigned as regions. Work caps are conservative, not a latency guarantee. Source/path authorization and filesystem races are separate from document history.".into(),
        ],
    };
    let plan = ResamplePlan {
        id: digest(&(ALGORITHM, song, &config))?,
        base: song.clone(),
        isolated,
        config,
        summary,
    };
    candidate(
        &plan,
        SampleAsset {
            path: plan.config.output.clone(),
            sha256: "0".repeat(64),
            sample_rate: RATE,
            channels: 2,
            frames: printed_frames,
        },
    )?;
    Ok(plan)
}

#[derive(Debug, Clone)]
struct ParentIdentity {
    path: PathBuf,
    #[cfg(unix)]
    handle: std::sync::Arc<File>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}
impl ParentIdentity {
    fn capture(output: &Path) -> Result<Self, String> {
        check_output_syntax(output)?;
        let parent = output.parent().ok_or("Output has no parent directory")?;
        let canonical = parent
            .canonicalize()
            .map_err(|e| format!("Output parent: {e}"))?;
        if canonical != parent {
            return Err("Output parent must be canonical and contain no symlinks".into());
        }
        #[cfg(unix)]
        let handle = File::open(parent).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        let metadata = handle.metadata().map_err(|e| e.to_string())?;
        #[cfg(not(unix))]
        let metadata = fs::metadata(parent).map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("Output parent is not a directory".into());
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path: canonical,
            #[cfg(unix)]
            handle: std::sync::Arc::new(handle),
            #[cfg(unix)]
            dev: metadata.dev(),
            #[cfg(unix)]
            ino: metadata.ino(),
        })
    }
    fn sync(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            self.handle.sync_all()
        }
        #[cfg(not(unix))]
        {
            Err(std::io::Error::other(
                "Stable directory identity unsupported",
            ))
        }
    }
    fn check(&self, output: &Path) -> Result<(), String> {
        let other = Self::capture(output)?;
        let same = self.path == other.path;
        #[cfg(unix)]
        let same = same && self.dev == other.dev && self.ino == other.ino;
        if !same {
            return Err("Output parent changed during rendering; publication cancelled".into());
        }
        Ok(())
    }
}
fn require_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Cannot inspect output: {e}")),
        Ok(_) => Err(
            "Resample destination already exists (including symlinks); choose a new .wav path"
                .into(),
        ),
    }
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}
struct Temp {
    path: PathBuf,
    identity: Metadata,
    // Keep the inode alive through cleanup/Drop, even after writer finalization.
    handle: File,
}
impl Temp {
    fn cleanup(&self) -> Result<(), String> {
        match fs::symlink_metadata(&self.path) {
            Ok(m) if same_file(&self.identity, &m) => fs::remove_file(&self.path)
                .map_err(|e| format!("Owned temporary WAV cleanup failed: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err("Temporary path identity changed; no foreign file was removed".into()),
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
fn temp_file(parent: &Path) -> Result<(Temp, File), String> {
    for _ in 0..64 {
        let path = parent.join(format!(
            ".leviathan-resample-{}-{}.wav",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                let identity = file.metadata().map_err(|e| format!("Created temporary WAV {} but cannot establish identity ({e}); left for manual recovery", path.display()))?;
                let temp = Temp {
                    path,
                    identity,
                    handle: file,
                };
                let writer = temp.handle.try_clone().map_err(|e| e.to_string())?;
                return Ok((temp, writer));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create owned temporary WAV: {e}")),
        }
    }
    Err("Cannot allocate owned resample temporary WAV".into())
}
fn check_cancel(cancelled: &mut impl FnMut() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("Resample cancelled before publication; no new artifact was published".into())
    } else {
        Ok(())
    }
}
/// Side-effectful WAV-only print. On success the published artifact exists even
/// if registration is later stale/cancelled/undone. No event companion is written.
pub fn render(
    plan: &ResamplePlan,
    mut authorize: impl FnMut(Access<'_>) -> Result<(), String>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<ResamplePreview, String> {
    if !PUBLICATION_SUPPORTED {
        return Err("Resample publication is not supported on this platform: stable file identity is required (Unix v1 only)".into());
    }
    check_cancel(&mut cancelled)?;
    authorize(Access::PublishOutput(&plan.config.output))?;
    let parent = ParentIdentity::capture(&plan.config.output)?;
    require_absent(&plan.config.output)?;
    for id in &plan.summary.source_samples {
        authorize(Access::ReadSource(&plan.base.samples[id].path))?;
    }
    check_cancel(&mut cancelled)?;
    let mut engine = Engine::new(&plan.isolated, None)?;
    if engine.manifest.master_gain != 0.5
        || engine.manifest.musical_frames != plan.summary.musical_frames
    {
        return Err("Engine no longer matches the captured resample clock/master".into());
    }
    check_cancel(&mut cancelled)?;
    let (temp, file) = temp_file(&parent.path)?;
    let outcome = (|| {
        let mut writer = hound::WavWriter::new(
            BufWriter::new(file),
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .map_err(|e| e.to_string())?;
        let (mut peak, mut asset_peak, mut energy) = (0.0f32, 0.0f32, 0.0f64);
        let mut mix = [0.0f32; 1024];
        while engine.position < plan.summary.printed_frames {
            check_cancel(&mut cancelled)?;
            let count = engine.block(&mut mix, plan.summary.printed_frames)?;
            if count == 0 {
                return Err("Engine stopped before the requested print boundary".into());
            }
            for &value in &mix[..count * 2] {
                let stored = value * 2.0;
                if !stored.is_finite() {
                    return Err("Nonfinite pre-master resample value".into());
                }
                peak = peak.max(value.abs());
                asset_peak = asset_peak.max(stored.abs());
                energy += f64::from(value).powi(2);
                writer.write_sample(stored).map_err(|e| e.to_string())?;
            }
        }
        writer.finalize().map_err(|e| e.to_string())?;
        temp.handle
            .sync_all()
            .map_err(|e| format!("Cannot sync temporary WAV: {e}"))?;
        drop(engine);
        check_cancel(&mut cancelled)?;
        let imported = sample_asset::import(&temp.path)?;
        if imported.asset.sample_rate != RATE
            || imported.asset.channels != 2
            || imported.asset.frames != plan.summary.printed_frames
        {
            return Err("Printed WAV importer metadata differs from the captured plan".into());
        }
        let mut asset = imported.asset;
        drop(imported.buffer);
        asset.path = plan.config.output.clone();
        let candidate = candidate(plan, asset.clone())?;
        let receipt = ResampleReceipt {
            config: plan.config.clone(),
            summary: plan.summary.clone(),
            base_sha256: digest(&plan.base)?,
            sha256: asset.sha256.clone(),
            encoded_bytes: fs::metadata(&temp.path).map_err(|e| e.to_string())?.len(),
            post_master_peak: peak,
            asset_peak,
            post_master_energy: energy,
            source_identities: plan
                .summary
                .source_samples
                .iter()
                .map(|&sample| ResampleSource {
                    sample,
                    asset: plan.base.samples[&sample].clone(),
                })
                .collect(),
        };
        let mut result = ResamplePreview {
            id: digest(&(&plan.id, &receipt))?,
            base: plan.base.clone(),
            candidate,
            config: plan.config.clone(),
            asset,
            receipt,
            parent: parent.clone(),
        };
        authorize(Access::PublishOutput(&plan.config.output))?;
        parent.check(&plan.config.output)?;
        require_absent(&plan.config.output)?;
        check_cancel(&mut cancelled)?;
        let current_temp = fs::symlink_metadata(&temp.path).map_err(|e| e.to_string())?;
        if !current_temp.is_file() || !same_file(&temp.identity, &current_temp) {
            return Err("Owned temporary WAV identity changed; publication cancelled".into());
        }
        sample_asset::verify_identity(&result.asset, &temp.path)?;
        check_cancel(&mut cancelled)?;
        // No overwrite rename. After this succeeds, all remaining failures become
        // warnings on a successful receipt, never an error hiding the artifact.
        fs::hard_link(&temp.path, &plan.config.output)
            .map_err(|e| format!("Cannot publish new resample WAV without clobbering: {e}"))?;
        if let Err(e) = temp.cleanup() {
            result.receipt.summary.warnings.push(e);
        }
        if let Err(e) = parent.sync() {
            result.receipt.summary.warnings.push(format!(
                "WAV published, but directory durability could not be confirmed: {e}"
            ));
        }
        Ok(result)
    })();
    match outcome {
        Err(error) => match temp.cleanup() {
            Ok(()) if parent.check(&plan.config.output).is_err() => Err(format!(
                "{error}; output parent changed; owned temporary may have moved from {}",
                temp.path.display()
            )),
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!(
                "{error}; {cleanup}; owned temporary path: {}",
                temp.path.display()
            )),
        },
        Ok(result) => Ok(result),
    }
}
/// Off-thread prospective project-source/storage preflight before registration.
/// Checks every legacy/registered source under Save policy, including output SHA;
/// output-specific parent/non-symlink checks remain stricter. No file is written.
/// This cannot eliminate a later external filesystem mutation (TOCTOU).
pub fn verify_artifact(
    preview: &ResamplePreview,
    mut authorize: impl FnMut(Access<'_>) -> Result<(), String>,
) -> Result<(), String> {
    authorize(Access::PublishOutput(&preview.asset.path))?;
    preview.parent.check(&preview.asset.path)?;
    let metadata = fs::symlink_metadata(&preview.asset.path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Published resample WAV is not a regular, non-symlink file".into());
    }
    crate::project::check_registration_sources(&preview.candidate, |path| {
        authorize(Access::ReadSource(path))
    })
    .map_err(|error| {
        format!("Resample registration project-source preflight: {error}; published WAV retained")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_work_and_import_boundaries() {
        assert_eq!(check_budget(256, 1 << 20).unwrap(), MAX_VOICE_FRAMES);
        assert!(check_budget(257, 1 << 20)
            .unwrap_err()
            .contains("voice-frames"));
        assert!(check_budget(u64::MAX, 2).unwrap_err().contains("overflow"));
        assert_eq!(check_budget(0, MAX_PRINTED_FRAMES).unwrap(), 0);
        assert!(check_budget(0, MAX_PRINTED_FRAMES + 1).is_err());
        assert!(check_budget(0, 0).is_err());
    }
}
