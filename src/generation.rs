//! Bounded, seeded, actual pattrns execution baked into ordinary Song cells.
//! No scripting, IO, live generator state, or second playback authority.
use crate::{
    break_compiler::timing::{self, Rational},
    song::*,
    RATE,
};
use pattrns::prelude::{
    new_note, new_note_sequence_emitter, BeatTimeBase, Pattern as _, ProbabilityGate, ToFixedRhythm,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "pattrns-euclidean-bake-v1";
pub const GATE_ALGORITHM: &str = "pattrns-0.9.3/rand-0.9.5/Xoshiro256PlusPlus/seed_from_u64";
const TICKS_PER_STEP: u64 = 1024;
/// Conservative control-side traversal limits; distinct from musical start limits.
pub const MAX_VISITS: usize = 65536;

/// Persisted Alpha 2 generation recipe retained for lossless project migration.
/// The recovered generator uses `GenerationConfig` and `GenerationProvenance`;
/// this named recipe is metadata and does not replace that engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub revision: String,
    pub rows: u32,
    pub step_rows: u16,
    pub cycle_steps: u16,
    pub hits: u16,
    pub rotation: i16,
    pub probability: f32,
    pub seed: u64,
    pub sequence: Vec<Vec<Note>>,
    pub instrument: InstrumentId,
    pub velocity: f32,
    pub pan: f32,
}
impl Recipe {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.revision != crate::pattern::REVISION {
            return Err("Unsupported generation recipe schema or pattrns revision".into());
        }
        if !(1..=1024).contains(&self.rows)
            || !(1..=64).contains(&self.step_rows)
            || !(1..=64).contains(&self.cycle_steps)
            || self.hits > self.cycle_steps
            || !(-63..=63).contains(&self.rotation)
        {
            return Err("Invalid generation recipe timing or hit settings".into());
        }
        if !self.probability.is_finite()
            || !(0.0..=1.0).contains(&self.probability)
            || !self.velocity.is_finite()
            || !(0.0..=1.0).contains(&self.velocity)
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
            || self.instrument.0 == 0
            || self.sequence.is_empty()
            || self.sequence.len() > 64
            || self.sequence.iter().any(|chord| {
                chord.is_empty()
                    || chord.len() > 8
                    || chord
                        .iter()
                        .any(|n| matches!(n, Note::Note(p) if !(12..=127).contains(p)))
            })
        {
            return Err(
                "Invalid generation recipe probability, instrument, sequence, velocity or pan"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum GenerationPreset {
    Euclidean {
        steps: u16,
        pulses: u16,
        rotation: u16,
        probability: f32,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationNote {
    pub note: u8,
    pub velocity: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationConfig {
    pub version: u32,
    pub name: String,
    pub bars: u16,
    pub instrument: InstrumentId,
    pub seed: u64,
    pub preset: GenerationPreset,
    pub sequence: Vec<GenerationNote>,
}
/// Historical recipe and digests, never a live instrument reference or playback
/// dependency. Ordinary editing may make content_sha256 no longer match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationProvenance {
    pub algorithm: String,
    pub pattrns_revision: String,
    pub gate_algorithm: String,
    pub config: GenerationConfig,
    pub bpm: f64,
    pub lines_per_beat: u16,
    pub prefix_rows: u64,
    pub raw_ticks: u32,
    pub raw_sha256: String,
    pub content_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedCell {
    pub step: u32,
    pub virtual_time: u64,
    pub row: u32,
    pub column: u16,
    pub frame: u64,
    pub cell: Cell,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationSummary {
    pub algorithm: String,
    pub pattern: PatternId,
    pub track: TrackId,
    pub occurrence: usize,
    pub rows: u32,
    pub columns: u16,
    pub hits: usize,
    pub raw_ticks: u32,
    pub start_frame: u64,
    pub end_frame: u64,
    pub bpm: f64,
    pub lines_per_beat: u16,
    pub raw_sha256: String,
    pub content_sha256: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct GenerationPreview {
    id: String,
    base: Song,
    candidate: Song,
    config: GenerationConfig,
    summary: GenerationSummary,
    cells: Vec<GeneratedCell>,
    provenance: GenerationProvenance,
}
impl GenerationPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &GenerationConfig {
        &self.config
    }
    pub fn summary(&self) -> &GenerationSummary {
        &self.summary
    }
    pub fn cells(&self) -> &[GeneratedCell] {
        &self.cells
    }
    pub fn provenance(&self) -> &GenerationProvenance {
        &self.provenance
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base {
            return Err("Generation preview is stale; prepare a new preview".into());
        }
        Ok(self.candidate.clone())
    }
}
fn digest(value: &impl Serialize) -> Result<String, String> {
    Ok(crate::sample_asset::content_sha256(
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
    ))
}
/// Name and historical metadata are excluded; rows, aliases and exact stored
/// track/cell content are included. Clones preserve this history, not regenerate.
pub fn content_digest(pattern: &Pattern) -> Result<String, String> {
    digest(&(pattern.rows, &pattern.track_aliases, &pattern.tracks))
}
pub fn content_matches(pattern: &Pattern) -> Result<Option<bool>, String> {
    pattern
        .generation
        .as_ref()
        .map(|p| content_digest(pattern).map(|d| d == p.content_sha256))
        .transpose()
}
fn validate_config(config: &GenerationConfig) -> Result<(), String> {
    if config.version != VERSION {
        return Err("Unsupported generation version; expected 1".into());
    }
    let GenerationPreset::Euclidean {
        steps,
        pulses,
        rotation,
        probability,
    } = config.preset;
    if config.name.trim().is_empty()
        || config.name.len() > 256
        || !(1..=16).contains(&config.bars)
        || config.instrument.0 == 0
        || !(1..=128).contains(&steps)
        || pulses == 0
        || pulses > steps
        || rotation >= steps
        || !probability.is_finite()
        || !(0.0..=1.0).contains(&probability)
        || !(1..=16).contains(&config.sequence.len())
    {
        return Err("Generation requires nonblank name <=256 bytes, bars 1..16, nonzero instrument, steps 1..128, pulses 1..steps, rotation <steps, probability 0..1, and sequence 1..16".into());
    }
    for note in &config.sequence {
        if !(12..=127).contains(&note.note)
            || !note.velocity.is_finite()
            || !(0.0..=1.0).contains(&note.velocity)
        {
            return Err("Generation notes/selectors must be MIDI 12..127 (pattrns +12 bridge) and velocity 0..1".into());
        }
    }
    Ok(())
}
impl GenerationProvenance {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_config(&self.config)?;
        let GenerationPreset::Euclidean { steps, .. } = self.config.preset;
        let sha = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        };
        if self.algorithm != ALGORITHM
            || self.pattrns_revision != crate::pattern::REVISION
            || self.gate_algorithm != GATE_ALGORITHM
            || !sha(&self.raw_sha256)
            || !sha(&self.content_sha256)
            || !self.bpm.is_finite()
            || !(20.0..=400.0).contains(&self.bpm)
            || !(1..=256).contains(&self.lines_per_beat)
            || self.raw_ticks != u32::from(steps) * u32::from(self.config.bars)
        {
            return Err("Invalid historical generation provenance".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
struct RawNote {
    note: u8,
    volume: f32,
    panning: f32,
    delay: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
struct RawTick {
    time: u64,
    duration: u64,
    note: Option<RawNote>,
}
fn run(config: &GenerationConfig) -> Result<Vec<RawTick>, String> {
    let GenerationPreset::Euclidean {
        steps,
        pulses,
        rotation,
        probability,
    } = config.preset;
    // Pinned API names its hit count `steps` and total count `pulses`.
    let rhythm: Vec<f32> = pattrns::prelude::euclidean::euclidean(
        u32::from(pulses),
        u32::from(steps),
        i32::from(rotation),
    )
    .into_iter()
    .map(|hit| if hit { probability } else { 0.0 })
    .collect();
    let sequence = config
        .sequence
        .iter()
        .map(|n| new_note((pattrns::Note::from(n.note - 12), None, n.velocity, 0.0)))
        .collect();
    let time = BeatTimeBase {
        beats_per_min: 60.0,
        beats_per_bar: 4,
        samples_per_sec: TICKS_PER_STEP as u32,
    };
    let mut pattern = time
        .every_nth_beat(1.0)
        .with_rhythm(rhythm.to_rhythm())
        .with_gate(ProbabilityGate::new(Some(config.seed)))
        .emit(new_note_sequence_emitter(sequence));
    let count = usize::from(steps) * usize::from(config.bars);
    let end = count as u64 * TICKS_PER_STEP;
    let mut raw = Vec::with_capacity(count);
    while let Some(event) = pattern.run_until_time(end) {
        if raw.len() >= count
            || event.time != raw.len() as u64 * TICKS_PER_STEP
            || event.duration != TICKS_PER_STEP
        {
            return Err("pattrns stream violates bounded integral step contract".into());
        }
        let note = match event.event {
            None => None,
            Some(pattrns::Event::NoteEvents(mut notes)) if notes.len() == 1 => {
                let note = notes
                    .pop()
                    .flatten()
                    .ok_or("pattrns emitted unexpected empty note")?;
                if !note.note.is_note_on()
                    || note.instrument.is_some()
                    || note.delay != 0.0
                    || note.panning != 0.0
                {
                    return Err("pattrns emitted unsupported generated note attributes".into());
                }
                Some(RawNote {
                    note: note.note as u8,
                    volume: note.volume,
                    panning: note.panning,
                    delay: note.delay,
                })
            }
            _ => return Err("pattrns emitted unsupported event shape".into()),
        };
        raw.push(RawTick {
            time: event.time,
            duration: event.duration,
            note,
        });
    }
    if raw.len() != count {
        return Err("pattrns ended before the requested bounded tick count".into());
    }
    Ok(raw)
}
/// Count empty blocks and row containers as real work too. This runs before
/// full Song validation, which may itself inspect ordered stored drum cells.
/// It neither copies cells nor expands aliases/repeats into an event stream.
pub(crate) fn check_work(song: &Song, append: bool) -> Result<(), String> {
    let extra = usize::from(append);
    let blocks = song.order.len().checked_add(extra).and_then(|orders| {
        song.track_order
            .len()
            .checked_add(extra)
            .and_then(|tracks| orders.checked_mul(tracks))
    });
    if blocks.is_none_or(|n| n > MAX_VISITS) {
        return Err("Generation exceeds 65536 ordered track blocks; reduce tracks or arrangement occurrences before generation".into());
    }
    let mut aliases = 0usize;
    let mut terminals = BTreeMap::new();
    let mut patterns = BTreeMap::new();
    for (&id, pattern) in &song.patterns {
        aliases = aliases.saturating_add(pattern.track_aliases.len());
        if aliases > 4096 {
            return Err("Generation source exceeds 4096 track aliases".into());
        }
        let (mut row_count, mut cell_count) = (0usize, 0usize);
        for &track in pattern.tracks.keys().chain(pattern.track_aliases.keys()) {
            let terminal = song.resolve_track_source(id, track)?;
            let (rows, cells) = *terminals.entry(terminal).or_insert_with(|| {
                song.patterns[&terminal.0]
                    .tracks
                    .get(&terminal.1)
                    .map_or((0, 0), |rows| {
                        (
                            rows.len(),
                            rows.values()
                                .fold(0usize, |n, cells| n.saturating_add(cells.len())),
                        )
                    })
            });
            row_count = row_count.saturating_add(rows);
            cell_count = cell_count.saturating_add(cells);
        }
        patterns.insert(id, (row_count, cell_count));
    }
    let (mut rows, mut cells) = (0usize, 0usize);
    for occurrence in &song.order {
        let &(add_rows, add_cells) = patterns
            .get(&occurrence.pattern)
            .ok_or("Generation references unknown ordered pattern")?;
        rows = rows.saturating_add(add_rows);
        cells = cells.saturating_add(add_cells);
        if rows > MAX_VISITS {
            return Err("Generation exceeds 65536 ordered row-container visits, including empty or muted rows; reduce stored rows or repeats".into());
        }
        if cells > MAX_VISITS {
            return Err("Generation exceeds 65536 ordered cell visits, including default or muted cells; reduce stored cells or repeats".into());
        }
    }
    Ok(())
}
/// Pure control-side preview: no asset IO or decoding. The existing evaluator
/// validates aggregate musical budgets and verifies exact generated ON frames.
pub fn preview(song: &Song, config: GenerationConfig) -> Result<GenerationPreview, String> {
    crate::project::check_serialized_size(song)?;
    check_work(song, true)?;
    song.validate()?;
    validate_config(&config)?;
    let instrument = song
        .instruments
        .get(&config.instrument)
        .ok_or("Unknown generation instrument")?;
    match instrument.kind {
        InstrumentKind::Pitched if instrument.sample.is_none() && instrument.region.is_none() => {
            return Err("Generation instrument has no assigned sample or region".into())
        }
        InstrumentKind::Multisample { map } => {
            for choice in &config.sequence {
                song.select_keyzone(map, choice.note, choice.velocity)?;
            }
        }
        InstrumentKind::DrumKit { kit } => {
            for choice in &config.sequence {
                if !song.drum_kits[&kit]
                    .pads
                    .values()
                    .any(|pad| pad.note == choice.note && pad.has_source())
                {
                    return Err(format!(
                        "Generation kit selector {} is unmapped or has no assigned region",
                        choice.note
                    ));
                }
            }
        }
        _ => (),
    }
    let raw = run(&config)?;
    if raw.iter().all(|tick| tick.note.is_none()) {
        return Err("Generation emitted no hits; change seed, probability or rhythm (no fallback note is inserted)".into());
    }
    let track = TrackId(
        song.tracks
            .keys()
            .next_back()
            .map_or(0, |id| id.0)
            .checked_add(1)
            .ok_or("Generation track ID space exhausted")?,
    );
    let pattern_id = PatternId(
        song.patterns
            .keys()
            .next_back()
            .map_or(0, |id| id.0)
            .checked_add(1)
            .ok_or("Generation pattern ID space exhausted")?,
    );
    let prefix = song.order.iter().try_fold(0u64, |n, occurrence| {
        n.checked_add(u64::from(song.patterns[&occurrence.pattern].rows))
            .ok_or("Generation prefix overflow")
    })?;
    let rows = u32::from(config.bars) * 4 * u32::from(song.lines_per_beat);
    let bpm = Rational::bpm(song.bpm);
    let per_row = f64::from(RATE) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
    let GenerationPreset::Euclidean { steps, .. } = config.preset;
    let mut data = TrackPattern::new();
    let mut cells = Vec::new();
    let mut columns = 1u16;
    for (step, tick) in raw.iter().enumerate() {
        let Some(note) = &tick.note else { continue };
        let beat = Rational::new(step as u128 * 4, u128::from(steps));
        let (row, delay, frame) = timing::encode(beat, prefix, song.lines_per_beat, per_row, bpm)?;
        let intended_row = step as u64 * 4 * u64::from(song.lines_per_beat) / u64::from(steps);
        if row >= rows || u64::from(row) != intended_row {
            return Err("Exact generation timestamp cannot be represented in its intended row; change BPM or LPB".into());
        }
        let row_cells = data.entry(row).or_default();
        let column =
            u16::try_from(row_cells.len()).map_err(|_| "Generation column budget exceeded")?;
        if column >= 256 {
            return Err("Generation exceeds 256 note columns".into());
        }
        columns = columns.max(column + 1);
        let cell = Cell {
            note: Note::Note(
                note.note
                    .checked_add(12)
                    .ok_or("pattrns pitch conversion overflow")?,
            ),
            instrument: Some(config.instrument),
            velocity: Some(note.volume),
            pan: Some(note.panning),
            delay: if delay == 0.0 { None } else { Some(delay) },
            ..Cell::default()
        };
        row_cells.insert(column, cell.clone());
        cells.push(GeneratedCell {
            step: step as u32,
            virtual_time: tick.time,
            row,
            column,
            frame,
            cell,
        });
    }
    let mut pattern = Pattern {
        generation: None,
        name: config.name.clone(),
        rows,
        track_aliases: BTreeMap::new(),
        tracks: BTreeMap::from([(track, data)]),
    };
    let raw_sha256 = digest(&raw)?;
    let content_sha256 = content_digest(&pattern)?;
    let provenance = GenerationProvenance {
        algorithm: ALGORITHM.into(),
        pattrns_revision: crate::pattern::REVISION.into(),
        gate_algorithm: GATE_ALGORITHM.into(),
        config: config.clone(),
        bpm: song.bpm,
        lines_per_beat: song.lines_per_beat,
        prefix_rows: prefix,
        raw_ticks: raw.len() as u32,
        raw_sha256: raw_sha256.clone(),
        content_sha256: content_sha256.clone(),
    };
    pattern.generation = Some(provenance.clone());
    let mut candidate = song.clone();
    candidate.tracks.insert(
        track,
        Track {
            name: config.name.clone(),
            note_columns: columns,
        },
    );
    candidate.track_order.push(track);
    candidate.patterns.insert(pattern_id, pattern);
    let occurrence = candidate.order.len();
    candidate.order.push(Occurrence {
        pattern: pattern_id,
        muted_tracks: Default::default(),
        section: None,
    });
    crate::project::check_serialized_size(&candidate)?;
    check_work(&candidate, false)?;
    candidate.validate()?;
    let manifest = crate::song_render::evaluate_model(&candidate)?;
    let actual: Vec<_> = manifest
        .events
        .iter()
        .filter(|event| {
            event.track == track && matches!(event.command, crate::song_render::Command::On { .. })
        })
        .map(|event| (event.frame, event.position, event.column))
        .collect();
    let expected: Vec<_> = cells
        .iter()
        .map(|cell| (cell.frame, Some((occurrence, cell.row)), cell.column))
        .collect();
    if actual != expected {
        return Err("Generated event frames differ from exact pattrns timing conversion".into());
    }
    let exact_end = Rational::new(
        u128::from(prefix) + u128::from(rows),
        u128::from(song.lines_per_beat),
    )
    .scale(u64::from(RATE) * 60)?
    .div(bpm)?
    .round()?;
    if manifest.musical_frames != exact_end {
        return Err(
            "Generated musical end is not exactly representable at this BPM/LPB/prefix".into(),
        );
    }
    let start_frame = Rational::new(u128::from(prefix), u128::from(song.lines_per_beat))
        .scale(u64::from(RATE) * 60)?
        .div(bpm)?
        .round()?;
    let summary = GenerationSummary { algorithm: ALGORITHM.into(), pattern: pattern_id, track, occurrence, rows, columns, hits: cells.len(), raw_ticks: raw.len() as u32, start_frame, end_frame: exact_end, bpm: song.bpm, lines_per_beat: song.lines_per_beat, raw_sha256, content_sha256, warnings: vec![
        "Appends a new ordinary pattern and track; Song BPM/LPB and existing cells stay unchanged. Extending SongEnd may extend held notes/OneShot tails and allow previously end-truncated phrase child notes to execute in this section.".into(),
        "Lowest free column per row prevents collisions; columns are reused in later rows. Ordinary note replacement, kit gate/choke and SongEnd rules apply; pattrns event duration is NOT imported as note length. Same-instrument kit choke may release older tracks.".into(),
        "Sequence advances only on emitted hits; seeded probability runs continuously across bars. Probability 0 or 1 consumes no random gate draws. MIDI 12..127 is the current pattrns +12 bridge range; kit notes select pads, not pitch.".into(),
        "Provenance is historical, not live generation: ordinary edits, later BPM/LPB changes, repeats or aliases do not regenerate or promise the original absolute frame proof. Pure preview caps prospective ordered track blocks, resolved row-container visits and cell visits separately at 65536, including empty/default/muted content. It conservatively enforces the pretty project 8 MiB limit using captured paths (save may shorten them); unresolved relative paths and other asset/path checks still belong to save/export. No asset IO, audio/clipping or physical-host check is performed by preview.".into(),
    ] };
    let id = digest(&(ALGORITHM, song, &config, &candidate))?;
    Ok(GenerationPreview {
        id,
        base: song.clone(),
        candidate,
        config,
        summary,
        cells,
        provenance,
    })
}
