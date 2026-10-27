//! Persistent musical content only: no playhead, voices, command memory or DSP state.
//! Public fields support draft editing; call `validate` before accepting a document.
//! Deserialization alone does not validate. Numeric IDs are caller-assigned, nonzero,
//! stable within a document, and never derived from display order or names.
mod arrangement;
mod multisamples;
pub(crate) use arrangement::FrameMutes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

macro_rules! id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}
id!(TrackId);
id!(InstrumentId);
id!(PatternId);
id!(PhraseId);
id!(SectionId);
id!(SampleId);
id!(RegionId);
id!(MarkerSetId);
id!(DrumKitId);
id!(PadId);
id!(PadLayerId);
id!(GrooveId);
id!(MultisampleId);
id!(KeyZoneId);

/// Number of empty tracks and instruments in a newly created song. Existing
/// projects and explicit expansion are not capped at this default capacity.
pub const NEW_SONG_CAPACITY: usize = 24;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub name: String,
    pub note_columns: u16,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    pub name: String,
    #[serde(default, skip_serializing_if = "InstrumentKind::is_pitched")]
    pub kind: InstrumentKind,
    /// Optional unresolved asset reference. Validation does not touch the filesystem.
    pub sample: Option<PathBuf>,
    /// Non-destructive source region; mutually exclusive with the legacy path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<RegionId>,
}
/// Additive instrument routing. Old instruments omit this field entirely.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum InstrumentKind {
    #[default]
    Pitched,
    DrumKit {
        kit: DrumKitId,
    },
    Multisample {
        map: MultisampleId,
    },
}
impl InstrumentKind {
    pub fn is_pitched(&self) -> bool {
        matches!(self, Self::Pitched)
    }
    /// Musical pitch semantics, distinct from legacy single-source routing.
    pub fn is_melodic(&self) -> bool {
        matches!(self, Self::Pitched | Self::Multisample { .. })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultisampleInstrument {
    pub name: String,
    /// Mutable shared mapping; stable zone IDs are local to this map.
    pub zones: BTreeMap<KeyZoneId, KeyZone>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyZone {
    pub name: String,
    pub note_min: u8,
    pub note_max: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    pub region: RegionId,
    /// MIDI note at native source pitch; overrides the region's root reference.
    pub root_note: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumKitInstrument {
    pub name: String,
    pub pads: BTreeMap<PadId, DrumPad>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PadMode {
    OneShot,
    Gated,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PadNewNote {
    Overlap,
    ReleasePrevious,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumPad {
    /// Visual bank position; stable PadId is independent of this slot.
    pub slot: u8,
    pub name: String,
    /// MIDI selector only: does not transpose the source.
    pub note: u8,
    pub region: Option<RegionId>,
    /// Empty retains the legacy single-region mapping. Nonempty zones choose
    /// exactly one region per trigger and are mutually exclusive with `region`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub velocity_zones: Vec<VelocityZone>,
    /// Native pitch reference for explicit glide; starts at original source rate.
    pub root_note: u8,
    #[serde(default, skip_serializing_if = "zero_tune")]
    pub tune_semitones: i8,
    pub gain: f32,
    pub pan: f32,
    pub mode: PadMode,
    pub new_note: PadNewNote,
    /// 1..=16; scoped to one instrument, not every user of this kit definition.
    pub choke_group: Option<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VelocityZone {
    /// Inclusive MIDI-style selection bins; does not quantize audible amplitude.
    pub min_velocity: u8,
    pub max_velocity: u8,
    /// Persisted vector order is the deterministic round-robin order.
    pub alternatives: Vec<PadLayer>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PadLayer {
    /// Nonzero, unique across ALL zones in one pad, not a global source ID.
    pub id: PadLayerId,
    pub region: RegionId,
}
impl DrumPad {
    pub fn has_source(&self) -> bool {
        self.region.is_some() || !self.velocity_zones.is_empty()
    }
    pub fn source_regions(&self) -> impl Iterator<Item = RegionId> + '_ {
        self.region.into_iter().chain(
            self.velocity_zones
                .iter()
                .flat_map(|zone| zone.alternatives.iter().map(|layer| layer.region)),
        )
    }
}
fn zero_tune(tune: &i8) -> bool {
    *tune == 0
}
/// Immutable encoded asset identity and its verified native decoding metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleAsset {
    pub path: PathBuf,
    pub sha256: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Real source frames; excludes the renderer's interpolation guard frame.
    pub frames: u64,
}
/// A non-destructive source view. Playback options are opt-in; None retains the
/// original one-shot and backend de-click behavior exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleRegion {
    pub name: String,
    pub sample: SampleId,
    /// Inclusive source frame.
    pub start_frame: u64,
    /// Exclusive source frame.
    pub end_frame: u64,
    /// This MIDI note plays at the source's original rate/pitch, regardless of BPM.
    pub root_note: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amp_envelope: Option<AmpEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_loop: Option<ForwardLoop>,
}
/// Linear amplitude ADSR in output-time milliseconds at the fixed 48 kHz engine
/// rate. This multiplies, rather than replaces, the existing voice gain/pan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmpEnvelope {
    pub attack_ms: u16,
    pub decay_ms: u16,
    pub sustain_percent: u8,
    pub release_ms: u16,
}
impl Default for AmpEnvelope {
    fn default() -> Self {
        Self {
            attack_ms: 0,
            decay_ms: 0,
            sustain_percent: 100,
            release_ms: 50,
        }
    }
}
/// Absolute source-frame [start,end) loop, at least two frames, inside its region.
/// The prefix plays once; the loop continues during release. No post-loop tail
/// or crossfade is implied, and discontinuous loop endpoints may click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardLoop {
    pub start_frame: u64,
    pub end_frame: u64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Note {
    #[default]
    Empty,
    /// MIDI pitch 0..=127; independent of library octave display conventions.
    Note(u8),
    Off,
}
/// Row-local commands. These are Leviathan units, not Renoise effect codes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepEffects {
    pub retrigger: Option<Retrigger>,
    /// Absolute fraction of this row; initiates the voice's release (legacy
    /// de-click or opted-in region ADSR). A region release of 0 ms is abrupt.
    /// May not precede the delayed note/retriggers; distinct from the Cut command.
    pub release: Option<f64>,
    pub glide: Option<Glide>,
    /// Absolute row fraction. Abruptly stops the active voice; may click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<f64>,
    /// Zero-based source sample frame, not output frames or a normalized fraction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
}
impl StepEffects {
    pub fn is_empty(&self) -> bool {
        self.retrigger.is_none()
            && self.release.is_none()
            && self.glide.is_none()
            && self.cut.is_none()
            && self.offset.is_none()
            && self.direction.is_none()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Forward,
    Reverse,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retrigger {
    /// Additional starts after the original pitched note, in 1..=16.
    pub repeats: u8,
    /// Spacing in row fractions. All starts must remain inside this row.
    pub spacing: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glide {
    /// Absolute MIDI target, independent of the original note's pitch.
    pub target: u8,
    /// Backend pitch-change rate, not seconds-to-target or a row fraction.
    pub semitones_per_second: f32,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub note: Note,
    pub instrument: Option<InstrumentId>,
    /// None means unspecified, distinct from explicit silence (0).
    pub velocity: Option<f32>,
    /// -1 is left, 0 center, +1 right.
    pub pan: Option<f32>,
    /// Fraction of a row, 0 inclusive to 1 exclusive. Not ticks or event duration.
    pub delay: Option<f64>,
    /// Missing in older schema-1 projects; empty effects do not change their JSON.
    #[serde(default, skip_serializing_if = "StepEffects::is_empty")]
    pub effects: StepEffects,
    /// Explicit document phrase trigger, distinct from pattrns::Phrase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase: Option<PhraseTrigger>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhraseTrigger {
    pub phrase: PhraseId,
    /// Total finite cycles: 1 is one-shot; 2..=64 is bounded repetition.
    pub cycles: u16,
}
/// An instrument-associated document phrase, not a pattrns generation object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    pub name: String,
    pub instrument: InstrumentId,
    pub rows: u32,
    pub lines_per_beat: u16,
    pub note_columns: u16,
    pub root_note: u8,
    /// Same scalar note/effect fields, but nested phrase triggers are forbidden.
    pub cells: BTreeMap<u32, BTreeMap<u16, Cell>>,
}
/// Accepted editable boundaries. Detector suggestions never mutate this record.
/// Musical beat zero is source frame zero; tempo is explicit, not inferred.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerSet {
    pub name: String,
    pub sample: SampleId,
    pub source_bpm: Option<f64>,
    /// Strictly increasing source-frame boundaries, including 0 and source end.
    pub markers: Vec<u64>,
    /// Protected internal boundaries, strictly increasing and present in markers.
    /// Source start/end are always fixed and are not represented as user locks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_frames: Vec<u64>,
}
/// Sparse rows and columns. Missing cells are empty; row and column indices are zero-based.
pub type TrackPattern = BTreeMap<u32, BTreeMap<u16, Cell>>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    /// Historical generation recipe; not live playback or reference authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<crate::generation::GenerationProvenance>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_aliases: BTreeMap<TrackId, TrackAlias>,
    pub name: String,
    pub rows: u32,
    pub tracks: BTreeMap<TrackId, TrackPattern>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackAlias {
    pub pattern: PatternId,
    pub track: TrackId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub id: SectionId,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub muted_tracks: BTreeSet<TrackId>,
    pub pattern: PatternId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<Section>,
}
impl Occurrence {
    pub fn new(pattern: PatternId) -> Self {
        Self {
            muted_tracks: BTreeSet::new(),
            pattern,
            section: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Song {
    pub schema: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generation_recipes: BTreeMap<String, crate::generation::Recipe>,
    pub title: String,
    pub bpm: f64,
    pub lines_per_beat: u16,
    pub tracks: BTreeMap<TrackId, Track>,
    pub track_order: Vec<TrackId>,
    pub instruments: BTreeMap<InstrumentId, Instrument>,
    pub patterns: BTreeMap<PatternId, Pattern>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub phrases: BTreeMap<PhraseId, Phrase>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub samples: BTreeMap<SampleId, SampleAsset>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<RegionId, SampleRegion>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub marker_sets: BTreeMap<MarkerSetId, MarkerSet>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub drum_kits: BTreeMap<DrumKitId, DrumKitInstrument>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub grooves: BTreeMap<GrooveId, crate::groove::GrooveTemplate>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub multisamples: BTreeMap<MultisampleId, MultisampleInstrument>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub region_catalog: BTreeMap<RegionId, crate::slice_features::RegionCatalogEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_mix: BTreeMap<TrackId, crate::track_mix::TrackMix>,
    pub order: Vec<Occurrence>,
}
impl Song {
    /// Expand a song to the standard 24-track and 24-instrument picker capacity.
    pub fn ensure_24_slots(&mut self) {
        while self.track_order.len() < NEW_SONG_CAPACITY {
            let id = TrackId(self.tracks.keys().next_back().map_or(0, |id| id.0) + 1);
            self.tracks.insert(
                id,
                Track {
                    name: format!("Track {}", id.0),
                    note_columns: 1,
                },
            );
            self.track_order.push(id);
        }
        while self.instruments.len() < NEW_SONG_CAPACITY {
            let id = InstrumentId(self.instruments.keys().next_back().map_or(0, |id| id.0) + 1);
            self.instruments.insert(
                id,
                Instrument {
                    name: format!("Instrument {}", id.0),
                    kind: InstrumentKind::Pitched,
                    sample: None,
                    region: None,
                },
            );
        }
    }
    /// Return the half-open order range bounded by the nearest section markers.
    pub fn section_range(&self, position: usize) -> Result<std::ops::Range<usize>, String> {
        if position >= self.order.len() {
            return Err("Unknown order position".into());
        }
        let start = (0..=position)
            .rev()
            .find(|i| self.order[*i].section.is_some())
            .unwrap_or(0);
        let end = (position + 1..self.order.len())
            .find(|i| self.order[*i].section.is_some())
            .unwrap_or(self.order.len());
        Ok(start..end)
    }
    /// Add one empty track without changing any existing pattern content.
    /// Stable IDs use max+1 and never reuse removed IDs.
    pub fn add_track(&mut self, name: String) -> Result<TrackId, String> {
        let id = TrackId(
            self.tracks
                .keys()
                .next_back()
                .map_or(0, |id| id.0)
                .checked_add(1)
                .ok_or("track IDs exhausted")?,
        );
        let mut candidate = self.clone();
        candidate.tracks.insert(
            id,
            Track {
                name,
                note_columns: 2,
            },
        );
        candidate.track_order.push(id);
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    /// Add one unassigned pitched instrument on a fresh stable ID.
    pub fn add_instrument(&mut self, name: String) -> Result<InstrumentId, String> {
        let id = InstrumentId(
            self.instruments
                .keys()
                .next_back()
                .map_or(0, |id| id.0)
                .checked_add(1)
                .ok_or("instrument IDs exhausted")?,
        );
        let mut candidate = self.clone();
        candidate.instruments.insert(
            id,
            Instrument {
                name,
                kind: InstrumentKind::Pitched,
                sample: None,
                region: None,
            },
        );
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    /// Validate content and every reference, including unused patterns.
    /// Current product limits: BPM 20..=400, LPB 1..=256, rows 1..=65536,
    /// columns 1..=256. These are not claims of Renoise compatibility.
    pub fn validate(&self) -> Result<(), String> {
        crate::slice_features::validate_catalog(self)?;
        crate::track_mix::validate_settings(self, &self.track_mix)?;
        if self.schema != 1 {
            return Err("unsupported song schema".into());
        }
        if self.generation_recipes.len() > 32 {
            return Err("At most 32 generation recipes".into());
        }
        for (name, recipe) in &self.generation_recipes {
            if name.trim().is_empty() || name != name.trim() || name.chars().count() > 64 {
                return Err("Invalid generation recipe name".into());
            }
            recipe.validate()?;
            if !self.instruments.contains_key(&recipe.instrument) {
                return Err("Unknown generation recipe instrument".into());
            }
        }
        if !self.bpm.is_finite() || !(20.0..=400.0).contains(&self.bpm) {
            return Err("BPM must be finite and in 20..=400".into());
        }
        if !(1..=256).contains(&self.lines_per_beat) {
            return Err("LPB must be 1..=256".into());
        }
        let ordered: BTreeSet<_> = self.track_order.iter().copied().collect();
        if ordered.len() != self.track_order.len()
            || ordered != self.tracks.keys().copied().collect()
        {
            return Err("track order must contain every track exactly once".into());
        }
        for (id, track) in &self.tracks {
            if id.0 == 0 || !(1..=256).contains(&track.note_columns) {
                return Err(format!("invalid track {id:?}"));
            }
        }
        if self.instruments.keys().any(|id| id.0 == 0) {
            return Err("zero instrument ID".into());
        }
        if self.samples.len() > 128 || self.regions.len() > 4096 {
            return Err("song exceeds 128 sources or 4096 regions".into());
        }
        for (id, sample) in &self.samples {
            if id.0 == 0
                || sample.path.as_os_str().is_empty()
                || sample.sha256.len() != 64
                || !sample
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                || !(1..=384000).contains(&sample.sample_rate)
                || !(1..=2).contains(&sample.channels)
                || sample.frames == 0
                || sample
                    .frames
                    .saturating_add(1)
                    .saturating_mul(u64::from(sample.channels))
                    > 16 * 1024 * 1024
            {
                return Err(
                    "invalid sample ID, path, SHA-256, rate, channels or decoded frame budget"
                        .into(),
                );
            }
        }
        if self.marker_sets.len() > 128 {
            return Err("song exceeds 128 marker sets".into());
        }
        for (id, set) in &self.marker_sets {
            let source = self
                .samples
                .get(&set.sample)
                .ok_or("marker set references unknown source")?;
            if id.0 == 0
                || !(2..=257).contains(&set.markers.len())
                || set.markers.first() != Some(&0)
                || set.markers.last() != Some(&source.frames)
                || set.markers.windows(2).any(|w| w[0] >= w[1])
            {
                return Err("marker sets require nonzero IDs and 2..257 increasing source-frame boundaries including 0 and source end".into());
            }
            if set
                .source_bpm
                .is_some_and(|bpm| !bpm.is_finite() || !(20.0..=400.0).contains(&bpm))
            {
                return Err("explicit source BPM must be finite and in 20..=400".into());
            }
            if set.locked_frames.windows(2).any(|w| w[0] >= w[1])
                || set.locked_frames.iter().any(|frame| {
                    *frame == 0
                        || *frame == source.frames
                        || set.markers.binary_search(frame).is_err()
                })
            {
                return Err("marker locks must be sorted unique internal boundaries; source endpoints are always fixed".into());
            }
        }
        for (id, region) in &self.regions {
            let sample = self
                .samples
                .get(&region.sample)
                .ok_or("region references unknown source")?;
            if id.0 == 0
                || region.start_frame >= region.end_frame
                || region.end_frame > sample.frames
                || region.root_note > 127
            {
                return Err("invalid region ID, source-frame bounds or root note".into());
            }
            if region.amp_envelope.is_some_and(|e| {
                e.attack_ms > 10000
                    || e.decay_ms > 10000
                    || e.sustain_percent > 100
                    || e.release_ms > 500
            }) {
                return Err("region amp envelope requires attack/decay 0..10000 ms, sustain 0..100 percent, release 0..500 ms".into());
            }
            if region.forward_loop.is_some_and(|l| {
                l.start_frame < region.start_frame
                    || l.end_frame > region.end_frame
                    || l.end_frame.checked_sub(l.start_frame).is_none_or(|n| n < 2)
            }) {
                return Err("region forward loop requires at least two source frames inside region [start,end)".into());
            }
        }
        if self.grooves.len() > crate::groove::MAX_TEMPLATES {
            return Err("Song exceeds 64 groove templates".into());
        }
        for (id, template) in &self.grooves {
            if id.0 == 0 {
                return Err("Groove ID must be nonzero".into());
            }
            template.validate()?;
        }
        if self.drum_kits.len() > 128 {
            return Err("song exceeds 128 drum kits".into());
        }
        for (id, kit) in &self.drum_kits {
            if id.0 == 0
                || kit.name.trim().is_empty()
                || kit.name.len() > 256
                || kit.pads.len() > 16
            {
                return Err("invalid drum kit ID/name or more than 16 pads".into());
            }
            let mut slots = BTreeSet::new();
            let mut notes = BTreeSet::new();
            for (pad_id, pad) in &kit.pads {
                self.validate_pad_sources(pad.region, &pad.velocity_zones)?;
                if pad_id.0 == 0
                    || pad.slot > 15
                    || !slots.insert(pad.slot)
                    || pad.note > 127
                    || !notes.insert(pad.note)
                    || pad.root_note > 127
                    || !(-48..=48).contains(&pad.tune_semitones)
                    || !(0..=127)
                        .contains(&(i16::from(pad.root_note) + i16::from(pad.tune_semitones)))
                    || pad.name.trim().is_empty()
                    || pad.name.len() > 128
                    || !pad.gain.is_finite()
                    || !(0.0..=1.0).contains(&pad.gain)
                    || !pad.pan.is_finite()
                    || !(-1.0..=1.0).contains(&pad.pan)
                    || pad.choke_group.is_some_and(|g| !(1..=16).contains(&g))
                    || pad.region.is_some_and(|id| !self.regions.contains_key(&id))
                {
                    return Err("invalid drum pad ID/slot/name/note/root/gain/pan/group or region; slots and selector notes must be unique".into());
                }
            }
        }
        self.validate_multisamples()?;
        for instrument in self.instruments.values() {
            if let InstrumentKind::DrumKit { kit } = instrument.kind {
                if !self.drum_kits.contains_key(&kit)
                    || instrument.sample.is_some()
                    || instrument.region.is_some()
                {
                    return Err("drum kit instrument references unknown kit or conflicts with sample/region".into());
                }
            }
            if let Some(region) = instrument.region {
                if instrument.sample.is_some() || !self.regions.contains_key(&region) {
                    return Err(
                        "instrument region is unknown or conflicts with a legacy sample path"
                            .into(),
                    );
                }
            }
        }
        if self.phrases.len() > 128 {
            return Err("song exceeds 128 phrase definitions".into());
        }
        let mut phrase_cells = 0usize;
        let mut pitch_ranges = BTreeMap::new();
        for (id, phrase) in &self.phrases {
            if id.0 == 0
                || !self.instruments.contains_key(&phrase.instrument)
                || !(1..=1024).contains(&phrase.rows)
                || !(1..=256).contains(&phrase.lines_per_beat)
                || !(1..=16).contains(&phrase.note_columns)
                || phrase.root_note > 127
            {
                return Err(
                    "invalid phrase ID, instrument, rows, LPB, columns or root note".into(),
                );
            }
            let mut range: Option<(i16, i16)> = None;
            for (row, columns) in &phrase.cells {
                if *row >= phrase.rows {
                    return Err("row outside phrase".into());
                }
                phrase_cells = phrase_cells.saturating_add(columns.len());
                if phrase_cells > 32768 {
                    return Err("song exceeds 32768 phrase-definition cells".into());
                }
                for (column, cell) in columns {
                    if *column >= phrase.note_columns {
                        return Err("column outside phrase".into());
                    }
                    if cell.phrase.is_some() {
                        return Err("nested phrase triggers are not supported".into());
                    }
                    if cell
                        .instrument
                        .is_some_and(|instrument| instrument != phrase.instrument)
                    {
                        return Err(
                            "phrase cell instrument must match its associated instrument".into(),
                        );
                    }
                    self.validate_cell(cell)?;
                    let mut include = |pitch: u8| {
                        let pitch = i16::from(pitch);
                        range = Some(
                            range.map_or((pitch, pitch), |(lo, hi)| (lo.min(pitch), hi.max(pitch))),
                        );
                    };
                    if let Note::Note(note) = cell.note {
                        include(note);
                    }
                    if let Some(glide) = &cell.effects.glide {
                        include(glide.target);
                    }
                }
            }
            pitch_ranges.insert(*id, range);
        }
        for (id, pattern) in &self.patterns {
            if let Some(provenance) = &pattern.generation {
                provenance.validate()?;
            }
            if id.0 == 0 || !(1..=65536).contains(&pattern.rows) {
                return Err(format!("invalid pattern {id:?}"));
            }
            for (track_id, rows) in &pattern.tracks {
                let track = self
                    .tracks
                    .get(track_id)
                    .ok_or_else(|| format!("unknown track {track_id:?}"))?;
                for (row, columns) in rows {
                    if *row >= pattern.rows {
                        return Err(format!("row {row} outside pattern {id:?}"));
                    }
                    for (column, cell) in columns {
                        if *column >= track.note_columns {
                            return Err(format!("column {column} outside track {track_id:?}"));
                        }
                        self.validate_cell(cell)?;
                        if let Some(trigger) = &cell.phrase {
                            let phrase = &self.phrases[&trigger.phrase];
                            let Note::Note(note) = cell.note else {
                                unreachable!()
                            };
                            let transpose = i16::from(note) - i16::from(phrase.root_note);
                            if transpose != 0
                                && !self.instruments[&phrase.instrument].kind.is_melodic()
                            {
                                return Err("drum kit phrases require their neutral root trigger note; selector transposition is not supported".into());
                            }
                            if let Some((minimum, maximum)) = pitch_ranges[&trigger.phrase] {
                                if minimum + transpose < 0 || maximum + transpose > 127 {
                                    return Err("phrase transposition moves a note or glide target outside MIDI 0..127".into());
                                }
                            }
                        }
                    }
                }
            }
        }
        if self
            .order
            .iter()
            .any(|o| !self.patterns.contains_key(&o.pattern))
        {
            return Err("unknown ordered pattern".into());
        }
        self.validate_arrangement()?;
        self.validate_drum_references()?;
        Ok(())
    }
    /// Conservative incoming selection analysis also protects unused patterns.
    /// Ambiguous omitted instruments can be disambiguated with an explicit cell.
    fn validate_drum_references(&self) -> Result<(), String> {
        if self
            .instruments
            .values()
            .all(|i| !matches!(i.kind, InstrumentKind::DrumKit { .. }))
        {
            return Ok(());
        }
        let check = |instrument: InstrumentId, note: u8| -> Result<(), String> {
            if let InstrumentKind::DrumKit { kit } = self.instruments[&instrument].kind {
                let pad = self.drum_kits[&kit]
                    .pads
                    .values()
                    .find(|pad| pad.note == note);
                if pad.is_none_or(|pad| !pad.has_source()) {
                    return Err(format!("Drum kit instrument {} selector note {note} is absent/unmapped (including possible incoming selection in unused patterns); assign a pad or explicit different instrument", instrument.0));
                }
            }
            Ok(())
        };
        for phrase in self.phrases.values() {
            for cell in phrase.cells.values().flat_map(|row| row.values()) {
                if let Note::Note(note) = cell.note {
                    check(phrase.instrument, note)?;
                }
            }
        }
        // Borrow resolved content: aliases keep target track/column state, not
        // their source's runtime identity. All chains were already validated.
        let mut resolved = BTreeMap::new();
        for (&id, pattern) in &self.patterns {
            let mut tracks = BTreeMap::new();
            for track in pattern.tracks.keys().chain(pattern.track_aliases.keys()) {
                if let Some(rows) = self.resolved_track(id, *track)? {
                    tracks.insert(*track, rows);
                }
            }
            resolved.insert(id, tracks);
        }
        let mut incoming: BTreeMap<(TrackId, u16), BTreeSet<InstrumentId>> = BTreeMap::new();
        for tracks in resolved.values() {
            for (&track, rows) in tracks {
                let mut local_selection = BTreeMap::new();
                for columns in rows.values() {
                    for (&column, cell) in columns {
                        let selected = cell
                            .phrase
                            .as_ref()
                            .map(|p| self.phrases[&p.phrase].instrument)
                            .or(cell.instrument);
                        if let Some(instrument) = selected {
                            local_selection.insert(column, instrument);
                            incoming
                                .entry((track, column))
                                .or_default()
                                .insert(instrument);
                        }
                        // Definite local references remain protected even when
                        // playback is muted. This selection is not runtime memory.
                        if cell.phrase.is_none() {
                            if let (Some(&instrument), Note::Note(note)) =
                                (local_selection.get(&column), cell.note)
                            {
                                check(instrument, note)?;
                            }
                        }
                    }
                }
            }
        }
        let frame_mutes = FrameMutes::new(self, crate::RATE);
        let per_row = f64::from(crate::RATE) * 60.0 / (self.bpm * f64::from(self.lines_per_beat));
        let end = (self
            .order
            .iter()
            .map(|o| u64::from(self.patterns[&o.pattern].rows))
            .sum::<u64>() as f64
            * per_row)
            .round() as u64;
        let check_pattern = |pattern: PatternId,
                             muted: &BTreeSet<TrackId>,
                             absolute_row: Option<u64>,
                             selected: &mut BTreeMap<(TrackId, u16), BTreeSet<InstrumentId>>,
                             conservative: bool|
         -> Result<(), String> {
            for (&track, rows) in &resolved[&pattern] {
                if muted.contains(&track) {
                    continue;
                }
                for (&row, columns) in *rows {
                    for (&column, cell) in columns {
                        if let Some(origin) = absolute_row {
                            let frame = (((origin + u64::from(row)) as f64
                                + cell.delay.unwrap_or(0.0))
                                * per_row)
                                .round() as u64;
                            if frame_mutes.contains(track, frame.min(end.saturating_sub(1))) {
                                continue;
                            }
                        }
                        let choices = selected.entry((track, column)).or_insert_with(|| {
                            if conservative {
                                incoming.get(&(track, column)).cloned().unwrap_or_default()
                            } else {
                                BTreeSet::new()
                            }
                        });
                        if let Some(trigger) = &cell.phrase {
                            *choices = BTreeSet::from([self.phrases[&trigger.phrase].instrument]);
                            continue;
                        }
                        if let Some(instrument) = cell.instrument {
                            *choices = BTreeSet::from([instrument]);
                        }
                        if let Note::Note(note) = cell.note {
                            for &instrument in choices.iter() {
                                check(instrument, note)?;
                            }
                        }
                    }
                }
            }
            Ok(())
        };
        // Ordered content has exact incoming memory. An unrelated unused kit
        // pattern must not reinterpret an actually selected pitched instrument.
        let kit_patterns: BTreeSet<_> = self
            .patterns
            .keys()
            .filter_map(|&id| {
                resolved[&id]
                    .values()
                    .flat_map(|rows| rows.values())
                    .flat_map(|columns| columns.values())
                    .any(|cell| {
                        let instrument = cell
                            .phrase
                            .as_ref()
                            .map(|p| self.phrases[&p.phrase].instrument)
                            .or(cell.instrument);
                        instrument.is_some_and(|id| {
                            matches!(self.instruments[&id].kind, InstrumentKind::DrumKit { .. })
                        })
                    })
                    .then_some(id)
            })
            .collect();
        let ordered_has_kit = self.order.iter().any(|o| kit_patterns.contains(&o.pattern));
        if ordered_has_kit {
            let mut selected = BTreeMap::new();
            let mut expanded_cells = 0usize;
            let mut origin = 0u64;
            for occurrence in &self.order {
                expanded_cells = expanded_cells.saturating_add(
                    resolved[&occurrence.pattern]
                        .iter()
                        .filter(|(track, _)| !occurrence.muted_tracks.contains(track))
                        .map(|(_, rows)| rows)
                        .flat_map(|rows| rows.values())
                        .map(BTreeMap::len)
                        .sum::<usize>(),
                );
                if expanded_cells > 65536 {
                    return Err("Drum mapping validation exceeds 65536 ordered cells; reduce arrangement repeats".into());
                }
                check_pattern(
                    occurrence.pattern,
                    &occurrence.muted_tracks,
                    Some(origin),
                    &mut selected,
                    false,
                )?;
                origin += u64::from(self.patterns[&occurrence.pattern].rows);
            }
        }
        let used: BTreeSet<_> = self.order.iter().map(|o| o.pattern).collect();
        for id in self.patterns.keys() {
            if !used.contains(id) {
                check_pattern(*id, &BTreeSet::new(), None, &mut BTreeMap::new(), true)?;
            }
        }
        Ok(())
    }
    /// Create an empty kit on a fresh instrument without rebinding existing notes.
    /// IDs are checked max+1 allocations; exhausted namespaces never reuse holes.
    pub fn create_drum_kit_instrument(
        &mut self,
        name: String,
    ) -> Result<(InstrumentId, DrumKitId), String> {
        let instrument = InstrumentId(
            self.instruments
                .keys()
                .next_back()
                .map_or(0, |id| id.0)
                .checked_add(1)
                .ok_or("instrument IDs exhausted")?,
        );
        let kit = DrumKitId(
            self.drum_kits
                .keys()
                .next_back()
                .map_or(0, |id| id.0)
                .checked_add(1)
                .ok_or("drum kit IDs exhausted")?,
        );
        let mut candidate = self.clone();
        candidate.drum_kits.insert(
            kit,
            DrumKitInstrument {
                name: name.clone(),
                pads: BTreeMap::new(),
            },
        );
        candidate.instruments.insert(
            instrument,
            Instrument {
                name,
                kind: InstrumentKind::DrumKit { kit },
                sample: None,
                region: None,
            },
        );
        candidate.validate()?;
        *self = candidate;
        Ok((instrument, kit))
    }
    pub fn upsert_drum_kit(&mut self, id: DrumKitId, kit: DrumKitInstrument) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.drum_kits.insert(id, kit);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_drum_kit(&mut self, id: DrumKitId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.drum_kits.remove(&id).ok_or("unknown drum kit")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn upsert_drum_pad(
        &mut self,
        kit: DrumKitId,
        id: PadId,
        pad: DrumPad,
    ) -> Result<(), String> {
        self.validate_pad_sources(pad.region, &pad.velocity_zones)?;
        let mut candidate = self.clone();
        candidate
            .drum_kits
            .get_mut(&kit)
            .ok_or("unknown drum kit")?
            .pads
            .insert(id, pad);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    fn validate_pad_sources(
        &self,
        region: Option<RegionId>,
        zones: &[VelocityZone],
    ) -> Result<(), String> {
        // Bound input before flattening, cloning the document or source work.
        if zones.len() > 8
            || zones
                .iter()
                .any(|z| z.alternatives.is_empty() || z.alternatives.len() > 8)
            || zones.iter().map(|z| z.alternatives.len()).sum::<usize>() > 32
        {
            return Err("Pad sources allow at most 8 velocity zones, 1..8 alternatives per zone and 32 alternatives total".into());
        }
        if region.is_some() && !zones.is_empty() {
            return Err("Pad single region and velocity zones are mutually exclusive".into());
        }
        if region.is_some_and(|id| !self.regions.contains_key(&id)) {
            return Err("Pad references unknown region".into());
        }
        let mut next = 0u16;
        let mut ids = BTreeSet::new();
        for zone in zones {
            if u16::from(zone.min_velocity) != next
                || zone.max_velocity < zone.min_velocity
                || zone.max_velocity > 127
            {
                return Err("Velocity zones must be sorted, gapless and non-overlapping inclusive bins covering 0..127".into());
            }
            next = u16::from(zone.max_velocity) + 1;
            for layer in &zone.alternatives {
                if layer.id.0 == 0
                    || !ids.insert(layer.id)
                    || !self.regions.contains_key(&layer.region)
                {
                    return Err("Pad layer IDs must be nonzero and unique across all zones in the pad, with existing region references".into());
                }
            }
        }
        if !zones.is_empty() && next != 128 {
            return Err("Velocity zones must cover all inclusive bins 0..127".into());
        }
        Ok(())
    }
    /// Explicitly replace the pad's entire source assignment. None + [] unmaps;
    /// Some + [] selects one region; None + zones selects velocity/round-robin.
    pub fn set_pad_sources(
        &mut self,
        kit: DrumKitId,
        pad: PadId,
        region: Option<RegionId>,
        zones: Vec<VelocityZone>,
    ) -> Result<(), String> {
        self.validate_pad_sources(region, &zones)?;
        let mut candidate = self.clone();
        let pad = candidate
            .drum_kits
            .get_mut(&kit)
            .ok_or("unknown drum kit")?
            .pads
            .get_mut(&pad)
            .ok_or("unknown drum pad")?;
        pad.region = region;
        pad.velocity_zones = zones;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_drum_pad(&mut self, kit: DrumKitId, id: PadId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate
            .drum_kits
            .get_mut(&kit)
            .ok_or("unknown drum kit")?
            .pads
            .remove(&id)
            .ok_or("unknown drum pad")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn assign_kit(
        &mut self,
        instrument: InstrumentId,
        kit: Option<DrumKitId>,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let instrument = candidate
            .instruments
            .get_mut(&instrument)
            .ok_or("unknown instrument")?;
        instrument.kind = kit.map_or(InstrumentKind::Pitched, |kit| InstrumentKind::DrumKit {
            kit,
        });
        if kit.is_some() {
            instrument.sample = None;
            instrument.region = None;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub(crate) fn validate_cell(&self, cell: &Cell) -> Result<(), String> {
        if matches!(cell.note, Note::Note(n) if n > 127) {
            return Err("pitch outside MIDI 0..=127".into());
        }
        if cell
            .instrument
            .is_some_and(|i| !self.instruments.contains_key(&i))
        {
            return Err("unknown instrument".into());
        }
        if cell
            .velocity
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("velocity must be finite in 0..=1".into());
        }
        if cell
            .pan
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
        {
            return Err("pan must be finite in -1..=1".into());
        }
        if cell
            .delay
            .is_some_and(|v| !v.is_finite() || !(0.0..1.0).contains(&v))
        {
            return Err("delay must be finite in 0..1 (exclusive)".into());
        }
        let delay = cell.delay.unwrap_or(0.0);
        let last_start = if let Some(retrigger) = &cell.effects.retrigger {
            if !matches!(cell.note, Note::Note(_)) {
                return Err("retrigger requires a pitched note".into());
            }
            if !(1..=16).contains(&retrigger.repeats)
                || !retrigger.spacing.is_finite()
                || retrigger.spacing <= 0.0
            {
                return Err(
                    "retrigger requires 1..=16 repeats and finite positive row spacing".into(),
                );
            }
            let last = delay + f64::from(retrigger.repeats) * retrigger.spacing;
            if last >= 1.0 {
                return Err("retrigger starts must remain inside their row after delay".into());
            }
            last
        } else {
            delay
        };
        if let Some(release) = cell.effects.release {
            if cell.note == Note::Off
                                || !release.is_finite()
                                || !(0.0..1.0).contains(&release)
                                // Decimal input such as .1 + 2*.1 and .3 must
                                // not disagree solely through f64 roundoff.
                                || release + 8.0 * f64::EPSILON < last_start
            {
                return Err("timed release requires a finite row fraction in 0..1, at or after delay/retriggers, and not an OFF cell".into());
            }
        }
        if let Some(glide) = &cell.effects.glide {
            if cell.note == Note::Off
                || glide.target > 127
                || !glide.semitones_per_second.is_finite()
                || !(0.01..=120.0).contains(&glide.semitones_per_second)
            {
                return Err("glide requires MIDI target 0..127, finite rate 0.01..=120 semitones/second, and not an OFF cell".into());
            }
        }
        if let Some(cut) = cell.effects.cut {
            if cell.note == Note::Off
                || cell.effects.release.is_some()
                || !cut.is_finite()
                || !(0.0..1.0).contains(&cut)
                || cut + 8.0 * f64::EPSILON < last_start
            {
                return Err("hard cut requires a finite row fraction in 0..1, at or after delay/retriggers, and cannot combine OFF or timed release".into());
            }
        }
        if cell
            .effects
            .offset
            .is_some_and(|offset| offset > u64::from(u32::MAX))
        {
            return Err("sample offset exceeds the source-frame limit (u32::MAX)".into());
        }
        if cell.note == Note::Off
            && (cell.effects.offset.is_some() || cell.effects.direction.is_some())
        {
            return Err("sample offset/direction cannot combine with OFF".into());
        }

        if let Some(trigger) = &cell.phrase {
            let phrase = self
                .phrases
                .get(&trigger.phrase)
                .ok_or("unknown phrase trigger")?;
            if !matches!(cell.note, Note::Note(_)) || !(1..=64).contains(&trigger.cycles) {
                return Err(
                    "phrase trigger requires a pitched note and 1..=64 total cycles".into(),
                );
            }
            if cell
                .instrument
                .is_some_and(|instrument| instrument != phrase.instrument)
            {
                return Err(
                    "phrase trigger instrument must match its associated instrument".into(),
                );
            }
            if cell.effects.retrigger.is_some()
                || cell.effects.glide.is_some()
                || cell.effects.offset.is_some()
                || cell.effects.direction.is_some()
            {
                return Err("phrase triggers support delay/gain/pan and timed release/hard cut only; edit internal phrase cells for other effects".into());
            }
        }
        Ok(())
    }
    /// Atomically create or replace a definition, validating all references.
    pub fn upsert_marker_set(&mut self, id: MarkerSetId, set: MarkerSet) -> Result<(), String> {
        if let Some(previous) = self.marker_sets.get(&id) {
            if previous.locked_frames != set.locked_frames {
                return Err(
                    "Use explicit marker lock/unlock operations to change saved locks".into(),
                );
            }
            if !previous.locked_frames.is_empty() && previous.sample != set.sample {
                return Err("Unlock the marker set before changing its source".into());
            }
        }
        let mut candidate = self.clone();
        candidate.marker_sets.insert(id, set);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_marker_set(&mut self, id: MarkerSetId) -> Result<(), String> {
        if self
            .marker_sets
            .get(&id)
            .is_some_and(|set| !set.locked_frames.is_empty())
        {
            return Err("Unlock saved boundaries before deleting their marker set".into());
        }
        let mut candidate = self.clone();
        candidate
            .marker_sets
            .remove(&id)
            .ok_or("unknown marker set")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Explicitly change one saved internal boundary's persistent protection.
    pub fn set_marker_lock(
        &mut self,
        id: MarkerSetId,
        frame: u64,
        locked: bool,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let set = candidate
            .marker_sets
            .get_mut(&id)
            .ok_or("unknown marker set")?;
        let index = set
            .markers
            .binary_search(&frame)
            .map_err(|_| "unknown marker boundary")?;
        if index == 0 || index + 1 == set.markers.len() {
            return Err(
                "Source endpoints are always fixed; only internal boundaries have user locks"
                    .into(),
            );
        }
        match (set.locked_frames.binary_search(&frame), locked) {
            (Ok(index), false) => {
                set.locked_frames.remove(index);
            }
            (Err(index), true) => set.locked_frames.insert(index, frame),
            _ => (),
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Add a new unlocked internal boundary without touching existing locks.
    pub fn add_marker(&mut self, id: MarkerSetId, frame: u64) -> Result<(), String> {
        let mut candidate = self.clone();
        let set = candidate
            .marker_sets
            .get_mut(&id)
            .ok_or("unknown marker set")?;
        let index = set
            .markers
            .binary_search(&frame)
            .err()
            .ok_or("marker boundary already exists")?;
        set.markers.insert(index, frame);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// A locked boundary must be explicitly unlocked before moving it.
    pub fn move_marker(&mut self, id: MarkerSetId, from: u64, to: u64) -> Result<(), String> {
        let mut candidate = self.clone();
        let set = candidate
            .marker_sets
            .get_mut(&id)
            .ok_or("unknown marker set")?;
        let index = set
            .markers
            .binary_search(&from)
            .map_err(|_| "unknown marker boundary")?;
        if index == 0 || index + 1 == set.markers.len() {
            return Err("Source endpoints are fixed and cannot be moved".into());
        }
        if set.locked_frames.binary_search(&from).is_ok() {
            return Err("Unlock the marker before moving it".into());
        }
        set.markers.remove(index);
        let index = set
            .markers
            .binary_search(&to)
            .err()
            .ok_or("marker boundary already exists")?;
        set.markers.insert(index, to);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Remove only an unlocked internal boundary; endpoints are never removable.
    pub fn remove_marker(&mut self, id: MarkerSetId, frame: u64) -> Result<(), String> {
        let mut candidate = self.clone();
        let set = candidate
            .marker_sets
            .get_mut(&id)
            .ok_or("unknown marker set")?;
        let index = set
            .markers
            .binary_search(&frame)
            .map_err(|_| "unknown marker boundary")?;
        if index == 0 || index + 1 == set.markers.len() {
            return Err("Source endpoints are fixed and cannot be removed".into());
        }
        if set.locked_frames.binary_search(&frame).is_ok() {
            return Err("Unlock the marker before removing it".into());
        }
        set.markers.remove(index);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Register immutable source identity; replacing its bytes/metadata needs a new ID.
    pub fn register_sample(&mut self, id: SampleId, sample: SampleAsset) -> Result<(), String> {
        if self
            .samples
            .get(&id)
            .is_some_and(|old| old != &sample || old.path.as_os_str() != sample.path.as_os_str())
        {
            return Err("sample ID already identifies another immutable source".into());
        }
        let mut candidate = self.clone();
        candidate.samples.insert(id, sample);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_sample(&mut self, id: SampleId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.samples.remove(&id).ok_or("unknown sample")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn upsert_region(&mut self, id: RegionId, region: SampleRegion) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.regions.insert(id, region);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Replace only the shared region's opt-in playback settings in one transaction.
    /// Source-dependent FX combinations are checked by the shared evaluator.
    pub fn set_region_playback(
        &mut self,
        id: RegionId,
        amp_envelope: Option<AmpEnvelope>,
        forward_loop: Option<ForwardLoop>,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let region = candidate.regions.get_mut(&id).ok_or("unknown region")?;
        region.amp_envelope = amp_envelope;
        region.forward_loop = forward_loop;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_region(&mut self, id: RegionId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.regions.remove(&id).ok_or("unknown region")?;
        candidate.region_catalog.remove(&id);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Mapping a region deliberately replaces a legacy path. Unmapping leaves it unassigned.
    pub fn assign_region(
        &mut self,
        instrument: InstrumentId,
        region: Option<RegionId>,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let instrument = candidate
            .instruments
            .get_mut(&instrument)
            .ok_or("unknown instrument")?;
        instrument.region = region;
        if region.is_some() {
            instrument.kind = InstrumentKind::Pitched;
            instrument.sample = None;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn upsert_phrase(&mut self, id: PhraseId, phrase: Phrase) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.phrases.insert(id, phrase);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Referenced definitions cannot be removed, even from unused patterns.
    pub fn remove_phrase(&mut self, id: PhraseId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.phrases.remove(&id).ok_or("unknown phrase")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn set_phrase_cell(
        &mut self,
        id: PhraseId,
        row: u32,
        column: u16,
        cell: Cell,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let phrase = candidate.phrases.get_mut(&id).ok_or("unknown phrase")?;
        if row >= phrase.rows || column >= phrase.note_columns {
            return Err("phrase cell outside rows/columns".into());
        }
        if cell == Cell::default() {
            if let Some(columns) = phrase.cells.get_mut(&row) {
                columns.remove(&column);
                if columns.is_empty() {
                    phrase.cells.remove(&row);
                }
            }
        } else {
            phrase.cells.entry(row).or_default().insert(column, cell);
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Add another occurrence of the same content; edits remain shared.
    pub fn append_repeat(&mut self, pattern: PatternId) -> Result<(), String> {
        self.validate()?;
        if !self.patterns.contains_key(&pattern) {
            return Err("unknown pattern".into());
        }
        let mut candidate = self.clone();
        candidate.order.push(Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern,
        });
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Deep-copy content into a new stable ID, without altering the order list.
    pub fn clone_pattern(
        &mut self,
        source: PatternId,
        destination: PatternId,
    ) -> Result<(), String> {
        self.validate()?;
        if destination.0 == 0 || self.patterns.contains_key(&destination) {
            return Err("destination ID must be new and nonzero".into());
        }
        let mut pattern = self
            .patterns
            .get(&source)
            .ok_or("unknown source pattern")?
            .clone();
        for &track in pattern.track_aliases.keys() {
            if let Some(rows) = self.resolved_track(source, track)? {
                pattern.tracks.insert(track, rows.clone());
            }
        }
        pattern.track_aliases.clear();
        let mut candidate = self.clone();
        candidate.patterns.insert(destination, pattern);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

/// Content fixture only; no filesystem access and no implicit sample generation.
pub fn example_song() -> Song {
    let tracks = (1..=4)
        .map(|n| {
            (
                TrackId(n),
                Track {
                    name: format!("Track {n}"),
                    note_columns: 2,
                },
            )
        })
        .collect();
    let instruments = (1..=4)
        .map(|n| {
            (
                InstrumentId(n),
                Instrument {
                    name: format!("Instrument {n}"),
                    kind: InstrumentKind::Pitched,
                    region: None,
                    sample: None,
                },
            )
        })
        .collect();
    let patterns = [(1, 32), (2, 64)]
        .into_iter()
        .map(|(n, rows)| {
            let tracks = (1..=4)
                .map(|t| {
                    let on = Cell {
                        note: Note::Note(48 + t as u8),
                        instrument: Some(InstrumentId(t)),
                        velocity: Some(0.7),
                        ..Cell::default()
                    };
                    let off = Cell {
                        note: Note::Off,
                        ..Cell::default()
                    };
                    (
                        TrackId(t),
                        BTreeMap::from([
                            (0, BTreeMap::from([(0, on)])),
                            (rows - 1, BTreeMap::from([(0, off)])),
                        ]),
                    )
                })
                .collect();
            (
                PatternId(n),
                Pattern {
                    generation: None,
                    track_aliases: Default::default(),
                    name: if n == 1 { "A" } else { "B" }.into(),
                    rows,
                    tracks,
                },
            )
        })
        .collect();
    Song {
        schema: 1,
        generation_recipes: BTreeMap::new(),
        title: "Leviathan A-B-A".into(),
        bpm: 120.0,
        lines_per_beat: 4,
        tracks,
        track_order: (1..=4).map(TrackId).collect(),
        instruments,
        patterns,
        phrases: BTreeMap::new(),
        samples: BTreeMap::new(),
        regions: BTreeMap::new(),
        marker_sets: BTreeMap::new(),
        drum_kits: BTreeMap::new(),
        grooves: BTreeMap::new(),
        multisamples: BTreeMap::new(),
        region_catalog: BTreeMap::new(),
        track_mix: BTreeMap::new(),
        order: [1, 2, 1].map(|n| Occurrence::new(PatternId(n))).to_vec(),
    }
}

/// Empty project template used by the New command. It retains the standard
/// 24-track/24-instrument working layout while removing demo notes and order.
pub fn new_song() -> Song {
    let mut song = example_song();
    song.ensure_24_slots();
    song.title = "Untitled".into();
    song.patterns.retain(|id, _| *id == PatternId(1));
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    song.order = vec![Occurrence::new(PatternId(1))];
    song
}

/// Expanded startup demo, preserving the original 4-track song content.
pub fn startup_song() -> Song {
    let mut song = example_song();
    song.ensure_24_slots();
    song
}
