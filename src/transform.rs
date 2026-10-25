//! Deterministic committed pattern edits. Preview is pure; Editor owns the transaction.
use crate::song::*;
use std::{collections::BTreeSet, ops::Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Column,
    Track,
    Pattern,
    Section,
    Song,
}
impl Scope {
    pub const ALL: [Self; 5] = [
        Self::Column,
        Self::Track,
        Self::Pattern,
        Self::Section,
        Self::Song,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Column => "Selected column",
            Self::Track => "Selected track",
            Self::Pattern => "Selected pattern",
            Self::Section => "Current section",
            Self::Song => "Ordered song",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum Transform {
    Transpose(i16),
    /// Only explicitly stored values; None retains column-memory semantics.
    ScaleVelocity(f32),
    OffsetPan(f32),
    /// Absolute row phase, replaces delay on every nonempty stored cell.
    Groove(Vec<f64>),
    /// Mirror row positions inside the selected range; preserve columns/delay/FX.
    ReverseRows,
}
impl Transform {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Transpose(n) if !(-127..=127).contains(n) => {
                Err("Transpose must be -127..127 semitones".into())
            }
            Self::ScaleVelocity(v) if !v.is_finite() || !(0.0..=4.0).contains(v) => {
                Err("Velocity factor must be finite in 0..4".into())
            }
            Self::OffsetPan(v) if !v.is_finite() || !(-2.0..=2.0).contains(v) => {
                Err("Pan offset must be finite in -2..2".into())
            }
            Self::Groove(values)
                if values.is_empty()
                    || values.len() > 16
                    || values
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..1.0).contains(v)) =>
            {
                Err("Groove needs 1..16 row delays, each finite in 0..<1".into())
            }
            _ => Ok(()),
        }
    }
}
/// The row interval is zero-based and end-exclusive for column/track/pattern.
/// Section/song deliberately use all rows of each distinct ordered pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub occurrence: usize,
    pub track: TrackId,
    pub column: u16,
    pub scope: Scope,
    pub rows: Range<u32>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CellChange {
    pub pattern: PatternId,
    pub track: TrackId,
    pub row: u32,
    pub column: u16,
    pub before: Cell,
    pub after: Cell,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub examples: Vec<CellChange>,
    pub changed_cells: usize,
    pub changed_blocks: usize,
    pub detached_aliases: usize,
    pub affected_positions: Vec<usize>,
    pub outside_scope_positions: Vec<usize>,
}
#[derive(Debug, Clone)]
pub struct Preview {
    pub summary: Summary,
    original: Song,
    candidate: Song,
}
impl Preview {
    pub fn is_current(&self, song: &Song) -> bool {
        &self.original == song
    }
    pub(crate) fn into_song(self, song: &Song) -> Result<Song, String> {
        if !self.is_current(song) {
            return Err("Song changed since preview; preview again".into());
        }
        Ok(self.candidate)
    }
}
fn changed_cells(a: &TrackPattern, b: &TrackPattern) -> usize {
    let keys: BTreeSet<_> = a
        .iter()
        .chain(b)
        .flat_map(|(r, cols)| cols.keys().map(move |c| (*r, *c)))
        .collect();
    keys.into_iter()
        .filter(|(r, c)| {
            a.get(r).and_then(|v| v.get(c)).cloned().unwrap_or_default()
                != b.get(r).and_then(|v| v.get(c)).cloned().unwrap_or_default()
        })
        .count()
}
fn transform_block(
    block: &TrackPattern,
    rows: Range<u32>,
    column: Option<u16>,
    operation: &Transform,
) -> Result<TrackPattern, String> {
    let mut result = block.clone();
    for (row, cols) in block.range(rows.clone()) {
        for (col, cell) in cols {
            if column.is_some_and(|c| c != *col) || *cell == Cell::default() {
                continue;
            }
            let mut cell = cell.clone();
            match operation {
                Transform::Transpose(n) => {
                    if let Note::Note(pitch) = cell.note {
                        let pitch = pitch as i16 + *n;
                        if !(0..=127).contains(&pitch) {
                            return Err(format!(
                                "Transposition exceeds MIDI range at row {row}, column {col}"
                            ));
                        }
                        cell.note = Note::Note(pitch as u8);
                    }
                }
                Transform::ScaleVelocity(factor) => {
                    if let Some(value) = cell.velocity.as_mut() {
                        *value *= factor;
                    }
                }
                Transform::OffsetPan(offset) => {
                    if let Some(value) = cell.pan.as_mut() {
                        *value += offset;
                    }
                }
                Transform::Groove(values) => {
                    let value = values[*row as usize % values.len()];
                    cell.delay = if value == 0.0 { None } else { Some(value) };
                }
                Transform::ReverseRows => (),
            }
            cell.validate_values()
                .map_err(|e| format!("Row {row}, column {col}: {e}"))?;
            if matches!(operation, Transform::ReverseRows) {
                // Remove all selected originals before inserting mirrored cells below.
                result.get_mut(row).unwrap().remove(col);
            } else {
                result.entry(*row).or_default().insert(*col, cell);
            }
        }
    }
    if matches!(operation, Transform::ReverseRows) {
        for (row, cols) in block.range(rows.clone()) {
            for (col, cell) in cols {
                if column.is_none_or(|c| c == *col) && *cell != Cell::default() {
                    result
                        .entry(rows.start + rows.end - 1 - *row)
                        .or_default()
                        .insert(*col, cell.clone());
                }
            }
        }
    }
    result.retain(|_, cols| !cols.is_empty());
    Ok(result)
}
/// Aliased destinations are transformed from the original resolved content and
/// detached only if their cells change. Repeats are processed once per pattern.
pub fn preview(
    song: &Song,
    selection: &Selection,
    operation: &Transform,
) -> Result<Preview, String> {
    song.validate()?;
    operation.validate()?;
    let occurrence = song
        .order
        .get(selection.occurrence)
        .ok_or("Unknown occurrence")?;
    let track = song
        .tracks
        .get(&selection.track)
        .ok_or("Unknown selected track")?;
    if selection.column >= track.note_columns {
        return Err("Unknown selected column".into());
    }
    let positions: BTreeSet<_> = match selection.scope {
        Scope::Section => song.section_range(selection.occurrence)?.collect(),
        Scope::Song => (0..song.order.len()).collect(),
        _ => BTreeSet::from([selection.occurrence]),
    };
    let patterns: BTreeSet<_> = positions.iter().map(|i| song.order[*i].pattern).collect();
    let wide = matches!(selection.scope, Scope::Section | Scope::Song);
    if !wide
        && (selection.rows.start >= selection.rows.end
            || selection.rows.end > song.patterns[&occurrence.pattern].rows)
    {
        return Err("Choose a nonempty row interval inside this pattern".into());
    }
    let mut candidate = song.clone();
    let mut summary = Summary {
        examples: Vec::new(),
        changed_cells: 0,
        changed_blocks: 0,
        detached_aliases: 0,
        affected_positions: Vec::new(),
        outside_scope_positions: Vec::new(),
    };
    for pattern in patterns {
        let rows = if wide {
            0..song.patterns[&pattern].rows
        } else {
            selection.rows.clone()
        };
        let tracks = if matches!(selection.scope, Scope::Column | Scope::Track) {
            vec![selection.track]
        } else {
            song.track_order.clone()
        };
        for track in tracks {
            let before = song.block(pattern, track)?.cloned().unwrap_or_default();
            let after = transform_block(
                &before,
                rows.clone(),
                if selection.scope == Scope::Column {
                    Some(selection.column)
                } else {
                    None
                },
                operation,
            )?;
            let count = changed_cells(&before, &after);
            if count == 0 {
                continue;
            }
            if summary.examples.len() < 12 {
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
                        summary.examples.push(CellChange {
                            pattern,
                            track,
                            row,
                            column,
                            before: a,
                            after: b,
                        });
                    }
                    if summary.examples.len() >= 12 {
                        break;
                    }
                }
            }
            summary.changed_cells += count;
            summary.changed_blocks += 1;
            let target = candidate.patterns.get_mut(&pattern).unwrap();
            if target.aliases.remove(&track).is_some() {
                summary.detached_aliases += 1;
            }
            if after.is_empty() {
                target.tracks.remove(&track);
            } else {
                target.tracks.insert(track, after);
            }
        }
    }
    // Include downstream aliases as well as direct pattern repeats in the preview.
    for (i, o) in song.order.iter().enumerate() {
        let mut affected = false;
        for track in &song.track_order {
            if song.block(o.pattern, *track)? != candidate.block(o.pattern, *track)? {
                affected = true;
                break;
            }
        }
        if affected {
            summary.affected_positions.push(i);
            if !positions.contains(&i) {
                summary.outside_scope_positions.push(i);
            }
        }
    }
    candidate.validate()?;
    Ok(Preview {
        summary,
        original: song.clone(),
        candidate,
    })
}
