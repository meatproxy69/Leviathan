//! Optional experimental detector; v1 remains the application default.
//! Original DSP informed by sources in docs/onset-v2-research.md.
//! RealFFT/RustFFT only; no runtime ML, classification, tempo, or sample editing.
use crate::break_analysis::{
    MAX_DURATION_SECONDS, MAX_SAMPLE_VALUES, MAX_SOURCE_RATE, MIN_SOURCE_RATE,
};
use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ALGORITHM: &str = "multiband-evidence-prototype-v2.1";
const MAX_WORK: u64 = 2_000_000_000;
const MAX_MEMORY: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Preset {
    Conservative,
    Balanced,
    GhostHunter,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StereoMode {
    LeftRight,
    MidSide,
    MidSideAndLeftRight,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub low_hz: f64,
    pub high_hz: f64,
    pub window_at_44100: usize,
    pub hop_multiple: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub preset: Preset,
    pub stereo: StereoMode,
    pub bands: [Band; 3],
    /// Reference fast hop at44.1k, scaled by rate and rounded to source frames.
    pub fast_hop_at_44100: usize,
    pub median_radius_ms: f64,
    pub mad_multiplier: f64,
    pub minimum_rms: f64,
    pub threshold: f64,
    pub min_gap_ms: f64,
    /// Hard lower gap. Nearby candidates must additionally show a novelty dip
    /// and both have strong fast-attack evidence to survive inside min_gap_ms.
    pub dense_gap_ms: f64,
    pub dense_evidence: f64,
    pub max_frames: u64,
    pub max_suggestions: usize,
    pub max_fft_size: usize,
    pub max_work_units: u64,
    pub max_working_bytes: u64,
    pub max_filter_bins: usize,
    pub enable_flux: bool,
    pub enable_energy: bool,
    pub enable_phase: bool,
    pub enable_fast_attack: bool,
    pub retain_envelope: bool,
    pub refinement: RefinementConfig,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RefinementConfig {
    pub enabled: bool,
    pub backtrack_ms: f64,
    pub lookahead_ms: f64,
    pub attack_fraction: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self::preset(Preset::Balanced)
    }
}
impl Config {
    pub fn preset(preset: Preset) -> Self {
        let (mad_multiplier, threshold, min_gap_ms, dense_gap_ms) = match preset {
            Preset::Conservative => (5.0, 0.62, 35.0, 11.0),
            Preset::Balanced => (3.0, 0.52, 25.0, 8.0),
            Preset::GhostHunter => (1.75, 0.43, 18.0, 5.0),
        };
        Self {
            preset,
            stereo: StereoMode::MidSide,
            bands: [
                Band {
                    low_hz: 20.0,
                    high_hz: 180.0,
                    window_at_44100: 2048,
                    hop_multiple: 4,
                },
                Band {
                    low_hz: 180.0,
                    high_hz: 2000.0,
                    window_at_44100: 1024,
                    hop_multiple: 2,
                },
                Band {
                    low_hz: 2000.0,
                    high_hz: 16000.0,
                    window_at_44100: 512,
                    hop_multiple: 1,
                },
            ],
            fast_hop_at_44100: 64,
            median_radius_ms: 80.0,
            mad_multiplier,
            minimum_rms: 0.00001,
            threshold,
            min_gap_ms,
            dense_gap_ms,
            dense_evidence: 0.60,
            max_frames: MAX_SAMPLE_VALUES as u64,
            max_suggestions: 4096,
            max_fft_size: 16384,
            max_work_units: 500_000_000,
            max_working_bytes: 128 * 1024 * 1024,
            max_filter_bins: 3,
            enable_flux: true,
            enable_energy: true,
            enable_phase: true,
            enable_fast_attack: true,
            retain_envelope: false,
            refinement: RefinementConfig {
                enabled: true,
                backtrack_ms: 30.0,
                lookahead_ms: 10.0,
                attack_fraction: 0.08,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveBand {
    pub low_hz: f64,
    pub high_hz: f64,
    pub window_frames: usize,
    pub hop_frames: usize,
    pub fft_bin_count: usize,
    pub alignment_frames: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub algorithm: String,
    pub fft_backend: String,
    pub rate: u32,
    pub channels: usize,
    pub source_frames: u64,
    /// Exact-bit cache fingerprint, explicitly NOT a cryptographic asset identity.
    pub pcm_fingerprint: u64,
    pub config: Config,
    pub effective_bands: [EffectiveBand; 3],
    pub fusion_hop_frames: usize,
    pub estimated_work_units: u64,
    pub estimated_working_bytes: u64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub flux: f64,
    pub energy: f64,
    pub phase: f64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeFrame {
    /// Current/fullband preceding20ms RMS ratio, used only to reject releases.
    pub level_ratio: f64,
    pub bands: [Evidence; 3],
    pub fast_attack: f64,
    pub confidence: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub peak_frame: u64,
    /// Unnormalized positive RMS/energy change, not a probability.
    pub strength: f64,
    pub detector_votes: [bool; 4],
    pub confidence: f64,
    pub evidence: EnvelopeFrame,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub provenance: Provenance,
    pub candidates: Vec<Candidate>,
    pub envelope: Vec<EnvelopeFrame>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub candidate: Candidate,
    pub attack_frame: u64,
    pub search_start: u64,
    pub search_end: u64,
    pub refinement_shift: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub detection: Detection,
    pub suggestions: Vec<Suggestion>,
    pub refinement_collisions: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidInput(&'static str),
    NonFinite(usize),
    InvalidConfig(&'static str),
    Budget(&'static str),
    OutputLimit,
    MismatchedSource,
    Fft,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "onset prototype: {self:?}")
    }
}
impl std::error::Error for Error {}

fn fingerprint(pcm: &[f32], channels: usize, rate: u32) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for byte in rate
        .to_le_bytes()
        .into_iter()
        .chain((channels as u32).to_le_bytes())
        .chain(pcm.iter().flat_map(|x| x.to_bits().to_le_bytes()))
    {
        h = (h ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    h
}
fn finite_range(x: f64, low: f64, high: f64) -> bool {
    x.is_finite() && (low..=high).contains(&x)
}
fn check(pcm: &[f32], channels: usize, rate: u32, c: &Config) -> Result<(), Error> {
    if !(1..=2).contains(&channels)
        || !(MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&rate)
        || pcm.is_empty()
        || !pcm.len().is_multiple_of(channels)
    {
        return Err(Error::InvalidInput(
            "nonempty mono/stereo native-rate PCM required",
        ));
    }
    if pcm.len() > MAX_SAMPLE_VALUES
        || (pcm.len() / channels) as u64 > c.max_frames
        || (pcm.len() / channels) as u64 > u64::from(rate) * MAX_DURATION_SECONDS
    {
        return Err(Error::Budget("input PCM/duration"));
    }
    if let Some(index) = pcm.iter().position(|v| !v.is_finite()) {
        return Err(Error::NonFinite(index));
    }
    if c.max_frames == 0
        || c.max_frames > MAX_SAMPLE_VALUES as u64
        || !(1..=65536).contains(&c.max_suggestions)
        || !(16..=256).contains(&c.fast_hop_at_44100)
        || !c.max_fft_size.is_power_of_two()
        || !(256..=32768).contains(&c.max_fft_size)
        || c.max_work_units == 0
        || c.max_work_units > MAX_WORK
        || c.max_working_bytes == 0
        || c.max_working_bytes > MAX_MEMORY
        || ![1, 3, 5].contains(&c.max_filter_bins)
    {
        return Err(Error::InvalidConfig("resource settings"));
    }
    if !finite_range(c.median_radius_ms, 20.0, 250.0)
        || !finite_range(c.mad_multiplier, 0.5, 12.0)
        || !finite_range(c.minimum_rms, 1e-12, 1.0)
        || !finite_range(c.threshold, 0.1, 0.95)
        || !finite_range(c.min_gap_ms, 1.0, 1000.0)
        || !finite_range(c.dense_gap_ms, 1.0, c.min_gap_ms)
        || !finite_range(c.dense_evidence, 0.5, 1.0)
    {
        return Err(Error::InvalidConfig("thresholds/gaps"));
    }
    for b in &c.bands {
        if !finite_range(b.low_hz, 1.0, 100_000.0)
            || !finite_range(b.high_hz, b.low_hz + 1.0, 192_000.0)
            || !b.window_at_44100.is_power_of_two()
            || !(128..=4096).contains(&b.window_at_44100)
            || ![1, 2, 4, 8].contains(&b.hop_multiple)
        {
            return Err(Error::InvalidConfig("bands/resolutions"));
        }
    }
    let r = c.refinement;
    if !finite_range(r.backtrack_ms, 0.0, 100.0)
        || !finite_range(r.lookahead_ms, 0.0, 50.0)
        || !finite_range(r.attack_fraction, 0.001, 0.5)
    {
        return Err(Error::InvalidConfig("refinement"));
    }
    Ok(())
}
fn closest_power_two(value: usize) -> usize {
    let high = value.next_power_of_two();
    let low = high / 2;
    if value - low < high - value {
        low.max(2)
    } else {
        high
    }
}
fn lanes(channels: usize, mode: StereoMode) -> usize {
    if channels == 1 {
        1
    } else if mode == StereoMode::MidSideAndLeftRight {
        4
    } else {
        2
    }
}
fn lane_sample(pcm: &[f32], channels: usize, frame: usize, lane: usize, mode: StereoMode) -> f64 {
    if channels == 1 {
        return f64::from(pcm[frame]);
    }
    let l = f64::from(pcm[frame * 2]);
    let r = f64::from(pcm[frame * 2 + 1]);
    match mode {
        StereoMode::LeftRight => {
            if lane == 0 {
                l
            } else {
                r
            }
        }
        _ => match lane {
            0 => (l + r) * 0.5,
            1 => (l - r) * 0.5,
            2 => l,
            _ => r,
        },
    }
}

fn normalization_work(grid_len: usize, hop: usize, rate: u32, radius_ms: f64) -> u64 {
    let width = (((2.0 * radius_ms * rate as f64 / 1000.0 / hop as f64).ceil() as usize) + 1)
        .min(grid_len)
        .max(1);
    grid_len as u64 * 10 * width as u64 * (2 * (width.ilog2() as u64 + 1) + 4)
}

/// Allocating worker-only stage1: immutable PCM -> auditable coarse candidates.
pub fn extract(pcm: &[f32], channels: usize, rate: u32, c: &Config) -> Result<NoveltyCache, Error> {
    check(pcm, channels, rate, c)?;
    let frames = pcm.len() / channels;
    let hop = ((c.fast_hop_at_44100 as u64 * rate as u64 + 22050) / 44100).max(1) as usize;
    let grid_len = frames.div_ceil(hop);
    let lane_count = lanes(channels, c.stereo);
    let bands: [EffectiveBand; 3] = std::array::from_fn(|i| {
        let b = c.bands[i];
        let n =
            closest_power_two(((b.window_at_44100 as u64 * rate as u64 + 22050) / 44100) as usize);
        let lo = b.low_hz.min(rate as f64 / 2.0);
        let hi = b.high_hz.min(rate as f64 / 2.0);
        let count = ((hi * n as f64 / rate as f64).ceil() as usize)
            .saturating_sub((lo * n as f64 / rate as f64).ceil() as usize);
        EffectiveBand {
            low_hz: lo,
            high_hz: hi,
            window_frames: n,
            hop_frames: hop * b.hop_multiple,
            fft_bin_count: count,
            alignment_frames: n / 4,
        }
    });
    if bands
        .iter()
        .any(|b| b.window_frames > c.max_fft_size || b.hop_frames > b.window_frames)
    {
        return Err(Error::Budget("FFT size/hop"));
    }
    let stats_work = normalization_work(grid_len, hop, rate, c.median_radius_ms);
    let refine_work = frames as u64 * 32;
    let work = bands
        .iter()
        .map(|b| {
            frames.div_ceil(b.hop_frames) as u64
                * b.window_frames as u64
                * b.window_frames.ilog2() as u64
                * lane_count as u64
        })
        .sum::<u64>()
        + stats_work
        + refine_work;
    let memory = grid_len as u64 * 512
        + bands
            .iter()
            .map(|b| b.window_frames as u64 * 512)
            .sum::<u64>();
    if work > c.max_work_units {
        return Err(Error::Budget("deterministic FFT/statistics work estimate"));
    }
    if memory > c.max_working_bytes {
        return Err(Error::Budget("conservative working-memory estimate"));
    }
    let mut raw = vec![EnvelopeFrame::default(); grid_len];
    for (band_index, b) in bands.iter().enumerate() {
        let mut planner = RealFftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(b.window_frames);
        let mut input = fft.make_input_vec();
        let mut output = fft.make_output_vec();
        let mut scratch = fft.make_scratch_vec();
        let window: Vec<_> = (0..b.window_frames)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / b.window_frames as f64).cos())
            .collect();
        let norm = 2.0 / window.iter().sum::<f64>();
        let first = (b.low_hz * b.window_frames as f64 / rate as f64).ceil() as usize;
        let end =
            ((b.high_hz * b.window_frames as f64 / rate as f64).ceil() as usize).min(output.len());
        for lane in 0..lane_count {
            let mut old_mag = vec![0.0; output.len()];
            let mut phase1 = old_mag.clone();
            let mut phase2 = old_mag.clone();
            let mut old_energy = 0.0;
            for step in 0..frames.div_ceil(b.hop_frames) {
                let center = step * b.hop_frames;
                for (i, value) in input.iter_mut().enumerate() {
                    let at = center as i64 + i as i64 - b.window_frames as i64 / 2;
                    *value = if at >= 0 && (at as usize) < frames {
                        lane_sample(pcm, channels, at as usize, lane, c.stereo) * window[i]
                    } else {
                        0.0
                    };
                }
                fft.process_with_scratch(&mut input, &mut output, &mut scratch)
                    .map_err(|_| Error::Fft)?;
                let mut evidence = Evidence::default();
                let mut energy = 0.0;
                for k in first..end {
                    let mag = output[k].norm() * norm;
                    let phase = output[k].arg();
                    energy += mag * mag;
                    if step > 0 {
                        let radius = c.max_filter_bins / 2;
                        let reference = old_mag
                            [k.saturating_sub(radius)..(k + radius + 1).min(old_mag.len())]
                            .iter()
                            .copied()
                            .fold(0.0, f64::max);
                        evidence.flux += ((mag / c.minimum_rms).ln_1p()
                            - (reference / c.minimum_rms).ln_1p())
                        .max(0.0);
                        if step > 1 && mag >= old_mag[k] {
                            let residual = phase - 2.0 * phase1[k] + phase2[k];
                            evidence.phase += (mag * mag + old_mag[k] * old_mag[k]
                                - 2.0 * mag * old_mag[k] * residual.cos())
                            .max(0.0)
                            .sqrt();
                        }
                    }
                    // Save phase/magnitude only AFTER all neighborhood references
                    // have been read; the separate post-loop update below does it.
                }
                evidence.energy = (energy.sqrt() - old_energy).max(0.0);
                old_energy = energy.sqrt();
                if step == 0 {
                    evidence = Evidence::default();
                } // no fabricated previous spectrum
                if energy.sqrt() < c.minimum_rms {
                    evidence = Evidence::default();
                }
                let shift = b.alignment_frames;
                let grid = ((center + shift) / hop).min(grid_len - 1);
                let radius = (b.hop_frames / hop) / 2;
                for value in
                    &mut raw[grid.saturating_sub(radius)..(grid + radius + 1).min(grid_len)]
                {
                    let target = &mut value.bands[band_index];
                    target.flux = target.flux.max(evidence.flux);
                    target.energy = target.energy.max(evidence.energy);
                    target.phase = target.phase.max(evidence.phase);
                }
                for k in 0..output.len() {
                    phase2[k] = phase1[k];
                    phase1[k] = output[k].arg();
                    old_mag[k] = output[k].norm() * norm;
                }
            }
        }
    }
    // Fast/fullband change, preserving antiphase by summing channel energies.
    let history_bins = ((0.020 * rate as f64 / hop as f64).round() as usize).max(1);
    let mut history = std::collections::VecDeque::with_capacity(history_bins);
    let mut sum = 0.0;
    for (i, block) in pcm.chunks(hop * channels).enumerate() {
        let energy = block.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / block.len() as f64;
        let rms = energy.sqrt();
        let background = if history.is_empty() {
            0.0
        } else {
            (sum / history.len() as f64).max(0.0f64).sqrt()
        };
        raw[i].level_ratio = (rms / background.max(c.minimum_rms)).min(1e30);
        raw[i].fast_attack = if rms > c.minimum_rms {
            (rms - 1.6 * background).max(0.0)
        } else {
            0.0
        };
        history.push_back(energy);
        sum += energy;
        if history.len() > history_bins {
            sum -= history.pop_front().unwrap();
        }
    }
    let provenance = Provenance {
        algorithm: ALGORITHM.into(),
        fft_backend: "realfft-3.5.0/rustfft-6.4.1".into(),
        rate,
        channels,
        source_frames: frames as u64,
        pcm_fingerprint: fingerprint(pcm, channels, rate),
        config: c.clone(),
        effective_bands: bands,
        fusion_hop_frames: hop,
        estimated_work_units: work,
        estimated_working_bytes: memory,
    };
    Ok(NoveltyCache { provenance, raw })
}

/// Immutable unthresholded curves retain weak candidates. This cache is not a
/// saved asset identity and cannot overwrite manual markers. Changing sensitivity,
/// MAD, evidence weights/ablations or refinement does not run another FFT.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoveltyCache {
    provenance: Provenance,
    raw: Vec<EnvelopeFrame>,
}
impl NoveltyCache {
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
    pub fn raw(&self) -> &[EnvelopeFrame] {
        &self.raw
    }
}

pub fn detect_cached(cache: &NoveltyCache, c: &Config) -> Result<Detection, Error> {
    let p = &cache.provenance;
    check(&[0.0], 1, p.rate, c)?;
    let old = &p.config;
    if c.stereo != old.stereo
        || c.bands != old.bands
        || c.fast_hop_at_44100 != old.fast_hop_at_44100
        || c.minimum_rms != old.minimum_rms
        || c.max_filter_bins != old.max_filter_bins
    {
        return Err(Error::InvalidConfig(
            "spectral settings changed; extract a new cache",
        ));
    }
    let work = p.estimated_work_units
        - normalization_work(
            cache.raw.len(),
            p.fusion_hop_frames,
            p.rate,
            old.median_radius_ms,
        )
        + normalization_work(
            cache.raw.len(),
            p.fusion_hop_frames,
            p.rate,
            c.median_radius_ms,
        );
    if p.source_frames > c.max_frames
        || p.effective_bands
            .iter()
            .any(|b| b.window_frames > c.max_fft_size)
        || work > c.max_work_units
        || p.estimated_working_bytes > c.max_working_bytes
    {
        return Err(Error::Budget("cached source exceeds requested budget"));
    }
    let rate = p.rate;
    let hop = p.fusion_hop_frames;
    let grid_len = cache.raw.len();
    let raw = &cache.raw;
    let bands = &p.effective_bands;
    let radius = (c.median_radius_ms * rate as f64 / 1000.0 / hop as f64).ceil() as usize;
    let mut envelope = vec![EnvelopeFrame::default(); grid_len];
    for band in 0..3 {
        for detector in 0..3 {
            let values: Vec<f64> = raw
                .iter()
                .map(|e| match detector {
                    0 => e.bands[band].flux,
                    1 => e.bands[band].energy,
                    _ => e.bands[band].phase,
                })
                .collect();
            let normalized = normalize(
                &values,
                radius,
                c.mad_multiplier,
                if detector == 0 {
                    0.01
                } else {
                    c.minimum_rms * 0.05
                },
            );
            for (e, score) in envelope.iter_mut().zip(normalized) {
                match detector {
                    0 => e.bands[band].flux = if c.enable_flux { score } else { 0.0 },
                    1 => e.bands[band].energy = if c.enable_energy { score } else { 0.0 },
                    _ => e.bands[band].phase = if c.enable_phase { score } else { 0.0 },
                }
            }
        }
    }
    let fast: Vec<_> = raw.iter().map(|e| e.fast_attack).collect();
    let fast = normalize(&fast, radius, c.mad_multiplier, c.minimum_rms * 0.05);
    for (i, (e, fast)) in envelope.iter_mut().zip(fast).enumerate() {
        e.level_ratio = raw[i].level_ratio;
        e.fast_attack = if c.enable_fast_attack { fast } else { 0.0 };
        let best = e
            .bands
            .iter()
            .map(|b| 0.55 * b.flux + 0.30 * b.energy + 0.15 * b.phase)
            .fold(0.0, f64::max);
        // Phase is supporting evidence, never a standalone onset authority,
        // including custom thresholds below the phase weight (0.12).
        let non_phase =
            e.fast_attack > 0.0 || e.bands.iter().any(|b| b.flux > 0.0 || b.energy > 0.0);
        e.confidence = if non_phase && raw[i].level_ratio >= 0.75 {
            (0.80 * best + 0.20 * e.fast_attack).max(0.85 * e.fast_attack)
        } else {
            0.0
        };
    }
    // Near the source edge no complete previous FFT exists. Only an actual
    // time-domain rise may authorize an onset here; never an FFT startup pulse.
    let startup = bands
        .iter()
        .map(|b| b.window_frames / 2 + b.alignment_frames)
        .max()
        .unwrap()
        / hop;
    for (i, e) in envelope.iter_mut().enumerate() {
        // Centered FFT zero padding at either source edge is not a physical
        // attack. Edge candidates require independent time-domain evidence.
        if (i < startup || i >= grid_len.saturating_sub(startup)) && e.fast_attack < 0.6 {
            e.confidence = 0.0;
        }
    }
    let mut proposed = Vec::new();
    for i in 0..grid_len {
        let score = envelope[i].confidence;
        if score < c.threshold {
            continue;
        }
        if (i.saturating_sub(1)..(i + 2).min(grid_len))
            .any(|j| envelope[j].confidence > score || (j < i && envelope[j].confidence == score))
        {
            continue;
        }
        proposed.push(Candidate {
            strength: raw[i]
                .fast_attack
                .max(raw[i].bands.iter().map(|b| b.energy).fold(0.0, f64::max)),
            detector_votes: [
                envelope[i].bands.iter().any(|b| b.flux >= 0.5),
                envelope[i].bands.iter().any(|b| b.energy >= 0.5),
                envelope[i].bands.iter().any(|b| b.phase >= 0.5),
                envelope[i].fast_attack >= 0.5,
            ],
            peak_frame: (i * hop) as u64,
            confidence: score,
            evidence: envelope[i],
        });
    }
    proposed.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.peak_frame.cmp(&b.peak_frame))
    });
    let gap = (c.min_gap_ms * rate as f64 / 1000.0).ceil() as u64;
    let dense = (c.dense_gap_ms * rate as f64 / 1000.0).ceil() as u64;
    let mut selected: BTreeMap<u64, Candidate> = BTreeMap::new();
    for candidate in proposed {
        let frame = candidate.peak_frame;
        let conflicts = selected
            .range(frame.saturating_sub(gap - 1)..=frame + gap - 1)
            .any(|(_, other)| {
                let distance = frame.abs_diff(other.peak_frame);
                if distance < dense {
                    return true;
                }
                let a = frame.min(other.peak_frame) as usize / hop;
                let b = frame.max(other.peak_frame) as usize / hop;
                let dip = envelope[a..=b]
                    .iter()
                    .map(|e| e.confidence)
                    .fold(1.0, f64::min);
                candidate.evidence.fast_attack < c.dense_evidence
                    || other.evidence.fast_attack < c.dense_evidence
                    || dip > 0.5 * candidate.confidence.min(other.confidence)
            });
        if !conflicts {
            selected.insert(frame, candidate);
        }
    }
    if selected.len() > c.max_suggestions {
        return Err(Error::OutputLimit);
    }
    let mut provenance = p.clone();
    provenance.config = c.clone();
    provenance.estimated_work_units = work;
    Ok(Detection {
        provenance,
        candidates: selected.into_values().collect(),
        envelope: if c.retain_envelope {
            envelope
        } else {
            Vec::new()
        },
    })
}
/// Convenience composition; repeated threshold changes should call detect_cached.
pub fn detect(pcm: &[f32], channels: usize, rate: u32, c: &Config) -> Result<Detection, Error> {
    let cache = extract(pcm, channels, rate, c)?;
    detect_cached(&cache, c)
}
fn normalize(values: &[f64], radius: usize, k: f64, floor: f64) -> Vec<f64> {
    if values.len() < 3 {
        // Too few observations for median/MAD. Preserve a genuine short
        // source-edge attack without pretending there is a noise estimate.
        let peak = values.iter().copied().fold(0.0, f64::max).max(floor);
        return values
            .iter()
            .map(|v| if *v > floor { *v / peak } else { 0.0 })
            .collect();
    }
    let mut scratch = Vec::with_capacity(2 * radius + 1);
    let mut deviations = scratch.clone();
    values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            if *value <= floor {
                return 0.0;
            }
            scratch.clear();
            scratch.extend_from_slice(
                &values[i.saturating_sub(radius)..(i + radius + 1).min(values.len())],
            );
            scratch.sort_by(f64::total_cmp);
            let median = scratch[scratch.len() / 2];
            deviations.clear();
            deviations.extend(scratch.iter().map(|x| (x - median).abs()));
            deviations.sort_by(f64::total_cmp);
            // Rectified novelty often has >50% exact zeros: median=MAD=0.
            // A local mean absolute-deviation floor prevents every residual
            // noise fluctuation acquiring confidence1 in that degenerate case.
            let mad = deviations[deviations.len() / 2]
                .max(2.0 * scratch.iter().sum::<f64>() / scratch.len() as f64);
            let excess = (value - median).max(0.0);
            let threshold = k * mad + 0.5 * median + floor;
            excess / (excess + threshold)
        })
        .collect()
}

/// Separate stage2: refine proposals, never manual markers. Validates source
/// fingerprint to prevent accidentally refining against a different PCM asset.
pub fn refine(
    pcm: &[f32],
    channels: usize,
    rate: u32,
    detection: &Detection,
) -> Result<(Vec<Suggestion>, usize), Error> {
    let c = &detection.provenance.config;
    check(pcm, channels, rate, c)?;
    if detection.provenance.source_frames != (pcm.len() / channels) as u64
        || detection.provenance.rate != rate
        || detection.provenance.channels != channels
        || detection.provenance.pcm_fingerprint != fingerprint(pcm, channels, rate)
        || detection.provenance.algorithm != ALGORITHM
    {
        return Err(Error::MismatchedSource);
    }
    if detection.candidates.len() > c.max_suggestions
        || detection.candidates.iter().any(|x| {
            x.peak_frame >= detection.provenance.source_frames
                || !finite_range(x.confidence, 0.0, 1.0)
        })
        || detection
            .candidates
            .windows(2)
            .any(|x| x[0].peak_frame >= x[1].peak_frame)
    {
        return Err(Error::InvalidInput("candidate bounds/order"));
    }
    let n = pcm.len() / channels;
    let r = c.refinement;
    let pre = (r.backtrack_ms * rate as f64 / 1000.0).ceil() as usize;
    let post = (r.lookahead_ms * rate as f64 / 1000.0).ceil() as usize;
    let smoothing = (rate as usize / 4000).max(1); // 0.25ms causal energy, not a5ms marker grid
    let mut result = Vec::new();
    for (index, candidate) in detection.candidates.iter().enumerate() {
        let center = candidate.peak_frame as usize;
        let left = if index == 0 {
            0
        } else {
            (center + detection.candidates[index - 1].peak_frame as usize) / 2
        };
        let right = if index + 1 == detection.candidates.len() {
            n
        } else {
            (center + detection.candidates[index + 1].peak_frame as usize) / 2
        };
        let start = center.saturating_sub(pre).max(left);
        let end = (center + post + 1).min(n).min(right.max(center + 1));
        let mut frame = center;
        if r.enabled {
            let mut energy = 0.0;
            let mut levels = Vec::with_capacity(end - start);
            for at in start.saturating_sub(smoothing - 1)..end {
                energy += pcm[at * channels..(at + 1) * channels]
                    .iter()
                    .map(|v| f64::from(*v).powi(2))
                    .sum::<f64>();
                if at >= start {
                    if at >= smoothing && at - smoothing >= start.saturating_sub(smoothing - 1) {
                        energy -= pcm[(at - smoothing) * channels..(at - smoothing + 1) * channels]
                            .iter()
                            .map(|v| f64::from(*v).powi(2))
                            .sum::<f64>();
                    }
                    levels.push((energy / ((at + 1).min(smoothing) * channels) as f64).max(0.0));
                }
            }
            // Find the strongest local positive envelope rise, then backtrack
            // to the preceding low-energy crossing. Bounds stop unrelated hits.
            let mut rise = 0.0;
            let mut peak = 0;
            for i in 0..levels.len() {
                let before = if i >= smoothing {
                    levels[i - smoothing]
                } else {
                    levels[0]
                };
                let proximity = 1.0 + (start + i).abs_diff(center) as f64 / (rate as f64 * 0.003);
                let novelty = (levels[i] - before).max(0.0) / proximity;
                if novelty > rise {
                    rise = novelty;
                    peak = i;
                }
            }
            if rise > c.minimum_rms.powi(2) {
                let mut prior = levels[..=peak.saturating_sub(smoothing)].to_vec();
                prior.sort_by(f64::total_cmp);
                let baseline = prior[prior.len() / 2];
                let target = baseline + (levels[peak] - baseline) * r.attack_fraction.powi(2);
                let crossing = (0..peak)
                    .rev()
                    .find(|&i| levels[i] <= target)
                    .map_or(0, |i| i + 1);
                frame = start + crossing;
                // An already nonzero first sample is a source-start edge;
                // do not move it later just because its next sample is louder.
                if start == 0
                    && peak < smoothing
                    && pcm[..channels]
                        .iter()
                        .any(|v| f64::from(v.abs()) >= c.minimum_rms)
                {
                    frame = 0;
                }
            }
        }
        result.push(Suggestion {
            candidate: candidate.clone(),
            attack_frame: frame as u64,
            search_start: start as u64,
            search_end: end as u64,
            refinement_shift: frame as i64 - center as i64,
        });
    }
    result.sort_by(|a, b| {
        b.candidate
            .confidence
            .total_cmp(&a.candidate.confidence)
            .then_with(|| a.attack_frame.cmp(&b.attack_frame))
    });
    let dense = (c.dense_gap_ms * rate as f64 / 1000.0).ceil() as u64;
    let mut selected: BTreeMap<u64, Suggestion> = BTreeMap::new();
    let mut collisions = 0;
    for suggestion in result {
        let f = suggestion.attack_frame;
        if selected
            .range(f.saturating_sub(dense - 1)..=f + dense - 1)
            .next()
            .is_some()
        {
            collisions += 1;
        } else {
            selected.insert(f, suggestion);
        }
    }
    Ok((selected.into_values().collect(), collisions))
}
pub fn analyze(pcm: &[f32], channels: usize, rate: u32, c: &Config) -> Result<Analysis, Error> {
    let detection = detect(pcm, channels, rate, c)?;
    let (suggestions, refinement_collisions) = refine(pcm, channels, rate, &detection)?;
    Ok(Analysis {
        detection,
        suggestions,
        refinement_collisions,
    })
}
