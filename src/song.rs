//! Persistent musical content only: no playhead, voices, command memory or DSP state.
//! Public fields support draft editing; call `validate` before accepting a document.
//! Deserialization alone does not validate. Numeric IDs are caller-assigned, nonzero,
//! stable within a document, and never derived from display order or names.
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
    /// Optional unresolved asset reference. Validation does not touch the filesystem.
    pub sample: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub phrases: BTreeMap<PhraseId, Phrase>,
}
/// An instrument-owned, non-recursive one-shot pattern. Pitches transpose from root_note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    pub name: String,
    pub rows: u32,
    pub lines_per_beat: u16,
    pub note_columns: u16,
    pub root_note: u8,
    pub cells: BTreeMap<u32, BTreeMap<u16, Cell>>,
}
impl Default for Phrase {
    fn default() -> Self {
        Self {
            name: "New phrase".into(),
            rows: 16,
            lines_per_beat: 4,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::new(),
        }
    }
}
impl Phrase {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=256).contains(&self.rows)
            || !(1..=256).contains(&self.lines_per_beat)
            || !(1..=16).contains(&self.note_columns)
            || self.root_note > 127
        {
            return Err(
                "Phrase requires 1..256 rows/LPB, 1..16 columns and a MIDI root note".into(),
            );
        }
        for (row, columns) in &self.cells {
            if *row >= self.rows {
                return Err("Phrase resize would discard occupied rows".into());
            }
            for (column, cell) in columns {
                if *column >= self.note_columns {
                    return Err("Phrase resize would discard occupied columns".into());
                }
                if cell.instrument.is_some() || cell.phrase.is_some() {
                    return Err("Phrase cells use their owning instrument; instrument overrides and nested phrases are forbidden".into());
                }
                cell.validate_values()?;
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Note {
    #[default]
    Empty,
    /// MIDI pitch 0..=127; independent of library octave display conventions.
    Note(u8),
    Off,
}
/// Per-cell effects. These do not latch into subsequent rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepEffects {
    /// Total attacks, including the initial note, across the remaining row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrigger: Option<u8>,
    /// Hard cut at a fraction of the remaining row after delay, inclusive 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<f64>,
    /// Skip this fraction of source frames in the chosen playback direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// Pitch glide rate in semitones/second, using the sampler's speed glide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glide: Option<f32>,
}
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}
impl StepEffects {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(&self, note: Note) -> Result<(), String> {
        if self.retrigger.is_some_and(|v| !(2..=16).contains(&v)) {
            return Err("Retrigger count must be 2..=16 total attacks".into());
        }
        if self
            .cut
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("Cut must be finite in 0..=1 of the remaining row".into());
        }
        if self
            .offset
            .is_some_and(|v| !v.is_finite() || !(0.0..1.0).contains(&v))
        {
            return Err("Offset must be finite in 0..<1 of source frames".into());
        }
        if self
            .glide
            .is_some_and(|v| !v.is_finite() || !(0.1..=1200.0).contains(&v))
        {
            return Err("Glide must be finite in 0.1..=1200 semitones/second".into());
        }
        if !matches!(note, Note::Note(_))
            && (self.retrigger.is_some()
                || self.offset.is_some()
                || self.reverse
                || self.glide.is_some())
        {
            return Err(
                "Retrigger, offset, reverse and glide require a pitched note in the same cell"
                    .into(),
            );
        }
        if note == Note::Off && self.cut.is_some() {
            return Err("Use cut on a note or empty cell, not OFF".into());
        }
        if self.glide.is_some()
            && (self.retrigger.is_some() || self.offset.is_some() || self.reverse)
        {
            return Err("Glide cannot combine with retrigger, offset or reverse".into());
        }
        Ok(())
    }
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
    #[serde(default, skip_serializing_if = "StepEffects::is_empty")]
    pub effects: StepEffects,
    /// Explicit phrase selection for this pitched trigger only; omission plays the sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase: Option<PhraseId>,
}
impl Cell {
    pub fn validate_values(&self) -> Result<(), String> {
        self.effects.validate(self.note)?;
        if matches!(self.note, Note::Note(n) if n > 127) {
            return Err("pitch outside MIDI 0..=127".into());
        }
        if self
            .velocity
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("velocity must be finite in 0..=1".into());
        }
        if self
            .pan
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
        {
            return Err("pan must be finite in -1..=1".into());
        }
        if self
            .delay
            .is_some_and(|v| !v.is_finite() || !(0.0..1.0).contains(&v))
        {
            return Err("delay must be finite in 0..1 (exclusive)".into());
        }
        if let Some(id) = self.phrase {
            if id.0 == 0 || !matches!(self.note, Note::Note(_)) {
                return Err("A phrase ID must be nonzero and accompany a pitched note".into());
            }
            if self.effects.retrigger.is_some()
                || self.effects.offset.is_some()
                || self.effects.reverse
                || self.effects.glide.is_some()
            {
                return Err(
                    "Phrase triggers support delay/cut; place other effects inside the phrase"
                        .into(),
                );
            }
        }
        Ok(())
    }
}
/// Sparse rows and columns. Missing cells are empty; row and column indices are zero-based.
pub type TrackPattern = BTreeMap<u32, BTreeMap<u16, Cell>>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<TrackId, BlockRef>,
    pub name: String,
    pub rows: u32,
    pub tracks: BTreeMap<TrackId, TrackPattern>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockRef {
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
    pub pattern: PatternId,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub muted_tracks: BTreeSet<TrackId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<Section>,
}
impl Occurrence {
    pub fn new(pattern: PatternId) -> Self {
        Self {
            pattern,
            muted_tracks: BTreeSet::new(),
            section: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Song {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generation_recipes: BTreeMap<String, crate::generation::Recipe>,
    pub schema: u32,
    pub title: String,
    pub bpm: f64,
    pub lines_per_beat: u16,
    pub tracks: BTreeMap<TrackId, Track>,
    pub track_order: Vec<TrackId>,
    pub instruments: BTreeMap<InstrumentId, Instrument>,
    pub patterns: BTreeMap<PatternId, Pattern>,
    pub order: Vec<Occurrence>,
}
impl Song {
    /// Expand a document to at least 24 tracker channels and instruments.
    /// Existing IDs, channel layouts, instruments, and pattern data are preserved.
    pub fn ensure_24_slots(&mut self) {
        while self.track_order.len() < 24 {
            let id = next_track_id(&self.tracks);
            self.tracks.insert(
                id,
                Track {
                    name: format!("Track {}", id.0),
                    note_columns: 1,
                },
            );
            self.track_order.push(id);
        }
        while self.instruments.len() < 24 {
            let id = next_instrument_id(&self.instruments);
            self.instruments.insert(
                id,
                Instrument {
                    name: format!("Instrument {}", id.0),
                    sample: None,
                    phrases: BTreeMap::new(),
                },
            );
        }
    }

    /// Resolve a block, rejecting missing references, incompatible shapes and cycles.
    pub fn block_source(&self, pattern: PatternId, track: TrackId) -> Result<BlockRef, String> {
        let mut at = BlockRef { pattern, track };
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(at) || visited.len() > 64 {
                return Err("Alias cycle or chain longer than 64 blocks".into());
            }
            let p = self
                .patterns
                .get(&at.pattern)
                .ok_or("Unknown alias pattern")?;
            let t = self.tracks.get(&at.track).ok_or("Unknown alias track")?;
            let Some(next) = p.aliases.get(&at.track) else {
                return Ok(at);
            };
            let source = self
                .patterns
                .get(&next.pattern)
                .ok_or("Unknown alias pattern")?;
            let source_track = self.tracks.get(&next.track).ok_or("Unknown alias track")?;
            if p.rows != source.rows || t.note_columns != source_track.note_columns {
                return Err("Alias blocks require equal row and note-column counts".into());
            }
            at = *next;
        }
    }
    pub fn block(
        &self,
        pattern: PatternId,
        track: TrackId,
    ) -> Result<Option<&TrackPattern>, String> {
        let source = self.block_source(pattern, track)?;
        Ok(self.patterns[&source.pattern].tracks.get(&source.track))
    }
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

    /// Validate content and every reference, including unused patterns.
    /// Current product limits: BPM 20..=400, LPB 1..=256, rows 1..=65536,
    /// columns 1..=256. These are not claims of Renoise compatibility.
    pub fn validate(&self) -> Result<(), String> {
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
        if self.schema != 1 {
            return Err("unsupported song schema".into());
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
        for (owner, instrument) in &self.instruments {
            if instrument.phrases.len() > 64 {
                return Err("An instrument supports at most 64 phrases".into());
            }
            for (id, phrase) in &instrument.phrases {
                if id.0 == 0 {
                    return Err("zero phrase ID".into());
                }
                phrase
                    .validate()
                    .map_err(|e| format!("Instrument {}, phrase {}: {e}", owner.0, id.0))?;
                if phrase.rows as f64 * 60.0 / (self.bpm * phrase.lines_per_beat as f64) > 600.0 {
                    return Err("Phrase duration at this song tempo exceeds ten minutes".into());
                }
            }
        }
        for (id, pattern) in &self.patterns {
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
                        cell.validate_values().map_err(|e| {
                            format!(
                                "Pattern {}, track {}, row {row}, column {column}: {e}",
                                id.0, track_id.0
                            )
                        })?;
                        if let Some(phrase) = cell.phrase {
                            let exists = if let Some(owner) = cell.instrument {
                                self.instruments
                                    .get(&owner)
                                    .is_some_and(|i| i.phrases.contains_key(&phrase))
                            } else {
                                self.instruments
                                    .values()
                                    .any(|i| i.phrases.contains_key(&phrase))
                            };
                            if !exists {
                                return Err("Unknown instrument phrase".into());
                            }
                        }
                        if *column >= track.note_columns {
                            return Err(format!("column {column} outside track {track_id:?}"));
                        }
                        if cell
                            .instrument
                            .is_some_and(|i| !self.instruments.contains_key(&i))
                        {
                            return Err("unknown instrument".into());
                        }
                    }
                }
            }
        }
        for (pattern, content) in &self.patterns {
            for track in content.aliases.keys() {
                if content.tracks.contains_key(track) {
                    return Err("Alias cannot also contain local cells".into());
                }
                self.block_source(*pattern, *track)?;
            }
        }
        if self.order.len() > 4096 {
            return Err("At most 4096 order positions".into());
        }
        let mut sections = BTreeSet::new();
        for occurrence in &self.order {
            if occurrence
                .muted_tracks
                .iter()
                .any(|t| !self.tracks.contains_key(t))
            {
                return Err("Unknown muted track".into());
            }
            if let Some(section) = &occurrence.section {
                if section.id.0 == 0
                    || !sections.insert(section.id)
                    || section.name.trim().is_empty()
                    || section.name.chars().count() > 64
                {
                    return Err(
                        "Sections need unique nonzero IDs and names of 1..64 characters".into(),
                    );
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
        Ok(())
    }
    /// Add another occurrence of the same content; edits remain shared.
    pub fn append_repeat(&mut self, pattern: PatternId) -> Result<(), String> {
        self.validate()?;
        if !self.patterns.contains_key(&pattern) {
            return Err("unknown pattern".into());
        }
        if self.order.len() >= 4096 {
            return Err("At most 4096 order positions".into());
        }
        self.order.push(Occurrence::new(pattern));
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
        for track in pattern.aliases.keys() {
            if let Some(block) = self.block(source, *track)? {
                pattern.tracks.insert(*track, block.clone());
            }
        }
        pattern.aliases.clear();
        self.patterns.insert(destination, pattern);
        Ok(())
    }
}

fn next_track_id(tracks: &BTreeMap<TrackId, Track>) -> TrackId {
    TrackId(
        (1..=u64::MAX)
            .find(|id| !tracks.contains_key(&TrackId(*id)))
            .expect("track ID space exhausted"),
    )
}
fn next_instrument_id(instruments: &BTreeMap<InstrumentId, Instrument>) -> InstrumentId {
    InstrumentId(
        (1..=u64::MAX)
            .find(|id| !instruments.contains_key(&InstrumentId(*id)))
            .expect("instrument ID space exhausted"),
    )
}

/// Blank 24-channel starting document. Kept separate from `example_song`, which
/// remains a small content fixture for tests and demos.
pub fn new_song_template() -> Song {
    let mut song = example_song();
    song.title = "Untitled".into();
    for track in song.tracks.values_mut() {
        track.note_columns = 1;
    }
    for pattern in song.patterns.values_mut() {
        pattern.tracks.clear();
        pattern.aliases.clear();
    }
    song.patterns.retain(|id, _| *id == PatternId(1));
    song.order = vec![Occurrence::new(PatternId(1))];
    song.ensure_24_slots();
    song
}

/// Expanded example content used by the editor on startup. Preserves the
/// example arrangement and its existing channel layouts while offering 24 slots.
pub fn startup_song() -> Song {
    let mut song = example_song();
    song.ensure_24_slots();
    song
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
                    phrases: BTreeMap::new(),
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
                    name: if n == 1 { "A" } else { "B" }.into(),
                    rows,
                    tracks,
                    aliases: BTreeMap::new(),
                },
            )
        })
        .collect();
    Song {
        generation_recipes: BTreeMap::new(),
        schema: 1,
        title: "Leviathan A-B-A".into(),
        bpm: 120.0,
        lines_per_beat: 4,
        tracks,
        track_order: (1..=4).map(TrackId).collect(),
        instruments,
        patterns,
        order: [1, 2, 1].map(|n| Occurrence::new(PatternId(n))).to_vec(),
    }
}
