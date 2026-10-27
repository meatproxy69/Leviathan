//! Shared optional experimental onset jobs for UI and MCP control workers.
//!
//! All functions allocate and may decode/hash the source; never call from the
//! audio callback or GUI paint handler. This service owns no song, markers,
//! tempo, undo, path-authority or document revision. Callers must authorize the
//! path, hold at most one cache per document, recheck source identity on repick,
//! and reject stale results using their existing revision/config guards.
use crate::{
    onset_v2::{self, Analysis, Config, EnvelopeFrame, NoveltyCache, Preset, Provenance},
    sample_asset,
    song::SampleAsset,
};
use phonic::AudioFileBuffer;
use serde::Serialize;
use std::{path::Path, sync::Arc};

/// A retained native PCM + raw novelty cache has a separate aggregate ceiling.
pub const MAX_RETAINED_BYTES: usize = 96 * 1024 * 1024;
pub const MAX_CURVE_PAGE: usize = 2048;
pub const POLICY: &str = "experimental-phase-off-v1";

/// Explicit application policy, not a change to the judged core preset.
/// Phase-off reduced false positives but also recall on development fixtures;
/// it is not a calibrated or universally better detector. v1 is still default.
pub fn experimental_config(preset: Preset) -> Config {
    Config {
        enable_phase: false,
        ..Config::preset(preset)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheBinding {
    pub source_sha256: String,
    pub source_rate: u32,
    pub channels: u16,
    pub source_frames: u64,
    pub algorithm: String,
    /// SHA-256 over source identity/metadata, algorithm and spectral settings.
    /// This is a content key, NOT an authorization token or document revision.
    pub extraction_key: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceAnalysis {
    pub binding: CacheBinding,
    /// SHA-256 of the complete effective Config (including phase/refinement).
    pub config_sha256: String,
    pub analysis: Analysis,
}
#[derive(Clone)]
pub struct Prepared {
    pub cache: Arc<CachedSource>,
    pub result: SourceAnalysis,
}
/// Only construction through verified native decoding is public. The cache is
/// immutable, not Deserialize-able, and clients cannot submit Detection JSON.
/// Clone its Arc, not the PCM or curves, when moving it to a control worker.
pub struct CachedSource {
    binding: CacheBinding,
    buffer: Arc<AudioFileBuffer>,
    novelty: NoveltyCache,
    retained_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CurvePoint {
    pub source_frame: u64,
    /// Unthresholded evidence, NOT normalized confidence or an onset label.
    pub evidence: EnvelopeFrame,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CurvePage {
    pub binding: CacheBinding,
    pub start: usize,
    pub total: usize,
    pub hop_frames: usize,
    pub points: Vec<CurvePoint>,
}
impl CachedSource {
    pub fn binding(&self) -> &CacheBinding {
        &self.binding
    }
    pub fn provenance(&self) -> &Provenance {
        self.novelty.provenance()
    }
    pub fn raw_curves(&self) -> &[EnvelopeFrame] {
        self.novelty.raw()
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    /// Bounded copy suitable for paginated inspectors/protocol responses.
    /// The caller must recheck document/source authority before exposing it.
    pub fn curve_page(&self, start: usize, limit: usize) -> Result<CurvePage, String> {
        if !(1..=MAX_CURVE_PAGE).contains(&limit) || start > self.raw_curves().len() {
            return Err("Invalid onset curve page range/limit".into());
        }
        let hop = self.provenance().fusion_hop_frames;
        let end = start.saturating_add(limit).min(self.raw_curves().len());
        Ok(CurvePage {
            binding: self.binding.clone(),
            start,
            total: self.raw_curves().len(),
            hop_frames: hop,
            points: self.raw_curves()[start..end]
                .iter()
                .enumerate()
                .map(|(i, evidence)| CurvePoint {
                    source_frame: (start as u64 + i as u64) * hop as u64,
                    evidence: *evidence,
                })
                .collect(),
        })
    }
    pub(crate) fn pcm(&self) -> &[f32] {
        let len = self.binding.source_frames as usize * self.binding.channels as usize;
        &self.buffer.buffer()[..len]
    }
}

fn extraction_key(asset: &SampleAsset, config: &Config) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(
        onset_v2::ALGORITHM,
        &asset.sha256,
        asset.sample_rate,
        asset.channels,
        asset.frames,
        config.stereo,
        config.bands,
        config.fast_hop_at_44100,
        config.minimum_rms,
        config.max_filter_bins,
    ))
    .map_err(|e| e.to_string())?;
    Ok(sample_asset::content_sha256(&bytes))
}

/// Authorize `path` before calling. Strict production importer verifies encoded
/// SHA-256 and native metadata, with no resampling/downmix or guard-frame input.
/// Read/parse/extract/pick/refine all occur on the calling control worker.
pub fn prepare_verified(
    asset: &SampleAsset,
    path: &Path,
    config: &Config,
) -> Result<Prepared, String> {
    let buffer = sample_asset::load_verified(asset, path, sample_asset::MAX_SAMPLE_VALUES)?;
    let real = usize::try_from(asset.frames)
        .ok()
        .and_then(|frames| frames.checked_mul(asset.channels as usize))
        .ok_or("Source frame count overflow")?;
    let pcm = buffer
        .buffer()
        .get(..real)
        .ok_or("Source PCM is too short")?;
    let novelty = onset_v2::extract(pcm, asset.channels as usize, asset.sample_rate, config)
        .map_err(|e| e.to_string())?;
    let retained_bytes = buffer
        .buffer()
        .len()
        .checked_mul(std::mem::size_of::<f32>())
        .and_then(|bytes| {
            novelty
                .raw()
                .len()
                .checked_mul(std::mem::size_of::<EnvelopeFrame>())
                .and_then(|curves| bytes.checked_add(curves))
        })
        .ok_or("Onset cache byte count overflow")?;
    if retained_bytes > MAX_RETAINED_BYTES {
        return Err("Onset native PCM + curve cache exceeds 96 MiB limit".into());
    }
    let binding = CacheBinding {
        source_sha256: asset.sha256.clone(),
        source_rate: asset.sample_rate,
        channels: asset.channels,
        source_frames: asset.frames,
        algorithm: onset_v2::ALGORITHM.into(),
        extraction_key: extraction_key(asset, config)?,
    };
    let cache = Arc::new(CachedSource {
        binding,
        buffer,
        novelty,
        retained_bytes,
    });
    let result = repick(&cache, asset, config)?;
    Ok(Prepared { cache, result })
}

/// Pick/refine cached evidence with NO FFT or source decoding. The source path
/// is intentionally not accessed: caller must reauthorize and verify_identity
/// on its worker before reuse, then reject stale document/config completions.
/// This snapshot remains bound to its originally verified bytes even if disk
/// changes. No arbitrary client Detection or PCM is accepted at this boundary.
pub fn repick(
    cache: &CachedSource,
    expected: &SampleAsset,
    config: &Config,
) -> Result<SourceAnalysis, String> {
    let binding = cache.binding();
    if binding.source_sha256 != expected.sha256
        || binding.source_rate != expected.sample_rate
        || binding.channels != expected.channels
        || binding.source_frames != expected.frames
        || binding.algorithm != onset_v2::ALGORITHM
        || binding.extraction_key != extraction_key(expected, config)?
    {
        return Err("Onset cache source/algorithm/spectral configuration mismatch".into());
    }
    let detection = onset_v2::detect_cached(&cache.novelty, config).map_err(|e| e.to_string())?;
    let (suggestions, refinement_collisions) = onset_v2::refine(
        cache.pcm(),
        binding.channels as usize,
        binding.source_rate,
        &detection,
    )
    .map_err(|e| e.to_string())?;
    Ok(SourceAnalysis {
        binding: binding.clone(),
        config_sha256: sample_asset::content_sha256(
            &serde_json::to_vec(config).map_err(|e| e.to_string())?,
        ),
        analysis: Analysis {
            detection,
            suggestions,
            refinement_collisions,
        },
    })
}
