//! A bounded projection/editor for ordinary tracker cells, not a second sequencer.
//! Page zero starts at pattern row zero. Retriggers remain one parent source cell.
//! Unknown incoming selection is never inferred from a particular order occurrence.
use crate::song::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_SCAN_CELLS: usize = 65_536;
pub const MAX_PAGE_EVENTS: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepDivision {
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
    OneTwentyEighth,
}
impl StepDivision {
    pub fn steps_per_beat(self) -> u32 {
        match self {
            Self::Quarter => 1,
            Self::Eighth => 2,
            Self::Sixteenth => 4,
            Self::ThirtySecond => 8,
            Self::SixtyFourth => 16,
            Self::OneTwentyEighth => 32,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepView {
    pub pattern: PatternId,
    pub track: TrackId,
    /// An explicit instrument, not just a shared kit definition.
    pub instrument: InstrumentId,
    pub division: StepDivision,
    pub steps: u8,
    pub page: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepEvent {
    pub pad: PadId,
    pub row: u32,
    pub column: u16,
    pub cell: Cell,
    pub on_grid: bool,
    pub instrument: InstrumentId,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepSlot {
    pub step: u8,
    pub row: u32,
    pub delay: f64,
    pub in_bounds: bool,
    pub events: Vec<StepEvent>,
    pub conflicts: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PadLane {
    pub pad: PadId,
    pub slot: u8,
    pub name: String,
    pub note: u8,
    pub mapped: bool,
    pub steps: Vec<StepSlot>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepIssue {
    pub row: u32,
    pub column: u16,
    pub cell: Cell,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepProjection {
    pub view: StepView,
    /// Terminal content identity; a differing source is displayed read-only.
    pub source: TrackAlias,
    pub read_only: bool,
    pub total_steps: u32,
    pub total_pages: u32,
    pub lanes: Vec<PadLane>,
    pub issues: Vec<StepIssue>,
    /// Opaque exact-content guard. Never serialized into inspect responses.
    #[serde(skip)]
    base: Song,
    #[serde(skip)]
    base_view: StepView,
}
impl StepProjection {
    pub fn is_current(&self, song: &Song) -> bool {
        &self.base == song
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum StepEdit {
    Toggle {
        pad: PadId,
        step: u8,
    },
    SetVelocity {
        event: StepEvent,
        value: Option<f32>,
    },
    /// Existing tracker units: absolute fraction of the source row, not a step.
    SetDelay {
        event: StepEvent,
        value: Option<f64>,
    },
    SetRetrigger {
        event: StepEvent,
        value: Option<Retrigger>,
    },
}
fn anchor(step: u32, lpb: u16, division: StepDivision) -> (u32, f64) {
    let numerator = u64::from(step) * u64::from(lpb);
    let denominator = u64::from(division.steps_per_beat());
    (
        (numerator / denominator) as u32,
        (numerator % denominator) as f64 / denominator as f64,
    )
}
fn at_or_after(row: u32, delay: f64, start: (u32, f64)) -> bool {
    row > start.0 || (row == start.0 && delay >= start.1)
}
fn other_effects(cell: &Cell) -> bool {
    let mut effects = cell.effects.clone();
    effects.retrigger = None;
    !effects.is_empty()
}
fn selected_by(song: &Song, cell: &Cell) -> Option<InstrumentId> {
    cell.phrase
        .as_ref()
        .map(|p| song.phrases[&p.phrase].instrument)
        .or(cell.instrument)
}
/// Projection never changes timing or content. Off-grid/duplicate hits retain exact
/// source references for explicit field editing; ambiguous toggles are rejected.
pub fn project(song: &Song, view: &StepView) -> Result<StepProjection, String> {
    let mut scanned = 0usize;
    for pattern in song.patterns.values() {
        for columns in pattern.tracks.values().flat_map(|rows| rows.values()) {
            scanned = scanned.saturating_add(columns.len());
            if scanned > MAX_SCAN_CELLS {
                return Err(
                    "Drum step projection exceeds 65536 stored pattern cells; use the tracker"
                        .into(),
                );
            }
        }
    }
    song.validate()?;
    for (&id, pattern) in &song.patterns {
        for &track in pattern.track_aliases.keys() {
            if let Some(rows) = song.resolved_track(id, track)? {
                scanned = scanned.saturating_add(rows.values().map(BTreeMap::len).sum::<usize>());
                if scanned > MAX_SCAN_CELLS {
                    return Err("Drum step projection exceeds 65536 stored/resolved pattern cells; use the tracker".into());
                }
            }
        }
    }
    if !matches!(view.steps, 16 | 32) {
        return Err("Drum step page length must be 16 or 32".into());
    }
    let pattern = song
        .patterns
        .get(&view.pattern)
        .ok_or("Unknown step pattern")?;
    song.tracks.get(&view.track).ok_or("Unknown step track")?;
    let (source_pattern, source_track) = song.resolve_track_source(view.pattern, view.track)?;
    let read_only = (source_pattern, source_track) != (view.pattern, view.track);
    let instrument = song
        .instruments
        .get(&view.instrument)
        .ok_or("Unknown step instrument")?;
    let InstrumentKind::DrumKit { kit } = instrument.kind else {
        return Err("Step view requires an explicit drum-kit instrument".into());
    };
    let kit = &song.drum_kits[&kit];
    let total_steps =
        (pattern.rows * view.division.steps_per_beat()).div_ceil(u32::from(song.lines_per_beat));
    let total_pages = total_steps.div_ceil(u32::from(view.steps));
    if view.page >= total_pages {
        return Err("Drum step page is outside the selected pattern".into());
    }
    let first = view.page * u32::from(view.steps);
    let start = anchor(first, song.lines_per_beat, view.division);
    let end = anchor(
        first + u32::from(view.steps),
        song.lines_per_beat,
        view.division,
    );
    let slots: Vec<_> = (0..view.steps)
        .map(|step| {
            let absolute = first + u32::from(step);
            let (row, delay) = anchor(absolute, song.lines_per_beat, view.division);
            StepSlot {
                step,
                row,
                delay,
                in_bounds: absolute < total_steps,
                events: Vec::new(),
                conflicts: Vec::new(),
            }
        })
        .collect();
    let mut result = StepProjection {
        view: view.clone(),
        source: TrackAlias {
            pattern: source_pattern,
            track: source_track,
        },
        read_only,
        total_steps,
        total_pages,
        lanes: kit
            .pads
            .iter()
            .map(|(&pad, data)| PadLane {
                pad,
                slot: data.slot,
                name: data.name.clone(),
                note: data.note,
                mapped: data.has_source(),
                steps: slots.clone(),
            })
            .collect(),
        issues: Vec::new(),
        base: song.clone(),
        base_view: view.clone(),
    };
    result.lanes.sort_by_key(|lane| lane.slot);
    let mut selected = BTreeMap::new();
    let mut records = 0usize;
    if let Some(rows) = song.resolved_track(view.pattern, view.track)? {
        for (&row, columns) in rows {
            for (&column, cell) in columns {
                if let Some(instrument) = selected_by(song, cell) {
                    selected.insert(column, instrument);
                }
                let delay = cell.delay.unwrap_or(0.0);
                let in_page = at_or_after(row, delay, start) && !at_or_after(row, delay, end);
                let owned_phrase = cell
                    .phrase
                    .as_ref()
                    .is_some_and(|p| song.phrases[&p.phrase].instrument == view.instrument);
                if (!in_page && !owned_phrase) || *cell == Cell::default() {
                    continue;
                }
                records += 1;
                if records > MAX_PAGE_EVENTS {
                    return Err(
                        "Drum step page exceeds 2048 source events/issues; use the tracker".into(),
                    );
                }
                let step = slots
                    .iter()
                    .rposition(|slot| at_or_after(row, delay, (slot.row, slot.delay)));
                if cell.phrase.is_some() {
                    result.issues.push(StepIssue {
                        row,
                        column,
                        cell: cell.clone(),
                        reason:
                            "Phrase trigger: expanded hits are not independently editable steps"
                                .into(),
                    });
                    if owned_phrase {
                        for lane in &mut result.lanes {
                            for slot in &mut lane.steps {
                                if !slot.conflicts.iter().any(|s| {
                                    s == "Kit phrase in this pattern; edit its notes in the tracker"
                                }) {
                                    slot.conflicts.push(
                                        "Kit phrase in this pattern; edit its notes in the tracker"
                                            .into(),
                                    );
                                }
                            }
                        }
                    }
                    continue;
                }
                let selected_instrument = selected.get(&column).copied();
                if let Note::Note(note) = cell.note {
                    if let Some(lane) = result.lanes.iter_mut().find(|lane| lane.note == note) {
                        let slot =
                            &mut lane.steps[step.ok_or("Step projection position mismatch")?];
                        if selected_instrument == Some(view.instrument) {
                            let on_grid = row == slot.row && delay == slot.delay;
                            slot.events.push(StepEvent {
                                pad: lane.pad,
                                row,
                                column,
                                cell: cell.clone(),
                                on_grid,
                                instrument: view.instrument,
                            });
                            if !on_grid {
                                slot.conflicts.push("Off-grid hit: edit the exact event or use the tracker; never snapped".into());
                            }
                            if other_effects(cell) {
                                slot.conflicts.push(
                                    "Additional cell commands: use the tracker to remove this hit"
                                        .into(),
                                );
                            }
                            continue;
                        }
                        if selected_instrument.is_none() {
                            slot.conflicts.push("Unknown incoming instrument selection; make it explicit in the tracker".into());
                        }
                    }
                }
                result.issues.push(StepIssue {
                    row,
                    column,
                    cell: cell.clone(),
                    reason: if selected_instrument.is_none() {
                        "Unknown incoming instrument selection or parameter-only cell"
                    } else {
                        "Other instrument or parameter-only cell; preserved outside pad steps"
                    }
                    .into(),
                });
            }
        }
    }
    for lane in &mut result.lanes {
        for slot in &mut lane.steps {
            if read_only {
                slot.conflicts.push(
                    "Aliased track is read-only: Make unique or explicitly edit its source".into(),
                );
            }
            if slot.events.len() > 1 {
                slot.conflicts.push(
                    "Multiple source hits in one step; select an exact event or use the tracker"
                        .into(),
                );
            }
        }
    }
    Ok(result)
}
fn selection_before(song: &Song, view: &StepView, row: u32, column: u16) -> Option<InstrumentId> {
    song.patterns[&view.pattern]
        .tracks
        .get(&view.track)
        .into_iter()
        .flat_map(|rows| rows.range(..row))
        .filter_map(|(_, columns)| columns.get(&column))
        .filter_map(|cell| selected_by(song, cell))
        .next_back()
}
/// A new explicit selection must not silently rebind later omitted-instrument notes.
fn safe_selection(song: &Song, view: &StepView, row: u32, column: u16) -> bool {
    let pattern = &song.patterns[&view.pattern];
    let cells: Vec<_> = pattern
        .tracks
        .get(&view.track)
        .into_iter()
        .flat_map(|rows| rows.iter())
        .filter_map(|(&r, columns)| columns.get(&column).map(|c| (r, c)))
        .collect();
    if selection_before(song, view, row, column) == Some(view.instrument) {
        return true;
    }
    for (_, cell) in cells.iter().filter(|(r, _)| *r > row) {
        if selected_by(song, cell).is_some() {
            return true;
        }
        if matches!(cell.note, Note::Note(_)) {
            return false;
        }
    }
    // Selection would survive the pattern end. Protect every possible next
    // pattern's unknown incoming prefix, including repeated/unused patterns.
    for &pattern in song.patterns.keys() {
        let Ok(rows) = song.resolved_track(pattern, view.track) else {
            return false;
        };
        for cell in rows
            .into_iter()
            .flat_map(|rows| rows.values())
            .filter_map(|columns| columns.get(&column))
        {
            if selected_by(song, cell).is_some() {
                break;
            }
            if matches!(cell.note, Note::Note(_)) {
                return false;
            }
        }
    }
    true
}
fn write_cell(song: &mut Song, view: &StepView, row: u32, column: u16, cell: Cell) {
    let pattern = song.patterns.get_mut(&view.pattern).unwrap();
    if cell == Cell::default() {
        if let Some(rows) = pattern.tracks.get_mut(&view.track) {
            if let Some(columns) = rows.get_mut(&row) {
                columns.remove(&column);
                if columns.is_empty() {
                    rows.remove(&row);
                }
            }
            if rows.is_empty() {
                pattern.tracks.remove(&view.track);
            }
        }
        return;
    }
    pattern
        .tracks
        .entry(view.track)
        .or_default()
        .entry(row)
        .or_default()
        .insert(column, cell);
}
/// Apply against the exact projected base. Callers also guard document revisions
/// to reject Undo/Redo ABA and replacing a document with identical content.
pub fn apply(song: &Song, projection: &StepProjection, edit: &StepEdit) -> Result<Song, String> {
    if !projection.is_current(song) || projection.view != projection.base_view {
        return Err("Drum step projection is stale; inspect again".into());
    }
    song.require_local_track(projection.view.pattern, projection.view.track)?;
    let fresh = project(song, &projection.view)?;
    let view = &fresh.view;
    let mut candidate = song.clone();
    match edit {
        StepEdit::Toggle { pad, step } => {
            let lane = fresh
                .lanes
                .iter()
                .find(|lane| lane.pad == *pad)
                .ok_or("Unknown step pad")?;
            if !lane.mapped {
                return Err("Assign source regions before inserting a pad step".into());
            }
            let slot = lane
                .steps
                .get(usize::from(*step))
                .ok_or("Step index outside page")?;
            if !slot.in_bounds {
                return Err("Step is outside the pattern length".into());
            }
            if !slot.conflicts.is_empty() {
                return Err(format!(
                    "Step is not safely toggleable: {}",
                    slot.conflicts.join("; ")
                ));
            }
            if let Some(event) = slot.events.first() {
                let mut cell = event.cell.clone();
                cell.note = Note::Empty;
                cell.effects.retrigger = None;
                write_cell(&mut candidate, view, event.row, event.column, cell);
            } else {
                let rows = song.patterns[&view.pattern].tracks.get(&view.track);
                let mut placement = None;
                for column in 0..song.tracks[&view.track].note_columns {
                    let mut cell = rows
                        .and_then(|rows| rows.get(&slot.row))
                        .and_then(|columns| columns.get(&column))
                        .cloned()
                        .unwrap_or_default();
                    let reclaimable = cell.note == Note::Empty
                        && (cell.instrument == Some(view.instrument)
                            || (cell.instrument.is_none()
                                && selection_before(song, view, slot.row, column)
                                    == Some(view.instrument)))
                        && cell.phrase.is_none()
                        && cell.effects.is_empty()
                        && cell.delay.unwrap_or(0.0) == slot.delay;
                    if (cell == Cell::default() || reclaimable)
                        && (reclaimable || safe_selection(song, view, slot.row, column))
                    {
                        cell.note = Note::Note(lane.note);
                        if !reclaimable {
                            cell.instrument = Some(view.instrument);
                        }
                        if !reclaimable && slot.delay != 0.0 {
                            cell.delay = Some(slot.delay);
                        }
                        placement = Some((column, cell));
                        break;
                    }
                }
                let (column, cell) = placement.ok_or("No safe existing note column: occupied cells or incoming instrument memory would be changed; use the tracker")?;
                write_cell(&mut candidate, view, slot.row, column, cell);
            }
        }
        StepEdit::SetVelocity { event, .. }
        | StepEdit::SetDelay { event, .. }
        | StepEdit::SetRetrigger { event, .. } => {
            if !fresh
                .lanes
                .iter()
                .flat_map(|lane| &lane.steps)
                .flat_map(|slot| &slot.events)
                .any(|actual| actual == event)
            {
                return Err("Step event reference is stale or not owned by this view".into());
            }
            let mut cell = event.cell.clone();
            match edit {
                StepEdit::SetVelocity { value, .. } => cell.velocity = *value,
                StepEdit::SetDelay { value, .. } => cell.delay = *value,
                StepEdit::SetRetrigger { value, .. } => cell.effects.retrigger = value.clone(),
                _ => unreachable!(),
            }
            write_cell(&mut candidate, view, event.row, event.column, cell);
        }
    }
    candidate.validate()?;
    Ok(candidate)
}
