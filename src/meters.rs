//! Optional worker-side observation of existing Engine contributions.
//! These are post-balance AND post-fixed-master sample peaks, not loudness or
//! true peak. Observed sums never feed back into the audio destination.
use crate::{song::*, track_mix::MixLayout};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const MAX_TRACKS: usize = 64;
pub const BLOCK_FRAMES: usize = 512;
pub const WINDOW_FRAMES: u64 = 2400;
pub const REPORT_CAPACITY: usize = 4;
pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "original-contribution-sample-peaks-v1";
pub const MAX_ANALYSIS_FRAMES: u64 = 120_000;
pub const MAX_VOICE_FRAMES: u64 = 1 << 28;
pub const MAX_REPORT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrackMeterAvailability {
    Available,
    SharedLegacy { instruments: Vec<InstrumentId> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackMeterRow {
    pub track: TrackId,
    pub availability: TrackMeterAvailability,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeterLayout {
    /// Actually scheduled ON owners, not all defined or configured tracks.
    pub tracks: Vec<TrackMeterRow>,
    /// Master remains observable when bounded per-track attribution is absent.
    pub tracks_unavailable: Option<String>,
}
impl MeterLayout {
    fn from_mix(mix: Option<&MixLayout>, unavailable: Option<&str>) -> Self {
        let tracks: Vec<_> = mix
            .map(|m| {
                m.tracks()
                    .iter()
                    .map(|&track| {
                        let instruments: Vec<_> = m
                            .legacy_groups()
                            .iter()
                            .filter(|g| g.tracks.len() > 1 && g.tracks.contains(&track))
                            .map(|g| g.instrument)
                            .collect();
                        TrackMeterRow {
                            track,
                            availability: if instruments.is_empty() {
                                TrackMeterAvailability::Available
                            } else {
                                TrackMeterAvailability::SharedLegacy { instruments }
                            },
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            tracks,
            tracks_unavailable: unavailable.map(str::to_owned),
        }
    }
}

/// Borrowed, successful Engine block only. Not a serialized PCM/report payload.
/// Song-frame start is absolute at the 48 kHz Engine rate, not native asset frames.
/// Bounds are half-open [start, start + frames).
/// A missing available-track sample denotes meter-only nonfinite accumulation;
/// callers must mark the encompassing window unavailable, never substitute zero.
pub struct MeterBlock {
    song_start: u64,
    frames: usize,
    valid: bool,
    master: [[f32; 2]; BLOCK_FRAMES],
    tracks: Vec<[[f32; 2]; BLOCK_FRAMES]>,
    available: Vec<bool>,
}
impl MeterBlock {
    pub fn song_start(&self) -> u64 {
        self.song_start
    }
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn master_sample(&self, frame: usize) -> Option<[f32; 2]> {
        (self.valid && frame < self.frames).then(|| self.master[frame])
    }
    pub fn track_sample(&self, slot: usize, frame: usize) -> Option<[f32; 2]> {
        if !self.valid || frame >= self.frames || !self.available.get(slot).copied()? {
            return None;
        }
        let sample = self.tracks[slot][frame];
        sample.iter().all(|s| s.is_finite()).then_some(sample)
    }
}

pub(crate) struct MeterTap {
    pub layout: MeterLayout,
    block: MeterBlock,
}
impl MeterTap {
    pub fn new(mix: Option<&MixLayout>, unavailable: Option<&str>) -> Self {
        let layout = MeterLayout::from_mix(mix, unavailable);
        let tracks = &layout.tracks;
        // MixLayout is opaque and already bounds scheduled ownership to 64.
        assert!(tracks.len() <= MAX_TRACKS);
        let available = tracks
            .iter()
            .map(|r| matches!(r.availability, TrackMeterAvailability::Available))
            .collect();
        let sums = vec![[[0.0; 2]; BLOCK_FRAMES]; tracks.len()];
        Self {
            layout,
            block: MeterBlock {
                song_start: 0,
                frames: 0,
                valid: false,
                master: [[0.0; 2]; BLOCK_FRAMES],
                tracks: sums,
                available,
            },
        }
    }
    pub fn invalidate(&mut self) {
        self.block.valid = false;
        self.block.frames = 0;
    }
    pub fn begin(&mut self, song_start: u64, frames: usize) {
        self.invalidate();
        self.block.song_start = song_start;
        self.block.frames = frames;
        for samples in &mut self.block.tracks {
            samples[..frames].fill([0.0; 2]);
        }
    }
    pub fn observe(
        &mut self,
        slot: usize,
        source: &[f32],
        master: f32,
        mix: &crate::track_mix::MixRuntime,
    ) {
        if !self.block.available[slot] {
            // Even this track's independently controlled voices are omitted:
            // reporting them alone would fabricate a complete track subtotal.
            return;
        }
        for (i, &value) in source.iter().enumerate() {
            let coefficients = mix.frame_coefficients(slot, i / 2);
            let gain = if i % 2 == 0 {
                coefficients.left
            } else {
                coefficients.right
            };
            // Same f32 arithmetic and contribution order as MixRuntime::add,
            // in a separate observation buffer. Never alter the audio sum.
            self.block.tracks[slot][i / 2][i % 2] += if gain == 1.0 {
                value * master
            } else {
                (value * master) * gain
            };
        }
    }
    pub fn finish(&mut self, output: &[f32]) {
        for (to, from) in self.block.master[..self.block.frames]
            .iter_mut()
            .zip(output.as_chunks::<2>().0)
        {
            *to = *from;
        }
        self.block.valid = true;
    }
    pub fn block(&self) -> Option<&MeterBlock> {
        self.block.valid.then_some(&self.block)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterAnalysisConfig {
    pub version: u32,
    /// 48 kHz Song/Engine frames, not native asset frames. Must be aligned.
    pub start_frame: u64,
    /// Aligned, except the exact natural render terminal may be partial.
    pub end_frame: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeterAnalysisSummary {
    pub version: u32,
    pub algorithm: String,
    pub sample_rate: u32,
    pub processed_frames: u64,
    pub observed_start: u64,
    pub observed_end: u64,
    pub total_frames: u64,
    pub starts: u64,
    pub voice_frames: u64,
    pub source_count: usize,
    pub source_sample_values: u64,
    pub layout: MeterLayout,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct MeterAnalysisPlan {
    id: String,
    song: Song,
    config: MeterAnalysisConfig,
    summary: MeterAnalysisSummary,
    source_samples: Vec<SampleId>,
}
impl MeterAnalysisPlan {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn song(&self) -> &Song {
        &self.song
    }
    pub fn config(&self) -> &MeterAnalysisConfig {
        &self.config
    }
    pub fn summary(&self) -> &MeterAnalysisSummary {
        &self.summary
    }
    pub fn source_samples(&self) -> &[SampleId] {
        &self.source_samples
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct MeterAnalysis {
    pub id: String,
    pub config: MeterAnalysisConfig,
    pub summary: MeterAnalysisSummary,
    pub reports: Vec<crate::meter_windows::MeterReport>,
    /// Full compiled used-source identities, verified by the same Engine load.
    /// This is not verification of unused project files or a future-file promise.
    pub verified_sources: usize,
}
fn digest(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| crate::sample_asset::content_sha256(&bytes))
        .map_err(|e| e.to_string())
}
/// Pure bounded scheduling and source metadata preflight. Never opens, hashes,
/// canonicalizes or decodes paths. Full context remains intact, even past end.
pub fn prepare(song: &Song, config: MeterAnalysisConfig) -> Result<MeterAnalysisPlan, String> {
    if config.version != VERSION
        || config.start_frame >= config.end_frame
        || config.end_frame > MAX_ANALYSIS_FRAMES
        || !config.start_frame.is_multiple_of(WINDOW_FRAMES)
    {
        return Err("Meter analysis requires version 1, aligned start < end, and at most 120000 processed Song frames; no rounding".into());
    }
    crate::track_mix::check_document_work(song)?;
    song.validate()?;
    let manifest = crate::song_render::evaluate_model(song)?;
    let total_frames = manifest
        .musical_frames
        .checked_add(manifest.tail_frames)
        .ok_or("Meter render duration overflow")?;
    if config.end_frame > total_frames
        || (!config.end_frame.is_multiple_of(WINDOW_FRAMES) && config.end_frame != total_frames)
    {
        return Err("Meter end must be 2400-frame aligned or the exact natural render terminal, and within the complete Song".into());
    }
    if manifest
        .assets
        .keys()
        .any(|i| !manifest.regions.contains_key(i))
    {
        return Err("Offline meter analysis uses a path-only legacy route; explicitly import/register AND assign a region first (a matching registered pathname alone is insufficient)".into());
    }
    let sources: BTreeSet<_> = manifest
        .regions
        .values()
        .chain(manifest.pad_voices.values().map(|p| &p.region))
        .chain(manifest.multisample_voices.values().map(|m| &m.region))
        .map(|r| r.sample)
        .collect();
    let mut identities = BTreeMap::new();
    let mut source_sample_values = 0u64;
    for id in &sources {
        let asset = &song.samples[id];
        if !asset.path.is_absolute() {
            return Err("Offline meter source paths must be absolute; open the project or import the source first, never infer process CWD".into());
        }
        let shape = (asset.sample_rate, asset.channels, asset.frames);
        if let Some(previous) = identities.insert(&asset.sha256, shape) {
            if previous != shape {
                return Err("Meter source metadata differs for a shared SHA-256 identity".into());
            }
        } else {
            let values = asset
                .frames
                .checked_add(1)
                .and_then(|v| v.checked_mul(u64::from(asset.channels)))
                .ok_or("Meter decoded sample budget overflow")?;
            source_sample_values = source_sample_values
                .checked_add(values)
                .ok_or("Meter decoded sample budget overflow")?;
            if source_sample_values > crate::sample_asset::MAX_SAMPLE_VALUES as u64 {
                return Err(
                    "Meter used sources exceed 64 MiB aggregate decoded PCM including guard frames"
                        .into(),
                );
            }
        }
    }
    let starts = manifest
        .events
        .iter()
        .filter(|e| matches!(e.command, crate::song_render::Command::On { .. }))
        .count() as u64;
    let voice_frames = starts
        .checked_mul(config.end_frame)
        .ok_or("Meter voice-frame work overflow")?;
    if voice_frames > MAX_VOICE_FRAMES {
        return Err(
            "Meter analysis exceeds 2^28 conservative voice-frames; shorten the processed prefix"
                .into(),
        );
    }
    let controlled = crate::song_render::control_specs(&manifest)
        .keys()
        .copied()
        .collect();
    let layout = match crate::track_mix::build_layout(song, &manifest, &controlled) {
        Ok(layout) => {
            layout.snapshot(&song.track_mix)?;
            MeterLayout::from_mix(Some(&layout), None)
        }
        Err(error) if crate::track_mix::nonneutral(song, &manifest) => return Err(error),
        Err(error) => MeterLayout::from_mix(None, Some(&error)),
    };
    let summary = MeterAnalysisSummary {
        version: VERSION, algorithm: ALGORITHM.into(), sample_rate: crate::RATE,
        processed_frames: config.end_frame, observed_start: config.start_frame,
        observed_end: config.end_frame, total_frames, starts, voice_frames,
        source_count: sources.len(), source_sample_values, layout,
        warnings: vec![
            "Offline analysis advances the full compiled Song from frame 0; range filters observations only. No seek/crop, event removal, normalization, file publication, document edit or live GUI/device IPC.".into(),
            "Track sample peaks sum existing f32 contributions AFTER track balance and fixed master 0.5, in original order. Master peaks measure actual final stereo PCM. Track regrouping need not reconstruct master bits. No true-peak, RMS, loudness or listening claim.".into(),
            "Only whole 2400-frame windows are selected, plus an explicitly partial exact natural-end window. Positions are 48 kHz Song/Engine frames, not native audio asset frames. No peaks include samples outside the requested interval.".into(),
            "Every full-compiled used source, including later notes past the report prefix, must use a registered region. Original absolute lexical paths are authorized before Engine canonicalization; its verified loader enforces 64 MiB per file, 256 MiB aggregate encoded and 64 MiB aggregate decoded limits. Unused source files are not opened or byte-verified; full model and bounded metadata validation still applies.".into(),
            "Meter-only nonfinite track accumulation makes that whole window unavailable, never a zero/partial total. Ordinary Engine clipping/nonfinite errors abort analysis; no incomplete report is returned. Cancellation is checked between bounded preparation/load/block stages, not inside a decoder call.".into(),
        ],
    };
    let id = digest(&(
        ALGORITHM,
        crate::cell_blocks::document_sha256(song)?,
        &config,
    ))?;
    Ok(MeterAnalysisPlan {
        id,
        song: song.clone(),
        config,
        summary,
        source_samples: sources.into_iter().collect(),
    })
}
/// Side-effect-free with respect to documents/files, but performs authorized
/// bounded source IO on the calling worker. No PCM is retained in the result.
pub fn analyze(
    plan: &MeterAnalysisPlan,
    mut authorize: impl FnMut(&Path) -> Result<(), String>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<MeterAnalysis, String> {
    let check_cancel = |cancelled: &mut dyn FnMut() -> bool| {
        if cancelled() {
            Err("Meter analysis cancelled; no completed report".to_owned())
        } else {
            Ok(())
        }
    };
    check_cancel(&mut cancelled)?;
    // ALL original lexical paths are authorized before any Engine path IO.
    for id in &plan.source_samples {
        authorize(&plan.song.samples[id].path)?;
    }
    check_cancel(&mut cancelled)?;
    let mut engine = crate::song_render::Engine::new(&plan.song, None)?;
    if engine.enable_meters() != &plan.summary.layout {
        return Err("Meter Engine layout differs from the prepared model".into());
    }
    let mut windows =
        crate::meter_windows::MeterWindows::new(&plan.summary.layout).map_err(|e| e.to_string())?;
    let mut reports = Vec::with_capacity((MAX_ANALYSIS_FRAMES / WINDOW_FRAMES) as usize);
    let mut retain = |report: crate::meter_windows::MeterReport| {
        if report.emitted_start >= plan.config.start_frame {
            reports.push(report);
        }
    };
    let mut output = [0.0f32; BLOCK_FRAMES * 2];
    while engine.position < plan.config.end_frame {
        check_cancel(&mut cancelled)?;
        let frames = engine.block(&mut output, plan.config.end_frame)?;
        if frames == 0 {
            return Err("Meter Engine stopped before the requested interval".into());
        }
        windows
            .push_block(
                engine
                    .meter_block()
                    .ok_or("Missing successful meter block")?,
                0,
                &mut retain,
            )
            .map_err(|e| e.to_string())?;
    }
    if plan.config.end_frame == plan.summary.total_frames {
        windows
            .finish_partial(&mut retain)
            .map_err(|e| e.to_string())?;
    }
    check_cancel(&mut cancelled)?;
    let result = MeterAnalysis {
        id: digest(&(&plan.id, &reports))?,
        config: plan.config.clone(),
        summary: plan.summary.clone(),
        reports,
        verified_sources: plan.source_samples.len(),
    };
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_REPORT_BYTES
    {
        return Err("Meter completed report exceeds 1 MiB; no report retained".into());
    }
    Ok(result)
}
