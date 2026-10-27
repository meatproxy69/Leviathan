//! Bounded track balance on the existing Engine's original contributions.
//! No voice partitioning, event rewriting, source IO, automation or device IPC.
use crate::{
    song::*,
    song_render::{self, Command, Manifest},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "track-balance-original-contributions-v1";
pub const MAX_TRACKS: usize = 64;
pub const RAMP_FRAMES: u16 = 128;
pub const MAX_PATH_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackMix {
    pub gain: f32,
    pub balance: f32,
    pub mute: bool,
    pub solo: bool,
}
impl Default for TrackMix {
    fn default() -> Self {
        Self {
            gain: 1.0,
            balance: 0.0,
            mute: false,
            solo: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackMixSetting {
    pub track: TrackId,
    pub gain: f64,
    pub balance: f64,
    pub mute: bool,
    pub solo: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackMixConfig {
    pub version: u32,
    pub settings: Vec<TrackMixSetting>,
}
impl TrackMixConfig {
    pub fn from_song(song: &Song) -> Self {
        Self {
            version: VERSION,
            settings: song
                .track_mix
                .iter()
                .map(|(&track, mix)| TrackMixSetting {
                    track,
                    gain: f64::from(mix.gain),
                    balance: f64::from(mix.balance),
                    mute: mix.mute,
                    solo: mix.solo,
                })
                .collect(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MixCoefficients {
    pub left: f32,
    pub right: f32,
}
impl MixCoefficients {
    const UNITY: Self = Self {
        left: 1.0,
        right: 1.0,
    };
    fn equal(self, other: Self) -> bool {
        self.left.to_bits() == other.left.to_bits() && self.right.to_bits() == other.right.to_bits()
    }
    fn neutral(self) -> bool {
        self.equal(Self::UNITY)
    }
}
fn zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}
fn coefficient(mix: TrackMix, any_solo: bool) -> MixCoefficients {
    if mix.mute || (any_solo && !mix.solo) {
        return MixCoefficients {
            left: 0.0,
            right: 0.0,
        };
    }
    let g = f64::from(mix.gain);
    let b = f64::from(mix.balance);
    MixCoefficients {
        left: zero((g * (1.0 - b.max(0.0))) as f32),
        right: zero((g * (1.0 + b.min(0.0))) as f32),
    }
}
fn for_track(
    settings: &BTreeMap<TrackId, TrackMix>,
    track: TrackId,
    solo: bool,
) -> MixCoefficients {
    coefficient(settings.get(&track).copied().unwrap_or_default(), solo)
}
pub(crate) fn validate_settings(
    song: &Song,
    settings: &BTreeMap<TrackId, TrackMix>,
) -> Result<(), String> {
    if settings.len() > MAX_TRACKS {
        return Err("Track mix exceeds 64 configured tracks; no truncation".into());
    }
    for (&track, m) in settings {
        if !song.tracks.contains_key(&track) {
            return Err(format!("Track mix references unknown track {}", track.0));
        }
        validate_value(*m)?;
    }
    Ok(())
}
fn validate_value(m: TrackMix) -> Result<(), String> {
    if !m.gain.is_finite()
        || !(0.0..=1.0).contains(&m.gain)
        || !m.balance.is_finite()
        || !(-1.0..=1.0).contains(&m.balance)
    {
        return Err("Track mix requires finite gain 0..1 and stereo balance -1..1".into());
    }
    Ok(())
}
fn settings(config: &TrackMixConfig) -> Result<BTreeMap<TrackId, TrackMix>, String> {
    if config.version != VERSION || config.settings.len() > MAX_TRACKS {
        return Err("Track mix requires version 1 and at most 64 settings".into());
    }
    let mut result = BTreeMap::new();
    for s in &config.settings {
        if !s.gain.is_finite()
            || !(0.0..=1.0).contains(&s.gain)
            || !s.balance.is_finite()
            || !(-1.0..=1.0).contains(&s.balance)
        {
            return Err(
                "Track mix input must be finite gain 0..1 and balance -1..1 before f32 conversion"
                    .into(),
            );
        }
        if result
            .insert(
                s.track,
                TrackMix {
                    gain: s.gain as f32,
                    balance: s.balance as f32,
                    mute: s.mute,
                    solo: s.solo,
                },
            )
            .is_some()
        {
            return Err(
                "Duplicate track mix entry; no sorting/deduplication of duplicate authority".into(),
            );
        }
    }
    Ok(result)
}
fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|b| crate::sample_asset::content_sha256(&b))
        .map_err(|e| e.to_string())
}
pub fn check_document_work(song: &Song) -> Result<(), String> {
    crate::cell_blocks::check_document_work(song)?;
    let longest = song
        .samples
        .values()
        .map(|s| s.path.as_os_str().as_encoded_bytes().len())
        .chain(
            song.instruments
                .values()
                .filter_map(|i| i.sample.as_ref())
                .map(|p| p.as_os_str().as_encoded_bytes().len()),
        )
        .max()
        .unwrap_or(0);
    if longest > MAX_PATH_BYTES {
        return Err("Track mix declared source path exceeds 8192 bytes / 16 MiB at 2048 starts; shorten metadata before preview/live controls".into());
    }
    Ok(())
}
pub fn content_sha256(song: &Song) -> Result<String, String> {
    crate::project::check_serialized_size(song)?;
    let mut content = song.clone();
    content.track_mix.clear();
    hash(&content)
}
pub fn same_playing_content(left: &Song, right: &Song) -> Result<bool, String> {
    crate::project::check_serialized_size(left)?;
    crate::project::check_serialized_size(right)?;
    let mut left = left.clone();
    let mut right = right.clone();
    left.track_mix.clear();
    right.track_mix.clear();
    crate::project::same_serialized_song(&left, &right)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyMixGroup {
    pub instrument: InstrumentId,
    pub tracks: Vec<TrackId>,
}
#[derive(Debug, Clone)]
pub struct MixLayout {
    id: String,
    content_sha256: String,
    tracks: Vec<TrackId>,
    legacy_groups: Vec<LegacyMixGroup>,
    defined_tracks: BTreeSet<TrackId>,
    pub(crate) controlled_tracks: BTreeMap<usize, usize>,
    pub(crate) legacy_tracks: BTreeMap<InstrumentId, usize>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MixSnapshot {
    layout_id: String,
    coefficients: Vec<MixCoefficients>,
}
impl MixSnapshot {
    pub fn layout_id(&self) -> &str {
        &self.layout_id
    }
    pub fn coefficients(&self) -> &[MixCoefficients] {
        &self.coefficients
    }
    pub fn is_neutral(&self) -> bool {
        self.coefficients.iter().all(|c| c.neutral())
    }
}
impl MixLayout {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }
    pub fn tracks(&self) -> &[TrackId] {
        &self.tracks
    }
    pub fn legacy_groups(&self) -> &[LegacyMixGroup] {
        &self.legacy_groups
    }
    pub fn snapshot(&self, settings: &BTreeMap<TrackId, TrackMix>) -> Result<MixSnapshot, String> {
        if settings.len() > MAX_TRACKS {
            return Err("Track mix exceeds 64 configured tracks".into());
        }
        for (&track, &m) in settings {
            if !self.defined_tracks.contains(&track) {
                return Err("Track mix references a track outside this compiled snapshot".into());
            }
            validate_value(m)?;
        }
        let solo = settings.values().any(|m| m.solo);
        let coefficients: Vec<_> = self
            .tracks
            .iter()
            .map(|&t| for_track(settings, t, solo))
            .collect();
        for group in &self.legacy_groups {
            let first = for_track(settings, group.tracks[0], solo);
            if group
                .tracks
                .iter()
                .any(|&t| !for_track(settings, t, solo).equal(first))
            {
                return Err(format!("Legacy instrument {} is shared by tracks {:?}; their effective L/R coefficients must match. Explicitly import/assign registered regions for independent balance; no automatic conversion or sampler splitting", group.instrument.0, group.tracks.iter().map(|t|t.0).collect::<Vec<_>>()));
            }
        }
        Ok(MixSnapshot {
            layout_id: self.id.clone(),
            coefficients,
        })
    }
}
pub(crate) fn contributing_tracks(manifest: &Manifest) -> BTreeSet<TrackId> {
    manifest
        .events
        .iter()
        .filter_map(|e| matches!(e.command, Command::On { .. }).then_some(e.track))
        .collect()
}
pub(crate) fn nonneutral(song: &Song, manifest: &Manifest) -> bool {
    let solo = song.track_mix.values().any(|m| m.solo);
    manifest.events.iter().any(|e| {
        matches!(e.command, Command::On { .. })
            && !for_track(&song.track_mix, e.track, solo).neutral()
    })
}
pub(crate) fn build_layout(
    song: &Song,
    manifest: &Manifest,
    controlled: &BTreeSet<usize>,
) -> Result<MixLayout, String> {
    check_document_work(song)?;
    let tracks: Vec<_> = contributing_tracks(manifest).into_iter().collect();
    if tracks.len() > MAX_TRACKS {
        return Err("Track mix supports at most 64 compiled contributing tracks; neutral playback remains available".into());
    }
    let mut groups: BTreeMap<InstrumentId, BTreeSet<TrackId>> = BTreeMap::new();
    let mut controlled_tracks = BTreeMap::new();
    for e in &manifest.events {
        if let Command::On { voice, .. } = e.command {
            let slot = tracks
                .binary_search(&e.track)
                .map_err(|_| "Missing track mix owner")?;
            if controlled.contains(&voice) {
                controlled_tracks.insert(voice, slot);
            } else {
                groups.entry(e.instrument).or_default().insert(e.track);
            }
        }
    }
    let legacy_groups: Vec<_> = groups
        .into_iter()
        .map(|(instrument, tracks)| LegacyMixGroup {
            instrument,
            tracks: tracks.into_iter().collect(),
        })
        .collect();
    let legacy_tracks = legacy_groups
        .iter()
        .map(|g| (g.instrument, tracks.binary_search(&g.tracks[0]).unwrap()))
        .collect();
    let content_sha256 = content_sha256(song)?;
    let id = hash(&(
        ALGORITHM,
        &content_sha256,
        &tracks,
        &legacy_groups,
        &controlled_tracks,
    ))?;
    Ok(MixLayout {
        id,
        content_sha256,
        tracks,
        legacy_groups,
        defined_tracks: song.tracks.keys().copied().collect(),
        controlled_tracks,
        legacy_tracks,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MixLayoutStatus {
    Available {
        contributing_tracks: usize,
        legacy_groups: usize,
        live_ready: bool,
    },
    Unavailable {
        reason: String,
    },
    Unsupported {
        reason: String,
        contributing_tracks: usize,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackMixTrackSummary {
    pub track: TrackId,
    pub coefficients: MixCoefficients,
    pub shared_legacy_tracks: Vec<TrackId>,
    pub shared_legacy_instruments: Vec<InstrumentId>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackMixSummary {
    pub version: u32,
    pub algorithm: String,
    pub configured_tracks: usize,
    pub changed_tracks: usize,
    pub any_solo: bool,
    pub layout_status: MixLayoutStatus,
    pub tracks: Vec<TrackMixTrackSummary>,
    pub content_sha256: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct TrackMixPreview {
    id: String,
    base: Song,
    candidate: Song,
    config: TrackMixConfig,
    base_sha256: String,
    summary: TrackMixSummary,
    layout: Option<MixLayout>,
}
impl TrackMixPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &TrackMixConfig {
        &self.config
    }
    pub fn summary(&self) -> &TrackMixSummary {
        &self.summary
    }
    pub fn layout(&self) -> Option<&MixLayout> {
        self.layout.as_ref()
    }
    pub fn is_current(&self, current: &Song) -> Result<bool, String> {
        Ok(current == &self.base
            && crate::cell_blocks::document_sha256(current)? == self.base_sha256)
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if !self.is_current(current)? {
            return Err("Track mix preview is stale; review again".into());
        }
        Ok(self.candidate.clone())
    }
}
pub fn preview(song: &Song, config: TrackMixConfig) -> Result<TrackMixPreview, String> {
    let settings = settings(&config)?;
    validate_settings(song, &settings)?;
    check_document_work(song)?;
    song.validate()?;
    let changed_tracks = song
        .track_mix
        .keys()
        .chain(settings.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .iter()
        .filter(|t| {
            serde_json::to_vec(&song.track_mix.get(t)).ok()
                != serde_json::to_vec(&settings.get(t)).ok()
        })
        .count();
    let mut candidate = song.clone();
    candidate.track_mix = settings;
    crate::project::check_serialized_size(&candidate)?;
    candidate.validate()?;
    let mut layout = None;
    let status = match song_render::evaluate_model(&candidate) {
        Err(reason) => MixLayoutStatus::Unavailable { reason },
        Ok(manifest) => {
            let routes = song_render::control_specs(&manifest);
            let controlled = routes.keys().copied().collect();
            match build_layout(&candidate, &manifest, &controlled) {
                Err(reason) if nonneutral(&candidate, &manifest) => return Err(reason),
                Err(reason) => MixLayoutStatus::Unsupported {
                    reason,
                    contributing_tracks: contributing_tracks(&manifest).len(),
                },
                Ok(value) => {
                    value.snapshot(&candidate.track_mix)?;
                    let status = MixLayoutStatus::Available {
                        contributing_tracks: value.tracks.len(),
                        legacy_groups: value.legacy_groups.len(),
                        live_ready: !value.tracks.is_empty(),
                    };
                    layout = Some(value);
                    status
                }
            }
        }
    };
    let any_solo = candidate.track_mix.values().any(|m| m.solo);
    let tracks = layout
        .as_ref()
        .map(|l| {
            l.tracks
                .iter()
                .map(|&track| {
                    let groups: Vec<_> = l
                        .legacy_groups
                        .iter()
                        .filter(|g| g.tracks.len() > 1 && g.tracks.contains(&track))
                        .collect();
                    TrackMixTrackSummary {
                        track,
                        coefficients: for_track(&candidate.track_mix, track, any_solo),
                        shared_legacy_tracks: groups
                            .iter()
                            .flat_map(|g| g.tracks.iter().copied())
                            .filter(|&t| t != track)
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        shared_legacy_instruments: groups.iter().map(|g| g.instrument).collect(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let summary=TrackMixSummary{version:VERSION,algorithm:ALGORITHM.into(),configured_tracks:candidate.track_mix.len(),changed_tracks,any_solo,layout_status:status,tracks,content_sha256:content_sha256(&candidate)?,warnings:vec![
        "Track balance uses original Engine contributions and complete scheduling. Muted/solo-excluded voices still advance and may affect shared kit RR/choke policies; no events or sources are removed.".into(),
        "Stereo balance only, not equal-power pan or crossfeed. Solo on an unused/silent track can silence other tracks; mute wins. Stored f32 bits are preserved, runtime zero coefficients are canonical +0.".into(),
        "Legacy shared-instrument samplers keep their original slot/smoothing history; all owning tracks require equal effective L/R targets. No source conversion, sampler splitting or new resampler.".into(),
        "Saved settings and live delivery are separate. Native explicit Apply can submit a bounded coalesced full snapshot only to the same owner/content session; worker-enqueued revision is not heard output. Live 128-frame ramps are not persisted automation or reproduced by offline export.".into(),
        "Model-valid but unrenderable settings may be saved with explicit unavailable status. No source-byte verification, IO, meter, FX, stems, live MCP/GUI IPC or device/listening/latency guarantee.".into(),
    ]};
    let base_sha256 = crate::cell_blocks::document_sha256(song)?;
    let id = hash(&(ALGORITHM, &base_sha256, &config, hash(&candidate)?))?;
    Ok(TrackMixPreview {
        id,
        base: song.clone(),
        candidate,
        config,
        base_sha256,
        summary,
        layout,
    })
}
impl Song {
    /// Full-set transaction with exactly the same pure preview/routing policy.
    pub fn set_track_mix(&mut self, config: TrackMixConfig) -> Result<(), String> {
        let plan = preview(self, config)?;
        *self = plan.commit(self)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Ramp {
    current: MixCoefficients,
    start: MixCoefficients,
    target: MixCoefficients,
    elapsed: u16,
}
pub(crate) struct MixRuntime {
    pub layout: MixLayout,
    ramps: Vec<Ramp>,
    coefficients: Vec<Vec<MixCoefficients>>,
}
impl MixRuntime {
    pub fn new(layout: MixLayout, initial: MixSnapshot) -> Self {
        let ramps = initial
            .coefficients
            .iter()
            .map(|&c| Ramp {
                current: c,
                start: c,
                target: c,
                elapsed: RAMP_FRAMES,
            })
            .collect();
        let coefficients = vec![vec![MixCoefficients::UNITY; 512]; layout.tracks.len()];
        Self {
            layout,
            ramps,
            coefficients,
        }
    }
    pub fn apply(&mut self, snapshot: &MixSnapshot) -> Result<(), String> {
        if snapshot.layout_id != self.layout.id || snapshot.coefficients.len() != self.ramps.len() {
            return Err("Track mix snapshot belongs to another compiled layout".into());
        }
        if self.ramps.is_empty() {
            return Err("No contributing tracks; settings were not live-applied".into());
        }
        for (r, &target) in self.ramps.iter_mut().zip(&snapshot.coefficients) {
            // A full-map revision affecting another track must not extend this
            // ramp. Only a changed canonical target restarts from current PCM.
            if r.target.equal(target) {
                continue;
            }
            r.start = r.current;
            r.target = target;
            r.elapsed = 0;
        }
        Ok(())
    }
    pub fn prepare(&mut self, frames: usize) {
        for (r, values) in self.ramps.iter_mut().zip(&mut self.coefficients) {
            for value in &mut values[..frames] {
                if r.elapsed < RAMP_FRAMES {
                    r.elapsed += 1;
                    if r.elapsed == RAMP_FRAMES {
                        r.current = r.target;
                    } else {
                        let t = f64::from(r.elapsed) / f64::from(RAMP_FRAMES);
                        let lerp = |a: f32, b: f32| {
                            zero((f64::from(a) + (f64::from(b) - f64::from(a)) * t) as f32)
                        };
                        r.current = MixCoefficients {
                            left: lerp(r.start.left, r.target.left),
                            right: lerp(r.start.right, r.target.right),
                        };
                    }
                }
                *value = r.current;
            }
        }
    }
    pub fn add(&self, slot: usize, data: &mut [f32], source: &[f32], master: f32) {
        for (i, (dest, &value)) in data.iter_mut().zip(source).enumerate() {
            let c = self.coefficients[slot][i / 2];
            let gain = if i % 2 == 0 { c.left } else { c.right };
            // Preserve the exact old arithmetic/order whenever this coefficient is unity.
            *dest += if gain == 1.0 {
                value * master
            } else {
                (value * master) * gain
            };
        }
    }
    pub fn frame_coefficients(&self, slot: usize, frame: usize) -> MixCoefficients {
        self.coefficients[slot][frame]
    }
}
