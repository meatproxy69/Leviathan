//! Deterministic, non-destructive transient *suggestions* for decoded source PCM.
//!
//! Run this allocating, synchronous analysis on an existing background worker,
//! never in an audio callback. It neither decodes files nor modifies samples,
//! song events or manual slice markers. The caller owns accepting suggestions.
//! This first detector measures adaptive energy and first-difference energy
//! rises; first-difference energy is NOT an FFT/spectral-flux detector. It does
//! not estimate tempo, beats, instrument identity or calibrated probabilities.
//!
//! Pass only real source frames. In particular, phonic's file decoder appends
//! one interpolation guard frame; callers using that decoder must exclude it.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

pub const ALGORITHM: &str = "adaptive-energy-difference-v1";
pub const MIN_SOURCE_RATE: u32 = 8_000;
pub const MAX_SOURCE_RATE: u32 = 384_000;
/// Same 64 MiB decoded-PCM ceiling as the existing controlled sampler.
pub const MAX_SAMPLE_VALUES: usize = 16 * 1024 * 1024;
pub const MAX_DURATION_SECONDS: u64 = 300;
pub const MAX_SUGGESTIONS: usize = 65_536;
pub const MAX_WAVEFORM_BINS: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfig {
    /// 0 = conservative, 1 = more sensitive. Not a target onset count.
    pub sensitivity: f32,
    /// Minimum distance between returned source frames, rounded up to frames.
    /// Detector bins/peak picking still limit resolution to roughly 2..6 ms.
    pub min_spacing_ms: f32,
    /// Absolute RMS floor; quieter material is ignored (no normalization).
    pub minimum_rms: f32,
    /// May reduce, but never increase, the hard decoded-PCM/duration ceilings.
    pub max_frames: u64,
    /// Exceeding this limit is an error, never silently truncated output.
    pub max_suggestions: usize,
    /// Requested overview bins, capped by source length; 0 disables overview.
    pub waveform_bins: usize,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            sensitivity: 0.5,
            min_spacing_ms: 25.0,
            minimum_rms: 0.00001,
            max_frames: MAX_SAMPLE_VALUES as u64,
            max_suggestions: 4_096,
            waveform_bins: 1_024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisProvenance {
    pub algorithm: String,
    /// FNV-1a of rate, channels and exact interleaved f32 bits (little endian).
    /// A deterministic cache/debug fingerprint, NOT a cryptographic asset ID.
    pub source_fingerprint: u64,
    pub source_rate: u32,
    pub channels: usize,
    pub source_frames: u64,
    pub hop_frames: u32,
    pub min_spacing_frames: u64,
    pub config: AnalysisConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnsetSuggestion {
    /// Integer position in the unmodified source, never destination frames.
    pub frame: u64,
    /// Heuristic threshold margin in [0, 1], NOT a probability of correctness.
    pub confidence: f32,
    pub energy_rise: bool,
    pub high_frequency_rise: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaveformBin {
    pub start_frame: u64,
    /// Exclusive source-frame bound; all bins partition the entire source.
    pub end_frame: u64,
    /// Independent channel extrema; antiphase stereo cannot cancel out.
    pub channels: Vec<ChannelExtrema>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChannelExtrema {
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransientAnalysis {
    pub provenance: AnalysisProvenance,
    /// Replaceable analysis proposals. Deliberately no manual-marker field.
    pub suggestions: Vec<OnsetSuggestion>,
    pub waveform: Vec<WaveformBin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    InvalidChannels(usize),
    InvalidSampleRate(u32),
    EmptyPcm,
    IncompleteFrame,
    InputLimit,
    NonFiniteSample { sample_index: usize },
    InvalidConfig(&'static str),
    SuggestionLimit { limit: usize },
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidChannels(n) => {
                write!(f, "analysis requires mono/stereo PCM, got {n} channels")
            }
            Self::InvalidSampleRate(n) => write!(
                f,
                "analysis rate {n} outside {MIN_SOURCE_RATE}..={MAX_SOURCE_RATE} Hz"
            ),
            Self::EmptyPcm => f.write_str("analysis PCM is empty"),
            Self::IncompleteFrame => {
                f.write_str("analysis PCM has an incomplete interleaved frame")
            }
            Self::InputLimit => {
                f.write_str("analysis PCM exceeds configured frames, 64 MiB, or 300 seconds")
            }
            Self::NonFiniteSample { sample_index } => {
                write!(f, "analysis PCM sample {sample_index} is not finite")
            }
            Self::InvalidConfig(field) => write!(f, "invalid analysis configuration: {field}"),
            Self::SuggestionLimit { limit } => write!(
                f,
                "analysis exceeds {limit} suggestions; increase spacing or the bounded limit"
            ),
        }
    }
}
impl Error for AnalysisError {}

#[derive(Clone, Copy, Default)]
struct Feature {
    energy: f64,
    difference: f64,
}

#[derive(Clone, Copy, Default)]
struct Candidate {
    strength: f64,
    baseline: f64,
    ratio: f64,
    energy_rise: bool,
    high_frequency_rise: bool,
}

/// Analyze borrowed immutable mono/stereo interleaved PCM off the audio thread.
/// Memory is O(source duration / 2 ms + requested overview), not another PCM
/// copy. Results are deterministic for identical samples/configuration on the
/// same floating-point platform. Returned frames are sorted, unique and valid.
pub fn analyze_transients(
    pcm: &[f32],
    channels: usize,
    source_rate: u32,
    config: &AnalysisConfig,
) -> Result<TransientAnalysis, AnalysisError> {
    validate(pcm, channels, source_rate, config)?;
    let frames = pcm.len() / channels;
    let hop = ((source_rate as usize + 250) / 500).max(1); // nearest 2 ms
    let spacing = (config.min_spacing_ms as f64 * source_rate as f64 / 1_000.0).ceil() as u64;
    // Below even the smallest f32 subnormal squared/maximum bin length, but
    // high enough to keep the largest finite f32 energy ratios finite in f64.
    let floor = (config.minimum_rms as f64).powi(2).max(1e-120);
    let threshold = 3.5 - 2.0 * config.sensitivity as f64;
    let (features, fingerprint) = features_and_fingerprint(pcm, channels, source_rate, hop)?;
    let candidates = candidates(&features, threshold, floor);
    let mut selected: Vec<(OnsetSuggestion, f64)> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.strength == 0.0 {
            continue;
        }
        // A 10 ms neighborhood, stable earliest-wins plateau tie breaking.
        let start = index.saturating_sub(2);
        let end = (index + 3).min(candidates.len());
        if (start..end).any(|other| {
            candidates[other].strength > candidate.strength
                || (other < index && candidates[other].strength == candidate.strength)
        }) {
            continue;
        }
        let frame = refine_attack(pcm, channels, hop, index, candidate.baseline);
        let onset = OnsetSuggestion {
            frame: frame as u64,
            confidence: ((candidate.ratio - threshold) / (candidate.ratio + threshold))
                .clamp(0.0, 1.0) as f32,
            energy_rise: candidate.energy_rise,
            high_frequency_rise: candidate.high_frequency_rise,
        };
        if let Some((previous, strength)) = selected.last_mut() {
            if onset.frame.saturating_sub(previous.frame) < spacing {
                if candidate.strength > *strength {
                    *previous = onset;
                    *strength = candidate.strength;
                }
                continue;
            }
        }
        if selected.len() == config.max_suggestions {
            return Err(AnalysisError::SuggestionLimit {
                limit: config.max_suggestions,
            });
        }
        selected.push((onset, candidate.strength));
    }
    Ok(TransientAnalysis {
        provenance: AnalysisProvenance {
            algorithm: ALGORITHM.into(),
            source_fingerprint: fingerprint,
            source_rate,
            channels,
            source_frames: frames as u64,
            hop_frames: hop as u32,
            min_spacing_frames: spacing,
            config: config.clone(),
        },
        suggestions: selected.into_iter().map(|(onset, _)| onset).collect(),
        waveform: waveform(pcm, channels, config.waveform_bins),
    })
}

fn validate(
    pcm: &[f32],
    channels: usize,
    rate: u32,
    config: &AnalysisConfig,
) -> Result<(), AnalysisError> {
    if !(1..=2).contains(&channels) {
        return Err(AnalysisError::InvalidChannels(channels));
    }
    if !(MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&rate) {
        return Err(AnalysisError::InvalidSampleRate(rate));
    }
    if !config.sensitivity.is_finite() || !(0.0..=1.0).contains(&config.sensitivity) {
        return Err(AnalysisError::InvalidConfig(
            "sensitivity must be finite in 0..=1",
        ));
    }
    if !config.min_spacing_ms.is_finite() || !(1.0..=1_000.0).contains(&config.min_spacing_ms) {
        return Err(AnalysisError::InvalidConfig(
            "min_spacing_ms must be finite in 1..=1000",
        ));
    }
    if !config.minimum_rms.is_finite() || !(0.0..=1.0).contains(&config.minimum_rms) {
        return Err(AnalysisError::InvalidConfig(
            "minimum_rms must be finite in 0..=1",
        ));
    }
    if config.max_frames == 0 || config.max_frames > MAX_SAMPLE_VALUES as u64 {
        return Err(AnalysisError::InvalidConfig(
            "max_frames must be in 1..=16777216",
        ));
    }
    if config.max_suggestions == 0 || config.max_suggestions > MAX_SUGGESTIONS {
        return Err(AnalysisError::InvalidConfig(
            "max_suggestions must be in 1..=65536",
        ));
    }
    if config.waveform_bins > MAX_WAVEFORM_BINS {
        return Err(AnalysisError::InvalidConfig(
            "waveform_bins must be in 0..=4096",
        ));
    }
    if pcm.is_empty() {
        return Err(AnalysisError::EmptyPcm);
    }
    if !pcm.len().is_multiple_of(channels) {
        return Err(AnalysisError::IncompleteFrame);
    }
    let frames = (pcm.len() / channels) as u64;
    if pcm.len() > MAX_SAMPLE_VALUES
        || frames > config.max_frames
        || frames > u64::from(rate) * MAX_DURATION_SECONDS
    {
        return Err(AnalysisError::InputLimit);
    }
    Ok(())
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn features_and_fingerprint(
    pcm: &[f32],
    channels: usize,
    rate: u32,
    hop: usize,
) -> Result<(Vec<Feature>, u64), AnalysisError> {
    let mut hash = 0xcbf29ce484222325;
    hash_bytes(&mut hash, &rate.to_le_bytes());
    hash_bytes(&mut hash, &(channels as u32).to_le_bytes());
    let mut previous = [0.0; 2];
    let mut features = Vec::with_capacity((pcm.len() / channels).div_ceil(hop));
    for (block_index, block) in pcm.chunks(hop * channels).enumerate() {
        let mut feature = Feature::default();
        for (sample_index, &value) in block.iter().enumerate() {
            if !value.is_finite() {
                return Err(AnalysisError::NonFiniteSample {
                    sample_index: block_index * hop * channels + sample_index,
                });
            }
            hash_bytes(&mut hash, &value.to_bits().to_le_bytes());
            let channel = sample_index % channels;
            let value = f64::from(value);
            feature.energy += value * value;
            feature.difference += (value - previous[channel]).powi(2);
            previous[channel] = value;
        }
        feature.energy /= block.len() as f64;
        feature.difference /= block.len() as f64;
        features.push(feature);
    }
    Ok((features, hash))
}

fn candidates(features: &[Feature], threshold: f64, floor: f64) -> Vec<Candidate> {
    // Ten preceding 2 ms bins provide a local noise/tail reference. Include
    // the immediate past so a noisy decaying hit does not repeatedly retrigger.
    let mut result = Vec::with_capacity(features.len());
    let mut recent = Feature::default();
    for (index, feature) in features.iter().enumerate() {
        let count = index.clamp(1, 10) as f64;
        let energy_baseline = (recent.energy / count).max(0.0);
        let difference_baseline = (recent.difference / count).max(0.0);
        let energy_ratio = feature.energy / energy_baseline.max(floor);
        let difference_ratio = feature.difference / difference_baseline.max(floor);
        let energy_rise = energy_ratio > threshold && feature.energy > floor;
        let high_frequency_rise =
            difference_ratio > threshold && feature.difference > floor && feature.energy > floor;
        let mut candidate = Candidate::default();
        if energy_rise || high_frequency_rise {
            candidate = Candidate {
                strength: (feature.energy - energy_baseline).max(0.0)
                    + 0.25 * (feature.difference - difference_baseline).max(0.0),
                baseline: energy_baseline,
                ratio: energy_ratio.max(difference_ratio),
                energy_rise,
                high_frequency_rise,
            };
        }
        result.push(candidate);
        recent.energy += feature.energy;
        recent.difference += feature.difference;
        if index >= 10 {
            recent.energy -= features[index - 10].energy;
            recent.difference -= features[index - 10].difference;
        }
    }
    result
}

fn refine_attack(pcm: &[f32], channels: usize, hop: usize, index: usize, baseline: f64) -> usize {
    let first = index.saturating_sub(2) * hop;
    let end = ((index + 1) * hop).min(pcm.len() / channels);
    let region = &pcm[first * channels..end * channels];
    let peak = region
        .iter()
        .fold(0.0f64, |peak, value| peak.max(f64::from(value.abs())));
    // Locate the rising attack, not a nearby zero crossing. The noise-aware
    // threshold avoids snapping quiet transients backward into a noise bed.
    let level = (peak * 0.1).max(baseline.sqrt() * 3.0).min(peak);
    region
        .chunks_exact(channels)
        .position(|frame| frame.iter().any(|value| f64::from(value.abs()) >= level))
        .map_or(index * hop, |offset| first + offset)
}

fn waveform(pcm: &[f32], channels: usize, requested: usize) -> Vec<WaveformBin> {
    let frames = pcm.len() / channels;
    let count = requested.min(frames);
    (0..count)
        .map(|index| {
            let start = index * frames / count;
            let end = (index + 1) * frames / count;
            let mut extrema = vec![
                ChannelExtrema {
                    min: f32::INFINITY,
                    max: f32::NEG_INFINITY
                };
                channels
            ];
            for frame in pcm[start * channels..end * channels].chunks_exact(channels) {
                for (value, extrema) in frame.iter().zip(&mut extrema) {
                    extrema.min = extrema.min.min(*value);
                    extrema.max = extrema.max.max(*value);
                }
            }
            WaveformBin {
                start_frame: start as u64,
                end_frame: end as u64,
                channels: extrema,
            }
        })
        .collect()
}
