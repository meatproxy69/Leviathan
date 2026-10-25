//! Editor state is separate from persistent song content and future playback state.
use crate::song::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Note,
    Instrument,
    Velocity,
    Pan,
    Delay,
    Effects,
    Phrase,
}
impl Field {
    pub const ALL: [Self; 7] = [
        Self::Note,
        Self::Instrument,
        Self::Velocity,
        Self::Pan,
        Self::Delay,
        Self::Effects,
        Self::Phrase,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap()
    }
}
/// Explicit typed entry; blank removes only this field. Numeric values use decimal.
pub fn parse_cell_value(mut cell: Cell, field: Field, text: &str) -> Result<Cell, String> {
    let text = text.trim();
    match field {
        Field::Note => {
            cell.note = if text.is_empty() {
                Note::Empty
            } else if text.eq_ignore_ascii_case("off") {
                Note::Off
            } else {
                Note::Note(
                    text.parse()
                        .map_err(|_| "Enter MIDI pitch 0..127, OFF or blank")?,
                )
            }
        }
        Field::Instrument => {
            cell.instrument = if text.is_empty() {
                None
            } else {
                Some(InstrumentId(
                    text.parse()
                        .map_err(|_| "Enter an instrument ID or blank")?,
                ))
            }
        }
        Field::Velocity => {
            cell.velocity = if text.is_empty() {
                None
            } else {
                Some(text.parse().map_err(|_| "Enter velocity 0..1 or blank")?)
            }
        }
        Field::Pan => {
            cell.pan = if text.is_empty() {
                None
            } else {
                Some(text.parse().map_err(|_| "Enter pan -1..1 or blank")?)
            }
        }
        Field::Delay => {
            cell.delay = if text.is_empty() {
                None
            } else {
                Some(text.parse().map_err(|_| "Enter row delay 0..<1 or blank")?)
            }
        }
        Field::Effects => cell.effects = parse_effects(text)?,
        Field::Phrase => {
            cell.phrase = if text.is_empty() {
                None
            } else {
                Some(PhraseId(
                    text.parse()
                        .map_err(|_| "Enter an instrument phrase ID or blank")?,
                ))
            }
        }
    }
    Ok(cell)
}
const HISTORY_LIMIT: usize = 64;
/// Human-readable effect entry, deliberately separate from any tracker FX dialect.
pub fn parse_effects(text: &str) -> Result<StepEffects, String> {
    let mut fx = StepEffects::default();
    let mut seen = std::collections::BTreeSet::new();
    for token in text.split_whitespace() {
        let token = token.to_ascii_lowercase();
        let (name, value) = token.split_once('=').unwrap_or((&token, ""));
        if !seen.insert(name.to_owned()) {
            return Err(format!("Duplicate effect {name}"));
        }
        match name {
            "retrigger" => fx.retrigger = Some(value.parse().map_err(|_| "Use retrigger=2..16")?),
            "cut" => fx.cut = Some(value.parse().map_err(|_| "Use cut=0..1")?),
            "offset" => fx.offset = Some(value.parse().map_err(|_| "Use offset=0..<1")?),
            "reverse" if value.is_empty() => fx.reverse = true,
            "glide" => {
                fx.glide = Some(
                    value
                        .parse()
                        .map_err(|_| "Use glide=0.1..1200 semitones/second")?,
                )
            }
            _ => {
                return Err(format!(
                "Unknown effect {token}; use retrigger=4 cut=0.5 offset=0.25 reverse or glide=12"
            ))
            }
        }
    }
    Ok(fx)
}
pub fn effects_label(fx: &StepEffects) -> String {
    let mut values = Vec::new();
    if let Some(v) = fx.retrigger {
        values.push(format!("retrigger={v}"));
    }
    if let Some(v) = fx.cut {
        values.push(format!("cut={v}"));
    }
    if let Some(v) = fx.offset {
        values.push(format!("offset={v}"));
    }
    if fx.reverse {
        values.push("reverse".into());
    }
    if let Some(v) = fx.glide {
        values.push(format!("glide={v}"));
    }
    values.join(" ")
}
#[derive(Debug, Clone)]
struct Snapshot {
    song: Song,
    occurrence: usize,
    track: TrackId,
    column: u16,
    row: u32,
    field: Field,
}
#[derive(Debug, Clone)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    saved: Song,
}
impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            saved: example_song(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct Editor {
    pub song: Song,
    pub occurrence: usize,
    pub track: TrackId,
    pub column: u16,
    pub row: u32,
    pub field: Field,
    pub edit_mode: bool,
    pub edit_step: u32,
    pub octave: u8,
    pub dirty: bool,
    pub history: History,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            song: example_song(),
            occurrence: 0,
            track: TrackId(1),
            column: 0,
            row: 0,
            field: Field::Note,
            edit_mode: false,
            edit_step: 1,
            octave: 4,
            dirty: false,
            history: History::default(),
        }
    }
}
impl Editor {
    pub fn preview_generation(
        &self,
        name: &str,
        recipe: &crate::generation::Recipe,
        mode: crate::generation::WriteMode,
    ) -> Result<crate::generation::Preview, String> {
        crate::generation::preview(
            &self.song,
            &crate::generation::Target {
                occurrence: self.occurrence,
                track: self.track,
                column: self.column,
                row: self.row,
            },
            name,
            recipe,
            mode,
        )
    }
    pub fn apply_generation(&mut self, preview: crate::generation::Preview) -> Result<(), String> {
        self.protect()?;
        let candidate = preview.into_song(&self.song)?;
        self.commit(candidate);
        Ok(())
    }
    pub fn remove_generation_recipe(&mut self, name: &str) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.generation_recipes.remove(name);
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }

    pub fn preview_transform(
        &self,
        scope: crate::transform::Scope,
        rows: std::ops::Range<u32>,
        operation: &crate::transform::Transform,
    ) -> Result<crate::transform::Preview, String> {
        crate::transform::preview(
            &self.song,
            &crate::transform::Selection {
                occurrence: self.occurrence,
                track: self.track,
                column: self.column,
                scope,
                rows,
            },
            operation,
        )
    }
    pub fn apply_transform(&mut self, preview: crate::transform::Preview) -> Result<(), String> {
        self.protect()?;
        let candidate = preview.into_song(&self.song)?;
        self.commit(candidate);
        Ok(())
    }

    /// Replace the selected destination block with a shared alias. Undo restores it.
    pub fn alias_block(&mut self, source: BlockRef) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let pattern = candidate
            .patterns
            .get_mut(&self.pattern_id())
            .ok_or("Unknown pattern")?;
        pattern.tracks.remove(&self.track);
        pattern.aliases.insert(self.track, source);
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn unique_block(&mut self, empty: bool) -> Result<(), String> {
        self.protect()?;
        let block = if empty {
            None
        } else {
            self.song.block(self.pattern_id(), self.track)?.cloned()
        };
        let mut candidate = self.song.clone();
        let pattern = candidate
            .patterns
            .get_mut(&self.pattern_id())
            .ok_or("Unknown pattern")?;
        pattern.aliases.remove(&self.track);
        pattern.tracks.remove(&self.track);
        if let Some(block) = block {
            pattern.tracks.insert(self.track, block);
        }
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn mute_track(&mut self, muted: bool) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let occurrence = candidate
            .order
            .get_mut(self.occurrence)
            .ok_or("Unknown occurrence")?;
        if muted {
            occurrence.muted_tracks.insert(self.track);
        } else {
            occurrence.muted_tracks.remove(&self.track);
        }
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn assign_pattern(&mut self, id: PatternId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate
            .order
            .get_mut(self.occurrence)
            .ok_or("Unknown occurrence")?
            .pattern = id;
        candidate.validate()?;
        self.commit(candidate);
        self.select(self.occurrence, self.track)
    }
    /// A marker begins a section; blank removes this marker and merges the ranges.
    pub fn mark_section(&mut self, name: &str) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let id = match candidate
            .order
            .get(self.occurrence)
            .ok_or("Unknown occurrence")?
            .section
            .as_ref()
        {
            Some(section) => section.id,
            None => SectionId(
                (1..)
                    .find(|id| {
                        !candidate
                            .order
                            .iter()
                            .any(|o| o.section.as_ref().is_some_and(|s| s.id.0 == *id))
                    })
                    .ok_or("Section IDs exhausted")?,
            ),
        };
        candidate.order[self.occurrence].section = if name.trim().is_empty() {
            None
        } else {
            Some(Section {
                id,
                name: name.trim().into(),
            })
        };
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn append_section(&mut self, independent: bool) -> Result<(), String> {
        self.protect()?;
        let range = self.song.section_range(self.occurrence)?;
        if self.song.order.len() + range.len() > 4096 {
            return Err("At most 4096 order positions".into());
        }
        let mut candidate = self.song.clone();
        let mut copies = self.song.order[range].to_vec();
        let mut remap = std::collections::BTreeMap::new();
        if independent {
            for occurrence in &mut copies {
                let source = occurrence.pattern;
                let id = if let Some(id) = remap.get(&source) {
                    *id
                } else {
                    let id = PatternId(
                        (1..)
                            .find(|id| !candidate.patterns.contains_key(&PatternId(*id)))
                            .ok_or("Pattern IDs exhausted")?,
                    );
                    candidate.clone_pattern(source, id)?;
                    candidate
                        .patterns
                        .get_mut(&id)
                        .unwrap()
                        .name
                        .push_str(" variation");
                    remap.insert(source, id);
                    id
                };
                occurrence.pattern = id;
            }
        }
        let id = SectionId(
            (1..)
                .find(|id| {
                    !candidate
                        .order
                        .iter()
                        .any(|o| o.section.as_ref().is_some_and(|s| s.id.0 == *id))
                })
                .ok_or("Section IDs exhausted")?,
        );
        let base = copies[0]
            .section
            .as_ref()
            .map_or("Section", |s| s.name.as_str());
        let name = format!(
            "{} {}",
            base.chars().take(48).collect::<String>(),
            if independent { "variation" } else { "repeat" }
        );
        copies[0].section = Some(Section { id, name });
        let selected = candidate.order.len();
        candidate.order.extend(copies);
        candidate.validate()?;
        self.commit(candidate);
        self.select(selected, self.track)
    }

    pub fn add_phrase(&mut self, owner: InstrumentId, phrase: Phrase) -> Result<PhraseId, String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let instrument = candidate
            .instruments
            .get_mut(&owner)
            .ok_or("Unknown phrase owner")?;
        let id = PhraseId(
            instrument
                .phrases
                .keys()
                .map(|id| id.0)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("Phrase ID exhausted")?,
        );
        instrument.phrases.insert(id, phrase);
        candidate.validate()?;
        self.commit(candidate);
        Ok(id)
    }
    pub fn update_phrase(
        &mut self,
        owner: InstrumentId,
        id: PhraseId,
        phrase: Phrase,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let target = candidate
            .instruments
            .get_mut(&owner)
            .and_then(|i| i.phrases.get_mut(&id))
            .ok_or("Unknown instrument phrase")?;
        *target = phrase;
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn set_phrase_cell(
        &mut self,
        owner: InstrumentId,
        id: PhraseId,
        row: u32,
        column: u16,
        cell: Cell,
    ) -> Result<(), String> {
        let mut phrase = self
            .song
            .instruments
            .get(&owner)
            .and_then(|i| i.phrases.get(&id))
            .ok_or("Unknown instrument phrase")?
            .clone();
        if row >= phrase.rows || column >= phrase.note_columns {
            return Err("Phrase cursor outside bounds".into());
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
        self.update_phrase(owner, id, phrase)
    }
    pub fn delete_phrase(&mut self, owner: InstrumentId, id: PhraseId) -> Result<(), String> {
        self.protect()?;
        if self.song.patterns.values().any(|p| {
            p.tracks.values().any(|rows| {
                rows.values().any(|columns| {
                    columns.values().any(|cell| {
                        cell.phrase == Some(id)
                            && (cell.instrument.is_none() || cell.instrument == Some(owner))
                    })
                })
            })
        }) {
            return Err(
                "Phrase is referenced by song cells; clear those selectors before deleting".into(),
            );
        }
        let mut candidate = self.song.clone();
        if candidate
            .instruments
            .get_mut(&owner)
            .and_then(|i| i.phrases.remove(&id))
            .is_none()
        {
            return Err("Unknown instrument phrase".into());
        }
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            song: self.song.clone(),
            occurrence: self.occurrence,
            track: self.track,
            column: self.column,
            row: self.row,
            field: self.field,
        }
    }
    fn restore(&mut self, state: Snapshot) {
        self.song = state.song;
        self.occurrence = state.occurrence;
        self.track = state.track;
        self.column = state.column;
        self.row = state.row;
        self.field = state.field;
        self.dirty = self.song != self.history.saved;
    }
    fn commit(&mut self, candidate: Song) {
        if candidate == self.song {
            return;
        }
        self.history.undo.push(self.snapshot());
        if self.history.undo.len() > HISTORY_LIMIT {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
        self.song = candidate;
        self.dirty = self.song != self.history.saved;
    }
    pub fn assign_sample(
        &mut self,
        instrument: InstrumentId,
        sample: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        self.protect()?;
        let sample = sample
            .map(|path| path.canonicalize().map_err(|e| format!("Sample path: {e}")))
            .transpose()?;
        if sample.as_ref().is_some_and(|p| !p.is_file()) {
            return Err("Sample must be a file".into());
        }
        let mut candidate = self.song.clone();
        candidate
            .instruments
            .get_mut(&instrument)
            .ok_or("Unknown instrument")?
            .sample = sample;
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }
    pub fn undo(&mut self) {
        if let Some(state) = self.history.undo.pop() {
            self.history.redo.push(self.snapshot());
            self.restore(state);
        }
    }
    pub fn redo(&mut self) {
        if let Some(state) = self.history.redo.pop() {
            self.history.undo.push(self.snapshot());
            self.restore(state);
        }
    }
    pub fn mark_saved(&mut self) {
        self.history.saved = self.song.clone();
        self.dirty = false;
    }
    /// Called only after a successful validated load. Empty documents are supported
    /// by the UI's empty-state view, with no unchecked pattern access.
    pub fn replace_document(&mut self, song: Song) -> Result<(), String> {
        song.validate()?;
        self.song = song;
        self.occurrence = 0;
        self.row = 0;
        self.column = 0;
        self.field = Field::Note;
        self.track = self.song.track_order.first().copied().unwrap_or(TrackId(0));
        self.edit_mode = false;
        self.history.undo.clear();
        self.history.redo.clear();
        self.mark_saved();
        Ok(())
    }
    pub fn has_grid(&self) -> bool {
        !self.song.order.is_empty() && !self.song.track_order.is_empty()
    }
    pub fn move_occurrence(&mut self, delta: i32) -> Result<(), String> {
        self.protect()?;
        let target = self.occurrence as i64 + delta as i64;
        if target < 0 || target >= self.song.order.len() as i64 {
            return Ok(());
        }
        let mut candidate = self.song.clone();
        candidate.order.swap(self.occurrence, target as usize);
        self.commit(candidate);
        self.occurrence = target as usize;
        Ok(())
    }

    pub fn pattern_id(&self) -> PatternId {
        self.song.order[self.occurrence].pattern
    }
    pub fn pattern(&self) -> &Pattern {
        &self.song.patterns[&self.pattern_id()]
    }
    pub fn cell(&self) -> Cell {
        self.song
            .block(self.pattern_id(), self.track)
            .ok()
            .flatten()
            .and_then(|r| r.get(&self.row))
            .and_then(|c| c.get(&self.column))
            .cloned()
            .unwrap_or_default()
    }
    pub fn select(&mut self, occurrence: usize, track: TrackId) -> Result<(), String> {
        if occurrence >= self.song.order.len() || !self.song.tracks.contains_key(&track) {
            return Err("Invalid occurrence or track".into());
        }
        self.occurrence = occurrence;
        self.track = track;
        self.row = self.row.min(self.pattern().rows - 1);
        self.column = self.column.min(self.song.tracks[&track].note_columns - 1);
        Ok(())
    }
    fn protect(&self) -> Result<(), String> {
        if self.edit_mode {
            Ok(())
        } else {
            Err("Enable Edit Mode to change song content".into())
        }
    }
    pub fn set_cell(&mut self, cell: Cell) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let source = candidate.block_source(self.pattern_id(), self.track)?;
        let rows = &mut candidate.patterns.get_mut(&source.pattern).unwrap().tracks;
        if cell == Cell::default() {
            if let Some(track) = rows.get_mut(&source.track) {
                if let Some(row) = track.get_mut(&self.row) {
                    row.remove(&self.column);
                    if row.is_empty() {
                        track.remove(&self.row);
                    }
                }
                if track.is_empty() {
                    rows.remove(&source.track);
                }
            }
        } else {
            rows.entry(source.track)
                .or_default()
                .entry(self.row)
                .or_default()
                .insert(self.column, cell);
        }
        candidate.validate()?;
        self.commit(candidate);
        Ok(())
    }
    pub fn enter_note(&mut self, note: Note) -> Result<(), String> {
        let mut cell = self.cell();
        cell.note = note;
        self.set_cell(cell)?;
        self.move_rows(self.edit_step as i64);
        Ok(())
    }
    pub fn clear(&mut self, whole_cell: bool) -> Result<(), String> {
        let mut cell = self.cell();
        if whole_cell {
            cell = Cell::default();
        } else {
            match self.field {
                Field::Note => cell.note = Note::Empty,
                Field::Instrument => cell.instrument = None,
                Field::Velocity => cell.velocity = None,
                Field::Pan => cell.pan = None,
                Field::Delay => cell.delay = None,
                Field::Effects => cell.effects = StepEffects::default(),
                Field::Phrase => cell.phrase = None,
            }
        }
        self.set_cell(cell)
    }
    /// Typed field entry is shared by the song tracker and instrument phrase editor.
    pub fn enter_value(&mut self, text: &str) -> Result<(), String> {
        self.set_cell(parse_cell_value(self.cell(), self.field, text)?)
    }
    pub fn move_rows(&mut self, amount: i64) {
        self.row = (self.row as i64 + amount).clamp(0, self.pattern().rows as i64 - 1) as u32;
    }
    pub fn move_field(&mut self, amount: i32) {
        let mut columns = Vec::new();
        for track in &self.song.track_order {
            for column in 0..self.song.tracks[track].note_columns {
                columns.push((*track, column));
            }
        }
        let index = columns
            .iter()
            .position(|c| *c == (self.track, self.column))
            .unwrap();
        let fields = Field::ALL.len();
        let next = (index as i32 * fields as i32 + self.field.index() as i32 + amount)
            .clamp(0, (columns.len() * fields) as i32 - 1) as usize;
        (self.track, self.column) = columns[next / fields];
        self.field = Field::ALL[next % fields];
    }
    pub fn resize(&mut self, rows: u32) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.patterns.get_mut(&self.pattern_id()).unwrap().rows = rows;
        candidate
            .validate()
            .map_err(|e| format!("Cannot resize: {e}. Clear occupied rows before shrinking."))?;
        self.commit(candidate);
        self.row = self.row.min(rows - 1);
        Ok(())
    }
    pub fn append(&mut self, independent: bool) -> Result<(), String> {
        self.protect()?;
        let source = self.pattern_id();
        let mut candidate = self.song.clone();
        let id = if independent {
            let id = PatternId(
                (1..)
                    .find(|id| !candidate.patterns.contains_key(&PatternId(*id)))
                    .ok_or("Pattern ID space exhausted")?,
            );
            candidate.clone_pattern(source, id)?;
            candidate
                .patterns
                .get_mut(&id)
                .unwrap()
                .name
                .push_str(" copy");
            id
        } else {
            source
        };
        candidate.append_repeat(id)?;
        candidate.order.last_mut().unwrap().muted_tracks =
            self.song.order[self.occurrence].muted_tracks.clone();
        self.commit(candidate);
        self.select(self.song.order.len() - 1, self.track)
    }
}

pub fn note_label(note: Note) -> String {
    match note {
        Note::Empty => "---".into(),
        Note::Off => "OFF".into(),
        Note::Note(n) => {
            let names = [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ];
            format!("{}{}", names[n as usize % 12], n as i16 / 12 - 1)
        }
    }
}
