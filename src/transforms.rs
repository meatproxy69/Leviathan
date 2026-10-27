//! Pure scoped edits baked into ordinary cells. No IO, random state, runtime
//! groove, new event authority or DSP. Preview/commit never writes through aliases.
use crate::{
    break_compiler::{timing::Rational, Subdivision},
    song::*,
    RATE,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const ALGORITHM: &str = "scoped-cell-transforms-v1";
const MAX_CELLS: usize = 4096;
const MAX_CHECKS: usize = 65536;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformScope {
    pub pattern: PatternId,
    pub track: TrackId,
    pub row_start: u32,
    pub row_end: u32,
    pub columns: Vec<u16>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformFields {
    pub notes: bool,
    pub glide_targets: bool,
    pub velocity: bool,
    pub timing: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformSwing {
    pub division: Subdivision,
    pub amount_percent: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformConfig {
    pub scope: TransformScope,
    pub fields: TransformFields,
    pub transpose_semitones: i8,
    pub velocity_scale: f32,
    pub nudge_frames: i32,
    pub swing: Option<TransformSwing>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellChange {
    pub from_row: u32,
    pub to_row: u32,
    pub column: u16,
    pub before: Cell,
    pub after: Cell,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformSummary {
    pub algorithm: String,
    pub selected_cells: usize,
    pub changed_cells: usize,
    pub moved_cells: usize,
    pub note_changes: usize,
    pub glide_changes: usize,
    pub velocity_changes: usize,
    pub timing_changes: usize,
    pub skipped_unspecified_velocity: usize,
    pub affected_occurrences: Vec<usize>,
    pub affected_aliases: Vec<TrackAlias>,
    pub swing_frames: u32,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct TransformPreview {
    id: String,
    base: Song,
    candidate: Song,
    config: TransformConfig,
    summary: TransformSummary,
    changes: Vec<CellChange>,
}
impl TransformPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &TransformConfig {
        &self.config
    }
    pub fn summary(&self) -> &TransformSummary {
        &self.summary
    }
    pub fn changes(&self) -> &[CellChange] {
        &self.changes
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base {
            return Err("Transform preview is stale; prepare a new preview".into());
        }
        Ok(self.candidate.clone())
    }
}
#[derive(Default)]
struct Column {
    selected: Option<InstrumentId>,
    active: Option<InstrumentId>,
    phrase: bool,
}
struct Selected<'a> {
    row: u32,
    column: u16,
    cell: &'a Cell,
    note_owner: Option<InstrumentId>,
    glide_owner: Option<InstrumentId>,
}
#[derive(Clone, Copy)]
pub(crate) struct Use {
    pub(crate) prefix: u64,
    pub(crate) end: u64,
}
pub(crate) fn validate_config(song: &Song, config: &TransformConfig) -> Result<(), String> {
    song.validate()?;
    let s = &config.scope;
    song.require_local_track(s.pattern, s.track)?;
    let pattern = &song.patterns[&s.pattern];
    if s.row_start >= s.row_end || s.row_end > pattern.rows {
        return Err(
            "Transform rows must be a nonempty [start,end) range inside the pattern".into(),
        );
    }
    if s.columns.is_empty()
        || s.columns.len() > 256
        || s.columns.windows(2).any(|w| w[0] >= w[1])
        || s.columns
            .iter()
            .any(|c| *c >= song.tracks[&s.track].note_columns)
    {
        return Err(
            "Transform columns must be nonempty, strictly sorted, unique and within the track"
                .into(),
        );
    }
    if !(-48..=48).contains(&config.transpose_semitones)
        || !config.velocity_scale.is_finite()
        || !(0.0..=4.0).contains(&config.velocity_scale)
        || !(-48000..=48000).contains(&config.nudge_frames)
        || config.swing.is_some_and(|s| s.amount_percent > 50)
    {
        return Err("Transform parameters require transpose -48..48, finite velocity scale 0..4, nudge +/-48000 output frames and swing 0..50 percent".into());
    }
    Ok(())
}
fn selected<'a>(song: &'a Song, scope: &TransformScope) -> Result<Vec<Selected<'a>>, String> {
    let Some(rows) = song.patterns[&scope.pattern].tracks.get(&scope.track) else {
        return Ok(Vec::new());
    };
    let mut states: BTreeMap<u16, Column> = BTreeMap::new();
    let mut selected = Vec::new();
    let mut scanned = 0usize;
    for (&row, columns) in rows.range(..scope.row_end) {
        for (&column, cell) in columns {
            scanned += 1;
            if scanned > MAX_CHECKS {
                return Err("Transform ownership scan exceeds 65536 cells".into());
            }
            if scope.columns.binary_search(&column).is_err() {
                continue;
            }
            let state = states.entry(column).or_default();
            if let Some(id) = cell.instrument {
                state.selected = Some(id);
            }
            let note_owner = if let Some(trigger) = &cell.phrase {
                Some(song.phrases[&trigger.phrase].instrument)
            } else {
                state.selected
            };
            match cell.note {
                Note::Note(_) => {
                    if cell.phrase.is_some() {
                        state.selected = note_owner;
                        state.active = None;
                        state.phrase = true;
                    } else {
                        state.active = state.selected;
                        state.phrase = false;
                    }
                }
                Note::Off => {
                    state.active = None;
                    state.phrase = false;
                }
                Note::Empty => (),
            }
            if row >= scope.row_start {
                if selected.len() >= MAX_CELLS {
                    return Err("Transform selection exceeds 4096 stored cells".into());
                }
                selected.push(Selected {
                    row,
                    column,
                    cell,
                    note_owner,
                    glide_owner: if state.phrase { None } else { state.active },
                });
            }
            if cell.effects.release.is_some() || cell.effects.cut.is_some() {
                state.active = None;
                state.phrase = false;
            }
        }
    }
    Ok(selected)
}
type Contexts = (Vec<Use>, Vec<usize>, Vec<TrackAlias>);
fn contexts(song: &Song, scope: &TransformScope, per_row: f64) -> Result<Contexts, String> {
    let mut patterns = BTreeSet::from([scope.pattern]);
    let mut aliases = Vec::new();
    for (&pattern, p) in &song.patterns {
        for &track in p.track_aliases.keys() {
            if song.resolve_track_source(pattern, track)? == (scope.pattern, scope.track) {
                patterns.insert(pattern);
                aliases.push(TrackAlias { pattern, track });
            }
        }
    }
    let end = (song
        .order
        .iter()
        .map(|o| u64::from(song.patterns[&o.pattern].rows))
        .sum::<u64>() as f64
        * per_row)
        .round() as u64;
    let mut uses = Vec::new();
    let mut occurrences = Vec::new();
    let mut prefix = 0u64;
    for (index, occurrence) in song.order.iter().enumerate() {
        if patterns.contains(&occurrence.pattern) {
            if uses.len() >= 4096 {
                return Err("Transform affects more than 4096 ordered uses".into());
            }
            uses.push(Use { prefix, end });
            occurrences.push(index);
        }
        prefix += u64::from(song.patterns[&occurrence.pattern].rows);
    }
    if uses.is_empty() {
        uses.push(Use {
            prefix: 0,
            end: (f64::from(song.patterns[&scope.pattern].rows) * per_row).round() as u64,
        });
    }
    Ok((uses, occurrences, aliases))
}
fn shifted_pitch(
    song: &Song,
    owner: Option<InstrumentId>,
    pitch: u8,
    shift: i8,
    kind: &str,
) -> Result<u8, String> {
    let owner=owner.ok_or_else(||format!("{kind} transpose requires a locally proven pitched voice/instrument; make selection explicit"))?;
    if !song.instruments[&owner].kind.is_melodic() {
        return Err(format!("{kind} transpose cannot change drum selectors or kit glide targets; no pad-retarget mapping is requested"));
    }
    u8::try_from(i16::from(pitch) + i16::from(shift))
        .ok()
        .filter(|p| *p <= 127)
        .ok_or_else(|| format!("{kind} transpose leaves MIDI 0..127; no clamping"))
}
fn swing_frames(song: &Song, config: &TransformConfig) -> Result<u32, String> {
    let Some(swing) = config.swing.filter(|_| config.fields.timing) else {
        return Ok(0);
    };
    let result = Rational::new(
        u128::from(RATE) * 60 * 4 * u128::from(swing.amount_percent),
        u128::from(swing.division.denominator()) * 100,
    )
    .div(Rational::bpm(song.bpm))?
    .round()?;
    u32::try_from(result).map_err(|_| "Swing displacement exceeds frame range".into())
}
fn odd_bin(song: &Song, row: u32, cell: &Cell, swing: TransformSwing) -> bool {
    // Division is a power of two: scaling a stored f64 delay is exact. Integer
    // floor before the final division avoids erasing tiny off-grid fractions.
    let division = u64::from(swing.division.denominator());
    let units =
        u64::from(row) * division + (cell.delay.unwrap_or(0.0) * division as f64).floor() as u64;
    (units / (u64::from(song.lines_per_beat) * 4)) % 2 == 1
}
pub(crate) fn fractions(cell: &Cell) -> Vec<f64> {
    let delay = cell.delay.unwrap_or(0.0);
    let mut out = vec![delay];
    if let Some(r) = &cell.effects.retrigger {
        for repeat in 1..=r.repeats {
            out.push(delay + f64::from(repeat) * r.spacing);
        }
    }
    let last = *out.last().unwrap();
    if let Some(value) = cell.effects.release {
        out.push(value.max(last));
    }
    if let Some(value) = cell.effects.cut {
        out.push(value.max(last));
    }
    out
}
pub(crate) fn frame(use_: Use, row: u32, fraction: f64, per_row: f64) -> u64 {
    ((((use_.prefix + u64::from(row)) as f64 + fraction) * per_row).round() as u64)
        .min(use_.end.saturating_sub(1))
}
fn candidates(center: f64) -> Vec<f64> {
    let mut values = vec![center];
    let mut low = center;
    let mut high = center;
    for _ in 0..32 {
        low = low.next_down();
        high = high.next_up();
        values.push(low);
        values.push(high);
    }
    values
}
fn desired(use_: Use, row: u32, fraction: f64, delta: i64, per_row: f64) -> Result<u64, String> {
    let value = i128::from(frame(use_, row, fraction, per_row)) + i128::from(delta);
    if value < 0 || value >= i128::from(use_.end) {
        return Err("Timing displacement leaves the musical bounds of an ordered use".into());
    }
    Ok(value as u64)
}
struct Clock<'a> {
    uses: &'a [Use],
    per_row: f64,
    comparisons: usize,
}
impl Clock<'_> {
    fn tick(&mut self) -> Result<(), String> {
        self.comparisons += 1;
        if self.comparisons > 4_194_304 {
            Err("Transform timing encoding exceeds 4194304 bounded frame comparisons; narrow the selection".into())
        } else {
            Ok(())
        }
    }
}
fn move_cell(
    song: &Song,
    scope: &TransformScope,
    row: u32,
    before: &Cell,
    after: &Cell,
    delta: i64,
    clock: &mut Clock<'_>,
) -> Result<(u32, Cell), String> {
    let uses = clock.uses;
    let per_row = clock.per_row;
    if before.phrase.is_some() {
        return Err("Timing transforms of phrase triggers are not supported; transpose its pitched trigger or narrow the selection".into());
    }
    let old = fractions(before);
    let expected: Vec<Vec<u64>> = uses
        .iter()
        .map(|&u| {
            old.iter()
                .map(|&fraction| desired(u, row, fraction, delta, per_row))
                .collect()
        })
        .collect::<Result<_, _>>()?;
    let shift = delta as f64 / per_row;
    let center = f64::from(row) + before.delay.unwrap_or(0.0) + shift;
    if center < f64::from(scope.row_start) || center >= f64::from(scope.row_end) {
        return Err("Ideal timing destination leaves the selected [start,end) row interval; no boundary clamping".into());
    }
    let intended_row = center.floor() as u32;
    if let Some(retrigger) = &before.effects.retrigger {
        let ideal_last = center + f64::from(retrigger.repeats) * retrigger.spacing;
        if !(0.0..1.0).contains(&(ideal_last - f64::from(intended_row))) {
            return Err("Ideal row-local FX retrigger span leaves the destination row; no boundary clamping".into());
        }
    }
    for absolute in candidates(center) {
        if absolute < f64::from(scope.row_start) || absolute >= f64::from(scope.row_end) {
            continue;
        }
        let new_row = absolute.floor() as u32;
        if new_row != intended_row {
            continue;
        }
        let mut cell = after.clone();
        cell.delay = Some(absolute - f64::from(new_row));
        let last = cell
            .effects
            .retrigger
            .as_ref()
            .map_or(cell.delay.unwrap(), |r| {
                cell.delay.unwrap() + f64::from(r.repeats) * r.spacing
            });
        if last >= 1.0 {
            continue;
        }
        let mut possible = true;
        for cut in [false, true] {
            let value = if cut {
                before.effects.cut
            } else {
                before.effects.release
            };
            let Some(value) = value else { continue };
            let old_last = before
                .effects
                .retrigger
                .as_ref()
                .map_or(before.delay.unwrap_or(0.0), |r| {
                    before.delay.unwrap_or(0.0) + f64::from(r.repeats) * r.spacing
                });
            let old_effect = value.max(old_last);
            let center = f64::from(row) + old_effect + shift;
            if !(0.0..1.0).contains(&(center - f64::from(new_row))) {
                possible = false;
                break;
            }
            let mut encoded = None;
            for absolute in candidates(center) {
                let fraction = absolute - f64::from(new_row);
                if !(0.0..1.0).contains(&fraction) {
                    continue;
                }
                let mut matches = true;
                for &use_ in uses {
                    clock.tick()?;
                    if frame(use_, new_row, fraction.max(last), per_row)
                        != desired(use_, row, old_effect, delta, per_row)?
                    {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    encoded = Some(fraction);
                    break;
                }
            }
            match encoded {
                Some(value) => {
                    if cut {
                        cell.effects.cut = Some(value)
                    } else {
                        cell.effects.release = Some(value)
                    }
                }
                None => {
                    possible = false;
                    break;
                }
            }
        }
        if !possible || song.validate_cell(&cell).is_err() {
            continue;
        }
        let new = fractions(&cell);
        let mut matches = true;
        'contexts: for (&use_, want) in uses.iter().zip(&expected) {
            for (&fraction, &target) in new.iter().zip(want) {
                clock.tick()?;
                if frame(use_, new_row, fraction, per_row) != target {
                    matches = false;
                    break 'contexts;
                }
            }
        }
        if matches {
            return Ok((new_row, cell));
        }
    }
    Err("Timing/row-local FX cannot preserve the exact integer displacement in every use within the selected rows (32-ULP encoding limit); narrow selection/change displacement".into())
}
/// Prepare an exact candidate and bounded review diff without mutating the base.
/// Inactive parameters are range-validated but ignored. Timing moves whole cells;
/// scalar masks alter only their explicitly named fields.
pub fn preview(song: &Song, config: TransformConfig) -> Result<TransformPreview, String> {
    preview_impl(song, config, None)
}
/// Shared exact whole-cell timing encoder/collision authority for groove baking.
pub(crate) fn preview_frame_moves(
    song: &Song,
    scope: TransformScope,
    moves: &BTreeMap<(u32, u16), i64>,
) -> Result<TransformPreview, String> {
    if moves.values().any(|d| !(-48000..=48000).contains(d)) {
        return Err("Groove movement exceeds +/-48000 output frames".into());
    }
    preview_impl(
        song,
        TransformConfig {
            scope,
            fields: TransformFields {
                timing: true,
                ..Default::default()
            },
            transpose_semitones: 0,
            velocity_scale: 1.0,
            nudge_frames: 0,
            swing: None,
        },
        Some(moves),
    )
}
fn preview_impl(
    song: &Song,
    config: TransformConfig,
    frame_moves: Option<&BTreeMap<(u32, u16), i64>>,
) -> Result<TransformPreview, String> {
    validate_config(song, &config)?;
    let chosen = selected(song, &config.scope)?;
    let per_row = f64::from(RATE) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
    let (uses, affected_occurrences, affected_aliases) = contexts(song, &config.scope, per_row)?;
    let swing_frames = swing_frames(song, &config)?;
    let mut summary = TransformSummary {
        algorithm: ALGORITHM.into(),
        selected_cells: chosen.len(),
        changed_cells: 0,
        moved_cells: 0,
        note_changes: 0,
        glide_changes: 0,
        velocity_changes: 0,
        timing_changes: 0,
        skipped_unspecified_velocity: 0,
        affected_occurrences,
        affected_aliases,
        swing_frames,
        warnings: Vec::new(),
    };
    let mut clock = Clock {
        uses: &uses,
        per_row,
        comparisons: 0,
    };
    let mut changes = Vec::new();
    let mut timing_checks = 0usize;
    for selected in chosen {
        let before = selected.cell;
        let mut after = before.clone();
        let mut to_row = selected.row;
        let result = (|| -> Result<(), String> {
            if config.fields.notes && config.transpose_semitones != 0 {
                if let Note::Note(pitch) = before.note {
                    after.note = Note::Note(shifted_pitch(
                        song,
                        selected.note_owner,
                        pitch,
                        config.transpose_semitones,
                        "Note",
                    )?);
                }
            }
            if config.fields.glide_targets && config.transpose_semitones != 0 {
                if let Some(glide) = &mut after.effects.glide {
                    glide.target = shifted_pitch(
                        song,
                        selected.glide_owner,
                        glide.target,
                        config.transpose_semitones,
                        "Glide target",
                    )?;
                }
            }
            if config.fields.velocity {
                if let Some(value) = before.velocity {
                    if config.velocity_scale != 1.0 {
                        let scaled = value * config.velocity_scale;
                        if !scaled.is_finite() || !(0.0..=1.0).contains(&scaled) {
                            return Err("Scaled explicit velocity leaves 0..1; no clamping".into());
                        }
                        after.velocity = Some(scaled);
                    }
                } else {
                    summary.skipped_unspecified_velocity += 1;
                }
            }
            if config.fields.timing && *before != Cell::default() {
                let swing = config
                    .swing
                    .filter(|s| odd_bin(song, selected.row, before, *s))
                    .map_or(0, |_| i64::from(swing_frames));
                let delta = frame_moves.map_or(i64::from(config.nudge_frames) + swing, |moves| {
                    *moves.get(&(selected.row, selected.column)).unwrap_or(&0)
                });
                if delta != 0 {
                    timing_checks = timing_checks
                        .saturating_add(fractions(before).len().saturating_mul(uses.len()));
                    if timing_checks > MAX_CHECKS {
                        return Err("Transform exceeds 65536 event-by-use timing checks".into());
                    }
                    (to_row, after) = move_cell(
                        song,
                        &config.scope,
                        selected.row,
                        before,
                        &after,
                        delta,
                        &mut clock,
                    )?;
                }
            }
            Ok(())
        })();
        result.map_err(|e| {
            format!(
                "Pattern {} track {} row {} column {}: {e}",
                config.scope.pattern.0, config.scope.track.0, selected.row, selected.column
            )
        })?;
        if to_row != selected.row || after != *before {
            summary.note_changes += usize::from(after.note != before.note);
            summary.glide_changes += usize::from(after.effects.glide != before.effects.glide);
            summary.velocity_changes += usize::from(after.velocity != before.velocity);
            summary.timing_changes += usize::from(
                to_row != selected.row
                    || after.delay != before.delay
                    || after.effects.release != before.effects.release
                    || after.effects.cut != before.effects.cut,
            );
            summary.moved_cells += usize::from(to_row != selected.row);
            changes.push(CellChange {
                from_row: selected.row,
                to_row,
                column: selected.column,
                before: before.clone(),
                after,
            });
        }
    }
    summary.changed_cells = changes.len();
    let changes_by_source: BTreeMap<_, _> = changes
        .iter()
        .map(|c| ((c.from_row, c.column), c))
        .collect();
    let mut destinations = BTreeSet::new();
    let rows = song.patterns[&config.scope.pattern]
        .tracks
        .get(&config.scope.track);
    for change in &changes {
        let destination = (change.to_row, change.column);
        if !destinations.insert(destination)
            || (rows
                .and_then(|r| r.get(&change.to_row))
                .is_some_and(|c| c.contains_key(&change.column))
                && !changes_by_source.contains_key(&destination))
        {
            return Err(format!("Timing collision: row {} column {} would overwrite occupied destination row {} column {}; no merging or deletion",change.from_row,change.column,change.to_row,change.column));
        }
    }
    // Preserve the complete source-column memory sequence, not just note order.
    // Explicit scalar-only/OFF/default entries are occupied destinations too.
    let mut previous: BTreeMap<u16, (u32, u32)> = BTreeMap::new();
    if let Some(rows) = rows {
        let mut inspected = 0usize;
        for (&row, columns) in rows {
            for (&column, cell) in columns {
                inspected += 1;
                if inspected > MAX_CHECKS {
                    return Err("Transform ordering scan exceeds 65536 cells".into());
                }
                if config.scope.columns.binary_search(&column).is_err() {
                    continue;
                }
                let (new_row, _) = changes_by_source
                    .get(&(row, column))
                    .map_or((row, cell), |c| (c.to_row, &c.after));
                // Distinct cells in one column have distinct row keys. Compare
                // exact row order: adding near-one delays to f64 row indices
                // could collapse valid adjacent rows onto the same value.
                let position = new_row;
                if let Some(&(old, source)) = previous.get(&column) {
                    if position <= old {
                        return Err(format!("Timing overtakes source row {source} at row {row} column {column}; preserve scalar/OFF/instrument memory order"));
                    }
                }
                previous.insert(column, (position, row));
            }
        }
    }
    let mut candidate = song.clone();
    if !changes.is_empty() {
        let rows = candidate
            .patterns
            .get_mut(&config.scope.pattern)
            .unwrap()
            .tracks
            .get_mut(&config.scope.track)
            .unwrap();
        for change in &changes {
            if let Some(columns) = rows.get_mut(&change.from_row) {
                columns.remove(&change.column);
                if columns.is_empty() {
                    rows.remove(&change.from_row);
                }
            }
        }
        for change in &changes {
            rows.entry(change.to_row)
                .or_default()
                .insert(change.column, change.after.clone());
        }
        candidate.validate()?;
    }
    if !summary.affected_aliases.is_empty() || summary.affected_occurrences.len() > 1 {
        summary.warnings.push("This shared source edit affects every listed repeat and alias; Make unique before preparing a local variation".into());
    }
    if config.fields.velocity {
        summary.warnings.push("Only explicit stored velocity values are scaled. This changes column memory: later notes with omitted velocity may inherit the new gain, even outside the selected rows. No values or restoration commands are inserted".into());
    }
    if config.fields.timing {
        summary.warnings.push("Timing is baked into whole ordinary cells. Swing is an integer output-frame displacement of odd pattern-local bins, not a runtime groove or grid snap. Phrase-trigger timing is not supported".into());
    }
    let bytes = serde_json::to_vec(&(ALGORITHM, song, &config, &candidate))
        .map_err(|e| format!("Cannot encode transform preview identity: {e}"))?;
    let id = crate::sample_asset::content_sha256(&bytes);
    Ok(TransformPreview {
        id,
        base: song.clone(),
        candidate,
        config,
        summary,
        changes,
    })
}
