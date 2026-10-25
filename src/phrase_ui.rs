//! Instrument phrase editing commits through the main editor's validation/history.
use crate::{
    editor::{effects_label, note_label, parse_cell_value, Editor, Field},
    song::*,
};
use eframe::egui::{self, Color32, RichText};
pub struct PhraseUi {
    owner: Option<InstrumentId>,
    selected: Option<PhraseId>,
    row: u32,
    column: u16,
    field: Field,
    value: String,
    settings: Phrase,
    seen: Option<(InstrumentId, PhraseId, Phrase)>,
    error: Option<String>,
}
impl Default for PhraseUi {
    fn default() -> Self {
        Self {
            owner: None,
            selected: None,
            row: 0,
            column: 0,
            field: Field::Note,
            value: String::new(),
            settings: Phrase::default(),
            seen: None,
            error: None,
        }
    }
}
fn field_text(cell: &Cell, field: Field) -> String {
    match field {
        Field::Note => match cell.note {
            Note::Empty => String::new(),
            Note::Off => "OFF".into(),
            Note::Note(p) => p.to_string(),
        },
        Field::Velocity => cell.velocity.map_or(String::new(), |v| v.to_string()),
        Field::Pan => cell.pan.map_or(String::new(), |v| v.to_string()),
        Field::Delay => cell.delay.map_or(String::new(), |v| v.to_string()),
        Field::Effects => effects_label(&cell.effects),
        _ => String::new(),
    }
}
impl PhraseUi {
    pub fn select(&mut self, owner: InstrumentId, phrase: PhraseId) {
        self.owner = Some(owner);
        self.selected = Some(phrase);
        self.seen = None;
    }
    pub(crate) fn contents(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if self
            .owner
            .is_none_or(|id| !editor.song.instruments.contains_key(&id))
        {
            self.owner = editor.song.instruments.keys().next().copied();
            self.selected = None;
        }
        let Some(mut owner) = self.owner else {
            ui.label("This project has no instruments.");
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut editor.edit_mode, "Edit Mode");
            ui.label("Owner");
            egui::ComboBox::from_id_salt("phrase-owner")
                .selected_text(format!(
                    "{} · {}",
                    owner.0, editor.song.instruments[&owner].name
                ))
                .show_ui(ui, |ui| {
                    for (id, instrument) in &editor.song.instruments {
                        ui.selectable_value(
                            &mut owner,
                            *id,
                            format!("{} · {}", id.0, instrument.name),
                        );
                    }
                });
            if self.owner != Some(owner) {
                self.owner = Some(owner);
                self.selected = None;
                self.seen = None;
            }
            let phrases = &editor.song.instruments[&owner].phrases;
            if self.selected.is_none_or(|id| !phrases.contains_key(&id)) {
                self.selected = phrases.keys().next().copied();
            }
            egui::ComboBox::from_id_salt("phrase-select")
                .selected_text(self.selected.map_or("No phrases".into(), |id| {
                    format!("{} · {}", id.0, phrases[&id].name)
                }))
                .show_ui(ui, |ui| {
                    for (id, phrase) in phrases {
                        ui.selectable_value(
                            &mut self.selected,
                            Some(*id),
                            format!("{} · {}", id.0, phrase.name),
                        );
                    }
                });
            if ui
                .add_enabled(editor.edit_mode, egui::Button::new("New phrase"))
                .clicked()
            {
                match editor.add_phrase(owner, Phrase::default()) {
                    Ok(id) => {
                        self.selected = Some(id);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if ui
                .add_enabled(
                    editor.edit_mode && self.selected.is_some(),
                    egui::Button::new("Clone phrase"),
                )
                .clicked()
            {
                let mut phrase =
                    editor.song.instruments[&owner].phrases[&self.selected.unwrap()].clone();
                phrase.name.push_str(" copy");
                match editor.add_phrase(owner, phrase) {
                    Ok(id) => {
                        self.selected = Some(id);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if ui
                .add_enabled(
                    editor.edit_mode && self.selected.is_some(),
                    egui::Button::new("Delete phrase"),
                )
                .on_hover_text("Undoable; referenced phrases must be unassigned first")
                .clicked()
            {
                self.error = editor.delete_phrase(owner, self.selected.unwrap()).err();
                if self.error.is_none() {
                    self.selected = None;
                }
            }
        });
        let Some(id) = self.selected else {
            ui.label("Enable Edit Mode and create a phrase. It uses this instrument's sample.");
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            return;
        };
        let Some(phrase) = editor.song.instruments[&owner].phrases.get(&id).cloned() else {
            return;
        };
        if self.seen.as_ref() != Some(&(owner, id, phrase.clone())) {
            self.settings = phrase.clone();
            self.seen = Some((owner, id, phrase.clone()));
            self.row = self.row.min(phrase.rows - 1);
            self.column = self.column.min(phrase.note_columns - 1);
        }
        ui.label(RichText::new("Select Tracker to return to the song tracker / Follow view. Phrase edits use project Undo/Redo; enter notes as MIDI numbers or OFF.").small());
        ui.label(RichText::new("One-shot phrase · implicit owner instrument. Select Phr in a song note cell to trigger this ID; note pitch transposes the root.").small());
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.name)
                    .desired_width(150.0)
                    .hint_text("Phrase name"),
            );
            ui.label("Rows");
            ui.add(egui::DragValue::new(&mut self.settings.rows).range(1..=256));
            ui.label("LPB");
            ui.add(egui::DragValue::new(&mut self.settings.lines_per_beat).range(1..=256));
            ui.label("Columns");
            ui.add(egui::DragValue::new(&mut self.settings.note_columns).range(1..=16));
            ui.label("Root MIDI");
            ui.add(egui::DragValue::new(&mut self.settings.root_note).range(0..=127));
            if ui
                .add_enabled(editor.edit_mode, egui::Button::new("Apply settings"))
                .clicked()
            {
                let mut updated = phrase.clone();
                updated.name = self.settings.name.clone();
                updated.rows = self.settings.rows;
                updated.lines_per_beat = self.settings.lines_per_beat;
                updated.note_columns = self.settings.note_columns;
                updated.root_note = self.settings.root_note;
                self.error = editor.update_phrase(owner, id, updated).err();
            }
        });
        let current = phrase
            .cells
            .get(&self.row)
            .and_then(|c| c.get(&self.column))
            .cloned()
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "Phrase row {} · column {}",
                self.row,
                self.column + 1
            ));
            let before = self.field;
            egui::ComboBox::from_id_salt("phrase-field")
                .selected_text(format!("{:?}", self.field))
                .show_ui(ui, |ui| {
                    for field in [
                        Field::Note,
                        Field::Velocity,
                        Field::Pan,
                        Field::Delay,
                        Field::Effects,
                    ] {
                        ui.selectable_value(&mut self.field, field, format!("{field:?}"));
                    }
                });
            if before != self.field {
                self.value = field_text(&current, self.field);
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.value)
                    .desired_width(230.0)
                    .hint_text(if self.field == Field::Effects {
                        "retrigger=4 cut=0.5"
                    } else {
                        "MIDI 0..127 / OFF / blank"
                    }),
            );
            if ui
                .add_enabled(editor.edit_mode, egui::Button::new("Apply phrase field"))
                .clicked()
            {
                self.error = parse_cell_value(current.clone(), self.field, &self.value)
                    .and_then(|cell| editor.set_phrase_cell(owner, id, self.row, self.column, cell))
                    .err();
            }
            if ui
                .add_enabled(editor.edit_mode, egui::Button::new("Clear phrase cell"))
                .clicked()
            {
                self.error = editor
                    .set_phrase_cell(owner, id, self.row, self.column, Cell::default())
                    .err();
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        egui::ScrollArea::both()
            .id_salt("phrase-cells")
            .max_height(ui.available_height().max(80.0))
            .show_rows(ui, 22.0, phrase.rows as usize, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{row:03}"));
                        for column in 0..phrase.note_columns {
                            let cell = phrase
                                .cells
                                .get(&(row as u32))
                                .and_then(|c| c.get(&column))
                                .cloned()
                                .unwrap_or_default();
                            let text = format!(
                                "{}  V:{} P:{} D:{} {}",
                                note_label(cell.note),
                                cell.velocity.map_or("·".into(), |v| v.to_string()),
                                cell.pan.map_or("·".into(), |v| v.to_string()),
                                cell.delay.map_or("·".into(), |v| v.to_string()),
                                effects_label(&cell.effects)
                            );
                            if ui
                                .add_sized(
                                    [280.0, 20.0],
                                    egui::Button::new(RichText::new(&text).monospace())
                                        .selected(self.row == row as u32 && self.column == column)
                                        .truncate(),
                                )
                                .on_hover_text(&text)
                                .clicked()
                            {
                                self.row = row as u32;
                                self.column = column;
                                self.value = field_text(&cell, self.field);
                                ui.memory_mut(|m| m.request_focus(egui::Id::new("phrase-grid")));
                            }
                        }
                    });
                }
            });
    }
}
