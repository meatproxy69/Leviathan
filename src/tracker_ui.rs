use crate::{
    editor::{note_label, Editor, Field},
    song::{
        Cell, Direction, Glide, InstrumentId, Note, Occurrence, Pattern, PatternId, Phrase,
        PhraseId, PhraseTrigger, Retrigger, Song, StepEffects, Track, TrackAlias, TrackId,
    },
};
use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, Stroke, Vec2};
use std::collections::BTreeMap;
mod agent;

pub(crate) fn cell_is_read_only(editor: &Editor) -> bool {
    editor.has_grid() && editor.pattern().track_aliases.contains_key(&editor.track)
}

pub fn grid_id() -> egui::Id {
    egui::Id::new("tracker-keyboard")
}
/// Only the explicitly focused grid receives musical keys. Text/path widgets retain theirs.
pub fn keyboard(editor: &mut Editor, ctx: &egui::Context) -> Result<(), String> {
    keyboard_input(editor, ctx, true)
}
fn keyboard_input(editor: &mut Editor, ctx: &egui::Context, history: bool) -> Result<(), String> {
    if !editor.has_grid() || !ctx.memory(|m| m.has_focus(grid_id())) {
        return Ok(());
    }
    for event in ctx.input(|i| i.events.clone()) {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            repeat,
            ..
        } = event
        {
            if modifiers.command || modifiers.ctrl {
                if history {
                    apply_history_key(editor, key, modifiers);
                }
                continue;
            }
            if modifiers.alt {
                continue;
            }
            match key {
                egui::Key::ArrowUp => editor.move_rows(-1),
                egui::Key::ArrowDown => editor.move_rows(1),
                egui::Key::PageUp => editor.move_rows(-16),
                egui::Key::PageDown => editor.move_rows(16),
                egui::Key::Home => editor.row = 0,
                egui::Key::End => editor.row = editor.pattern().rows - 1,
                egui::Key::ArrowLeft => editor.move_field(-1),
                egui::Key::ArrowRight => editor.move_field(1),
                egui::Key::Tab => editor.move_field(if modifiers.shift { -5 } else { 5 }),
                egui::Key::Delete | egui::Key::Backspace if editor.edit_mode => {
                    editor.clear(modifiers.shift)?
                }
                egui::Key::Space if !repeat => editor.edit_mode = !editor.edit_mode,
                egui::Key::Backtick if editor.edit_mode && editor.field == Field::Note => {
                    editor.enter_note(Note::Off)?
                }
                _ if editor.edit_mode && editor.field == Field::Note => {
                    let keys = [
                        egui::Key::Z,
                        egui::Key::S,
                        egui::Key::X,
                        egui::Key::D,
                        egui::Key::C,
                        egui::Key::V,
                        egui::Key::G,
                        egui::Key::B,
                        egui::Key::H,
                        egui::Key::N,
                        egui::Key::J,
                        egui::Key::M,
                        egui::Key::Comma,
                    ];
                    if let Some(offset) = keys.iter().position(|k| *k == key) {
                        let pitch = (editor.octave as u16 + 1) * 12 + offset as u16;
                        if pitch <= 127 {
                            editor.enter_note(Note::Note(pitch as u8))?;
                        }
                    }
                }
                _ => (),
            }
        }
    }
    Ok(())
}

fn apply_history_key(editor: &mut Editor, key: egui::Key, modifiers: egui::Modifiers) {
    match key {
        egui::Key::Z if modifiers.shift => editor.redo(),
        egui::Key::Z => editor.undo(),
        egui::Key::Y => editor.redo(),
        _ => (),
    }
}
fn phrase_projection(
    parent: &Editor,
    phrase: &Phrase,
    cursor: Option<Selection>,
) -> Result<Editor, String> {
    let instrument = parent
        .song
        .instruments
        .get(&phrase.instrument)
        .ok_or("Phrase instrument is missing")?
        .clone();
    let song = Song {
        schema: parent.song.schema,
        generation_recipes: BTreeMap::new(),
        title: phrase.name.clone(),
        bpm: parent.song.bpm,
        lines_per_beat: phrase.lines_per_beat,
        instruments: BTreeMap::from([(phrase.instrument, instrument)]),
        tracks: BTreeMap::from([(
            TrackId(1),
            Track {
                name: format!("Instrument {} · fixed", phrase.instrument.0),
                note_columns: phrase.note_columns,
            },
        )]),
        track_order: vec![TrackId(1)],
        // Phrase-author projection has synthetic routing, not the parent mix.
        track_mix: BTreeMap::new(),
        patterns: BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                name: phrase.name.clone(),
                rows: phrase.rows,
                tracks: BTreeMap::from([(TrackId(1), phrase.cells.clone())]),
                track_aliases: BTreeMap::new(),
            },
        )]),
        order: vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }],
        phrases: BTreeMap::new(),
        samples: parent.song.samples.clone(),
        region_catalog: parent.song.region_catalog.clone(),
        regions: parent.song.regions.clone(),
        marker_sets: parent.song.marker_sets.clone(),
        drum_kits: parent.song.drum_kits.clone(),
        multisamples: parent.song.multisamples.clone(),
        grooves: parent.song.grooves.clone(),
    };
    let mut projection = Editor::default();
    projection.replace_document(song)?;
    projection.edit_mode = parent.edit_mode;
    projection.edit_step = parent.edit_step;
    projection.octave = parent.octave;
    if let Some(cursor) = cursor {
        projection.row = cursor.row.min(phrase.rows - 1);
        projection.column = cursor.column.min(phrase.note_columns - 1);
        projection.field = cursor.field;
    }
    Ok(projection)
}
fn step_summary(cell: &Cell) -> String {
    let Some(trigger) = &cell.phrase else {
        return effects_summary(&cell.effects);
    };
    let tag = format!("P{}×{}", trigger.phrase.0, trigger.cycles);
    let tag = if tag.chars().count() > 10 {
        format!("P…×{}", trigger.cycles)
    } else {
        tag
    };
    if cell.effects.is_empty() {
        tag
    } else {
        format!("{tag} {}", effects_summary(&cell.effects))
    }
}
fn step_description(song: &Song, cell: &Cell) -> String {
    let mut text = effects_description(&cell.effects);
    if let Some(trigger) = &cell.phrase {
        if let Some(cut) = cell.effects.cut {
            text = format!("Phrase hard cut: {cut} of row; cancels instance and kills all its voices/tails, not other instances");
        }
        let details = song
            .phrases
            .get(&trigger.phrase)
            .map(|phrase| {
                format!(
                    "{} · instrument {} · LPB {} · root {}",
                    phrase.name,
                    phrase.instrument.0,
                    phrase.lines_per_beat,
                    note_label(Note::Note(phrase.root_note))
                )
            })
            .unwrap_or_else(|| "Missing phrase".into());
        text = format!(
            "Phrase {} × {} total cycles\n{}\n{}",
            trigger.phrase.0, trigger.cycles, details, text
        );
    }
    text
}

// The tracker is a single instrument-like surface, not a table of buttons.
const INK: Color32 = Color32::from_rgb(207, 220, 224);
const MUTED: Color32 = Color32::from_rgb(120, 142, 151);
const EMPTY: Color32 = Color32::from_rgb(76, 95, 105);
pub(crate) const AQUA: Color32 = Color32::from_rgb(105, 211, 203);
const PLAY: Color32 = Color32::from_rgb(229, 185, 112);
const SURFACE: Color32 = Color32::from_rgb(17, 23, 28);
const LINE: Color32 = Color32::from_rgb(42, 56, 64);
const ROW_HEIGHT: f32 = 22.0;
const HEADER_HEIGHT: f32 = 48.0;
const GUTTER: f32 = 54.0;
const MIN_GRID_VIEWPORT: f32 = 64.0;
const FIELD_WIDTHS: [f32; 5] = [44.0, 34.0, 42.0, 48.0, 54.0];
const COLUMN_WIDTH: f32 = 232.0;
const EFFECT_WIDTH: f32 = 94.0;
const EFFECT_INK: Color32 = Color32::from_rgb(157, 182, 213);

pub(crate) fn install_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(24, 31, 37);
    style.visuals.window_fill = Color32::from_rgb(28, 36, 43);
    style.visuals.extreme_bg_color = SURFACE;
    style.visuals.faint_bg_color = Color32::from_rgb(30, 40, 47);
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = Color32::from_rgb(39, 88, 90);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, AQUA);
    style.visuals.window_corner_radius = 2.into();
    style.visuals.menu_corner_radius = 2.into();
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = 1.into();
        widget.expansion = 0.0;
        widget.fg_stroke = Stroke::new(1.0_f32, INK);
        widget.bg_stroke = Stroke::new(1.0_f32, LINE);
    }
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 44, 51);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(31, 41, 48);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(44, 63, 70);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 63, 70);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, MUTED);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(39, 88, 90);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(39, 88, 90);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, AQUA);
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.window_margin = 10.into();
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(16.0));
    ctx.set_style(style);
}

fn values(cell: &Cell) -> [String; 5] {
    [
        note_label(cell.note),
        cell.instrument
            .map_or("··".into(), |i| format!("{:02}", i.0)),
        cell.velocity.map_or("····".into(), |v| format!("{v:.2}")),
        cell.pan.map_or("·····".into(), |v| format!("{v:+.2}")),
        cell.delay.map_or("·····".into(), |v| {
            let rounded = format!("{v:.3}");
            if rounded == "1.000" && v < 1.0 {
                "<1.00".into()
            } else if rounded == "0.000" && v > 0.0 {
                ">0.00".into()
            } else {
                rounded
            }
        }),
    ]
}

fn field_name(field: Field) -> &'static str {
    match field {
        Field::Note => "Note",
        Field::Instrument => "Instrument",
        Field::Velocity => "Velocity",
        Field::Pan => "Pan",
        Field::Delay => "Delay",
    }
}

fn field_present(cell: &Cell, field: Field) -> bool {
    match field {
        Field::Note => cell.note != Note::Empty,
        Field::Instrument => cell.instrument.is_some(),
        Field::Velocity => cell.velocity.is_some(),
        Field::Pan => cell.pan.is_some(),
        Field::Delay => cell.delay.is_some(),
    }
}

fn effects_summary(effects: &StepEffects) -> String {
    let mut labels = Vec::new();
    if let Some(retrigger) = &effects.retrigger {
        labels.push(format!("R{}", retrigger.repeats));
    }
    if let Some(cut) = effects.cut {
        labels.push(format!("C{:.0}", cut * 100.0));
    }
    if let Some(release) = effects.release {
        labels.push(format!("L{:.0}", release * 100.0));
    }
    if let Some(offset) = effects.offset {
        labels.push(format!("O{offset}"));
    }
    if let Some(direction) = effects.direction {
        labels.push(
            match direction {
                Direction::Forward => "FWD",
                Direction::Reverse => "REV",
            }
            .into(),
        );
    }
    if let Some(glide) = &effects.glide {
        labels.push(format!("G{}", glide.target));
    }
    if labels.is_empty() {
        return "·".into();
    }
    let full = labels.join(" ");
    if full.len() <= 13 {
        return full;
    }
    // Keep dense fixed geometry, but never silently hide commands: +N counts the rest.
    for count in (1..labels.len()).rev() {
        let short = format!("{} +{}", labels[..count].join(" "), labels.len() - count);
        if short.len() <= 13 {
            return short;
        }
    }
    format!("{} FX", labels.len())
}
fn effects_description(effects: &StepEffects) -> String {
    let mut labels = Vec::new();
    if let Some(retrigger) = &effects.retrigger {
        labels.push(format!(
            "Retrigger: {} extra starts, spacing {} rows",
            retrigger.repeats, retrigger.spacing
        ));
    }
    if let Some(cut) = effects.cut {
        labels.push(format!("Hard cut: {cut} of row; stops the active logical voice immediately, may click; prior release tails remain"));
    }
    if let Some(release) = effects.release {
        labels.push(format!(
            "Timed release: {release} of row (region ADSR release or legacy de-click; R=0 is abrupt and may click)"
        ));
    }
    if let Some(offset) = effects.offset {
        labels.push(format!(
            "Source offset: frame {offset} (zero-based source frames, not output frames)"
        ));
    }
    if let Some(direction) = effects.direction {
        labels.push(format!(
            "Direction: {direction:?}; no latching to later notes"
        ));
    }
    if let Some(glide) = &effects.glide {
        labels.push(format!(
            "Glide: MIDI {} at {} semitones/second",
            glide.target, glide.semitones_per_second
        ));
    }
    if labels.is_empty() {
        "No step commands".into()
    } else {
        labels.join("\n")
    }
}
fn effects_count(effects: &StepEffects) -> usize {
    usize::from(effects.retrigger.is_some())
        + usize::from(effects.release.is_some())
        + usize::from(effects.cut.is_some())
        + usize::from(effects.offset.is_some())
        + usize::from(effects.direction.is_some())
        + usize::from(effects.glide.is_some())
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum EndAction {
    None,
    Release,
    Cut,
}
impl EndAction {
    fn from_effects(effects: &StepEffects) -> Self {
        if effects.cut.is_some() {
            Self::Cut
        } else if effects.release.is_some() {
            Self::Release
        } else {
            Self::None
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::None => "No end command",
            Self::Release => "Timed release",
            Self::Cut => "Hard cut",
        }
    }
    fn apply(self, effects: &mut StepEffects) {
        let position = effects.cut.or(effects.release).unwrap_or(0.75);
        effects.cut = (self == Self::Cut).then_some(position);
        effects.release = (self == Self::Release).then_some(position);
    }
}

#[derive(Clone, Copy)]
struct GridColumn {
    left: f32,
    width: f32,
    effects: bool,
    phrase: bool,
}
/// Add FX space only to populated command columns; no-FX songs retain their old geometry.
fn grid_columns(song: &Song, occurrence: usize) -> Vec<GridColumn> {
    let pattern_id = song.order[occurrence].pattern;
    let mut result = Vec::new();
    let mut left = 0.0;
    for track in &song.track_order {
        let mut effects = vec![false; song.tracks[track].note_columns as usize];
        let mut phrases = vec![false; effects.len()];
        if let Ok(Some(rows)) = song.resolved_track(pattern_id, *track) {
            for columns in rows.values() {
                for (column, cell) in columns {
                    if cell.phrase.is_some() {
                        if let Some(visible) = phrases.get_mut(*column as usize) {
                            *visible = true;
                        }
                    }
                    if !cell.effects.is_empty() || cell.phrase.is_some() {
                        if let Some(visible) = effects.get_mut(*column as usize) {
                            *visible = true;
                        }
                    }
                }
            }
        }
        for (effects, phrase) in effects.into_iter().zip(phrases) {
            let width = COLUMN_WIDTH + if effects { EFFECT_WIDTH } else { 0.0 };
            result.push(GridColumn {
                left,
                width,
                effects,
                phrase,
            });
            left += width;
        }
    }
    result
}

type EffectsSource = (u64, usize, TrackId, u16, u32, bool, Cell);
type TriggerSource = (u64, usize, TrackId, u16, u32, bool, Option<PhraseTrigger>);

#[derive(Default)]
struct EffectsDraft {
    source: Option<EffectsSource>,
    value: StepEffects,
    retrigger_spacing: String,
}
impl EffectsDraft {
    fn set_value(&mut self, value: StepEffects) {
        self.retrigger_spacing = value.retrigger.as_ref().map_or_else(String::new, |r| {
            if r.spacing < 0.000001 {
                format!("{:e}", r.spacing)
            } else {
                r.spacing.to_string()
            }
        });
        self.value = value;
    }
    fn effective(&self) -> Result<StepEffects, String> {
        let mut value = self.value.clone();
        if let Some(retrigger) = &mut value.retrigger {
            let spacing = self.retrigger_spacing.parse::<f64>().map_err(|_| "Retrigger spacing must be a finite positive number of rows; decimal or scientific notation, no clamping".to_string())?;
            if !spacing.is_finite() || spacing <= 0.0 {
                return Err(
                    "Retrigger spacing must be finite and greater than zero; no clamping".into(),
                );
            }
            retrigger.spacing = spacing;
        }
        Ok(value)
    }
    fn sync(&mut self, editor: &Editor) {
        let source = (
            editor.revision(),
            editor.occurrence,
            editor.track,
            editor.column,
            editor.row,
            editor.edit_mode,
            editor.cell(),
        );
        if self.source.as_ref() != Some(&source) {
            self.set_value(source.6.effects.clone());
            self.source = Some(source);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    row: u32,
    track: TrackId,
    column: u16,
    field: Field,
}
impl Selection {
    fn from_editor(editor: &Editor) -> Self {
        Self {
            row: editor.row,
            track: editor.track,
            column: editor.column,
            field: editor.field,
        }
    }
    fn column_index(self, song: &Song) -> usize {
        let mut index = 0;
        for track in &song.track_order {
            if *track == self.track {
                return index + self.column as usize;
            }
            index += song.tracks[track].note_columns as usize;
        }
        index
    }
    fn bounds(self, song: &Song, occurrence: usize) -> Rect {
        let left = grid_columns(song, occurrence)[self.column_index(song)].left
            + FIELD_WIDTHS[..self.field.index()].iter().sum::<f32>();
        Rect::from_min_size(
            egui::pos2(left, self.row as f32 * ROW_HEIGHT),
            egui::vec2(FIELD_WIDTHS[self.field.index()], ROW_HEIGHT),
        )
    }
}

fn reveal_axis(offset: f32, start: f32, end: f32, viewport: f32) -> f32 {
    if start < offset {
        start.max(0.0)
    } else if end > offset + viewport {
        (end - viewport).max(0.0)
    } else {
        offset
    }
}
fn reveal_selection(offset: Vec2, viewport: Vec2, bounds: Rect) -> Vec2 {
    egui::vec2(
        reveal_axis(
            offset.x,
            bounds.left(),
            bounds.right(),
            (viewport.x - GUTTER).max(1.0),
        ),
        reveal_axis(
            offset.y,
            bounds.top(),
            bounds.bottom(),
            (viewport.y - HEADER_HEIGHT).max(1.0),
        ),
    )
}

struct GridView<'a> {
    song: &'a Song,
    occurrence: usize,
    selection: Option<Selection>,
    rectangle: Option<&'a crate::cell_blocks::RectScope>,
    playhead: Option<u32>,
    editing: bool,
    focused: bool,
    scroll: Option<Vec2>,
    id: &'static str,
}
struct GridResult {
    clicked: Option<Selection>,
    offset: Vec2,
    viewport: Vec2,
    viewport_settled: bool,
    rect: Rect,
}

/// Virtualized in both dimensions; headers and row numbers stay pinned while scrolling.
fn show_grid(ui: &mut egui::Ui, view: GridView<'_>) -> GridResult {
    let pattern = &view.song.patterns[&view.song.order[view.occurrence].pattern];
    let columns = grid_columns(view.song, view.occurrence);
    let content_width = columns
        .last()
        .map_or(0.0, |column| column.left + column.width);
    let mut clicked = None;
    let outer_rect = ui.available_rect_before_wrap();
    let outer_size = outer_rect.size();
    let bar_width = ui.spacing().scroll.allocated_width();
    let mut scroll = egui::ScrollArea::both()
        .id_salt(view.id)
        .auto_shrink([false, false])
        .min_scrolled_width(MIN_GRID_VIEWPORT)
        .min_scrolled_height(MIN_GRID_VIEWPORT)
        .animated(false);
    if let Some(offset) = view.scroll {
        scroll = scroll.scroll_offset(offset);
    }
    let output = scroll.show_viewport(ui, |ui, viewport| {
        let origin = ui.max_rect().min;
        let width = (GUTTER + content_width).max(viewport.width());
        ui.set_min_size(egui::vec2(
            width,
            HEADER_HEIGHT + pattern.rows as f32 * ROW_HEIGHT,
        ));
        let screen = Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size());
        let body = Rect::from_min_max(screen.min + egui::vec2(GUTTER, HEADER_HEIGHT), screen.max)
            .intersect(ui.clip_rect());
        let painter = ui
            .painter()
            .with_clip_rect(screen.intersect(ui.clip_rect()));
        let body_painter = painter.with_clip_rect(body);
        painter.rect_filled(screen, 0.0, SURFACE);
        let first_row = (viewport.min.y / ROW_HEIGHT).floor().max(0.0) as u32;
        let last_row = ((viewport.max.y / ROW_HEIGHT).ceil() as u32).min(pattern.rows);
        let beat = view.song.lines_per_beat as u32;
        for row in first_row..last_row {
            let y = origin.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT;
            let row_rect = Rect::from_min_size(
                egui::pos2(screen.left(), y),
                egui::vec2(screen.width(), ROW_HEIGHT),
            );
            let fill = if view.selection.is_some_and(|s| s.row == row) {
                Color32::from_rgb(27, 43, 49)
            } else if (row / beat).is_multiple_of(2) {
                Color32::from_rgb(20, 28, 34)
            } else {
                SURFACE
            };
            body_painter.rect_filled(row_rect, 0.0, fill);
            if row.is_multiple_of(beat) {
                body_painter.hline(
                    row_rect.x_range(),
                    y,
                    Stroke::new(
                        1.0_f32,
                        if row.is_multiple_of(beat * 4) {
                            Color32::from_rgb(58, 78, 87)
                        } else {
                            LINE
                        },
                    ),
                );
            }
            if view.playhead == Some(row) {
                body_painter.rect_filled(
                    row_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(229, 185, 112, 16),
                );
                body_painter.hline(row_rect.x_range(), y + 1.0, Stroke::new(1.0_f32, PLAY));
            }
        }
        let mut index = 0;
        for track in &view.song.track_order {
            let track_info = &view.song.tracks[track];
            let source = view.song.resolve_track_source(view.song.order[view.occurrence].pattern, *track).ok();
            let resolved_rows = view.song.resolved_track(view.song.order[view.occurrence].pattern, *track).ok().flatten();
            let aliased = pattern.track_aliases.contains_key(track);
            let muted = view.song.order[view.occurrence].muted_tracks.contains(track);
            let track_start = index;
            let track_x = origin.x + GUTTER + columns[index].left;
            let track_width: f32 = columns[index..index + track_info.note_columns as usize].iter().map(|column| column.width).sum();
            for column in 0..track_info.note_columns {
                let geometry = columns[index];
                let x = origin.x + GUTTER + geometry.left;
                index += 1;
                if x + geometry.width < body.left() || x > body.right() {
                    continue;
                }
                body_painter.vline(x - 1.0, body.y_range(), Stroke::new(1.0_f32, LINE));
                for row in first_row..last_row {
                    let cell = resolved_rows
                        .and_then(|rows| rows.get(&row))
                        .and_then(|columns| columns.get(&column))
                        .cloned()
                        .unwrap_or_default();
                    let y = origin.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT;
                    if let Some(scope) = view.rectangle.filter(|scope| scope.pattern == view.song.order[view.occurrence].pattern
                        && scope.track == *track && (scope.row_start..scope.row_end).contains(&row)
                        && (scope.column_start..scope.column_end).contains(&column)) {
                        let rect = Rect::from_min_size(egui::pos2(x,y),egui::vec2(FIELD_WIDTHS.iter().sum(),ROW_HEIGHT));
                        body_painter.rect_filled(rect,0.0,Color32::from_rgba_unmultiplied(91,204,201,25));
                        let edge=Stroke::new(1.0_f32,AQUA);
                        if row==scope.row_start {body_painter.hline(rect.x_range(),rect.top(),edge);}
                        if row+1==scope.row_end {body_painter.hline(rect.x_range(),rect.bottom(),edge);}
                        if column==scope.column_start {body_painter.vline(rect.left(),rect.y_range(),edge);}
                        if column+1==scope.column_end {body_painter.vline(rect.right(),rect.y_range(),edge);}
                    }
                    let mut field_x = x;
                    for (field, text) in Field::ALL.into_iter().zip(values(&cell)) {
                        let rect = Rect::from_min_size(
                            egui::pos2(field_x, y),
                            egui::vec2(FIELD_WIDTHS[field.index()], ROW_HEIGHT),
                        );
                        field_x += FIELD_WIDTHS[field.index()];
                        if !rect.intersects(body) {
                            continue;
                        }
                        let selection = Selection {
                            row,
                            track: *track,
                            column,
                            field,
                        };
                        let selected = view.selection == Some(selection);
                        let response = if view.selection.is_some() {
                            Some(ui.interact(
                                rect.intersect(body),
                                ui.id().with((view.id, row, track.0, column, field.index())),
                                egui::Sense::click(),
                            ))
                        } else {
                            None
                        };
                        if selected {
                            body_painter.rect_filled(
                                rect.shrink(1.0),
                                0.0,
                                if view.editing && !aliased {
                                    Color32::from_rgb(38, 90, 91)
                                } else {
                                    Color32::from_rgb(40, 56, 65)
                                },
                            );
                            body_painter.rect_stroke(
                                rect.shrink(0.5),
                                0.0,
                                Stroke::new(1.0_f32, if view.focused { AQUA } else { MUTED }),
                                egui::StrokeKind::Inside,
                            );
                        } else if response.as_ref().is_some_and(|r| r.hovered()) {
                            body_painter.rect_filled(
                                rect.shrink(1.0),
                                0.0,
                                Color32::from_rgb(34, 49, 57),
                            );
                        }
                        let color = if selected {
                            INK
                        } else if muted && field_present(&cell, field) {
                            MUTED
                        } else if !field_present(&cell, field) {
                            EMPTY
                        } else if field == Field::Note {
                            if cell.note == Note::Off {
                                PLAY
                            } else {
                                AQUA
                            }
                        } else {
                            INK
                        };
                        body_painter
                            .with_clip_rect(rect.shrink2(egui::vec2(2.0, 0.0)).intersect(body))
                            .text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                &text,
                                FontId::monospace(13.0),
                                color,
                            );
                        if let Some(response) = response {
                            if response.clicked() {
                                clicked = Some(selection);
                            }
                            response.on_hover_text(format!(
                                "{} · track {} / column {} / row {}\n{}{}",
                                field_name(field),
                                track.0,
                                column + 1,
                                row,
                                if field == Field::Delay {
                                    cell.delay.map_or(text.clone(), |delay| format!("{delay} row fraction (stored value; grid rounded to 3 decimals)"))
                                } else { text.clone() },
                                if field == Field::Note {
                                    "  (typed entry uses MIDI 0–127 or OFF)"
                                } else {
                                    ""
                                }
                            ));
                        }
                    }
                    if geometry.effects {
                        let rect = Rect::from_min_size(egui::pos2(x + COLUMN_WIDTH - 6.0, y), egui::vec2(EFFECT_WIDTH, ROW_HEIGHT));
                        if rect.intersects(body) {
                            body_painter.with_clip_rect(rect.intersect(body)).text(
                                egui::pos2(rect.left() + 4.0, rect.center().y), Align2::LEFT_CENTER,
                                step_summary(&cell), FontId::monospace(11.0),
                                if cell.effects.is_empty() && cell.phrase.is_none() { EMPTY } else { EFFECT_INK });
                            ui.interact(rect.intersect(body), ui.id().with((view.id, "fx", row, track.0, column)), egui::Sense::hover())
                                .on_hover_text(format!("{}\nSelect a musical field in this cell; use Commands or Trigger to edit.", step_description(view.song, &cell)));
                        }
                    }
                }
            }
            // A track title spans its note columns; field labels retain exact cell alignment.
            let header = Rect::from_min_size(
                egui::pos2(track_x, screen.top()),
                egui::vec2(track_width, HEADER_HEIGHT),
            );
            let header_clip = Rect::from_min_max(
                egui::pos2(screen.left() + GUTTER, screen.top()),
                egui::pos2(screen.right(), screen.top() + HEADER_HEIGHT),
            )
            .intersect(ui.clip_rect());
            if header.intersects(header_clip) {
                let p = painter.with_clip_rect(header.intersect(header_clip));
                p.rect_filled(header, 0.0, Color32::from_rgb(29, 40, 47));
                p.text(
                    egui::pos2(track_x + 8.0, screen.top() + 12.0),
                    Align2::LEFT_CENTER,
                    if aliased || muted {format!("{}{:02} {}{}", if muted {"X "}else{""}, track.0, track_info.name, source.filter(|_|aliased).map_or(String::new(), |(p,t)|format!("  L P{}/T{} · READ ONLY",p.0,t.0)))} else {format!("{:02}  {}", track.0, track_info.name)},
                    FontId::proportional(12.0),
                    if view.selection.is_some_and(|s| s.track == *track) {
                        AQUA
                    } else {
                        INK
                    },
                );
                p.vline(track_x, header.y_range(), Stroke::new(1.0_f32, LINE));
                p.hline(
                    header.x_range(),
                    screen.top() + 24.0,
                    Stroke::new(1.0_f32, LINE),
                );
                for column in 0..track_info.note_columns {
                    let geometry = columns[track_start + column as usize];
                    let mut x = origin.x + GUTTER + geometry.left;
                    for (field, label) in Field::ALL.into_iter().zip([
                        format!("N{}", column + 1),
                        "INS".into(),
                        "VEL".into(),
                        "PAN".into(),
                        "DLY".into(),
                    ]) {
                        let width = FIELD_WIDTHS[field.index()];
                        p.text(
                            egui::pos2(x + width / 2.0, screen.top() + 36.0),
                            Align2::CENTER_CENTER,
                            label,
                            FontId::monospace(10.0),
                            MUTED,
                        );
                        x += width;
                    }
                    if geometry.effects {
                        p.text(egui::pos2(x + EFFECT_WIDTH / 2.0, screen.top() + 36.0), Align2::CENTER_CENTER, if geometry.phrase { "PHR / FX" } else { "STEP FX" }, FontId::monospace(10.0), EFFECT_INK);
                    }
                }
            }
        }
        // Draw the frozen row gutter last, never over the fixed header.
        let gutter = Rect::from_min_max(
            screen.min + egui::vec2(0.0, HEADER_HEIGHT),
            egui::pos2(screen.left() + GUTTER, screen.bottom()),
        )
        .intersect(ui.clip_rect());
        let p = painter.with_clip_rect(gutter);
        p.rect_filled(gutter, 0.0, Color32::from_rgb(23, 31, 37));
        for row in first_row..last_row {
            let y = origin.y + HEADER_HEIGHT + row as f32 * ROW_HEIGHT;
            let active = view.playhead == Some(row);
            let selected = view.selection.is_some_and(|s| s.row == row);
            if selected {
                p.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(screen.left(), y),
                        egui::vec2(GUTTER, ROW_HEIGHT),
                    ),
                    0.0,
                    Color32::from_rgb(36, 55, 62),
                );
            }
            if active {
                p.text(
                    egui::pos2(screen.left() + 4.0, y + ROW_HEIGHT / 2.0),
                    Align2::LEFT_CENTER,
                    "▶",
                    FontId::proportional(10.0),
                    PLAY,
                );
            }
            p.text(
                egui::pos2(screen.left() + GUTTER - 7.0, y + ROW_HEIGHT / 2.0),
                Align2::RIGHT_CENTER,
                format!("{row:03}"),
                FontId::monospace(12.0),
                if active {
                    PLAY
                } else if selected {
                    AQUA
                } else if row.is_multiple_of(beat) {
                    INK
                } else {
                    MUTED
                },
            );
            if row.is_multiple_of(beat) {
                p.hline(gutter.x_range(), y, Stroke::new(1.0_f32, LINE));
            }
        }
        p.vline(
            gutter.right() - 1.0,
            gutter.y_range(),
            Stroke::new(1.0_f32, LINE),
        );
        let corner = Rect::from_min_size(screen.min, egui::vec2(GUTTER, HEADER_HEIGHT));
        painter.rect_filled(corner, 0.0, Color32::from_rgb(29, 40, 47));
        painter.text(
            corner.center(),
            Align2::CENTER_CENTER,
            "ROW",
            FontId::monospace(10.0),
            MUTED,
        );
        painter.hline(
            screen.x_range(),
            screen.top() + HEADER_HEIGHT,
            Stroke::new(1.0_f32, LINE),
        );
    });
    // Use the actual current overflow state, not a hypothetical no-bars layout:
    // near-fit content can legitimately retain both bars. This also distinguishes
    // an unadvanced animation at time zero from a settled measured viewport.
    let viewport = output.inner_rect.size();
    let allocated = egui::vec2(
        if output.content_size.y > viewport.y {
            bar_width
        } else {
            0.0
        },
        if output.content_size.x > viewport.x {
            bar_width
        } else {
            0.0
        },
    );
    let settled = (outer_size - allocated).max(Vec2::splat(MIN_GRID_VIEWPORT));
    GridResult {
        clicked,
        offset: output.state.offset,
        viewport,
        // Rect reconstruction at fractional origins can differ by a few f32
        // ULPs. This is numerical tolerance, not a pixel-sized clipping allowance.
        viewport_settled: (viewport - settled).abs().max_elem()
            <= 8.0
                * f32::EPSILON
                * outer_rect
                    .max
                    .to_vec2()
                    .abs()
                    .max_elem()
                    .max(outer_rect.min.to_vec2().abs().max_elem())
                    .max(outer_size.max_elem())
                    .max(1.0),
        rect: output.inner_rect,
    }
}

struct MatrixDraft {
    revision: u64,
    occurrence: usize,
    pattern: PatternId,
    track: TrackId,
    source: TrackAlias,
}
enum MatrixAction {
    Link(Option<TrackAlias>),
    Unique,
    Mute(bool),
}

#[derive(Default)]
pub struct TrackerUi {
    pub editor: Editor,
    pub(crate) pads: crate::pad_ui::PadUi,
    pub(crate) steps: crate::drum_step_ui::DrumStepUi,
    pub(crate) transforms: crate::transform_ui::TransformUi,
    pub(crate) cell_blocks: crate::cell_blocks_ui::CellBlocksUi,
    pub(crate) thin: crate::thin_ui::ThinUi,
    pub(crate) grooves: crate::groove_ui::GrooveUi,
    pub(crate) generation: crate::generation_ui::GenerationUi,
    generation_status: Option<(u64, PatternId, Result<bool, String>)>,
    /// Playback snapshot location; never changes the edit cursor.
    pub playhead: Option<(usize, u32)>,
    value: String,
    scroll_to_cursor: bool,
    reveal_last_viewport: Option<Vec2>,
    reveal_stable_frames: u8,
    scroll_offset: Vec2,
    viewport: Vec2,
    row_input: String,
    matrix_draft: Option<MatrixDraft>,
    effects_draft: EffectsDraft,
    phrase_browser_open: bool,
    phrase_owner: u64,
    phrase_selected: Option<PhraseId>,
    phrase_target: Option<PhraseId>,
    phrase_cursor: Option<Selection>,
    song_view: Option<(Vec2, Vec2)>,
    projection_active: bool,
    phrase_settings: Option<(PhraseId, Phrase, Phrase)>,
    trigger_source: Option<TriggerSource>,
    trigger_draft: Option<PhraseTrigger>,
    pub error: Option<String>,
}
impl TrackerUi {
    pub(crate) fn is_phrase_authoring(&self) -> bool {
        self.phrase_target.is_some() || self.projection_active
    }
    /// One-shot viewport request only: never changes cursor, document, or history.
    pub(crate) fn reveal_current_cursor(&mut self) {
        self.scroll_to_cursor = true;
        self.reveal_last_viewport = None;
        self.reveal_stable_frames = 0;
    }
    pub(crate) fn reset_generation_document(&mut self) {
        self.generation.reset_document();
        self.generation_status = None;
    }
    pub(crate) fn set_step_view(&mut self, open: bool) {
        if open {
            self.return_to_song();
            self.pads.open = true;
        }
        self.steps.open = open;
    }
    pub(crate) fn set_pad_view(&mut self, open: bool) {
        if open {
            self.return_to_song();
        }
        self.pads.open = open;
        if !open {
            self.steps.open = false;
        }
    }
    fn result(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let cell_writable = self.editor.edit_mode && !cell_is_read_only(&self.editor);
        ui.horizontal_wrapped(|ui| {
            if !self.projection_active {
                ui.label(RichText::new("PATTERN").strong().color(AQUA));
                ui.label(format!("{:02}  {}", self.editor.pattern_id().0, self.editor.pattern().name));
                if self.editor.pattern().generation.is_some() {
                    let key=(self.editor.revision(),self.editor.pattern_id());
                    if self.generation_status.as_ref().is_none_or(|(revision,pattern,_)|(*revision,*pattern)!=key) {
                        let status=crate::generation::content_matches(self.editor.pattern()).map(|status|status.unwrap_or(false));
                        self.generation_status=Some((key.0,key.1,status));
                    }
                    let status=&self.generation_status.as_ref().unwrap().2;
                    let label=match status {Ok(true)=>"GEN cells OK",Ok(false)=>"GEN edited",Err(_)=>"GEN history ?"};
                    ui.label(RichText::new(label).small().color(AQUA)).on_hover_ui(|ui| {
                        ui.set_max_width(560.0);
                        ui.label(match status {Ok(true)=>"Stored-cell fingerprint matches the original bake.",Ok(false)=>"Stored-cell fingerprint differs; generation metadata remains historical.",Err(_)=>"Could not verify the stored-cell fingerprint."});
                        if let Err(error)=status {ui.colored_label(Color32::LIGHT_RED,error);}
                        ui.label("Fingerprint covers rows, aliases and track/cell content, not name, global clock, instruments or assets. This does not prove current audio/frame reproduction.");
                        crate::generation_ui::provenance_details(ui,self.editor.pattern().generation.as_ref().unwrap());
                    });
                } else { self.generation_status=None; }
                ui.separator();
            }
            ui.toggle_value(&mut self.editor.edit_mode, "Edit mode")
                .on_hover_text("Space toggles Edit mode when the grid is focused. Navigation is always available.");
            ui.label(RichText::new(if self.editor.edit_mode { "WRITE" } else { "READ" }).small().color(if self.editor.edit_mode { AQUA } else { MUTED }));
            ui.separator();
            ui.label("Step");
            ui.add(egui::DragValue::new(&mut self.editor.edit_step).range(0..=64));
            ui.label("Oct");
            ui.add(egui::DragValue::new(&mut self.editor.octave).range(0..=9));
            if self.projection_active { ui.monospace(format!("{} rows · LPB {}", self.editor.pattern().rows, self.editor.song.lines_per_beat)); }
            else {
            ui.menu_button(format!("{} rows", self.editor.pattern().rows), |ui| {
                ui.label("Pattern length (1–65,536 rows)");
                ui.add(egui::TextEdit::singleline(&mut self.row_input).desired_width(100.0).hint_text("row count"));
                if ui.add_enabled(self.editor.edit_mode, egui::Button::new("Set length")).clicked() {
                    let result = self.row_input.parse::<u32>().map_err(|_| "Enter a row count".into()).and_then(|rows| self.editor.resize(rows));
                    self.result(result);
                }
                ui.label(RichText::new("Shortening refuses to discard populated rows.").small().weak());
            });
            }
            ui.label(RichText::new("Keys ?").small().color(MUTED)).on_hover_text("Click a field to focus the grid\nArrows / Tab: navigate · Page Up/Down: 16 rows\nZ–M: notes · `: note OFF · Space: Edit mode\nDelete: clear field · Shift+Delete: clear cell\nCtrl/Cmd+Z: undo · Ctrl/Cmd+Shift+Z: redo");
            if !self.projection_active && ui.button("Groove…").on_hover_text("Capture timing from accepted markers, or preview timing-only transfer to ordinary cells").clicked() {
                self.thin.reset_document(); self.transforms.reset_document(); self.generation.reset_document(); self.cell_blocks.close(); self.grooves.open(&self.editor);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.monospace(if self.projection_active {
                format!("ROW {:03}  N{}", self.editor.row, self.editor.column + 1)
            } else {
                format!(
                    "{:02}:{:03}  T{} / N{}",
                    self.editor.occurrence,
                    self.editor.row,
                    self.editor.track.0,
                    self.editor.column + 1
                )
            });
            ui.label(RichText::new(field_name(self.editor.field)).color(AQUA));
            ui.add(
                egui::TextEdit::singleline(&mut self.value)
                    .desired_width(90.0)
                    .hint_text("value / OFF"),
            )
            .on_hover_text(
                "Decimal entry. Note: MIDI 0–127 or OFF. Blank removes the selected field.",
            );
            if ui
                .add_enabled(cell_writable, egui::Button::new("Apply"))
                .clicked()
            {
                let result = self.editor.enter_value(&self.value);
                self.result(result);
            }
            if ui
                .add_enabled(cell_writable, egui::Button::new("Clear"))
                .clicked()
            {
                let result = self.editor.clear(false);
                self.result(result);
            }
            if ui
                .add_enabled(cell_writable, egui::Button::new("OFF"))
                .on_hover_text("Write note OFF at the edit cursor")
                .clicked()
            {
                let result = self.editor.enter_note(Note::Off);
                self.result(result);
                self.scroll_to_cursor = true;
            }
            self.commands_menu(ui);
            if !self.projection_active {
                self.phrase_trigger_menu(ui);
                if ui
                    .selectable_label(self.phrase_browser_open, "Phrases")
                    .clicked()
                {
                    self.phrase_browser_open = !self.phrase_browser_open;
                }
                if ui.button("Cells…").on_hover_text("Rectangular whole-cell Copy / Paste Replace / Clear; private document clipboard").clicked() {
                    self.transforms.reset_document(); self.grooves.reset_document(); self.generation.reset_document(); self.thin.reset_document();
                    self.cell_blocks.open(&self.editor);
                }
                if ui.button("Transform…").on_hover_text("Preview scoped edits to ordinary pattern cells; no runtime groove or phrase-definition edits").clicked() {
                    self.generation.reset_document();
                    self.grooves.reset_document();
                    self.thin.reset_document();
                    self.cell_blocks.close();
                    self.transforms.open(&self.editor);
                }
                if ui.button("Generate…").on_hover_text("Preview a seeded pattrns preset, then append ordinary editable notes once").clicked() {
                    self.transforms.reset_document();
                    self.grooves.reset_document();
                    self.thin.reset_document();
                    self.cell_blocks.close();
                    self.generation.open(&self.editor);
                }
            }
        });
    }
    fn commands_menu(&mut self, ui: &mut egui::Ui) {
        self.effects_draft.sync(&self.editor);
        let current = self.editor.cell().effects;
        let count = effects_count(&current);
        let writable = self.editor.edit_mode && ui.is_enabled() && !cell_is_read_only(&self.editor);
        let (response, contents) = egui::containers::menu::MenuButton::new(format!("Commands ({count})"))
            .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
                ui.set_max_width(440.0);
                ui.label(RichText::new("STEP COMMANDS").strong().color(EFFECT_INK));
                if cell_is_read_only(&self.editor) {ui.colored_label(PLAY,"ALIASED SOURCE · read only here; Edit source or Make unique");}
                ui.monospace(format!("POS {:02}  ROW {:03}  T{} / N{}", self.editor.occurrence, self.editor.row, self.editor.track.0, self.editor.column + 1));
                ui.label(RichText::new(format!("1 row = {:.2} ms · local to this note column", 60_000.0 / (self.editor.song.bpm * f64::from(self.editor.song.lines_per_beat)))).small().color(MUTED));
                ui.separator();
                egui::ScrollArea::vertical().id_salt("step-command-fields").max_height(360.0).show(ui, |ui| {
                    ui.add_enabled_ui(writable, |ui| self.command_fields(ui));
                });
                ui.separator();
                let effective = self.effects_draft.effective();
                let pending = !effective.as_ref().is_ok_and(|value| value == &current);
                if let Err(error) = &effective { ui.colored_label(Color32::LIGHT_RED, error); }
                ui.label(RichText::new(if !self.editor.edit_mode { "READ ONLY · enable Edit mode to change commands" } else if pending { "PENDING DRAFT · not in the song until Apply" } else { "Current song commands · no pending changes" }).small().color(if pending { PLAY } else { MUTED }));
                ui.horizontal(|ui| {
                    if ui.add_enabled(writable && pending && effective.is_ok(), egui::Button::new("Apply commands")).clicked() {
                        let result = effective.clone().and_then(|value| self.editor.set_effects(value)); self.result(result);
                    }
                    if ui.add_enabled(pending, egui::Button::new("Reset draft")).clicked() { self.effects_draft.set_value(current.clone()); }
                    if ui.add_enabled(writable && !current.is_empty(), egui::Button::new("Clear commands")).clicked() {
                        let result = self.editor.set_effects(StepEffects::default()); self.result(result);
                    }
                    if ui.button("Close").clicked() { ui.close(); }
                });
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
                ui.label(RichText::new("Offset/direction require mono/stereo samples without embedded loops and use linear interpolation from voice start; quality can differ. Leviathan commands, not Renoise FX codes.").small().color(MUTED));
            });
        if contents.is_some() {
            // A pending command draft must never receive the grid's musical shortcuts.
            ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        }
        response.on_hover_text("Inspect or edit row-local commands for the selected note cell. Draft values apply only on explicit Apply. Keyboard fields remain Note / Instrument / Velocity / Pan / Delay.");
    }
    fn command_fields(&mut self, ui: &mut egui::Ui) {
        let phrase_trigger = self.editor.cell().phrase.is_some();
        if phrase_trigger {
            ui.label(RichText::new("PHRASE INSTANCE · only end action applies here; edit inner notes for other FX.").small().color(EFFECT_INK));
        }
        ui.add_enabled_ui(!phrase_trigger, |ui| {
        ui.horizontal(|ui| {
            let mut enabled = self.effects_draft.value.retrigger.is_some();
            if ui.checkbox(&mut enabled, "Retrigger").changed() {
                self.effects_draft.value.retrigger = enabled.then_some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                });
                self.effects_draft.retrigger_spacing = if enabled { "0.25".into() } else { String::new() };
            }
            if let Some(retrigger) = &mut self.effects_draft.value.retrigger {
                ui.add(egui::DragValue::new(&mut retrigger.repeats).range(1..=16));
                ui.label("extra starts");
            }
        });
        if self.effects_draft.value.retrigger.is_some() {
            ui.horizontal(|ui| {
                ui.label("Spacing");
                ui.add(egui::TextEdit::singleline(&mut self.effects_draft.retrigger_spacing).desired_width(125.0))
                    .on_hover_text(format!("Exact draft spacing: {} rows. Decimal or scientific notation; no rounding or clamping.", self.effects_draft.retrigger_spacing));
                ui.label("rows apart");
            });
        }
        ui.label(RichText::new("Pitched note required. Delay + repeats × spacing < 1 row. Each start reuses this cell's initial offset/direction.").small().color(MUTED));
        });
        let mut end_action = EndAction::from_effects(&self.effects_draft.value);
        let previous = end_action;
        ui.horizontal_wrapped(|ui| {
            ui.label("End action");
            for action in [EndAction::None, EndAction::Release, EndAction::Cut] {
                ui.selectable_value(&mut end_action, action, action.label());
            }
        });
        if end_action != previous {
            end_action.apply(&mut self.effects_draft.value);
        }
        let position = match end_action {
            EndAction::Cut => self.effects_draft.value.cut.as_mut(),
            EndAction::Release => self.effects_draft.value.release.as_mut(),
            EndAction::None => None,
        };
        if let Some(position) = position {
            ui.horizontal(|ui| {
                ui.label("At");
                ui.add(
                    egui::DragValue::new(position)
                        .range(0.0..=1.0)
                        .speed(0.005)
                        .max_decimals(6),
                );
                ui.label("of row, absolute");
            });
        }
        ui.label(RichText::new(match end_action {
            EndAction::Cut if phrase_trigger => "Hard cut cancels this phrase instance and kills all its voices/tails; other instances continue.",
            EndAction::Release if phrase_trigger => "Timed release cancels this phrase instance using region ADSR releases or legacy de-click; R=0 is abrupt. Other instances continue.",
            EndAction::Cut => "Hard cut stops the active logical voice immediately; may click. Earlier release tails remain.",
            EndAction::Release => "Timed release sends NoteOff: region ADSR release or legacy exponential de-click. R=0 stops abruptly and may click; earlier release tails remain.",
            EndAction::None => "Choose one: timed release or hard cut. These are distinct end actions.",
        }).small().color(MUTED));
        ui.label(
            RichText::new(
                "End position: 0 ≤ fraction < 1, at/after delay and final retrigger. No OFF cells.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_enabled_ui(!phrase_trigger, |ui| {
        ui.horizontal(|ui| {
            let mut enabled = self.effects_draft.value.offset.is_some();
            if ui.checkbox(&mut enabled, "Source offset").changed() {
                self.effects_draft.value.offset = enabled.then_some(0);
            }
            if let Some(offset) = &mut self.effects_draft.value.offset {
                ui.add(
                    egui::DragValue::new(offset)
                        .range(0..=u32::MAX as u64)
                        .speed(1.0),
                );
                ui.label("source frame (0-based)");
            }
        });
        ui.label(RichText::new("Source-rate frames, not output frames or percent. Must be inside the decoded sample; checked when rendering.").small().color(MUTED));
        ui.horizontal(|ui| {
            let mut enabled = self.effects_draft.value.direction.is_some();
            if ui.checkbox(&mut enabled, "Direction").changed() {
                self.effects_draft.value.direction = enabled.then_some(Direction::Reverse);
            }
            if let Some(direction) = &mut self.effects_draft.value.direction {
                ui.selectable_value(direction, Direction::Forward, "Forward");
                ui.selectable_value(direction, Direction::Reverse, "Reverse");
            }
        });
        ui.label(RichText::new("Note: Reverse with no offset starts at the last source frame. Empty: seek/flip an active voice without restarting. No OFF cells; no latching to later notes.").small().color(MUTED));
        ui.horizontal(|ui| {
            let mut enabled = self.effects_draft.value.glide.is_some();
            if ui.checkbox(&mut enabled, "Glide").changed() {
                self.effects_draft.value.glide = enabled.then_some(Glide {
                    target: match self.editor.cell().note {
                        Note::Note(pitch) => pitch,
                        _ => 60,
                    },
                    semitones_per_second: 12.0,
                });
            }
            if let Some(glide) = &mut self.effects_draft.value.glide {
                ui.label("MIDI");
                ui.add(egui::DragValue::new(&mut glide.target).range(0..=127));
                ui.add(
                    egui::DragValue::new(&mut glide.semitones_per_second)
                        .range(0.01..=120.0)
                        .speed(0.25)
                        .max_decimals(2),
                );
                ui.label("semitones/s");
            }
        });
        ui.label(RichText::new("Changes the active/new voice toward its target; does not latch to later notes. No OFF cells.").small().color(MUTED));
        });
    }
    fn enter_phrase(&mut self, id: PhraseId) {
        if self.phrase_target.is_none() {
            self.song_view = Some((self.scroll_offset, self.viewport));
        }
        if self.phrase_target != Some(id) {
            self.phrase_cursor = None;
            self.scroll_offset = Vec2::ZERO;
            self.viewport = Vec2::ZERO;
        }
        self.phrase_target = Some(id);
        self.phrase_selected = Some(id);
        self.phrase_browser_open = true;
        self.effects_draft = EffectsDraft::default();
        self.value.clear();
        self.scroll_to_cursor = false;
    }
    pub(crate) fn open_song_position(
        &mut self,
        occurrence: usize,
        track: TrackId,
    ) -> Result<(), String> {
        self.editor.select(occurrence, track)?;
        self.return_to_song();
        self.editor.row = 0;
        self.editor.column = 0;
        self.editor.field = Field::Note;
        self.scroll_offset = egui::vec2(
            Selection::from_editor(&self.editor)
                .bounds(&self.editor.song, occurrence)
                .left(),
            0.0,
        );
        self.scroll_to_cursor = true;
        Ok(())
    }
    fn return_to_song(&mut self) {
        self.phrase_target = None;
        self.projection_active = false;
        self.effects_draft = EffectsDraft::default();
        self.value.clear();
        if let Some((offset, viewport)) = self.song_view.take() {
            self.scroll_offset = offset;
            self.viewport = viewport;
        }
    }
    fn phrase_browser(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("INSTRUMENT PHRASES")
                .strong()
                .color(EFFECT_INK),
        );
        if !self
            .editor
            .song
            .instruments
            .contains_key(&InstrumentId(self.phrase_owner))
        {
            self.phrase_owner = self
                .editor
                .song
                .instruments
                .keys()
                .next()
                .map_or(0, |id| id.0);
        }
        let before = self.phrase_owner;
        egui::ComboBox::from_id_salt("phrase-owner")
            .width(175.0)
            .selected_text(
                self.editor
                    .song
                    .instruments
                    .get(&InstrumentId(self.phrase_owner))
                    .map_or("No instruments".into(), |instrument| {
                        format!("{} · {}", self.phrase_owner, instrument.name)
                    }),
            )
            .show_ui(ui, |ui| {
                for (id, instrument) in &self.editor.song.instruments {
                    ui.selectable_value(
                        &mut self.phrase_owner,
                        id.0,
                        format!("{} · {}", id.0, instrument.name),
                    );
                }
            });
        if before != self.phrase_owner {
            self.phrase_selected = None;
            self.phrase_settings = None;
        }
        let phrases: Vec<_> = self
            .editor
            .song
            .phrases
            .iter()
            .filter(|(_, phrase)| phrase.instrument == InstrumentId(self.phrase_owner))
            .map(|(id, phrase)| (*id, phrase.name.clone()))
            .collect();
        if !phrases
            .iter()
            .any(|(id, _)| Some(*id) == self.phrase_selected)
        {
            self.phrase_selected = phrases.first().map(|(id, _)| *id);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.editor.edit_mode && self.phrase_owner != 0,
                    egui::Button::new("+ New"),
                )
                .clicked()
            {
                let next = self
                    .editor
                    .song
                    .phrases
                    .keys()
                    .next_back()
                    .map_or(Some(1), |id| id.0.checked_add(1));
                if let Some(next) = next {
                    let id = PhraseId(next);
                    let phrase = Phrase {
                        name: format!("Phrase {next}"),
                        instrument: InstrumentId(self.phrase_owner),
                        rows: 16,
                        lines_per_beat: self.editor.song.lines_per_beat,
                        note_columns: 1,
                        root_note: 60,
                        cells: BTreeMap::new(),
                    };
                    let result = self.editor.upsert_phrase(id, phrase);
                    let success = result.is_ok();
                    self.result(result);
                    if success {
                        self.enter_phrase(id);
                        ui.ctx().request_repaint();
                    }
                } else {
                    self.error = Some("Phrase ID space exhausted".into());
                }
            }
            if ui
                .add_enabled(
                    self.phrase_selected.is_some(),
                    egui::Button::new("Edit phrase"),
                )
                .clicked()
            {
                if let Some(id) = self.phrase_selected {
                    self.enter_phrase(id);
                    ui.ctx().request_repaint();
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("phrase-list")
            .max_height((ui.available_height() - 135.0).max(55.0))
            .show(ui, |ui| {
                if phrases.is_empty() {
                    ui.label(
                        RichText::new("No phrases for this instrument.")
                            .small()
                            .color(MUTED),
                    );
                }
                for (id, name) in &phrases {
                    if ui
                        .selectable_label(
                            self.phrase_selected == Some(*id),
                            format!("{:02}  {name}", id.0),
                        )
                        .clicked()
                    {
                        self.phrase_selected = Some(*id);
                    }
                }
            });
        if let Some(id) = self.phrase_selected {
            if let Some(phrase) = self.editor.song.phrases.get(&id).cloned() {
                ui.label(
                    RichText::new(format!(
                        "{} rows · {} columns · LPB {}\nRoot {} (MIDI {})",
                        phrase.rows,
                        phrase.note_columns,
                        phrase.lines_per_beat,
                        note_label(Note::Note(phrase.root_note)),
                        phrase.root_note
                    ))
                    .small()
                    .color(MUTED),
                );
                let source_changed = self
                    .phrase_settings
                    .as_ref()
                    .is_none_or(|(current_id, source, _)| *current_id != id || source != &phrase);
                if source_changed {
                    self.phrase_settings = Some((id, phrase.clone(), phrase));
                }
                let writable = self.editor.edit_mode && ui.is_enabled();
                let (_, open) = egui::containers::menu::MenuButton::new("Phrase settings")
                    .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                    .ui(ui, |ui| {
                        ui.set_max_width(340.0);
                        ui.label(RichText::new("PHRASE DEFINITION").strong());
                        let draft = &mut self.phrase_settings.as_mut().unwrap().2;
                        ui.add_enabled_ui(writable, |ui| {
                            ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(230.0).hint_text("Phrase name"));
                            ui.horizontal(|ui| { ui.label("Rows"); ui.add(egui::DragValue::new(&mut draft.rows).range(1..=1024)); ui.label("Columns"); ui.add(egui::DragValue::new(&mut draft.note_columns).range(1..=16)); });
                            ui.horizontal(|ui| { ui.label("Own LPB"); ui.add(egui::DragValue::new(&mut draft.lines_per_beat).range(1..=256)); ui.label("Root MIDI"); ui.add(egui::DragValue::new(&mut draft.root_note).range(0..=127)); });
                        });
                        ui.label(RichText::new("Own LPB sets phrase clock. Root sets transposition reference; changing trigger pitch does not change phrase speed. Shrink never discards populated cells.").small().color(MUTED));
                        if ui.add_enabled(writable, egui::Button::new("Apply definition")).clicked() {
                            let candidate = self.phrase_settings.as_ref().unwrap().2.clone();
                            let result = self.editor.upsert_phrase(id, candidate); self.result(result);
                        }
                        if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
                    });
                if open.is_some() {
                    ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
                }
                if ui
                    .add_enabled(writable, egui::Button::new("Remove phrase"))
                    .on_hover_text("Refuses while referenced by song cells; removal is undoable")
                    .clicked()
                {
                    let result = self.editor.remove_phrase(id);
                    self.result(result);
                    ui.ctx().request_repaint();
                }
            }
        }
        ui.label(
            RichText::new("Shared definition · every trigger uses these notes. No nested phrases.")
                .small()
                .color(MUTED),
        );
    }
    fn phrase_trigger_menu(&mut self, ui: &mut egui::Ui) {
        let cell = self.editor.cell();
        let source = (
            self.editor.revision(),
            self.editor.occurrence,
            self.editor.track,
            self.editor.column,
            self.editor.row,
            self.editor.edit_mode,
            cell.phrase.clone(),
        );
        if self.trigger_source.as_ref() != Some(&source) {
            self.trigger_draft = cell.phrase.clone();
            self.trigger_source = Some(source);
        }
        let writable = self.editor.edit_mode && ui.is_enabled() && !cell_is_read_only(&self.editor);
        let label = cell.phrase.as_ref().map_or("Trigger".into(), |trigger| {
            format!("Phrase P{}", trigger.phrase.0)
        });
        let (_, contents) = egui::containers::menu::MenuButton::new(label)
            .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
                ui.set_max_width(430.0);
                ui.label(RichText::new("SONG CELL / PHRASE TRIGGER").strong().color(EFFECT_INK));
                if cell_is_read_only(&self.editor) {ui.colored_label(PLAY,"ALIASED SOURCE · read only here; Edit source or Make unique");}
                ui.monospace(format!("POS {:02} ROW {:03} T{} / N{}", self.editor.occurrence, self.editor.row, self.editor.track.0, self.editor.column + 1));
                egui::ScrollArea::vertical().id_salt("trigger-phrases").max_height(125.0).show(ui, |ui| {
                    if self.editor.song.phrases.is_empty() { ui.label("No phrase definitions. Open Phrases > + New first."); }
                    for (id, phrase) in &self.editor.song.phrases {
                        if ui.selectable_label(self.trigger_draft.as_ref().is_some_and(|trigger| trigger.phrase == *id), format!("P{:02}  {} · instrument {}", id.0, phrase.name, phrase.instrument.0)).clicked() {
                            let cycles = self.trigger_draft.as_ref().map_or(1, |trigger| trigger.cycles);
                            self.trigger_draft = Some(PhraseTrigger { phrase: *id, cycles });
                        }
                    }
                });
                if let Some(trigger) = &mut self.trigger_draft {
                    ui.horizontal(|ui| { ui.label("Total cycles"); ui.add_enabled(writable, egui::DragValue::new(&mut trigger.cycles).range(1..=64)); ui.label("1 = one-shot"); });
                    if let Some(phrase) = self.editor.song.phrases.get(&trigger.phrase) {
                        ui.label(format!("{} rows · own LPB {} · fixed instrument {}", phrase.rows, phrase.lines_per_beat, phrase.instrument.0));
                        ui.label(match cell.note { Note::Note(pitch) => format!("{} / root {} = {:+} semitones; clock unchanged", note_label(cell.note), note_label(Note::Note(phrase.root_note)), i16::from(pitch) - i16::from(phrase.root_note)), _ => "Enter a pitched note at this song cell before applying.".into() });
                    }
                }
                ui.label(RichText::new("Velocity/pan are snapshots at trigger time. Later parent volume/pan edits affect future triggers, not this running instance. OFF or replacement cancels only this column's instance.").small().color(MUTED));
                ui.label(RichText::new("Only timed release / hard cut are supported on a trigger. Other FX belong inside the phrase. Incompatible existing fields are rejected, never silently cleared.").small().color(MUTED));
                let pending = self.trigger_draft != cell.phrase;
                ui.label(RichText::new(if pending { "PENDING · Apply attaches the selected phrase" } else { "Current song-cell trigger" }).small().color(if pending { PLAY } else { MUTED }));
                ui.horizontal(|ui| {
                    if ui.add_enabled(writable && pending && matches!(cell.note, Note::Note(_)), egui::Button::new("Apply trigger")).clicked() { let result = self.editor.set_phrase_trigger(self.trigger_draft.clone()); self.result(result); }
                    if ui.add_enabled(writable && cell.phrase.is_some(), egui::Button::new("Clear trigger")).clicked() { let result = self.editor.set_phrase_trigger(None); self.result(result); }
                    if ui.button("Reset draft").clicked() { self.trigger_draft = cell.phrase.clone(); }
                    if ui.button("Close").clicked() { ui.close(); }
                });
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
            });
        if contents.is_some() {
            ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        }
    }
    fn show_phrase(&mut self, ui: &mut egui::Ui) {
        if ui.is_enabled() && ui.memory(|memory| memory.has_focus(grid_id())) {
            for event in ui.input(|input| input.events.clone()) {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                {
                    if modifiers.command || modifiers.ctrl {
                        apply_history_key(&mut self.editor, key, modifiers);
                    }
                }
            }
        }
        let Some(id) = self.phrase_target else {
            return;
        };
        let Some(phrase) = self.editor.song.phrases.get(&id).cloned() else {
            self.return_to_song();
            ui.label("Phrase no longer exists. Returning to song…");
            ui.ctx().request_repaint();
            return;
        };
        let mut back = false;
        ui.horizontal_wrapped(|ui| {
            back = ui.button("< Song").clicked();
            ui.label(
                RichText::new(format!("PHRASE {:02}  {}", id.0, phrase.name))
                    .strong()
                    .color(EFFECT_INK),
            );
            ui.monospace(format!(
                "INST {} fixed · ROOT {} · LPB {}",
                phrase.instrument.0,
                note_label(Note::Note(phrase.root_note)),
                phrase.lines_per_beat
            ));
            ui.label(
                RichText::new("Song transport plays triggers; no standalone preview.")
                    .small()
                    .color(MUTED),
            );
        });
        if back {
            self.return_to_song();
            ui.ctx().request_repaint();
            return;
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.separator();
        let available = ui.available_size();
        let order_width = if available.x >= 850.0 { 196.0 } else { 170.0 };
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(order_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(order_width);
                    self.phrase_browser(ui);
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                let Some(id) = self.phrase_target else {
                    return;
                };
                let Some(phrase) = self.editor.song.phrases.get(&id).cloned() else {
                    return;
                };
                let projection = match phrase_projection(&self.editor, &phrase, self.phrase_cursor)
                {
                    Ok(editor) => editor,
                    Err(error) => {
                        self.error = Some(error);
                        return;
                    }
                };
                let parent = std::mem::replace(&mut self.editor, projection);
                self.projection_active = true;
                self.toolbar(ui);
                ui.separator();
                self.editor_grid(ui);
                self.editor_keys(ui);
                self.phrase_cursor = Some(Selection::from_editor(&self.editor));
                self.projection_active = false;
                let projection = std::mem::replace(&mut self.editor, parent);
                self.editor.edit_mode = projection.edit_mode;
                self.editor.edit_step = projection.edit_step;
                self.editor.octave = projection.octave;
                let mut candidate = phrase.clone();
                candidate.cells = projection
                    .pattern()
                    .tracks
                    .get(&TrackId(1))
                    .cloned()
                    .unwrap_or_default();
                if candidate != phrase {
                    let result = self.editor.upsert_phrase(id, candidate);
                    self.result(result);
                    ui.ctx().request_repaint();
                }
            });
        });
    }
    fn open_matrix_block(&mut self) {
        if !self.editor.has_grid() {
            return;
        }
        let pattern = self.editor.pattern_id();
        let track = self.editor.track;
        let existing = self.editor.pattern().track_aliases.get(&track).cloned();
        let mut source = existing.unwrap_or(TrackAlias { pattern, track });
        if existing.is_none() {
            'source: for (id, info) in &self.editor.song.patterns {
                if info.rows != self.editor.pattern().rows {
                    continue;
                }
                for candidate in &self.editor.song.track_order {
                    if (*id, *candidate) != (pattern, track)
                        && self.editor.song.tracks[candidate].note_columns
                            == self.editor.song.tracks[&track].note_columns
                    {
                        source = TrackAlias {
                            pattern: *id,
                            track: *candidate,
                        };
                        break 'source;
                    }
                }
            }
        }
        self.matrix_draft = Some(MatrixDraft {
            revision: self.editor.revision(),
            occurrence: self.editor.occurrence,
            pattern,
            track,
            source,
        });
    }
    fn source_destination(&self) -> Option<(usize, TrackId)> {
        let (pattern, track) = self.editor.content_source().ok()?;
        if pattern == self.editor.pattern_id() {
            Some((self.editor.occurrence, track))
        } else {
            self.editor
                .song
                .order
                .iter()
                .position(|order| order.pattern == pattern)
                .map(|index| (index, track))
        }
    }
    fn edit_source(&mut self) {
        let result=self.source_destination().ok_or_else(|| "Source is not in the arrangement. Make unique here; no source occurrence is added automatically.".to_string()).and_then(|(index,track)|self.editor.select(index,track));
        if result.is_ok() {
            self.matrix_draft = None;
            self.value.clear();
            self.scroll_to_cursor = true;
            self.steps.open = false;
        }
        self.result(result);
    }
    fn matrix_notice(&mut self, ui: &mut egui::Ui) {
        let pattern = self.editor.pattern_id();
        let track = self.editor.track;
        let aliased = cell_is_read_only(&self.editor);
        let muted = self.editor.song.order[self.editor.occurrence]
            .muted_tracks
            .contains(&track);
        if !aliased && !muted {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if aliased {
                let source=self.editor.content_source().ok();
                ui.label(RichText::new(source.map_or("ALIAS / INVALID SOURCE".into(),|(p,t)|format!("READ ONLY · P{}/T{} displays P{}/T{}",pattern.0,track.0,p.0,t.0))).small().color(PLAY));
                let destination=self.source_destination();
                if ui.add_enabled(destination.is_some(),egui::Button::new(destination.map_or("Edit source unavailable".into(),|(index,_)|format!("Edit source · POS {index:02}")))).on_disabled_hover_text("Source is not in the arrangement; Make unique here. No occurrence is added implicitly.").clicked(){self.edit_source();}
                if ui.add_enabled(self.editor.edit_mode,egui::Button::new("Make unique")).on_hover_text("Copy the terminal source block locally. Other occurrences of this target pattern still share this local block; source/other aliases are unchanged.").clicked(){let result=self.editor.make_track_unique(pattern,track);self.result(result);}
            }
            if muted {ui.label(RichText::new(format!("X · muted only at POS {:02}, T{}",self.editor.occurrence,track.0)).small().color(PLAY));}
            if ui.small_button("Block…").clicked(){self.open_matrix_block();}
        });
    }
    fn matrix_window(&mut self, ui: &mut egui::Ui) {
        let Some(mut draft) = self.matrix_draft.take() else {
            return;
        };
        if !self.editor.has_grid()
            || draft.revision != self.editor.revision()
            || draft.occurrence != self.editor.occurrence
            || draft.pattern != self.editor.pattern_id()
            || draft.track != self.editor.track
        {
            return;
        }
        let enabled = ui.is_enabled();
        let writable = enabled && self.editor.edit_mode;
        let local_count = self
            .editor
            .pattern()
            .tracks
            .get(&draft.track)
            .map_or(0, |rows| rows.values().map(BTreeMap::len).sum::<usize>());
        let alias = self
            .editor
            .pattern()
            .track_aliases
            .get(&draft.track)
            .cloned();
        let source = self.editor.content_source().ok();
        let uses = self
            .editor
            .song
            .order
            .iter()
            .filter(|order| order.pattern == draft.pattern)
            .count();
        let mut muted = self.editor.song.order[draft.occurrence]
            .muted_tracks
            .contains(&draft.track);
        let mut action = None;
        let mut navigate = false;
        let mut open = true;
        let mut cancel = false;
        egui::Window::new("Matrix block").id(egui::Id::new("matrix-block-window")).open(&mut open).collapsible(false).resizable(false).default_width(470.0).default_pos(egui::pos2(220.0,170.0)).max_height((ui.ctx().content_rect().height()-260.0).max(200.0)).vscroll(true).show(ui.ctx(),|ui| {
            ui.set_max_width(470.0);ui.monospace(format!("POS {:02} · target P{} / T{}",draft.occurrence,draft.pattern.0,draft.track.0));
            ui.label(RichText::new(format!("Pattern content shared by {uses} ordered position(s). Linking/localizing affects all of them.")).small().color(MUTED));
            if let Some(alias)=&alias {
                ui.label(format!("Direct link: P{} / T{}",alias.pattern.0,alias.track.0));
                if let Some((p,t))=source {ui.label(RichText::new(format!("Resolved terminal: P{} / T{} · target is read only",p.0,t.0)).color(PLAY));}
            } else {ui.label(format!("Local block · {local_count} stored cell(s)"));}
            ui.separator();
            ui.add_enabled_ui(enabled,|ui| {
                ui.horizontal(|ui|{
                    ui.label("Source pattern");egui::ComboBox::from_id_salt("alias-source-pattern").selected_text(format!("P{}",draft.source.pattern.0)).show_ui(ui,|ui|{for (id,pattern) in &self.editor.song.patterns {ui.selectable_value(&mut draft.source.pattern,*id,format!("P{} · {} · {} rows",id.0,pattern.name,pattern.rows));}});
                    ui.label("Track");egui::ComboBox::from_id_salt("alias-source-track").selected_text(format!("T{}",draft.source.track.0)).show_ui(ui,|ui|{for id in &self.editor.song.track_order {ui.selectable_value(&mut draft.source.track,*id,format!("T{} · {}",id.0,self.editor.song.tracks[id].name));}});
                });
                ui.label(RichText::new("Equal rows and note columns required. Cycles refused; no hidden local data or automation aliasing.").small().color(MUTED));
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(writable&&local_count==0,egui::Button::new("Link source")).on_disabled_hover_text("Enable Edit mode and choose an empty local target; linking never discards local cells.").clicked(){action=Some(MatrixAction::Link(Some(draft.source)));}
                    if ui.add_enabled(writable&&alias.is_some(),egui::Button::new("Make unique")).on_hover_text("Materialize the terminal block here; keep source and other aliases unchanged.").clicked(){action=Some(MatrixAction::Unique);}
                    if ui.add_enabled(writable&&alias.is_some(),egui::Button::new("Unlink to empty")).on_hover_text("Remove only this pattern's link, leaving an empty local block. Source data stays intact; Undo restores link.").clicked(){action=Some(MatrixAction::Link(None));}
                });
                if local_count>0 {ui.label(RichText::new("Link unavailable: target has local cells. Choose another empty block; no automatic clearing.").small().color(PLAY));}
                let destination=self.source_destination();
                if alias.is_some() {
                    if ui.add_enabled(destination.is_some(),egui::Button::new(destination.map_or("Edit source unavailable".into(),|(index,track)|format!("Edit source · POS {index:02} / T{}",track.0)))).clicked(){navigate=true;}
                    if destination.is_none(){ui.label(RichText::new("Source is not in the arrangement. Make unique here. This GUI does not append an arbitrary source occurrence.").small().color(PLAY));}
                }
                ui.separator();
                if ui.add_enabled(writable,egui::Checkbox::new(&mut muted,"X · mute this occurrence's track")).changed(){action=Some(MatrixAction::Mute(muted));}
                ui.label(RichText::new("Only this POS × track. Entry releases held voices/cancels pending phrases; fading tails may remain. Not hard-zero, global mute or automation.").small().color(MUTED));
                ui.separator();
                if ui.button("Close").clicked(){cancel=true;}
            });
            if let Some(error)=&self.error {ui.colored_label(Color32::LIGHT_RED,error);}
        });
        ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        if navigate {
            self.edit_source();
            return;
        }
        if let Some(action) = action {
            let result = match action {
                MatrixAction::Link(source) => {
                    self.editor
                        .set_track_alias(draft.pattern, draft.track, source)
                }
                MatrixAction::Unique => self.editor.make_track_unique(draft.pattern, draft.track),
                MatrixAction::Mute(muted) => {
                    self.editor
                        .set_occurrence_track_mute(draft.occurrence, draft.track, muted)
                }
            };
            if result.is_ok() {
                draft.revision = self.editor.revision();
                self.value.clear();
                self.scroll_to_cursor = true;
            }
            self.result(result);
        }
        if open && !cancel {
            self.matrix_draft = Some(draft);
        }
    }
    fn order_matrix(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ORDER").strong().color(MUTED));
            if ui.small_button("Block…").clicked() {
                self.open_matrix_block();
            }
            ui.label(
                RichText::new(format!("{} positions", self.editor.song.order.len()))
                    .small()
                    .weak(),
            );
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.editor.edit_mode, egui::Button::new("+ Repeat"))
                .on_hover_text("Append a reference to this pattern; edits stay shared")
                .clicked()
            {
                let result = self.editor.append(false);
                self.result(result);
                self.scroll_to_cursor = true;
            }
            if ui
                .add_enabled(self.editor.edit_mode, egui::Button::new("+ Clone"))
                .on_hover_text("Append an independent pattern copy")
                .clicked()
            {
                let result = self.editor.append(true);
                self.result(result);
                self.scroll_to_cursor = true;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.editor.edit_mode && self.editor.occurrence > 0,
                    egui::Button::new("Move up"),
                )
                .clicked()
            {
                let result = self.editor.move_occurrence(-1);
                self.result(result);
            }
            if ui
                .add_enabled(
                    self.editor.edit_mode
                        && self.editor.occurrence + 1 < self.editor.song.order.len(),
                    egui::Button::new("Down"),
                )
                .clicked()
            {
                let result = self.editor.move_occurrence(1);
                self.result(result);
            }
            if ui
                .add_enabled(
                    self.editor.edit_mode && self.editor.song.order.len() > 1,
                    egui::Button::new("Remove"),
                )
                .on_hover_text("Remove this order occurrence only. Keep the pattern, shared references and all sample mappings. Undo restores the occurrence and cursor.")
                .on_disabled_hover_text("Enable Edit mode; keep at least one order occurrence. No content is removed automatically.")
                .clicked()
            {
                let result = self.editor.remove_occurrence();
                self.result(result);
                self.scroll_to_cursor = true;
                self.value.clear();
            }
        });
        ui.separator();
        egui::ScrollArea::both()
            .id_salt("order-matrix")
            .auto_shrink([false, false])
            .max_height((ui.available_height() - 36.0).max(40.0))
            .show(ui, |ui| {
                egui::Grid::new("matrix")
                    .min_col_width(18.0)
                    .spacing([3.0, 3.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("POS / PAT").monospace().small().color(MUTED));
                        let tracks = self.editor.song.track_order.clone();
                        for track in &tracks {
                            ui.label(
                                RichText::new(format!("{:02}", track.0))
                                    .monospace()
                                    .small()
                                    .color(MUTED),
                            );
                        }
                        ui.end_row();
                        for index in 0..self.editor.song.order.len() {
                            let id = self.editor.song.order[index].pattern;
                            if ui
                                .selectable_label(
                                    self.editor.occurrence == index,
                                    RichText::new(format!("{index:02} / {:02}", id.0)).monospace(),
                                )
                                .clicked()
                            {
                                let result = self.editor.select(index, self.editor.track);
                                self.result(result);
                                self.scroll_to_cursor = true;
                            }
                            for track in &tracks {
                                let source = self.editor.song.resolve_track_source(id, *track).ok();
                                let aliased = self.editor.song.patterns[&id]
                                    .track_aliases
                                    .contains_key(track);
                                let muted =
                                    self.editor.song.order[index].muted_tracks.contains(track);
                                let populated = self
                                    .editor
                                    .song
                                    .resolved_track(id, *track)
                                    .ok()
                                    .flatten()
                                    .is_some_and(|rows| {
                                        rows.values().any(|columns| {
                                            columns.values().any(|cell| *cell != Cell::default())
                                        })
                                    });
                                if ui
                                    .selectable_label(
                                        self.editor.occurrence == index
                                            && self.editor.track == *track,
                                        RichText::new(if muted && aliased {
                                            "XL"
                                        } else if muted {
                                            "X"
                                        } else if aliased {
                                            "L"
                                        } else if populated {
                                            "■"
                                        } else {
                                            "·"
                                        })
                                        .color(if muted {
                                            PLAY
                                        } else if populated || aliased {
                                            AQUA
                                        } else {
                                            EMPTY
                                        }),
                                    )
                                    .on_hover_text(format!(
                                        "POS {index:02} · target P{} / T{}
{}
{}
{}",
                                        id.0,
                                        track.0,
                                        self.editor.song.tracks[track].name,
                                        source.map_or("Invalid source".into(), |(p, t)| format!(
                                            "Resolved source P{}/T{}{}",
                                            p.0,
                                            t.0,
                                            if aliased {
                                                " · read only; Block… for source/unique"
                                            } else {
                                                " · local"
                                            }
                                        )),
                                        if muted {
                                            "X: muted only in this occurrence"
                                        } else if populated {
                                            "Contains events"
                                        } else {
                                            "Empty track block"
                                        }
                                    ))
                                    .clicked()
                                {
                                    let result = self.editor.select(index, *track);
                                    self.result(result);
                                    self.scroll_to_cursor = true;
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        ui.label(
            RichText::new("Repeated IDs share content.\n■ events  · empty  L alias  X mute")
                .small()
                .color(MUTED),
        );
    }
    fn editor_grid(&mut self, ui: &mut egui::Ui) {
        let rectangle = if self.projection_active {
            None
        } else {
            self.cell_blocks.highlight(&self.editor)
        };
        let selected = Selection::from_editor(&self.editor);
        // A deliberate pointer/scroll action takes ownership back from a settling
        // explicit reveal. No later manual viewport changes are recentered.
        if self.scroll_to_cursor
            && ui.rect_contains_pointer(ui.available_rect_before_wrap())
            && ui.input(|input| {
                input.raw_scroll_delta != Vec2::ZERO
                    || input.pointer.any_pressed()
                    || input.pointer.is_decidedly_dragging()
            })
        {
            self.scroll_to_cursor = false;
            self.reveal_last_viewport = None;
            self.reveal_stable_frames = 0;
        }
        let requested = if self.scroll_to_cursor {
            self.scroll_to_cursor = false;
            Some(reveal_selection(
                self.scroll_offset,
                self.viewport,
                selected.bounds(&self.editor.song, self.editor.occurrence),
            ))
        } else {
            None
        };
        let result = show_grid(
            ui,
            GridView {
                song: &self.editor.song,
                occurrence: self.editor.occurrence,
                selection: Some(selected),
                rectangle: rectangle.as_ref(),
                playhead: self
                    .playhead
                    .filter(|_| !self.projection_active)
                    .filter(|(occ, _)| *occ == self.editor.occurrence)
                    .map(|(_, row)| row),
                editing: self.editor.edit_mode,
                focused: ui.memory(|m| m.has_focus(grid_id())),
                scroll: requested,
                id: if self.projection_active {
                    "phrase-tracker-scroll"
                } else {
                    "tracker-scroll"
                },
            },
        );
        self.scroll_offset = result.offset;
        self.viewport = result.viewport;
        if requested.is_some() {
            // ScrollArea may clamp against its previous/animated bar geometry.
            // Retain only this explicit request until its actual measured viewport
            // is stable; do not require a guessed scrollbar state to be reached.
            self.reveal_stable_frames = if self.reveal_last_viewport == Some(result.viewport) {
                self.reveal_stable_frames.saturating_add(1)
            } else {
                0
            };
            self.reveal_last_viewport = Some(result.viewport);
            self.scroll_to_cursor = !result.viewport_settled || self.reveal_stable_frames < 2;
            if self.scroll_to_cursor {
                ui.ctx().request_repaint();
            } else {
                self.reveal_last_viewport = None;
                self.reveal_stable_frames = 0;
            }
        }
        if let Some(selected) = result.clicked {
            self.editor.track = selected.track;
            self.editor.column = selected.column;
            self.editor.row = selected.row;
            self.editor.field = selected.field;
            self.value.clear();
        }
        let response = ui.interact(
            result.rect,
            grid_id(),
            egui::Sense::focusable_noninteractive(),
        );
        // Register the enclosing focus target before requesting focus: egui can
        // otherwise treat the child field's click as a click outside this target.
        if result.clicked.is_some() {
            ui.memory_mut(|memory| memory.request_focus(grid_id()));
        }
        if response.has_focus() || result.clicked.is_some() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    grid_id(),
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                )
            });
        }
    }
    fn editor_keys(&mut self, ui: &mut egui::Ui) {
        let before = Selection::from_editor(&self.editor);
        let result = if ui.is_enabled() {
            if !self.projection_active {
                self.cell_blocks.shortcuts(&self.editor, ui.ctx());
            }
            keyboard_input(&mut self.editor, ui.ctx(), !self.projection_active)
        } else {
            Ok(())
        };
        if before != Selection::from_editor(&self.editor) {
            self.reveal_current_cursor();
            self.value.clear();
            ui.ctx().request_repaint();
        }
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if self.phrase_target.is_some() {
            self.cell_blocks.close();
            self.thin.reset_document();
            self.grooves.reset_document();
            if self.generation.is_open() {
                self.generation.reset_document();
            }
            if self.transforms.is_open() {
                self.transforms.reset_document();
            }
            self.matrix_draft = None;
            self.show_phrase(ui);
            return;
        }
        if !self.editor.has_grid() {
            self.cell_blocks.close();
            self.thin.reset_document();
            ui.heading("No pattern selected");
            ui.label("This project has no ordered patterns or tracks. Use New to start a blank editable song, or generate a new pattern using an assigned instrument.");
            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.editor.edit_mode, "Edit mode");
                if ui.button("Groove…").clicked() {
                    self.grooves.open(&self.editor);
                }
                if ui.button("Generate…").clicked() {
                    self.grooves.reset_document();
                    self.thin.reset_document();
                    self.cell_blocks.close();
                    self.generation.open(&self.editor);
                }
            });
            self.grooves.show(ui, &mut self.editor);
            if self.generation.show(ui, &mut self.editor) {
                let result = self.open_song_position(self.editor.occurrence, self.editor.track);
                self.result(result);
            }
            return;
        }
        self.toolbar(ui);
        self.matrix_notice(ui);
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.separator();
        let available = ui.available_size();
        let order_width = if available.x >= 850.0 { 196.0 } else { 170.0 };
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(order_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(order_width);
                    if self.pads.open {
                        egui::ScrollArea::vertical()
                            .id_salt("pad-inspector")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if self.pads.show(ui, &mut self.editor) {
                                    self.scroll_to_cursor = true;
                                    self.value.clear();
                                }
                                if !self.pads.open {
                                    self.steps.open = false;
                                }
                            });
                    } else if self.phrase_browser_open {
                        self.phrase_browser(ui);
                    } else {
                        self.order_matrix(ui);
                    }
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                if self.steps.open {
                    egui::ScrollArea::vertical()
                        .id_salt("step-workspace")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.steps.show(
                                ui,
                                &mut self.editor,
                                self.pads.selected_instrument(),
                                available.y,
                            )
                        });
                } else {
                    self.editor_grid(ui);
                }
            });
        });
        if let Some((instrument, pad)) = self.steps.take_pad_selection() {
            self.pads
                .select_projected_pad(&self.editor, instrument, pad);
            ui.ctx().request_repaint();
        }
        if !self.steps.open
            && !self.transforms.is_open()
            && !self.cell_blocks.is_open()
            && !self.thin.is_open()
            && !self.generation.is_open()
            && !self.grooves.is_open()
        {
            self.editor_keys(ui);
        }
        self.matrix_window(ui);
        self.cell_blocks.show(ui, &mut self.editor);
        self.transforms.show(ui, &mut self.editor);
        self.thin.show(ui, &mut self.editor);
        self.grooves.show(ui, &mut self.editor);
        if self.generation.show(ui, &mut self.editor) {
            self.steps.open = false;
            self.pads.open = false;
            let result = self.open_song_position(self.editor.occurrence, self.editor.track);
            self.result(result);
        }
    }
}

/// Follow is a read-only snapshot viewport; it never changes the editor selection.
pub fn playback_view(ui: &mut egui::Ui, song: &Song, occurrence: usize, row: u32) {
    let Some(order) = song.order.get(occurrence) else {
        return;
    };
    let Some(pattern) = song.patterns.get(&order.pattern) else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("FOLLOW / READ ONLY").strong().color(PLAY));
        ui.monospace(format!("POS {occurrence:02}  ROW {row:03}"));
        ui.label(format!("{} · {} rows", pattern.name, pattern.rows));
        ui.label(
            RichText::new("Turn Follow off to return to your edit cursor.")
                .small()
                .color(MUTED),
        );
    });
    ui.separator();
    let available = ui.available_size();
    let order_width = if available.x >= 850.0 { 196.0 } else { 170.0 };
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(order_width, available.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(order_width);
                ui.label(RichText::new("ORDER / SNAPSHOT").strong().color(MUTED));
                ui.label(RichText::new("Playback position").small().color(PLAY));
                ui.separator();
                egui::ScrollArea::both()
                    .id_salt("follow-order")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("follow-matrix")
                            .min_col_width(18.0)
                            .spacing([3.0, 3.0])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("POS / PAT").monospace().small().color(MUTED),
                                );
                                for track in &song.track_order {
                                    ui.label(
                                        RichText::new(format!("{:02}", track.0))
                                            .monospace()
                                            .small()
                                            .color(MUTED),
                                    );
                                }
                                ui.end_row();
                                for (index, order) in song.order.iter().enumerate() {
                                    let active = index == occurrence;
                                    let label = ui.label(
                                        RichText::new(format!(
                                            "{index:02} / {:02}",
                                            order.pattern.0
                                        ))
                                        .monospace()
                                        .color(if active { PLAY } else { MUTED }),
                                    );
                                    if active {
                                        label.scroll_to_me(Some(egui::Align::Center));
                                    }
                                    for track in &song.track_order {
                                        let aliased = song.patterns[&order.pattern]
                                            .track_aliases
                                            .contains_key(track);
                                        let muted = order.muted_tracks.contains(track);
                                        let populated = song
                                            .resolved_track(order.pattern, *track)
                                            .ok()
                                            .flatten()
                                            .is_some_and(|rows| {
                                                rows.values().any(|columns| {
                                                    columns
                                                        .values()
                                                        .any(|cell| *cell != Cell::default())
                                                })
                                            });
                                        ui.label(
                                            RichText::new(if muted && aliased {
                                                "XL"
                                            } else if muted {
                                                "X"
                                            } else if aliased {
                                                "L"
                                            } else if populated {
                                                "■"
                                            } else {
                                                "·"
                                            })
                                            .color(
                                                if active {
                                                    PLAY
                                                } else if populated {
                                                    MUTED
                                                } else {
                                                    EMPTY
                                                },
                                            ),
                                        );
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            },
        );
        ui.separator();
        ui.vertical(|ui| {
            // Keep horizontal browsing independent while following vertical musical progress.
            let horizontal = ui.ctx().data_mut(|data| {
                data.get_temp::<f32>(egui::Id::new("follow-horizontal"))
                    .unwrap_or(0.0)
            });
            let result = show_grid(
                ui,
                GridView {
                    song,
                    occurrence,
                    selection: None,
                    rectangle: None,
                    playhead: Some(row),
                    editing: false,
                    focused: false,
                    scroll: Some(egui::vec2(
                        horizontal,
                        row.saturating_sub(3) as f32 * ROW_HEIGHT,
                    )),
                    id: "playback-follow",
                },
            );
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("follow-horizontal"), result.offset.x)
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(size: Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        }
    }
    fn frame(
        ctx: &egui::Context,
        song: &Song,
        scroll: Vec2,
        events: Vec<egui::Event>,
        editable: bool,
    ) -> (GridResult, egui::FullOutput) {
        let mut result = None;
        let output = ctx.run(input(egui::vec2(600.0, 360.0), events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(show_grid(
                    ui,
                    GridView {
                        song,
                        occurrence: 0,
                        selection: editable.then_some(Selection {
                            row: 0,
                            track: TrackId(1),
                            column: 0,
                            field: Field::Note,
                        }),
                        rectangle: None,
                        playhead: Some(8),
                        editing: editable,
                        focused: false,
                        scroll: Some(scroll),
                        id: "grid-test",
                    },
                ));
            });
        });
        (result.unwrap(), output)
    }

    #[test]
    fn delay_grid_never_rounds_valid_subrow_values_to_invalid_endpoints() {
        for (delay, expected) in [
            (0.0, "0.000"),
            (0.00001, ">0.00"),
            (0.5, "0.500"),
            (0.9999999999999999, "<1.00"),
        ] {
            let cell = Cell {
                delay: Some(delay),
                ..Cell::default()
            };
            assert_eq!(values(&cell)[4], expected);
            assert_eq!(delay.to_string().parse::<f64>().unwrap(), delay);
        }
        assert_eq!(values(&Cell::default())[4], "·····");
    }

    #[test]
    fn opening_generated_position_reveals_new_track_without_document_edit() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.row = 12;
        tracker.editor.column = 1;
        tracker.editor.field = Field::Delay;
        tracker.scroll_offset = egui::vec2(25.0, 200.0);
        tracker.song_view = Some((egui::vec2(50.0, 75.0), egui::vec2(600.0, 360.0)));
        let song = tracker.editor.song.clone();
        let revision = tracker.editor.revision();
        tracker.open_song_position(0, TrackId(4)).unwrap();
        assert_eq!(tracker.editor.row, 0);
        assert_eq!(tracker.editor.column, 0);
        assert_eq!(tracker.editor.field, Field::Note);
        assert_eq!(tracker.scroll_offset.y, 0.0);
        assert_eq!(
            tracker.scroll_offset.x,
            Selection::from_editor(&tracker.editor)
                .bounds(&song, 0)
                .left()
        );
        tracker_frame(&ctx, &mut tracker, vec![], true);
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let bounds = Selection::from_editor(&tracker.editor).bounds(&song, 0);
        assert!(bounds.left() >= tracker.scroll_offset.x);
        assert!(bounds.right() <= tracker.scroll_offset.x + tracker.viewport.x - GUTTER);
        assert_eq!(tracker.editor.song, song);
        assert_eq!(tracker.editor.revision(), revision);
        assert!(!tracker.editor.dirty);
    }

    #[test]
    fn field_only_navigation_reveals_the_field_without_recentering_visible_rows() {
        let song = crate::song::example_song();
        let selection = Selection {
            row: 8,
            track: TrackId(1),
            column: 1,
            field: Field::Delay,
        };
        let viewport = egui::vec2(400.0, 300.0);
        let offset = reveal_selection(Vec2::ZERO, viewport, selection.bounds(&song, 0));
        assert!(offset.x > 0.0);
        assert_eq!(offset.y, 0.0);
        let bounds = selection.bounds(&song, 0);
        assert_eq!(bounds.right(), offset.x + viewport.x - GUTTER);
        assert_eq!(reveal_selection(offset, viewport, bounds), offset);
        let first = Selection {
            row: 0,
            track: TrackId(1),
            column: 0,
            field: Field::Note,
        };
        assert_eq!(
            reveal_selection(offset, viewport, first.bounds(&song, 0)),
            Vec2::ZERO
        );
    }

    #[test]
    fn virtual_grid_hit_testing_matches_both_axis_scroll_and_read_only_is_inert() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let song = crate::song::example_song();
        let offset = egui::vec2(COLUMN_WIDTH, 6.0 * ROW_HEIGHT);
        let (layout, _) = frame(&ctx, &song, offset, vec![], true);
        let target = layout.rect.min
            + egui::vec2(
                GUTTER + FIELD_WIDTHS[0] / 2.0,
                HEADER_HEIGHT + ROW_HEIGHT / 2.0,
            );
        let click = |pressed| {
            vec![
                egui::Event::PointerMoved(target),
                egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&ctx, &song, offset, click(true), true);
        let (result, _) = frame(&ctx, &song, offset, click(false), true);
        assert_eq!(
            result.clicked,
            Some(Selection {
                row: 6,
                track: TrackId(1),
                column: 1,
                field: Field::Note
            })
        );
        frame(&ctx, &song, offset, click(true), false);
        let (result, _) = frame(&ctx, &song, offset, click(false), false);
        assert_eq!(result.clicked, None);
    }

    #[test]
    fn max_length_pattern_only_paints_visible_rows_and_keeps_header_pinned() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut song = crate::song::example_song();
        song.patterns.get_mut(&song.order[0].pattern).unwrap().rows = 65_536;
        let (first, output) = frame(&ctx, &song, Vec2::ZERO, vec![], true);
        assert!(
            output.shapes.len() < 1200,
            "Only visible rows and columns should be painted"
        );
        let (last, output) = frame(
            &ctx,
            &song,
            egui::vec2(300.0, 60_000.0 * ROW_HEIGHT),
            vec![],
            true,
        );
        assert!(output.shapes.len() < 1200);
        assert_eq!(first.rect.min, last.rect.min);
        assert!(last.offset.y > 1_000_000.0);
        let row_header = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == "ROW" => Some(text.pos),
                _ => None,
            })
            .expect("Pinned row header remains painted at distant rows");
        assert!(row_header.y >= last.rect.top());
        assert!(row_header.y < last.rect.top() + HEADER_HEIGHT);
    }

    #[test]
    fn minimum_window_preserves_a_useful_tracker_viewport() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        for _ in 0..2 {
            let _ = ctx.run(input(egui::vec2(900.0, 620.0), vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| tracker.show(ui));
            });
        }
        assert!(
            tracker.viewport.x > 600.0,
            "Grid, not order controls, should dominate width: {:?}",
            tracker.viewport
        );
        assert!(
            tracker.viewport.y > 450.0,
            "Grid should retain more than twenty rows: {:?}",
            tracker.viewport
        );
    }
    fn effects_fixture() -> StepEffects {
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 2,
                spacing: 0.2,
            }),
            release: Some(0.75),
            glide: Some(Glide {
                target: 67,
                semitones_per_second: 12.0,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn fx_geometry_preserves_legacy_columns_and_expands_only_the_populated_column() {
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            ..Editor::default()
        };
        let plain = grid_columns(&editor.song, 0);
        assert!(plain
            .iter()
            .all(|column| !column.effects && column.width == COLUMN_WIDTH));
        editor.enter_note(Note::Note(60)).unwrap();
        editor.set_effects(effects_fixture()).unwrap();
        let commands = grid_columns(&editor.song, 0);
        assert!(commands[0].effects);
        assert_eq!(commands[0].width, COLUMN_WIDTH + EFFECT_WIDTH);
        assert!(!commands[1].effects);
        assert_eq!(commands[1].left, plain[1].left + EFFECT_WIDTH);
        let selection = Selection {
            row: 0,
            track: TrackId(1),
            column: 1,
            field: Field::Note,
        };
        assert_eq!(selection.bounds(&editor.song, 0).left(), commands[1].left);
        assert_eq!(
            Field::ALL.len(),
            5,
            "FX inspector must not change legacy musical navigation"
        );
    }
    #[test]
    fn command_drafts_do_not_change_song_and_reset_when_cell_or_mode_changes() {
        let mut editor = Editor {
            edit_mode: true,
            ..Editor::default()
        };
        let before = editor.song.clone();
        let mut draft = EffectsDraft::default();
        draft.sync(&editor);
        draft.value = effects_fixture();
        draft.sync(&editor);
        assert!(
            !draft.value.is_empty(),
            "Unapplied edits survive ordinary repaint"
        );
        assert_eq!(editor.song, before);
        editor.row = 1;
        draft.sync(&editor);
        assert!(
            draft.value.is_empty(),
            "Never carry a pending command to a different cell"
        );
        draft.value = effects_fixture();
        editor.edit_mode = false;
        draft.sync(&editor);
        assert!(
            draft.value.is_empty(),
            "Read mode drops unapplied write draft"
        );
        assert_eq!(editor.song, before);
    }
    #[test]
    fn fx_summary_and_tooltip_distinguish_release_from_hard_cut_and_show_units() {
        let effects = effects_fixture();
        assert_eq!(effects_summary(&effects), "R2 L75 G67");
        let description = effects_description(&effects);
        assert!(description.contains("2 extra starts"));
        assert!(description.contains("spacing 0.2 rows"));
        assert!(description.contains("region ADSR release"));
        assert!(description.contains("R=0 is abrupt"));
        assert!(description.contains("12 semitones/second"));
    }
    fn tracker_frame(
        ctx: &egui::Context,
        tracker: &mut TrackerUi,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> egui::FullOutput {
        ctx.run(input(egui::vec2(900.0, 620.0), events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| tracker.show(ui));
            });
        })
    }
    fn text_center(output: &egui::FullOutput, wanted: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing rendered control: {wanted}"))
    }
    fn click_tracker(
        ctx: &egui::Context,
        tracker: &mut TrackerUi,
        position: egui::Pos2,
        enabled: bool,
    ) -> egui::FullOutput {
        let event = |pressed| {
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        tracker_frame(ctx, tracker, event(true), enabled);
        tracker_frame(ctx, tracker, event(false), enabled)
    }
    #[test]
    fn actual_command_menu_applies_atomically_and_does_not_receive_musical_keys() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.enter_note(Note::Note(60)).unwrap();
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        let button = text_center(&output, "Commands (0)");
        ctx.memory_mut(|memory| memory.request_focus(grid_id()));
        click_tracker(&ctx, &mut tracker, button, true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        text_center(&output, "STEP COMMANDS");
        assert!(!ctx.memory(|memory| memory.has_focus(grid_id())));
        let before = tracker.editor.song.clone();
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        assert_eq!(
            tracker.editor.song, before,
            "Popup must not leak keys into notes"
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        let retrigger = text_center(&output, "Retrigger");
        click_tracker(&ctx, &mut tracker, retrigger, true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        text_center(&output, "STEP COMMANDS");
        assert!(
            tracker.effects_draft.value.retrigger.is_some(),
            "Checkbox edits must remain a pending draft with the menu open"
        );
        assert_eq!(tracker.editor.song, before);
        let number = text_center(&output, "1");
        click_tracker(&ctx, &mut tracker, number, true);
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![
                egui::Event::Text("2".into()),
                egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        assert_eq!(
            tracker.editor.song, before,
            "Numeric editing must not leak musical notes or commit drafts"
        );
        tracker.effects_draft.set_value(effects_fixture());
        tracker.effects_draft.retrigger_spacing = "1.0".into();
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        let apply = text_center(&output, "Apply commands");
        click_tracker(&ctx, &mut tracker, apply, true);
        assert_eq!(
            tracker.editor.song, before,
            "Invalid command combinations must be rejected atomically"
        );
        assert!(tracker.error.is_some());
        tracker.effects_draft.set_value(effects_fixture());
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        let apply = text_center(&output, "Apply commands");
        click_tracker(&ctx, &mut tracker, apply, true);
        assert_eq!(tracker.editor.cell().effects, effects_fixture());
        assert_eq!(tracker.editor.cell().note, Note::Note(60));
        tracker.editor.undo();
        assert!(tracker.editor.cell().effects.is_empty());
    }
    #[test]
    fn command_menu_cannot_write_when_parent_is_blocked_or_edit_mode_is_off() {
        for read_only in [false, true] {
            let ctx = egui::Context::default();
            install_theme(&ctx);
            let mut tracker = TrackerUi::default();
            tracker.editor.edit_mode = true;
            tracker.editor.edit_step = 0;
            tracker.editor.enter_note(Note::Note(60)).unwrap();
            tracker.editor.edit_mode = !read_only;
            tracker_frame(&ctx, &mut tracker, vec![], true);
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            let button = text_center(&output, "Commands (0)");
            click_tracker(&ctx, &mut tracker, button, true);
            tracker_frame(&ctx, &mut tracker, vec![], true);
            tracker.effects_draft.set_value(effects_fixture());
            let before = tracker.editor.song.clone();
            let output = tracker_frame(&ctx, &mut tracker, vec![], read_only);
            let apply = text_center(&output, "Apply commands");
            click_tracker(&ctx, &mut tracker, apply, read_only);
            assert_eq!(tracker.editor.song, before);
            assert!(tracker.editor.cell().effects.is_empty());
        }
    }
    #[test]
    fn fx_column_expansion_keeps_mouse_hits_aligned_after_both_axis_scroll() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            ..Editor::default()
        };
        editor.enter_note(Note::Note(60)).unwrap();
        editor.set_effects(effects_fixture()).unwrap();
        let offset = egui::vec2(COLUMN_WIDTH + EFFECT_WIDTH, 6.0 * ROW_HEIGHT);
        let (layout, _) = frame(&ctx, &editor.song, offset, vec![], true);
        let target = layout.rect.min
            + egui::vec2(
                GUTTER + FIELD_WIDTHS[0] / 2.0,
                HEADER_HEIGHT + ROW_HEIGHT / 2.0,
            );
        let click = |pressed| {
            vec![
                egui::Event::PointerMoved(target),
                egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&ctx, &editor.song, offset, click(true), true);
        let (result, _) = frame(&ctx, &editor.song, offset, click(false), true);
        assert_eq!(
            result.clicked,
            Some(Selection {
                row: 6,
                track: TrackId(1),
                column: 1,
                field: Field::Note
            })
        );
    }
    fn full_effects_fixture() -> StepEffects {
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 2,
                spacing: 0.2,
            }),
            cut: Some(0.75),
            offset: Some(32),
            direction: Some(Direction::Reverse),
            glide: Some(Glide {
                target: 67,
                semitones_per_second: 12.0,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn end_action_switch_is_explicit_and_never_combines_release_and_cut() {
        let mut effects = effects_fixture();
        EndAction::Cut.apply(&mut effects);
        assert_eq!(effects.cut, Some(0.75));
        assert_eq!(effects.release, None);
        EndAction::Release.apply(&mut effects);
        assert_eq!(effects.cut, None);
        assert_eq!(effects.release, Some(0.75));
        EndAction::None.apply(&mut effects);
        assert!(effects.cut.is_none() && effects.release.is_none());
        assert!(effects.retrigger.is_some() && effects.glide.is_some());
    }
    #[test]
    fn full_fx_summary_counts_overflow_and_tooltip_keeps_exact_source_frame_units() {
        let effects = full_effects_fixture();
        assert_eq!(effects_count(&effects), 5);
        let summary = effects_summary(&effects);
        assert!(summary.len() <= 13);
        assert!(summary.contains('+'));
        let description = effects_description(&effects);
        assert!(description.contains("Hard cut: 0.75 of row"));
        assert!(description.contains("may click; prior release tails remain"));
        assert!(description.contains("frame 32 (zero-based source frames, not output frames)"));
        assert!(description.contains("Direction: Reverse"));
        assert!(!description.contains("Timed release:"));
    }
    fn visible_text_center(output: &egui::FullOutput, wanted: &str) -> Option<egui::Pos2> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                let center = text.pos + text.galley.size() * 0.5;
                shape.clip_rect.contains(center).then_some(center)
            }
            _ => None,
        })
    }
    #[test]
    fn minimum_size_full_command_popup_keeps_footer_reachable_and_scrolls_to_last_field() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.enter_note(Note::Note(60)).unwrap();
        tracker.editor.set_effects(full_effects_fixture()).unwrap();
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Commands (5)"),
            true,
        );
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        let heading = visible_text_center(&output, "STEP COMMANDS").unwrap();
        let close = visible_text_center(&output, "Close").unwrap();
        assert!(heading.y >= 0.0 && close.y < 620.0);
        assert!(visible_text_center(&output, "Retrigger").is_some());
        let before = tracker.editor.song.clone();
        let body = heading + egui::vec2(30.0, 160.0);
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![
                egui::Event::PointerMoved(body),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -600.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert!(
            visible_text_center(&output, "Glide").is_some(),
            "Final field must be reachable by scrolling the native inspector"
        );
        assert_eq!(
            visible_text_center(&output, "Close"),
            Some(close),
            "Apply/Close footer stays fixed outside scrolling"
        );
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![
                egui::Event::PointerMoved(body),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 600.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert!(visible_text_center(&output, "Retrigger").is_some());
        assert_eq!(tracker.editor.song, before);
    }
    #[test]
    fn actual_new_command_controls_apply_hard_cut_source_offset_and_direction() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.enter_note(Note::Note(60)).unwrap();
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Commands (0)"),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "No end command").unwrap(),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "Hard cut").unwrap(),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert_eq!(tracker.effects_draft.value.cut, Some(0.75));
        assert!(tracker.effects_draft.value.release.is_none());
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "Source offset").unwrap(),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert_eq!(tracker.effects_draft.value.offset, Some(0));
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "Direction").unwrap(),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert_eq!(
            tracker.effects_draft.value.direction,
            Some(Direction::Reverse)
        );
        assert!(
            tracker.editor.cell().effects.is_empty(),
            "Control changes remain a draft until Apply"
        );
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "Apply commands").unwrap(),
            true,
        );
        let effects = tracker.editor.cell().effects;
        assert_eq!(effects.cut, Some(0.75));
        assert_eq!(effects.offset, Some(0));
        assert_eq!(effects.direction, Some(Direction::Reverse));
        assert!(effects.release.is_none());
        tracker.editor.undo();
        assert!(tracker.editor.cell().effects.is_empty());
    }
    fn phrase_fixture() -> Phrase {
        Phrase {
            name: "Pulse".into(),
            instrument: InstrumentId(1),
            rows: 16,
            lines_per_beat: 8,
            note_columns: 2,
            root_note: 60,
            cells: BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        ..Default::default()
                    },
                )]),
            )]),
        }
    }
    fn note_key(key: egui::Key, command: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: if command {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            },
        }
    }
    #[test]
    fn phrase_projection_keys_commit_to_real_history_and_preserve_song_cursor() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.select(1, TrackId(3)).unwrap();
        tracker.editor.row = 12;
        tracker.editor.column = 1;
        tracker.editor.field = Field::Pan;
        let cursor = Selection::from_editor(&tracker.editor);
        let occurrence = tracker.editor.occurrence;
        tracker
            .editor
            .upsert_phrase(PhraseId(1), phrase_fixture())
            .unwrap();
        tracker.enter_phrase(PhraseId(1));
        tracker_frame(&ctx, &mut tracker, vec![], true);
        ctx.memory_mut(|memory| memory.request_focus(grid_id()));
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![note_key(egui::Key::X, false)],
            true,
        );
        assert_eq!(
            tracker.editor.song.phrases[&PhraseId(1)].cells[&0][&0].note,
            Note::Note(62)
        );
        assert_eq!(Selection::from_editor(&tracker.editor), cursor);
        assert_eq!(tracker.editor.occurrence, occurrence);
        tracker_frame(&ctx, &mut tracker, vec![note_key(egui::Key::Z, true)], true);
        assert_eq!(
            tracker.editor.song.phrases[&PhraseId(1)].cells[&0][&0].note,
            Note::Note(60)
        );
        tracker.return_to_song();
        assert_eq!(Selection::from_editor(&tracker.editor), cursor);
        assert_eq!(tracker.editor.occurrence, occurrence);
    }
    #[test]
    fn removed_phrase_context_does_not_recreate_definition_or_double_undo() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.enter_note(Note::Note(65)).unwrap();
        tracker
            .editor
            .upsert_phrase(PhraseId(1), phrase_fixture())
            .unwrap();
        tracker.enter_phrase(PhraseId(1));
        tracker_frame(&ctx, &mut tracker, vec![], true);
        ctx.memory_mut(|memory| memory.request_focus(grid_id()));
        tracker_frame(&ctx, &mut tracker, vec![note_key(egui::Key::Z, true)], true);
        assert!(!tracker.editor.song.phrases.contains_key(&PhraseId(1)));
        assert!(tracker.phrase_target.is_none());
        assert_eq!(
            tracker.editor.cell().note,
            Note::Note(65),
            "Returning from removed phrase must not apply the same undo twice"
        );
        tracker_frame(&ctx, &mut tracker, vec![], true);
        assert!(!tracker.editor.song.phrases.contains_key(&PhraseId(1)));
    }
    #[test]
    fn phrase_authoring_respects_disabled_parent_and_read_mode() {
        for blocked in [false, true] {
            let ctx = egui::Context::default();
            install_theme(&ctx);
            let mut tracker = TrackerUi::default();
            tracker.editor.edit_mode = true;
            tracker.editor.edit_step = 0;
            tracker
                .editor
                .upsert_phrase(PhraseId(1), phrase_fixture())
                .unwrap();
            tracker.enter_phrase(PhraseId(1));
            tracker.editor.edit_mode = blocked;
            tracker_frame(&ctx, &mut tracker, vec![], true);
            ctx.memory_mut(|memory| memory.request_focus(grid_id()));
            let before = tracker.editor.song.clone();
            tracker_frame(
                &ctx,
                &mut tracker,
                vec![note_key(egui::Key::X, false)],
                !blocked,
            );
            assert_eq!(tracker.editor.song, before);
        }
    }
    #[test]
    fn actual_browser_new_phrase_note_entry_and_return_are_real_document_edits() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.row = 4;
        let cursor = Selection::from_editor(&tracker.editor);
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(&ctx, &mut tracker, text_center(&output, "Phrases"), true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "+ New").unwrap(),
            true,
        );
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert_eq!(tracker.phrase_target, Some(PhraseId(1)));
        click_tracker(
            &ctx,
            &mut tracker,
            visible_text_center(&output, "---").unwrap(),
            true,
        );
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![note_key(egui::Key::Z, false)],
            true,
        );
        assert_eq!(
            tracker.editor.song.phrases[&PhraseId(1)].cells[&0][&0].note,
            Note::Note(60)
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(&ctx, &mut tracker, text_center(&output, "< Song"), true);
        assert!(tracker.phrase_target.is_none());
        assert_eq!(Selection::from_editor(&tracker.editor), cursor);
    }
    #[test]
    fn actual_phrase_trigger_is_explicit_and_parent_command_controls_are_contextual() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker
            .editor
            .upsert_phrase(PhraseId(1), phrase_fixture())
            .unwrap();
        tracker.editor.enter_note(Note::Note(64)).unwrap();
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(&ctx, &mut tracker, text_center(&output, "Trigger"), true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "P01  Pulse · instrument 1"),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        assert!(tracker.editor.cell().phrase.is_none());
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Apply trigger"),
            true,
        );
        assert_eq!(
            tracker.editor.cell().phrase,
            Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 1
            })
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(&ctx, &mut tracker, text_center(&output, "Close"), true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Commands (0)"),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(&ctx, &mut tracker, text_center(&output, "Retrigger"), true);
        assert!(
            tracker.effects_draft.value.retrigger.is_none(),
            "Parent phrase trigger cannot enable unsupported retrigger"
        );
        assert!(tracker.editor.cell().effects.is_empty());
    }
    #[test]
    fn phrase_trigger_width_is_opt_in_and_hard_cut_tooltip_describes_instance_tails() {
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            ..Editor::default()
        };
        let before = grid_columns(&editor.song, 0);
        assert!(before.iter().all(|column| !column.phrase));
        editor.upsert_phrase(PhraseId(1), phrase_fixture()).unwrap();
        assert!(
            grid_columns(&editor.song, 0)
                .iter()
                .all(|column| !column.phrase),
            "Unused phrase library must not widen song grid"
        );
        editor.enter_note(Note::Note(64)).unwrap();
        editor
            .set_phrase_trigger(Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 2,
            }))
            .unwrap();
        editor
            .set_effects(StepEffects {
                cut: Some(0.5),
                ..Default::default()
            })
            .unwrap();
        let columns = grid_columns(&editor.song, 0);
        assert!(columns[0].phrase);
        assert_eq!(columns[0].width, COLUMN_WIDTH + EFFECT_WIDTH);
        assert_eq!(step_summary(&editor.cell()), "P1×2 C50");
        assert!(
            step_description(&editor.song, &editor.cell()).contains("kills all its voices/tails")
        );
    }
    #[test]
    fn phrase_trigger_popup_rejects_incompatible_fx_atomically() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker
            .editor
            .upsert_phrase(PhraseId(1), phrase_fixture())
            .unwrap();
        tracker.editor.enter_note(Note::Note(64)).unwrap();
        tracker
            .editor
            .set_effects(StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                }),
                ..Default::default()
            })
            .unwrap();
        tracker_frame(&ctx, &mut tracker, vec![], true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(&ctx, &mut tracker, text_center(&output, "Trigger"), true);
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "P01  Pulse · instrument 1"),
            true,
        );
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        let before = tracker.editor.song.clone();
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Apply trigger"),
            true,
        );
        assert_eq!(
            tracker.editor.song, before,
            "Incompatible trigger must not clear a note, instrument, or FX"
        );
        assert!(tracker.error.is_some());
        assert!(
            tracker.trigger_draft.is_some(),
            "Keep the pending choice so the user can correct the cell"
        );
    }
    #[test]
    fn already_open_phrase_popups_cannot_commit_when_parent_is_disabled() {
        for definition in [false, true] {
            let ctx = egui::Context::default();
            install_theme(&ctx);
            let mut tracker = TrackerUi::default();
            tracker.editor.edit_mode = true;
            tracker.editor.edit_step = 0;
            tracker
                .editor
                .upsert_phrase(PhraseId(1), phrase_fixture())
                .unwrap();
            tracker.editor.enter_note(Note::Note(64)).unwrap();
            tracker.phrase_browser_open = definition;
            tracker_frame(&ctx, &mut tracker, vec![], true);
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            click_tracker(
                &ctx,
                &mut tracker,
                text_center(
                    &output,
                    if definition {
                        "Phrase settings"
                    } else {
                        "Trigger"
                    },
                ),
                true,
            );
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            if definition {
                tracker.phrase_settings.as_mut().unwrap().2.rows = 24;
            } else {
                click_tracker(
                    &ctx,
                    &mut tracker,
                    text_center(&output, "P01  Pulse · instrument 1"),
                    true,
                );
            }
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            let before = tracker.editor.song.clone();
            click_tracker(
                &ctx,
                &mut tracker,
                text_center(
                    &output,
                    if definition {
                        "Apply definition"
                    } else {
                        "Apply trigger"
                    },
                ),
                false,
            );
            tracker_frame(
                &ctx,
                &mut tracker,
                vec![note_key(egui::Key::Z, false)],
                false,
            );
            assert_eq!(tracker.editor.song, before);
            assert!(!ctx.memory(|memory| memory.has_focus(grid_id())));
        }
    }
    #[test]
    fn replacing_document_clears_removed_phrase_context_and_stale_settings() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker
            .editor
            .upsert_phrase(PhraseId(1), phrase_fixture())
            .unwrap();
        tracker.enter_phrase(PhraseId(1));
        tracker_frame(&ctx, &mut tracker, vec![], true);
        tracker.phrase_settings.as_mut().unwrap().2.rows = 24;
        tracker
            .editor
            .replace_document(Editor::default().song)
            .unwrap();
        let before = tracker.editor.song.clone();
        tracker_frame(
            &ctx,
            &mut tracker,
            vec![note_key(egui::Key::X, false)],
            true,
        );
        tracker_frame(&ctx, &mut tracker, vec![], true);
        assert!(tracker.phrase_target.is_none());
        assert!(tracker.phrase_selected.is_none());
        assert_eq!(tracker.editor.song, before);
    }
    fn type_command_spacing(ctx: &egui::Context, tracker: &mut TrackerUi, value: &str) {
        let output = tracker_frame(ctx, tracker, vec![], true);
        let old = &tracker.effects_draft.retrigger_spacing;
        let position = output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if &text.galley.job.text == old => {
                    let visible = Rect::from_min_size(text.pos, text.galley.size())
                        .intersect(shape.clip_rect);
                    visible.is_positive().then_some(visible.center())
                }
                _ => None,
            })
            .expect("Exact spacing input is visible");
        click_tracker(ctx, tracker, position, true);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let select = |pressed| egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        tracker_frame(ctx, tracker, vec![select(true)], true);
        tracker_frame(
            ctx,
            tracker,
            vec![select(false), egui::Event::Text(value.into())],
            true,
        );
        assert_eq!(
            tracker.effects_draft.retrigger_spacing, value,
            "Actual TextEdit must retain requested text exactly"
        );
    }
    #[test]
    fn commands_spacing_preserves_tiny_and_near_one_values_on_open_and_unchanged_apply() {
        for spacing in [f64::from_bits(1), 1e-18, 1e-8, 0.9999999999999999] {
            let ctx = egui::Context::default();
            install_theme(&ctx);
            let mut tracker = TrackerUi::default();
            tracker.editor.edit_mode = true;
            tracker.editor.edit_step = 0;
            tracker.editor.enter_note(Note::Note(60)).unwrap();
            tracker
                .editor
                .set_effects(StepEffects {
                    retrigger: Some(Retrigger {
                        repeats: 1,
                        spacing,
                    }),
                    ..Default::default()
                })
                .unwrap();
            let before = tracker.editor.song.clone();
            let revision = tracker.editor.revision();
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            let point = text_center(&output, "Commands (1)");
            click_tracker(&ctx, &mut tracker, point, true);
            for _ in 0..4 {
                tracker_frame(&ctx, &mut tracker, vec![], true);
            }
            assert_eq!(
                tracker
                    .effects_draft
                    .effective()
                    .unwrap()
                    .retrigger
                    .unwrap()
                    .spacing
                    .to_bits(),
                spacing.to_bits()
            );
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            let point = text_center(&output, "Apply commands");
            click_tracker(&ctx, &mut tracker, point, true);
            assert_eq!(tracker.editor.song, before);
            assert_eq!(
                tracker.editor.revision(),
                revision,
                "Opening or unchanged Apply is not an edit"
            );
            assert_eq!(
                tracker
                    .editor
                    .cell()
                    .effects
                    .retrigger
                    .unwrap()
                    .spacing
                    .to_bits(),
                spacing.to_bits()
            );
        }
    }
    #[test]
    fn commands_spacing_actual_text_rejects_invalid_without_clamping_then_commits_exactly_once() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        tracker.editor.edit_mode = true;
        tracker.editor.edit_step = 0;
        tracker.editor.enter_note(Note::Note(60)).unwrap();
        tracker
            .editor
            .set_effects(StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                }),
                ..Default::default()
            })
            .unwrap();
        let before = tracker.editor.song.clone();
        let revision = tracker.editor.revision();
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Commands (1)"),
            true,
        );
        for value in ["0", "-1e-30", "NaN", "inf", "1e-400", "invalid", "1", "1.1"] {
            type_command_spacing(&ctx, &mut tracker, value);
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            click_tracker(
                &ctx,
                &mut tracker,
                text_center(&output, "Apply commands"),
                true,
            );
            assert_eq!(tracker.editor.song, before, "Invalid {value} cannot commit");
            assert_eq!(tracker.editor.revision(), revision);
            assert_eq!(tracker.effects_draft.retrigger_spacing, value);
            let output = tracker_frame(&ctx, &mut tracker, vec![], true);
            let error = tracker
                .effects_draft
                .effective()
                .err()
                .or_else(|| tracker.error.clone())
                .expect("Invalid input has an explicit error");
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.job.text == error)), "The exact invalid-input error must be painted");
        }
        type_command_spacing(&ctx, &mut tracker, "1e-18");
        let output = tracker_frame(&ctx, &mut tracker, vec![], true);
        click_tracker(
            &ctx,
            &mut tracker,
            text_center(&output, "Apply commands"),
            true,
        );
        assert_eq!(
            tracker
                .editor
                .cell()
                .effects
                .retrigger
                .unwrap()
                .spacing
                .to_bits(),
            1e-18_f64.to_bits()
        );
        assert_eq!(tracker.editor.revision(), revision + 1);
        assert_eq!(
            tracker.editor.cell().note,
            Note::Note(60),
            "Text input must not leak musical keys"
        );
        tracker.editor.undo();
        assert_eq!(tracker.editor.song, before);
    }
    #[test]
    fn one_shot_reveal_preserves_exact_cursor_and_subsequent_manual_viewport() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        for pattern in tracker.editor.song.patterns.values_mut() {
            pattern.rows = 128;
        }
        for _ in 0..2 {
            tracker_frame(&ctx, &mut tracker, vec![], true);
        }
        tracker.editor.select(0, TrackId(4)).unwrap();
        tracker.editor.row = 90;
        tracker.editor.column = 1;
        tracker.editor.field = Field::Delay;
        let before = tracker.editor.song.clone();
        let revision = tracker.editor.revision();
        let cursor = Selection::from_editor(&tracker.editor);
        tracker.reveal_current_cursor();
        for _ in 0..3 {
            tracker_frame(&ctx, &mut tracker, vec![], true);
        }
        let expected = cursor.bounds(&tracker.editor.song, 0);
        assert!(
            expected.left() >= tracker.scroll_offset.x
                && expected.right() <= tracker.scroll_offset.x + tracker.viewport.x
        );
        assert!(
            expected.top() >= tracker.scroll_offset.y
                && expected.bottom() <= tracker.scroll_offset.y + tracker.viewport.y
        );
        assert_eq!(Selection::from_editor(&tracker.editor), cursor);
        // A deliberate later manual scroll stays where the user put it, not per-frame recentered.
        let revealed = tracker.scroll_offset;
        let output = tracker_frame(
            &ctx,
            &mut tracker,
            vec![
                egui::Event::PointerMoved(egui::pos2(700.0, 430.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(500.0, 900.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        assert!(!output.shapes.is_empty());
        for _ in 0..30 {
            tracker_frame(&ctx, &mut tracker, vec![], true);
        }
        let manual = tracker.scroll_offset;
        assert_ne!(
            manual, revealed,
            "Actual wheel input must move the viewport before checking it stays put"
        );
        for _ in 0..4 {
            tracker_frame(&ctx, &mut tracker, vec![], true);
        }
        assert_eq!(tracker.scroll_offset, manual);
        assert!(manual.x < expected.left() || manual.y < expected.top());
        assert_eq!(Selection::from_editor(&tracker.editor), cursor);
        assert_eq!(tracker.editor.song, before);
        assert_eq!(tracker.editor.revision(), revision);
    }

    #[test]
    fn measured_reveal_settles_with_bistable_scrollbars_and_tiny_viewports() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        let render = |tracker: &mut TrackerUi, size| {
            ctx.run(input(size, vec![]), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| tracker.editor_grid(ui));
            })
        };
        for _ in 0..35 {
            render(&mut tracker, egui::vec2(291.0, 295.0));
        }
        assert_eq!(tracker.viewport, egui::vec2(281.0, 285.0));
        tracker.editor.song.track_order = vec![TrackId(1)];
        tracker
            .editor
            .song
            .tracks
            .retain(|track, _| *track == TrackId(1));
        tracker
            .editor
            .song
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .note_columns = 1;
        for pattern in tracker.editor.song.patterns.values_mut() {
            pattern.rows = 11;
            pattern.tracks.retain(|track, _| *track == TrackId(1));
            for rows in pattern.tracks.values_mut() {
                rows.retain(|row, _| *row < 11);
                for columns in rows.values_mut() {
                    columns.retain(|column, _| *column == 0);
                }
            }
        }
        tracker.editor.row = 10;
        tracker.editor.field = Field::Delay;
        tracker.reveal_current_cursor();
        for _ in 0..35 {
            render(&mut tracker, egui::vec2(291.0, 295.0));
        }
        assert_eq!(
            tracker.viewport,
            egui::vec2(281.0, 285.0),
            "Both bars persist in egui's near-fit state even though no-bar content would fit"
        );
        assert!(!tracker.scroll_to_cursor);
        assert!(tracker.reveal_last_viewport.is_none());
        tracker.reveal_current_cursor();
        for _ in 0..35 {
            render(&mut tracker, egui::vec2(80.0, 80.0));
        }
        assert!(
            !tracker.scroll_to_cursor,
            "A field wider than a tiny viewport cannot cause an endless repaint request"
        );
    }
    #[test]
    fn frozen_time_does_not_finish_a_pending_scrollbar_reveal() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut tracker = TrackerUi::default();
        for pattern in tracker.editor.song.patterns.values_mut() {
            pattern.rows = 4;
        }
        let render = |tracker: &mut TrackerUi, time| {
            let mut input = input(egui::vec2(900.0, 620.0), vec![]);
            input.time = Some(time);
            ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| tracker.editor_grid(ui));
            })
        };
        for frame in 0..30 {
            render(&mut tracker, f64::from(frame) / 60.0);
        }
        for pattern in tracker.editor.song.patterns.values_mut() {
            pattern.rows = 128;
        }
        tracker.editor.select(0, TrackId(4)).unwrap();
        tracker.editor.row = 127;
        tracker.editor.column = 1;
        tracker.editor.field = Field::Delay;
        tracker.reveal_current_cursor();
        for _ in 0..10 {
            let output = render(&mut tracker, 0.5);
            assert!(
                tracker.scroll_to_cursor,
                "Equal-frame geometry at frozen time is not proof the scrollbar animation finished"
            );
            assert_eq!(
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                std::time::Duration::ZERO
            );
        }
        for frame in 1..40 {
            render(&mut tracker, 0.5 + f64::from(frame) / 60.0);
        }
        assert!(!tracker.scroll_to_cursor);
        let bounds = Selection::from_editor(&tracker.editor).bounds(&tracker.editor.song, 0);
        assert_eq!(
            reveal_selection(tracker.scroll_offset, tracker.viewport, bounds),
            tracker.scroll_offset
        );
    }
    #[test]
    fn fractional_origin_and_scale_reveal_terminates_without_repaint_loop() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        ctx.set_pixels_per_point(1.25);
        let mut tracker = TrackerUi::default();
        tracker.editor.select(0, TrackId(4)).unwrap();
        tracker.editor.column = 1;
        tracker.editor.row = tracker.editor.pattern().rows - 1;
        tracker.editor.field = Field::Delay;
        tracker.reveal_current_cursor();
        for frame in 0..45 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_max(
                    egui::pos2(246.47885, 53.125),
                    egui::pos2(762.74164, 613.375),
                )),
                time: Some(f64::from(frame) / 60.0),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| tracker.editor_grid(ui));
            });
        }
        assert!(
            !tracker.scroll_to_cursor,
            "Fractional Rect arithmetic must not retain a finished reveal forever"
        );
        assert!(tracker.reveal_last_viewport.is_none());
        let bounds = Selection::from_editor(&tracker.editor).bounds(&tracker.editor.song, 0);
        assert!(
            (reveal_selection(tracker.scroll_offset, tracker.viewport, bounds)
                - tracker.scroll_offset)
                .abs()
                .max_elem()
                < 0.001
        );
    }
}
