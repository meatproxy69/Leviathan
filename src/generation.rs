//! Bounded pattrns control-side generation; only committed cells reach playback.
use crate::{
    song::*,
    transform::{CellChange, Summary},
};
use pattrns::prelude::{
    euclidean, new_note, new_polyphonic_note_sequence_emitter, BeatTimeBase, Pattern,
    ProbabilityGate, ToFixedRhythm,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
    /// Each accepted pulse advances to the next chord; Empty sustains, Off releases.
    pub sequence: Vec<Vec<Note>>,
    pub instrument: InstrumentId,
    pub velocity: f32,
    pub pan: f32,
}
impl Default for Recipe {
    fn default() -> Self {
        Self {
            schema: 1,
            revision: crate::pattern::REVISION.into(),
            rows: 16,
            step_rows: 1,
            cycle_steps: 8,
            hits: 3,
            rotation: 0,
            probability: 1.0,
            seed: 1,
            sequence: vec![
                vec![Note::Note(60)],
                vec![Note::Note(63)],
                vec![Note::Note(67)],
                vec![Note::Off],
            ],
            instrument: InstrumentId(1),
            velocity: 0.4,
            pan: 0.0,
        }
    }
}
impl Recipe {
    pub fn columns(&self) -> usize {
        self.sequence.iter().map(Vec::len).max().unwrap_or(0)
    }
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
            return Err("Generation requires 1..1024 rows, 1..64 step rows/cycle steps, hits <= cycle steps and rotation -63..63".into());
        }
        if !self.probability.is_finite()
            || !(0.0..=1.0).contains(&self.probability)
            || !self.velocity.is_finite()
            || !(0.0..=1.0).contains(&self.velocity)
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
            || self.instrument.0 == 0
        {
            return Err("Invalid generation probability, instrument, velocity or pan".into());
        }
        if self.sequence.is_empty()
            || self.sequence.len() > 64
            || self.sequence.iter().any(|chord| {
                chord.is_empty()
                    || chord.len() > 8
                    || chord
                        .iter()
                        .any(|n| matches!(n,Note::Note(p) if !(12..=127).contains(p)))
            })
        {
            return Err(
                "Sequence requires 1..64 steps of 1..8 columns; MIDI pitches 12..127, off or ."
                    .into(),
            );
        }
        Ok(())
    }
}
pub fn parse_sequence(text: &str) -> Result<Vec<Vec<Note>>, String> {
    if text.len() > 2048 {
        return Err("Sequence text exceeds 2048 bytes".into());
    }
    let result = text
        .split_whitespace()
        .map(|step| {
            step.split(',')
                .map(|note| {
                    if note == "." {
                        Ok(Note::Empty)
                    } else if note.eq_ignore_ascii_case("off") {
                        Ok(Note::Off)
                    } else {
                        note.parse::<u8>().map(Note::Note).map_err(|_| {
                            "Use MIDI numbers, off, . and comma-separated chords".to_owned()
                        })
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let recipe = Recipe {
        sequence: result.clone(),
        ..Default::default()
    };
    recipe.validate()?;
    Ok(result)
}
pub fn sequence_text(sequence: &[Vec<Note>]) -> String {
    sequence
        .iter()
        .map(|chord| {
            chord
                .iter()
                .map(|note| match note {
                    Note::Empty => ".".into(),
                    Note::Off => "off".into(),
                    Note::Note(n) => n.to_string(),
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join(" ")
}
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub cells: TrackPattern,
    pub columns: u16,
    pub notes: usize,
    pub offs: usize,
}
/// A canonical exact 6000-sample row is just the carrier grid. Song tempo/LPB
/// applies when committed cells play; generation is independent of floating tempo.
pub fn generate(recipe: &Recipe) -> Result<Generated, String> {
    recipe.validate()?;
    // Pinned pattrns calls its arguments steps/pulses, but tests/source define
    // euclidean(hit_count, cycle_length, rotation), in that order.
    let rhythm = euclidean::euclidean(
        recipe.hits as u32,
        recipe.cycle_steps as u32,
        recipe.rotation as i32,
    )
    .into_iter()
    .map(|hit| if hit { recipe.probability } else { 0.0 })
    .collect::<Vec<_>>()
    .to_rhythm();
    let sequence = recipe
        .sequence
        .iter()
        .map(|chord| {
            chord
                .iter()
                .map(|note| match note {
                    Note::Empty => None,
                    Note::Off => new_note(pattrns::Note::OFF),
                    Note::Note(n) => new_note((
                        pattrns::Note::from(*n - 12),
                        None,
                        recipe.velocity,
                        recipe.pan,
                    )),
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut pattern = BeatTimeBase {
        beats_per_min: 120.0,
        beats_per_bar: 4,
        samples_per_sec: 48000,
    }
    .every_nth_beat(recipe.step_rows as f32 / 4.0)
    .with_rhythm(rhythm)
    .with_gate(ProbabilityGate::new(Some(recipe.seed)))
    .emit(new_polyphonic_note_sequence_emitter(sequence));
    let end = recipe.rows as u64 * 6000;
    let mut result = Generated {
        cells: TrackPattern::new(),
        columns: recipe.columns() as u16,
        notes: 0,
        offs: 0,
    };
    let mut items = 0;
    while let Some(item) = pattern.run_until_time(end) {
        items += 1;
        if items > 1024 {
            return Err("Generation exceeds 1024 pulse items".into());
        }
        if item.time >= end {
            break;
        }
        if item.time % 6000 != 0 {
            return Err("Generated event is outside the exact row grid".into());
        }
        if let Some(event) = item.event {
            let pattrns::Event::NoteEvents(notes) = event else {
                return Err("Unsupported generated parameter event".into());
            };
            if notes.len() > result.columns as usize {
                return Err("Generated chord exceeds selected width".into());
            }
            for (column, note) in notes.into_iter().enumerate() {
                let Some(note) = note else {
                    continue;
                };
                let cell = if note.note == pattrns::Note::EMPTY {
                    continue;
                } else if note.note == pattrns::Note::OFF {
                    result.offs += 1;
                    Cell {
                        note: Note::Off,
                        ..Default::default()
                    }
                } else {
                    result.notes += 1;
                    if result.notes > 2048 {
                        return Err("Generation exceeds 2048 note triggers".into());
                    }
                    Cell {
                        note: Note::Note(
                            crate::pattern::midi_note(note.note).map_err(|e| e.to_string())?,
                        ),
                        instrument: Some(recipe.instrument),
                        velocity: Some(note.volume),
                        pan: Some(note.panning),
                        ..Default::default()
                    }
                };
                cell.validate_values()?;
                result
                    .cells
                    .entry((item.time / 6000) as u32)
                    .or_default()
                    .insert(column as u16, cell);
            }
        }
    }
    Ok(result)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Replace,
    Merge,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub occurrence: usize,
    pub track: TrackId,
    pub column: u16,
    pub row: u32,
}
#[derive(Debug, Clone)]
pub struct Preview {
    pub summary: Summary,
    pub generated: Generated,
    pub recipe_changed: bool,
    original: Song,
    candidate: Song,
}
impl Preview {
    pub fn is_current(&self, song: &Song) -> bool {
        &self.original == song
    }
    pub(crate) fn into_song(self, song: &Song) -> Result<Song, String> {
        if !self.is_current(song) {
            return Err("Song changed since generation preview; preview again".into());
        }
        Ok(self.candidate)
    }
}
pub fn preview(
    song: &Song,
    target: &Target,
    name: &str,
    recipe: &Recipe,
    mode: WriteMode,
) -> Result<Preview, String> {
    song.validate()?;
    recipe.validate()?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Recipe name must have 1..64 characters".into());
    }
    if !song.instruments.contains_key(&recipe.instrument) {
        return Err("Unknown generation instrument".into());
    }
    let pattern = song
        .order
        .get(target.occurrence)
        .ok_or("Unknown generation occurrence")?
        .pattern;
    let track = song
        .tracks
        .get(&target.track)
        .ok_or("Unknown generation track")?;
    let end = target
        .row
        .checked_add(recipe.rows)
        .ok_or("Generation row overflow")?;
    let col_end = target
        .column
        .checked_add(recipe.columns() as u16)
        .ok_or("Generation column overflow")?;
    if end > song.patterns[&pattern].rows || col_end > track.note_columns {
        return Err("Generated rectangle does not fit this pattern/track".into());
    }
    let generated = generate(recipe)?;
    let before = song
        .block(pattern, target.track)?
        .cloned()
        .unwrap_or_default();
    let mut after = before.clone();
    if mode == WriteMode::Replace {
        for (_, columns) in after.range_mut(target.row..end) {
            columns.retain(|col, _| *col < target.column || *col >= col_end);
        }
    }
    for (row, columns) in &generated.cells {
        for (col, cell) in columns {
            let row = target.row + row;
            let col = target.column + col;
            if mode == WriteMode::Merge
                && before
                    .get(&row)
                    .and_then(|c| c.get(&col))
                    .is_some_and(|c| *c != Cell::default())
            {
                return Err(format!(
                    "Merge collision at row {row}, column {}. Choose Replace or clear the cell.",
                    col + 1
                ));
            }
            after.entry(row).or_default().insert(col, cell.clone());
        }
    }
    after.retain(|_, cols| !cols.is_empty());
    let mut summary = Summary {
        examples: Vec::new(),
        changed_cells: 0,
        changed_blocks: 0,
        detached_aliases: 0,
        affected_positions: Vec::new(),
        outside_scope_positions: Vec::new(),
    };
    let keys: BTreeSet<_> = before
        .iter()
        .chain(&after)
        .flat_map(|(r, cols)| cols.keys().map(move |c| (*r, *c)))
        .collect();
    for (row, column) in keys {
        let a = before
            .get(&row)
            .and_then(|c| c.get(&column))
            .cloned()
            .unwrap_or_default();
        let b = after
            .get(&row)
            .and_then(|c| c.get(&column))
            .cloned()
            .unwrap_or_default();
        if a != b {
            summary.changed_cells += 1;
            if summary.examples.len() < 12 {
                summary.examples.push(CellChange {
                    pattern,
                    track: target.track,
                    row,
                    column,
                    before: a,
                    after: b,
                });
            }
        }
    }
    let mut candidate = song.clone();
    if summary.changed_cells > 0 {
        summary.changed_blocks = 1;
        let p = candidate.patterns.get_mut(&pattern).unwrap();
        if p.aliases.remove(&target.track).is_some() {
            summary.detached_aliases = 1;
        }
        if after.is_empty() {
            p.tracks.remove(&target.track);
        } else {
            p.tracks.insert(target.track, after);
        }
    }
    let recipe_changed = song.generation_recipes.get(name) != Some(recipe);
    candidate
        .generation_recipes
        .insert(name.into(), recipe.clone());
    candidate.validate()?;
    for (i, o) in song.order.iter().enumerate() {
        if song.track_order.iter().any(|t| {
            song.block(o.pattern, *t).ok().flatten()
                != candidate.block(o.pattern, *t).ok().flatten()
        }) {
            summary.affected_positions.push(i);
            if i != target.occurrence {
                summary.outside_scope_positions.push(i);
            }
        }
    }
    Ok(Preview {
        summary,
        generated,
        recipe_changed,
        original: song.clone(),
        candidate,
    })
}
