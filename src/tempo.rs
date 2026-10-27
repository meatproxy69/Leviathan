//! Bounded periodicity hypotheses from declared timing points. No automatic
//! tempo assignment, downbeat inference, new decoding, or playback changes.
use crate::{onset_service, onset_v2, song::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "timing-point-periodicity-v1";
pub const MAX_POINTS: usize = 1024;
pub const MAX_SHORTLIST: usize = 64;
pub const MAX_CANDIDATES: usize = 8;
pub const MAX_PULSES: usize = 2048;
pub const MAX_PAGE: usize = 256;
pub const MAX_WORK: u64 = 16_000_000;
pub const MAX_WINDOW_SECONDS: u64 = 300;
pub const MAX_CACHED_POINTS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoConfig {
    pub version: u32,
    pub min_bpm_tenths: u16,
    pub max_bpm_tenths: u16,
}
impl Default for TempoConfig {
    fn default() -> Self {
        Self {
            version: VERSION,
            min_bpm_tenths: 200,
            max_bpm_tenths: 4000,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerRequest {
    pub marker_set: MarkerSetId,
    pub start_index: u16,
    pub end_index: u16,
    pub include_start: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointBasis {
    Peak,
    RefinedAttack,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedRequest {
    pub start_frame: u64,
    pub end_frame: u64,
    pub basis: PointBasis,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceKind {
    AcceptedMarkers {
        request: MarkerRequest,
        marker_set_sha256: String,
    },
    CachedV2 {
        request: CachedRequest,
        extraction_key: String,
        onset_config_sha256: String,
        onset_algorithm: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvidenceBinding {
    pub sample: SampleId,
    pub source_sha256: String,
    pub source_rate: u32,
    pub source_frames: u64,
    pub window_start_frame: u64,
    pub window_end_frame: u64,
    pub kind: EvidenceKind,
    /// Identity of the declared snapshot, not proof of current on-disk bytes.
    pub identity_basis: String,
    pub points_sha256: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    Weak,
    Moderate,
    Strong,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TempoAlias {
    pub bpm_tenths: u16,
    pub ratio_numerator: u8,
    pub ratio_denominator: u8,
    pub returned: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TempoCandidate {
    /// Exact integer BPM authority. `bpm` is its display/conversion value.
    pub bpm_tenths: u16,
    pub bpm: f64,
    /// Exact source-frame pulse phase relative to window start; not a downbeat.
    pub phase_numerator: u64,
    pub phase_denominator: u32,
    pub phase_frames: f64,
    pub score: f64,
    pub phase_fit: f64,
    pub interval_vote: f64,
    pub relative_interval_vote: f64,
    pub pulse_coverage: f64,
    pub occupied_pulses: usize,
    pub pulse_count: usize,
    /// Kernel-positive points whose nearest pulse is in the displayed grid.
    pub matched_points: usize,
    /// Kernel-positive points assigned to negative/end-excluded pulse indices.
    pub outside_window_matches: usize,
    /// matched_points minus occupied_pulses; coincident evidence is not merged.
    pub multiple_points_per_pulse: usize,
    /// (Last evidence frame - first evidence frame) / exact candidate period.
    pub span_beats: f64,
    /// Mean nearest circular residual over ALL selected timing points.
    pub mean_error_beats: f64,
    /// Versioned timing-evidence heuristic, never calibrated tempo confidence.
    pub strength: EvidenceStrength,
    pub aliases: Vec<TempoAlias>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShortlistedTempo {
    pub bpm_tenths: u16,
    pub interval_vote: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TempoAnalysis {
    pub algorithm: String,
    pub config: TempoConfig,
    pub binding: EvidenceBinding,
    pub insufficient_reason: Option<String>,
    pub evidence_count: usize,
    pub span_frames: u64,
    pub interval_pairs: usize,
    pub estimated_work: u64,
    pub local_maxima_count: usize,
    pub shortlist_truncated: bool,
    pub shortlist: Vec<ShortlistedTempo>,
    pub separated_candidate_count: usize,
    pub candidates: Vec<TempoCandidate>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PulsePage {
    pub candidate_index: usize,
    pub start: usize,
    pub total: usize,
    pub frames: Vec<u64>,
}
#[derive(Debug, Clone)]
pub struct TempoPreview {
    id: String,
    base: Song,
    analysis: TempoAnalysis,
    points: Vec<u64>,
    pulses: Vec<Vec<u64>>,
}
impl TempoPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn config(&self) -> &TempoConfig {
        &self.analysis.config
    }
    pub fn analysis(&self) -> &TempoAnalysis {
        &self.analysis
    }
    pub fn points(&self) -> &[u64] {
        &self.points
    }
    pub fn pulse_page(
        &self,
        candidate_index: usize,
        start: usize,
        limit: usize,
    ) -> Result<PulsePage, String> {
        let pulses = self
            .pulses
            .get(candidate_index)
            .ok_or("Unknown tempo candidate")?;
        if !(1..=MAX_PAGE).contains(&limit) || start > pulses.len() {
            return Err("Invalid tempo pulse page range/limit".into());
        }
        Ok(PulsePage {
            candidate_index,
            start,
            total: pulses.len(),
            frames: pulses[start..start.saturating_add(limit).min(pulses.len())].to_vec(),
        })
    }
    pub fn use_bpm(
        &self,
        target: MarkerSetId,
        candidate_index: usize,
    ) -> Result<UseBpmPreview, String> {
        let selected = self
            .analysis
            .candidates
            .get(candidate_index)
            .ok_or("Unknown tempo candidate; insufficient evidence has no fallback BPM")?;
        let mut set = self
            .base
            .marker_sets
            .get(&target)
            .cloned()
            .ok_or("Unknown target marker set")?;
        if set.sample != self.analysis.binding.sample {
            return Err("Use BPM requires a target marker set for the same source sample".into());
        }
        let summary = UseBpmSummary { marker_set: target, sample: set.sample, candidate_index, bpm_tenths: selected.bpm_tenths, before_bpm: set.source_bpm, after_bpm: selected.bpm, warnings: vec![
            "Only this marker set's source BPM changes. Pulse phase is advisory and is NOT applied; source frame zero remains the compiler origin.".into(),
            "Exact analysis BPM is integer tenths; Use stores the existing source_bpm f64 approximation. Future compilation uses that stored binary clock, not this advisory rational pulse grid.".into(),
            "Future break compilation and groove capture use the reviewed BPM. Existing frozen grooves, baked cells, Song BPM, markers and locks stay unchanged.".into(),
            "This is an explicit choice among timing-evidence hypotheses, not verified tempo, meter or downbeat detection. This pure metadata plan does not verify files: cached-mode UI/MCP adapters must reauthorize and reverify source identity at Use; marker mode stays historical without source hashing or decoding (normal adapter path authorization may still apply).".into(),
        ] };
        set.source_bpm = Some(selected.bpm);
        let mut candidate = self.base.clone();
        candidate.upsert_marker_set(target, set)?;
        crate::project::check_serialized_size(&candidate)?;
        let id = hash(&(&self.id, &summary, &candidate))?;
        Ok(UseBpmPreview {
            id,
            base: self.base.clone(),
            candidate,
            summary,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UseBpmSummary {
    pub marker_set: MarkerSetId,
    pub sample: SampleId,
    pub candidate_index: usize,
    pub bpm_tenths: u16,
    pub before_bpm: Option<f64>,
    pub after_bpm: f64,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct UseBpmPreview {
    id: String,
    base: Song,
    candidate: Song,
    summary: UseBpmSummary,
}
impl UseBpmPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn summary(&self) -> &UseBpmSummary {
        &self.summary
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base {
            return Err("Tempo BPM preview is stale; analyze and review again".into());
        }
        Ok(self.candidate.clone())
    }
}
fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|b| crate::sample_asset::content_sha256(&b))
        .map_err(|e| e.to_string())
}
fn preflight(song: &Song, config: TempoConfig) -> Result<(), String> {
    if config.version != VERSION
        || !(200..=4000).contains(&config.min_bpm_tenths)
        || !(config.min_bpm_tenths..=4000).contains(&config.max_bpm_tenths)
    {
        return Err("Tempo version must be 1 and BPM range must be ordered within 200..=4000 tenths (20..=400 BPM)".into());
    }
    crate::project::check_serialized_size(song)?;
    crate::generation::check_work(song, false)
        .map_err(|e| format!("Tempo analysis work preflight: {e}"))?;
    song.validate()
}
fn window(asset: &SampleAsset, start: u64, end: u64) -> Result<(), String> {
    if !(8000..=384000).contains(&asset.sample_rate) {
        return Err("Tempo hypotheses support source rates 8000..=384000 only; lower-rate assets remain usable elsewhere".into());
    }
    if start >= end
        || end > asset.frames
        || end - start > u64::from(asset.sample_rate) * MAX_WINDOW_SECONDS
    {
        return Err(
            "Tempo window must be nonempty, inside the source and at most 300 seconds".into(),
        );
    }
    Ok(())
}
/// Pure declared-marker analysis. Stored identity is not file verification.
pub fn analyze_markers(
    song: &Song,
    request: MarkerRequest,
    config: TempoConfig,
) -> Result<TempoPreview, String> {
    preflight(song, config)?;
    let set = song
        .marker_sets
        .get(&request.marker_set)
        .ok_or("Unknown marker set")?;
    let a = usize::from(request.start_index);
    let b = usize::from(request.end_index);
    if a >= b || b >= set.markers.len() {
        return Err("Tempo marker window requires ordered start/end indices".into());
    }
    let asset = song
        .samples
        .get(&set.sample)
        .ok_or("Unknown marker source")?;
    let start = set.markers[a];
    let end = set.markers[b];
    window(asset, start, end)?;
    let points = set.markers[a + usize::from(!request.include_start)..b].to_vec();
    let binding = EvidenceBinding {
        sample: set.sample,
        source_sha256: asset.sha256.clone(),
        source_rate: asset.sample_rate,
        source_frames: asset.frames,
        window_start_frame: start,
        window_end_frame: end,
        kind: EvidenceKind::AcceptedMarkers {
            request,
            marker_set_sha256: hash(set)?,
        },
        identity_basis:
            "stored asset/accepted-marker metadata; no source file read or verification".into(),
        points_sha256: hash(&points)?,
    };
    analyze(song, config, binding, points)
}
/// Re-picks an opaque cache; no FFT, decode or file access here. Callers must
/// authorize and verify current bytes before reuse, and retain exact cache/config
/// epoch plus document revision through review. Cached-mode UI/MCP adapters must
/// reauthorize and verify again at Use before the pure metadata commit. Never pass
/// client-generated points.
pub fn analyze_cached(
    song: &Song,
    sample: SampleId,
    cache: &onset_service::CachedSource,
    onset_config: &onset_v2::Config,
    request: CachedRequest,
    config: TempoConfig,
) -> Result<TempoPreview, String> {
    preflight(song, config)?;
    let asset = song
        .samples
        .get(&sample)
        .ok_or("Unknown cached source sample")?;
    window(asset, request.start_frame, request.end_frame)?;
    if onset_config.max_suggestions > MAX_CACHED_POINTS {
        return Err(
            "Tempo cached repick requires max_suggestions <=4096; no silent truncation".into(),
        );
    }
    let result = onset_service::repick(cache, asset, onset_config)?;
    let all: Vec<u64> = match request.basis {
        PointBasis::Peak => result
            .analysis
            .detection
            .candidates
            .iter()
            .map(|p| p.peak_frame)
            .collect(),
        PointBasis::RefinedAttack => result
            .analysis
            .suggestions
            .iter()
            .map(|p| p.attack_frame)
            .collect(),
    };
    if all.len() > MAX_CACHED_POINTS {
        return Err("Tempo cached timing-point scan exceeds 4096".into());
    }
    let points: Vec<_> = all
        .into_iter()
        .filter(|&p| p >= request.start_frame && p < request.end_frame)
        .collect();
    let binding = EvidenceBinding { sample, source_sha256: asset.sha256.clone(), source_rate: asset.sample_rate, source_frames: asset.frames, window_start_frame: request.start_frame, window_end_frame: request.end_frame, kind: EvidenceKind::CachedV2 { request, extraction_key: result.binding.extraction_key, onset_config_sha256: result.config_sha256, onset_algorithm: result.binding.algorithm }, identity_basis: "immutable verified-decoding cache snapshot; caller must reauthorize and verify current file before reuse".into(), points_sha256: hash(&points)? };
    analyze(song, config, binding, points)
}
fn checked_work(n: usize) -> Result<(usize, u64), String> {
    if n > MAX_POINTS {
        return Err(
            "Tempo evidence exceeds 1024 points; select a smaller window (no truncation)".into(),
        );
    }
    let pairs = n
        .checked_mul(n.saturating_sub(1))
        .and_then(|v| v.checked_div(2))
        .ok_or("Tempo pair-count overflow")?;
    let work = (pairs as u64)
        .checked_mul(8)
        .and_then(|v| v.checked_add(MAX_SHORTLIST as u64 * 144 * n as u64))
        .ok_or("Tempo work-count overflow")?;
    if work > MAX_WORK {
        return Err("Tempo periodicity work exceeds 16000000 units".into());
    }
    Ok((pairs, work))
}
fn round_ratio(n: u128, d: u128) -> u128 {
    let q = n / d;
    q + u128::from(n % d >= d.div_ceil(2))
}
fn analyze(
    song: &Song,
    config: TempoConfig,
    mut binding: EvidenceBinding,
    mut points: Vec<u64>,
) -> Result<TempoPreview, String> {
    let (pairs, work) = checked_work(points.len())?;
    points.sort_unstable();
    if points.windows(2).any(|p| p[0] == p[1]) {
        return Err("Tempo evidence contains duplicate source frames; no implicit merging".into());
    }
    if points
        .iter()
        .any(|&p| p < binding.window_start_frame || p >= binding.window_end_frame)
    {
        return Err("Tempo evidence lies outside its half-open window".into());
    }
    binding.points_sha256 = hash(&points)?;
    let span = points.last().zip(points.first()).map_or(0, |(b, a)| b - a);
    let mut analysis = TempoAnalysis { algorithm: ALGORITHM.into(), config, binding, insufficient_reason: None, evidence_count: points.len(), span_frames: span, interval_pairs: pairs, estimated_work: work, local_maxima_count: 0, shortlist_truncated: false, shortlist: vec![], separated_candidate_count: 0, candidates: vec![], warnings: vec![
        "Periodicity hypotheses from declared timing points, with unit weights; not full raw-audio tempo analysis, calibrated confidence, meter or downbeat detection.".into(),
        "Accepted window end is excluded and accepted start is opt-in. Cached peaks/refined attacks are threshold-dependent; no user-dismissal filter, inferred attacks or amplitude weights are used.".into(),
        "BPM authority is integer tenths (20..400 BPM); phase is a rational source-frame offset relative to this window, modulo one pulse, NOT a downbeat. Predicted pulses are not accepted markers.".into(),
        "Search: beat spans 1..8 per point pair; smoothed vote local maxima, top64 by vote then lower BPM; 128 phase bins plus first16 distinct observed residues. All selected points are scored. Final candidates are separated by >=1 BPM and limited to8; shortlist aliases may be omitted from the returned ranking.".into(),
        "Weak/Moderate/Strong is a versioned timing-evidence heuristic; strong evidence can still support several beat units. Missing beats, syncopation and noise remain ambiguous.".into(),
        "Cached mode re-picks the existing bounded novelty/PCM cache; no new FFT or decode, but normalization, peak picking and refinement still cost work. Snapshot identity is not current-file verification.".into(),
        "Use BPM is explicit metadata-only. Phase is advisory; source frame zero remains the compiler origin. Future compilation/capture uses the chosen BPM, while existing frozen grooves and baked cells stay unchanged.".into(),
    ] };
    let mut pulses = vec![];
    if points.len() < 4 {
        analysis.insufficient_reason = Some("At least four distinct timing points (three adjacent intervals) are required; no fallback BPM".into());
    } else {
        let mut votes = vec![0.0f64; 4002];
        let rate600 = u64::from(analysis.binding.source_rate) * 600;
        for i in 0..points.len() {
            for j in i + 1..points.len() {
                let delta = u128::from(points[j] - points[i]);
                for beats in 1..=8u64 {
                    let bpm = round_ratio(u128::from(rate600) * u128::from(beats), delta);
                    if bpm >= u128::from(config.min_bpm_tenths)
                        && bpm <= u128::from(config.max_bpm_tenths)
                    {
                        votes[bpm as usize] += 1.0 / beats as f64;
                    }
                }
            }
        }
        let lo = usize::from(config.min_bpm_tenths);
        let hi = usize::from(config.max_bpm_tenths);
        let mut smoothed = vec![0.0; votes.len()];
        for b in lo..=hi {
            smoothed[b] = votes[b - 1] * 0.25 + votes[b] * 0.5 + votes[b + 1] * 0.25;
        }
        let mut maxima: Vec<_> = (lo..=hi)
            .filter(|&b| {
                smoothed[b] > 0.0
                    && smoothed[b] >= smoothed[b - 1]
                    && smoothed[b] >= smoothed[b + 1]
            })
            .collect();
        maxima.sort_by(|&a, &b| smoothed[b].total_cmp(&smoothed[a]).then(a.cmp(&b)));
        analysis.local_maxima_count = maxima.len();
        analysis.shortlist_truncated = maxima.len() > MAX_SHORTLIST;
        maxima.truncate(MAX_SHORTLIST);
        let best_vote = maxima.first().map_or(0.0, |&b| smoothed[b]);
        analysis.shortlist = maxima
            .iter()
            .map(|&b| ShortlistedTempo {
                bpm_tenths: b as u16,
                interval_vote: smoothed[b],
            })
            .collect();
        let mut fitted = vec![];
        for &b in &maxima {
            fitted.push(fit(
                &analysis.binding,
                &points,
                b as u16,
                smoothed[b],
                smoothed[b] / best_vote,
            )?);
        }
        fitted.sort_by(|a, b| {
            b.0.score
                .total_cmp(&a.0.score)
                .then(a.0.bpm_tenths.cmp(&b.0.bpm_tenths))
                .then(a.0.phase_numerator.cmp(&b.0.phase_numerator))
        });
        let mut separated: Vec<(TempoCandidate, Vec<u64>)> = vec![];
        for candidate in fitted {
            if separated
                .iter()
                .all(|old| old.0.bpm_tenths.abs_diff(candidate.0.bpm_tenths) >= 10)
            {
                separated.push(candidate);
            }
        }
        analysis.separated_candidate_count = separated.len();
        separated.truncate(MAX_CANDIDATES);
        let returned: BTreeSet<_> = separated.iter().map(|c| c.0.bpm_tenths).collect();
        for (mut candidate, frames) in separated {
            candidate.aliases = aliases(candidate.bpm_tenths, &analysis.shortlist, &returned);
            analysis.candidates.push(candidate);
            pulses.push(frames);
        }
        if analysis.candidates.is_empty() {
            analysis.insufficient_reason = Some(
                "No in-range periodicity votes for pair beat spans 1..8; no fallback BPM".into(),
            );
        }
    }
    let id = hash(&(song, &analysis))?;
    Ok(TempoPreview {
        id,
        base: song.clone(),
        analysis,
        points,
        pulses,
    })
}
fn aliases(bpm: u16, shortlist: &[ShortlistedTempo], returned: &BTreeSet<u16>) -> Vec<TempoAlias> {
    let mut aliases = vec![];
    for other in shortlist {
        if other.bpm_tenths == bpm {
            continue;
        }
        for (n, d) in [
            (1u8, 2u8),
            (2, 1),
            (1, 3),
            (3, 1),
            (1, 4),
            (4, 1),
            (2, 3),
            (3, 2),
        ] {
            let lhs = u64::from(other.bpm_tenths) * u64::from(d);
            let rhs = u64::from(bpm) * u64::from(n);
            if lhs.abs_diff(rhs) * 500 <= rhs {
                // <=0.2%, exact integer comparison
                aliases.push(TempoAlias {
                    bpm_tenths: other.bpm_tenths,
                    ratio_numerator: n,
                    ratio_denominator: d,
                    returned: returned.contains(&other.bpm_tenths),
                });
                break;
            }
        }
    }
    aliases.sort_by_key(|a| a.bpm_tenths);
    aliases
}
fn fit(
    binding: &EvidenceBinding,
    points: &[u64],
    bpm: u16,
    vote: f64,
    relative: f64,
) -> Result<(TempoCandidate, Vec<u64>), String> {
    let period = u64::from(binding.source_rate) * 600;
    let scaled_period = period * 128;
    let denominator = u32::from(bpm) * 128;
    let residues: Vec<_> = points
        .iter()
        .map(|&p| ((p - binding.window_start_frame) * u64::from(bpm) % period) * 128)
        .collect();
    let mut phases: BTreeSet<u64> = (0..128).map(|bin| bin * period).collect();
    let mut observed = BTreeSet::new();
    for &r in &residues {
        if observed.len() < 16 {
            observed.insert(r);
        }
    }
    phases.extend(observed);
    let mut best = (0u128, 0u64);
    for phase in phases {
        let fit_sum: u128 = residues
            .iter()
            .map(|&r| {
                let d = r.abs_diff(phase);
                let d = d.min(scaled_period - d);
                u128::from(scaled_period.saturating_sub(d * 8))
            })
            .sum();
        if fit_sum > best.0 {
            best = (fit_sum, phase);
        }
    }
    let phase = best.1;
    let mut frames = vec![];
    for k in 0..=MAX_PULSES {
        let numerator = u128::from(phase) + k as u128 * u128::from(scaled_period);
        let delta = round_ratio(numerator, u128::from(denominator));
        let absolute = u128::from(binding.window_start_frame) + delta;
        if absolute >= u128::from(binding.window_end_frame) {
            break;
        }
        if frames.len() == MAX_PULSES {
            return Err("Tempo predicted pulse grid exceeds 2048; choose a shorter window".into());
        }
        frames.push(u64::try_from(absolute).map_err(|_| "Tempo pulse frame overflow")?);
    }
    let mut occupied = BTreeSet::new();
    let mut outside = 0;
    let mut in_window_matches = 0;
    let mut errors = 0u128;
    for &p in points {
        let x = i128::from(p - binding.window_start_frame) * i128::from(denominator)
            - i128::from(phase);
        let d = i128::from(scaled_period);
        let floor = x.div_euclid(d);
        // Nearest pulse; exact half ties go to the higher pulse index, including negatives.
        let k = floor + i128::from(x.rem_euclid(d) * 2 >= d);
        let error = (x - k * d).unsigned_abs();
        errors += error;
        if error * 8 < u128::from(scaled_period) {
            if k >= 0 && (k as usize) < frames.len() {
                in_window_matches += 1;
                occupied.insert(k as usize);
            } else {
                outside += 1;
            }
        }
    }
    let fit = best.0 as f64 / (scaled_period as f64 * points.len() as f64);
    let coverage = if frames.is_empty() {
        0.0
    } else {
        occupied.len() as f64 / frames.len() as f64
    };
    let span = (points[points.len() - 1] - points[0]) as f64 * f64::from(bpm) / period as f64;
    let strength = if fit < 0.5 || occupied.len() < 3 || span < 2.0 {
        EvidenceStrength::Weak
    } else if points.len() >= 8
        && fit >= 0.8
        && coverage >= 0.5
        && occupied.len() >= 6
        && span >= 4.0
    {
        EvidenceStrength::Strong
    } else {
        EvidenceStrength::Moderate
    };
    let candidate = TempoCandidate {
        bpm_tenths: bpm,
        bpm: f64::from(bpm) / 10.0,
        phase_numerator: phase,
        phase_denominator: denominator,
        phase_frames: phase as f64 / f64::from(denominator),
        score: 0.60 * fit + 0.25 * coverage + 0.15 * relative,
        phase_fit: fit,
        interval_vote: vote,
        relative_interval_vote: relative,
        pulse_coverage: coverage,
        occupied_pulses: occupied.len(),
        pulse_count: frames.len(),
        matched_points: in_window_matches,
        outside_window_matches: outside,
        multiple_points_per_pulse: in_window_matches - occupied.len(),
        span_beats: span,
        mean_error_beats: errors as f64 / (scaled_period as f64 * points.len() as f64),
        strength,
        aliases: vec![],
    };
    Ok((candidate, frames))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> EvidenceBinding {
        EvidenceBinding {
            sample: SampleId(1),
            source_sha256: "a".repeat(64),
            source_rate: 48000,
            source_frames: 480000,
            window_start_frame: 0,
            window_end_frame: 480000,
            kind: EvidenceKind::AcceptedMarkers {
                request: MarkerRequest {
                    marker_set: MarkerSetId(1),
                    start_index: 0,
                    end_index: 1,
                    include_start: false,
                },
                marker_set_sha256: "a".repeat(64),
            },
            identity_basis: "test".into(),
            points_sha256: String::new(),
        }
    }
    #[test]
    fn point_work_limits_precede_pair_expansion_and_duplicates_reject() {
        assert_eq!(checked_work(1024).unwrap(), (523776, 13627392));
        assert!(checked_work(1025).is_err());
        assert!(checked_work(usize::MAX).is_err());
        let s = example_song();
        assert!(
            analyze(&s, TempoConfig::default(), binding(), vec![0; 1025])
                .unwrap_err()
                .contains("1024")
        );
        assert!(
            analyze(&s, TempoConfig::default(), binding(), vec![1, 2, 2, 3])
                .unwrap_err()
                .contains("duplicate")
        );
        let a = analyze(
            &s,
            TempoConfig::default(),
            binding(),
            vec![72017, 17, 48017, 24017],
        )
        .unwrap();
        let b = analyze(
            &s,
            TempoConfig::default(),
            binding(),
            vec![17, 24017, 48017, 72017],
        )
        .unwrap();
        assert_eq!(a.id(), b.id());
    }
    #[test]
    fn exact_phase_ties_choose_lower_phase_and_signed_edges_exclude_outside_support() {
        let b = binding();
        let (c, _) = fit(&b, &[100, 12100, 24100, 36100], 1200, 1.0, 1.0).unwrap();
        assert_eq!(c.phase_numerator, 100 * 128 * 1200); // Equal opposite phases: lower wins exactly.
        assert_eq!(c.phase_denominator, 128 * 1200);
        let mut b = b;
        b.window_end_frame = 72001;
        let (c, pulses) = fit(&b, &[1, 23999, 47999, 71999], 1200, 1.0, 1.0).unwrap();
        assert_eq!(c.outside_window_matches, 1);
        assert_eq!(c.matched_points, 3);
        assert_eq!(c.occupied_pulses, 3);
        assert_eq!(pulses, [23999, 47999, 71999]);
        assert_eq!(c.pulse_coverage, 1.0);
    }
    #[test]
    fn full_1024_point_budget_is_complete_and_not_silently_sampled() {
        let s = example_song();
        let mut b = binding();
        b.source_rate = 8000;
        b.window_end_frame = 2400000;
        b.source_frames = 2400000;
        let points: Vec<_> = (0..1024).map(|i| 17 + i * 1200).collect();
        let config = TempoConfig {
            version: 1,
            min_bpm_tenths: 4000,
            max_bpm_tenths: 4000,
        };
        let a = analyze(&s, config, b, points.clone()).unwrap();
        assert_eq!(a.points(), points);
        assert_eq!(a.analysis().evidence_count, 1024);
        let c = &a.analysis().candidates[0];
        assert_eq!(c.matched_points, 1024);
        assert_eq!(c.occupied_pulses, 1024);
        assert_eq!(c.phase_fit, 1.0);
    }
    #[test]
    fn half_up_rounding_and_maximum_window_have_bounded_complete_pulses() {
        assert_eq!(round_ratio(1, 2), 1);
        assert_eq!(round_ratio(3, 2), 2);
        assert_eq!(round_ratio(0, 2), 0);
        let mut b = binding();
        b.source_rate = 8000;
        b.window_end_frame = 2400000;
        b.source_frames = 2400000;
        let (c, p) = fit(&b, &[0, 1200, 2400, 3600], 4000, 1.0, 1.0).unwrap();
        assert_eq!(c.pulse_count, 2000);
        assert_eq!(p.last(), Some(&2398800));
    }
}
