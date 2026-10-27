use crate::{
    editor::{effects_label, note_label, Editor, Field},
    song::{Cell, Note},
};
use eframe::egui::{self, Color32, RichText};

pub fn grid_id() -> egui::Id {
    egui::Id::new("tracker-keyboard")
}
/// Only the explicitly focused grid receives musical keys. Text/path widgets retain theirs.
pub fn keyboard(editor: &mut Editor, ctx: &egui::Context) -> Result<(), String> {
    keyboard_with_instrument(editor, ctx, None)
}
fn keyboard_with_instrument(
    editor: &mut Editor,
    ctx: &egui::Context,
    instrument: Option<crate::song::InstrumentId>,
) -> Result<(), String> {
    if !editor.has_grid() || !ctx.memory(|m| m.has_focus(grid_id())) {
        return Ok(());
    }
    for event in ctx.input(|i| i.events.clone()) {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        {
            if modifiers.command || modifiers.ctrl {
                match key {
                    egui::Key::Z if modifiers.shift => editor.redo(),
                    egui::Key::Z => editor.undo(),
                    egui::Key::Y => editor.redo(),
                    _ => (),
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
                egui::Key::Tab => editor.move_field(if modifiers.shift {
                    -(Field::ALL.len() as i32)
                } else {
                    Field::ALL.len() as i32
                }),
                egui::Key::Delete | egui::Key::Backspace if editor.edit_mode => {
                    editor.clear(modifiers.shift)?
                }
                egui::Key::Space => editor.edit_mode = !editor.edit_mode,
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
                            if let Some(instrument) =
                                instrument.filter(|id| editor.song.instruments.contains_key(id))
                            {
                                let mut cell = editor.cell();
                                cell.note = Note::Note(pitch as u8);
                                cell.instrument = Some(instrument);
                                editor.set_cell(cell)?;
                                editor.move_rows(editor.edit_step as i64);
                            } else {
                                editor.enter_note(Note::Note(pitch as u8))?;
                            }
                        }
                    }
                }
                _ => (),
            }
        }
    }
    Ok(())
}
fn values(cell: &Cell) -> [String; 7] {
    [
        note_label(cell.note),
        cell.instrument
            .map_or("··".into(), |i| format!("{:02}", i.0)),
        cell.velocity.map_or("···".into(), |v| format!("{v:.2}")),
        cell.pan.map_or("···".into(), |v| format!("{v:+.2}")),
        cell.delay.map_or("···".into(), |v| format!("{v:.3}")),
        if cell.effects.is_empty() {
            "···".into()
        } else {
            effects_label(&cell.effects)
                .replace("retrigger=", "R")
                .replace("cut=", "C")
                .replace("offset=", "O")
                .replace("reverse", "REV")
                .replace("glide=", "G")
        },
        cell.phrase.map_or("··".into(), |id| format!("{:02}", id.0)),
    ]
}
#[derive(Default)]
pub struct TrackerUi {
    pub editor: Editor,
    /// Playback snapshot occurrence/row, independent of the edit cursor.
    pub playhead: Option<(usize, u32)>,
    /// Instrument selected in the tracker for newly entered notes.
    pub selected_instrument: Option<crate::song::InstrumentId>,
    value: String,
    scroll_to_cursor: bool,
    prior_selection: Option<(usize, crate::song::TrackId)>,
    row_input: String,
    pub error: Option<String>,
}
impl TrackerUi {
    fn result(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.editor.has_grid() {
            ui.label("This valid project has no ordered patterns or tracks. Use New to start a blank editable song.");
            return;
        }
        let selection = (self.editor.occurrence, self.editor.track);
        if self.prior_selection != Some(selection) {
            self.prior_selection = Some(selection);
            self.scroll_to_cursor = true;
        }
        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("PATTERN {} · {}", self.editor.pattern_id().0, self.editor.pattern().name));
            ui.checkbox(&mut self.editor.edit_mode, "Edit Mode");
            ui.label("Step");
            ui.add(egui::DragValue::new(&mut self.editor.edit_step).range(0..=64));
            ui.label("Octave");
            ui.add(egui::DragValue::new(&mut self.editor.octave).range(0..=9));
            ui.label(RichText::new("Keyboard help ⓘ").small()).on_hover_text("Click a grid field to focus. Arrows / Tab navigate; Z–M enter notes; ` enters OFF; Delete clears a field; Shift+Delete clears a cell; Space toggles Edit Mode.");
        });
        let id = self.editor.pattern_id();
        if let Ok(source) = self.editor.song.block_source(id, self.editor.track) {
            if source.pattern != id || source.track != self.editor.track {
                ui.colored_label(Color32::YELLOW, format!("Alias edits source P{} / T{}. Use Arrangement → Make unique for local edits.", source.pattern.0, source.track.0));
            }
        }
        egui::CollapsingHeader::new("Pattern and selected field controls")
            .default_open(false)
            .show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let id = self.editor.pattern_id();
            ui.label(format!("Pattern {} · {}", id.0, self.editor.pattern().name));
            ui.label(format!("Rows {}", self.editor.pattern().rows));
            ui.add(
                egui::TextEdit::singleline(&mut self.row_input)
                    .desired_width(50.0)
                    .hint_text("rows"),
            );
            if ui
                .add_enabled(self.editor.edit_mode, egui::Button::new("Set rows"))
                .clicked()
            {
                let r = self
                    .row_input
                    .parse::<u32>()
                    .map_err(|_| "Enter a row count".into())
                    .and_then(|rows| self.editor.resize(rows));
                self.result(r);
            }
            ui.label(format!(
                "Edit cursor {:03} / track {} / column {} / {:?}",
                self.editor.row,
                self.editor.track.0,
                self.editor.column + 1,
                self.editor.field
            ));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Selected field (blank clears):");
            ui.add(
                egui::TextEdit::singleline(&mut self.value)
                    .desired_width(if self.editor.field == Field::Effects {
                        300.0
                    } else {
                        110.0
                    })
                    .hint_text(if self.editor.field == Field::Effects {
                        "retrigger=4 cut=0.5"
                    } else {
                        "value / OFF"
                    }),
            );
            if ui
                .add_enabled(self.editor.edit_mode, egui::Button::new("Apply field"))
                .clicked()
            {
                let r = self.editor.enter_value(&self.value);
                self.result(r);
            }
            if ui
                .add_enabled(self.editor.edit_mode, egui::Button::new("Clear field"))
                .clicked()
            {
                let r = self.editor.clear(false);
                self.result(r);
            }
            if ui
                .add_enabled(self.editor.edit_mode, egui::Button::new("Note OFF"))
                .clicked()
            {
                let r = self.editor.enter_note(Note::Off);
                self.result(r);
            }
        });
        if self.editor.field == Field::Effects {
            ui.label(RichText::new("FX: retrigger=4 cut=0.5 offset=0.25 reverse • or glide=12 (semitones/s). Cut uses the remaining row after delay. Blank clears all FX.").small());
        }
        if self.editor.field == Field::Phrase {
            ui.label(RichText::new("Phrase: enter an ID owned by the selected instrument. The note transposes its root; blank plays the sample directly. Delay/cut work on the trigger; other FX belong inside the phrase.").small());
        }
            });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.separator();
        let height = ui.available_height().max(100.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(160.0, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.strong("ORDER");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.editor.edit_mode, egui::Button::new("+ Repeat"))
                            .on_hover_text("Append a reference; edits stay shared")
                            .clicked()
                        {
                            let r = self.editor.append(false);
                            self.result(r);
                        }
                        if ui
                            .add_enabled(self.editor.edit_mode, egui::Button::new("+ Clone"))
                            .on_hover_text("Append independent copied content")
                            .clicked()
                        {
                            let r = self.editor.append(true);
                            self.result(r);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.editor.edit_mode && self.editor.occurrence > 0,
                                egui::Button::new("Up"),
                            )
                            .clicked()
                        {
                            let r = self.editor.move_occurrence(-1);
                            self.result(r);
                        }
                        if ui
                            .add_enabled(
                                self.editor.edit_mode
                                    && self.editor.occurrence + 1 < self.editor.song.order.len(),
                                egui::Button::new("Down"),
                            )
                            .clicked()
                        {
                            let r = self.editor.move_occurrence(1);
                            self.result(r);
                        }
                    });
                    egui::ScrollArea::both()
                        .id_salt("order-matrix")
                        .max_height(height - 90.0)
                        .show(ui, |ui| {
                            egui::Grid::new("matrix")
                                .spacing([3.0, 5.0])
                                .show(ui, |ui| {
                                    ui.label("Pos / Pat");
                                    let tracks = self.editor.song.track_order.clone();
                                    for t in &tracks {
                                        ui.label(format!("T{}", t.0));
                                    }
                                    ui.end_row();
                                    for index in 0..self.editor.song.order.len() {
                                        let id = self.editor.song.order[index].pattern;
                                        if ui
                                            .selectable_label(
                                                self.editor.occurrence == index,
                                                format!("{:02} / {:02}{}", index, id.0, if self.editor.song.order[index].section.is_some() { " §" } else { "" }),
                                            )
                                            .clicked()
                                        {
                                            self.scroll_to_cursor = true;
                                            let r = self.editor.select(index, self.editor.track);
                                            self.result(r);
                                        }
                                        for track in &tracks {
                                            let populated = self.editor.song.block(id, *track).ok().flatten()
                                                .is_some_and(|r| {
                                                    r.values().any(|c| {
                                                        c.values().any(|v| *v != Cell::default())
                                                    })
                                                });
                                            if ui
                                                .selectable_label(
                                                    self.editor.occurrence == index
                                                        && self.editor.track == *track,
                                                    if self.editor.song.order[index].muted_tracks.contains(track) { "M" } else if self.editor.song.patterns[&id].aliases.contains_key(track) { "↗" } else if populated { "■" } else { "·" },
                                                )
                                                .on_hover_text(format!(
                                                    "Pattern {} · {}",
                                                    id.0, self.editor.song.tracks[track].name
                                                ))
                                                .clicked()
                                            {
                                                self.scroll_to_cursor = true;
                                                let r = self.editor.select(index, *track);
                                                self.result(r);
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    ui.label(RichText::new("Order help ⓘ").small().weak()).on_hover_text(
                        "Repeated patterns and aliases share edits. Use Arrange for sections and occurrence mutes."
                    );
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                let rows = self.editor.pattern().rows;
                let mut selected_rect = None;
                let mut scroll = egui::ScrollArea::both().id_salt("tracker-scroll");
                if self.scroll_to_cursor {
                    self.scroll_to_cursor = false;
                    let mut index = 0;
                    for track in &self.editor.song.track_order {
                        if *track == self.editor.track {
                            index += self.editor.column as usize;
                            break;
                        }
                        index += self.editor.song.tracks[track].note_columns as usize;
                    }
                    scroll = scroll
                        .vertical_scroll_offset(self.editor.row.saturating_sub(4) as f32 * 23.0)
                        .horizontal_scroll_offset(index as f32 * 364.0);
                }
                scroll.auto_shrink([false, false]).show_rows(
                    ui,
                    23.0,
                    rows as usize + 1,
                    |ui, range| {
                        egui::Grid::new("tracker-cells")
                            .spacing([2.0, 2.0])
                            .min_row_height(21.0)
                            .show(ui, |ui| {
                                let tracks = self.editor.song.track_order.clone();
                                for line in range {
                                    let playing = line > 0
                                        && self.playhead
                                            == Some((self.editor.occurrence, line as u32 - 1));
                                    ui.label(
                                        RichText::new(if line == 0 {
                                            "ROW".into()
                                        } else if playing {
                                            format!("▶{:03}", line - 1)
                                        } else {
                                            format!("{:03}", line - 1)
                                        })
                                        .monospace()
                                        .color(
                                            if playing {
                                                Color32::YELLOW
                                            } else {
                                                Color32::GRAY
                                            },
                                        ),
                                    );
                                    for track in &tracks {
                                        for column in 0..self.editor.song.tracks[track].note_columns
                                        {
                                            if line == 0 {
                                                for name in [
                                                    format!("T{}:{}", track.0, column + 1),
                                                    "Ins".into(),
                                                    "Vol".into(),
                                                    "Pan".into(),
                                                    "Delay".into(),
                                                    "FX".into(),
                                                    "Phr".into(),
                                                ] {
                                                    ui.monospace(name);
                                                }
                                            } else {
                                                let row = line as u32 - 1;
                                                let cell = self
                                                    .editor
                                                    .song.block(self.editor.pattern_id(), *track).ok().flatten()
                                                    .and_then(|r| r.get(&row))
                                                    .and_then(|c| c.get(&column))
                                                    .cloned()
                                                    .unwrap_or_default();
                                                for (field, text) in
                                                    Field::ALL.into_iter().zip(values(&cell))
                                                {
                                                    let selected = self.editor.row == row
                                                        && self.editor.track == *track
                                                        && self.editor.column == column
                                                        && self.editor.field == field;
                                                    let color = if row.is_multiple_of(4) {
                                                        Color32::from_rgb(131, 221, 201)
                                                    } else {
                                                        Color32::LIGHT_GRAY
                                                    };
                                                    let response = ui.add_sized(
                                                        [
                                                            if field == Field::Effects {
                                                                72.0
                                                            } else if field == Field::Note {
                                                                48.0
                                                            } else if matches!(
                                                                field,
                                                                Field::Velocity | Field::Pan | Field::Delay
                                                            ) {
                                                                50.0
                                                            } else {
                                                                32.0
                                                            },
                                                            21.0,
                                                        ],
                                                        egui::Button::new(
                                                            RichText::new(text)
                                                                .monospace()
                                                                .color(color),
                                                        )
                                                        .fill(if selected {
                                                            Color32::from_rgb(63, 104, 90)
                                                        } else if row.is_multiple_of(4) {
                                                            Color32::from_rgb(35, 59, 65)
                                                        } else {
                                                            Color32::from_rgb(8, 13, 16)
                                                        })
                                                        .selected(selected)
                                                        .truncate(),
                                                    );
                                                    if field == Field::Effects {
                                                        response.clone().on_hover_text(
                                                            effects_label(&cell.effects),
                                                        );
                                                    }
                                                    if selected {
                                                        selected_rect = Some(response.rect);
                                                    }
                                                    if response.clicked() {
                                                        self.editor.track = *track;
                                                        self.editor.column = column;
                                                        self.editor.row = row;
                                                        self.editor.field = field;
                                                        ui.memory_mut(|m| {
                                                            m.request_focus(grid_id())
                                                        });
                                                        self.value = if field == Field::Effects {
                                                            effects_label(&cell.effects)
                                                        } else if field == Field::Phrase {
                                                            cell.phrase.map_or(String::new(), |id| id.0.to_string())
                                                        } else {
                                                            String::new()
                                                        };
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    },
                );
                // Register a dedicated focus target, distinct from path and value text fields.
                let focus_rect = selected_rect.unwrap_or_else(|| {
                    egui::Rect::from_min_size(ui.min_rect().min, egui::Vec2::ZERO)
                });
                let response = ui.interact(
                    focus_rect,
                    grid_id(),
                    egui::Sense::focusable_noninteractive(),
                );
                if response.has_focus() {
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
            });
        });
        let before = (self.editor.row, self.editor.track, self.editor.column);
        let r = if ui.is_enabled() {
            keyboard_with_instrument(&mut self.editor, ui.ctx(), self.selected_instrument)
        } else {
            Ok(())
        };
        if before != (self.editor.row, self.editor.track, self.editor.column) {
            self.scroll_to_cursor = true;
            ui.ctx().request_repaint();
        }
        if let Err(error) = r {
            self.error = Some(error);
        }
    }
}

/// Follow is a read-only snapshot viewport; it never changes the editor selection.
pub fn playback_view(ui: &mut egui::Ui, song: &crate::song::Song, occurrence: usize, row: u32) {
    let pattern = &song.patterns[&song.order[occurrence].pattern];
    ui.heading(format!(
        "Playback snapshot · position {} · {}",
        occurrence + 1,
        pattern.name
    ));
    ui.label("Follow is read-only. Turn Follow off to edit; your edit cursor is preserved.");
    egui::ScrollArea::both()
        .id_salt("playback-follow")
        .vertical_scroll_offset(row.saturating_sub(3) as f32 * 23.0)
        .show_rows(ui, 23.0, pattern.rows as usize, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        if r == row as usize {
                            Color32::YELLOW
                        } else {
                            Color32::GRAY
                        },
                        format!("{:03} {}", r, if r == row as usize { "▶" } else { " " }),
                    );
                    for track in &song.track_order {
                        for column in 0..song.tracks[track].note_columns {
                            let cell = song
                                .block(song.order[occurrence].pattern, *track)
                                .ok()
                                .flatten()
                                .and_then(|rows| rows.get(&(r as u32)))
                                .and_then(|c| c.get(&column))
                                .cloned()
                                .unwrap_or_default();
                            ui.label(
                                RichText::new(format!(
                                    "T{}:{} {} {}",
                                    track.0,
                                    column + 1,
                                    if song.order[occurrence].muted_tracks.contains(track) {
                                        "MUTED"
                                    } else {
                                        ""
                                    },
                                    values(&cell).join(" ")
                                ))
                                .monospace(),
                            );
                        }
                    }
                });
            }
        });
}
#[cfg(test)]
mod selected_instrument_tests {
    use super::*;
    #[test]
    fn note_entry_records_selected_instrument_in_one_undo_step() {
        let ctx = egui::Context::default();
        let mut editor = Editor {
            edit_mode: true,
            ..Editor::default()
        };
        let before = editor.cell();
        ctx.memory_mut(|m| m.request_focus(grid_id()));
        let event = egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.interact(
                        ui.max_rect(),
                        grid_id(),
                        egui::Sense::focusable_noninteractive(),
                    );
                });
                keyboard_with_instrument(&mut editor, ctx, Some(crate::song::InstrumentId(2)))
                    .unwrap();
            },
        );
        editor.row = 0;
        assert_eq!(editor.cell().note, Note::Note(60));
        assert_eq!(editor.cell().instrument, Some(crate::song::InstrumentId(2)));
        editor.undo();
        assert_eq!(editor.cell(), before);
    }
}
