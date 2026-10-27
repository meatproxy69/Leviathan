//! Connected occurrence/track matrix; all mutations use validated editor history.
use crate::{editor::Editor, song::*};
use eframe::egui::{self, Color32};
#[derive(Default)]
pub struct ArrangementUi {
    pub open: bool,
    source: Option<BlockRef>,
    section_name: String,
    seen: Option<(usize, Option<Section>)>,
    error: Option<String>,
}
impl ArrangementUi {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
        egui::CollapsingHeader::new("Arrangement · matrix & sections")
            .default_open(self.open)
            .show(ui, |ui| self.contents(ui, editor))
            .body_response
            .is_some()
    }
    fn result(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }
    pub(crate) fn contents(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if !editor.has_grid() {
            ui.label("Add tracks and an order position to edit the arrangement.");
            return;
        }
        let section = editor.song.order[editor.occurrence].section.clone();
        let seen = (editor.occurrence, section.clone());
        if self.seen.as_ref() != Some(&seen) {
            self.section_name = section.map_or(String::new(), |s| s.name);
            self.seen = Some(seen);
        }
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut editor.edit_mode, "Edit Mode");
            ui.label(format!(
                "Position {} · P{} · T{}",
                editor.occurrence,
                editor.pattern_id().0,
                editor.track.0
            ));
            if ui
                .add_enabled(editor.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                editor.undo();
            }
            if ui
                .add_enabled(editor.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                editor.redo();
            }
        });
        ui.add_enabled_ui(editor.edit_mode, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut pattern = editor.pattern_id();
                egui::ComboBox::from_id_salt("order-pattern").selected_text(format!("P{}", pattern.0)).show_ui(ui, |ui| {
                    for (id, p) in &editor.song.patterns { ui.selectable_value(&mut pattern, *id, format!("P{} · {}", id.0, p.name)); }
                });
                if pattern != editor.pattern_id() { self.result(editor.assign_pattern(pattern)); }
                for (label, delta) in [("Move up", -1), ("Move down", 1)] {
                    if ui.button(label).clicked() { self.result(editor.move_occurrence(delta)); }
                }
                if ui.button("+Repeat").clicked() { self.result(editor.append(false)); }
                if ui.button("+Clone").clicked() { self.result(editor.append(true)); }
                let mut mute = editor.song.order[editor.occurrence].muted_tracks.contains(&editor.track);
                if ui.checkbox(&mut mute, "Mute track here").changed() { self.result(editor.mute_track(mute)); }
            });
            ui.horizontal_wrapped(|ui| {
                let source = self.source.get_or_insert(BlockRef { pattern: editor.pattern_id(), track: editor.track });
                ui.label("Alias source:");
                egui::ComboBox::from_id_salt("alias-pattern").selected_text(format!("P{}", source.pattern.0)).show_ui(ui, |ui| {
                    for (id, p) in &editor.song.patterns { ui.selectable_value(&mut source.pattern, *id, format!("P{} · {}", id.0, p.name)); }
                });
                egui::ComboBox::from_id_salt("alias-track").selected_text(format!("T{}", source.track.0)).show_ui(ui, |ui| {
                    for id in &editor.song.track_order { ui.selectable_value(&mut source.track, *id, format!("T{} · {}", id.0, editor.song.tracks[id].name)); }
                });
                let source = *source;
                if ui.button("Replace block with alias").on_hover_text("Replaces destination content. Undo restores it. Row and column counts must match.").clicked() { self.result(editor.alias_block(source)); }
                if ui.button("Make unique").clicked() { self.result(editor.unique_block(false)); }
                if ui.button("Empty block").on_hover_text("Detaches this block without deleting alias source cells.").clicked() { self.result(editor.unique_block(true)); }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Section starts here:");
                ui.add(egui::TextEdit::singleline(&mut self.section_name).desired_width(170.0));
                if ui.button("Set name / clear blank").clicked() { self.result(editor.mark_section(&self.section_name)); }
                if ui.button("Repeat section").clicked() { self.result(editor.append_section(false)); }
                if ui.button("Independent variation").clicked() { self.result(editor.append_section(true)); }
            });
        });
        let range = editor.song.section_range(editor.occurrence).unwrap();
        ui.small(format!("Section positions {}–{} · Select Tracker to edit the selected block.", range.start, range.end - 1))
            .on_hover_text("Repeats share patterns; variations copy resolved blocks. Aliases edit their source. Mute cuts voices at the position boundary.");
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.separator();
        egui::ScrollArea::both()
            .id_salt("arrangement-scroll")
            .show(ui, |ui| {
                egui::Grid::new("arrangement-matrix")
                    .striped(true)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong("Position / pattern / section");
                        let tracks = editor.song.track_order.clone();
                        for t in &tracks {
                            ui.strong(format!("T{} · {}", t.0, editor.song.tracks[t].name));
                        }
                        ui.end_row();
                        for i in 0..editor.song.order.len() {
                            let o = &editor.song.order[i];
                            let id = o.pattern;
                            let name = o.section.as_ref().map_or("", |s| s.name.as_str());
                            if ui
                                .selectable_label(
                                    i == editor.occurrence,
                                    format!("{i:03} / P{}  {name}", id.0),
                                )
                                .clicked()
                            {
                                self.result(editor.select(i, editor.track));
                            }
                            for t in &tracks {
                                let o = &editor.song.order[i];
                                let source = editor.song.block_source(id, *t).unwrap();
                                let alias = editor.song.patterns[&id].aliases.contains_key(t);
                                let populated = editor
                                    .song
                                    .block(id, *t)
                                    .unwrap()
                                    .is_some_and(|b| !b.is_empty());
                                let label = format!(
                                    "{}{}",
                                    if o.muted_tracks.contains(t) {
                                        "MUTE · "
                                    } else {
                                        ""
                                    },
                                    if alias {
                                        format!("↗ P{} / T{}", source.pattern.0, source.track.0)
                                    } else if populated {
                                        "■ local".into()
                                    } else {
                                        "· empty".into()
                                    }
                                );
                                if ui
                                    .selectable_label(
                                        i == editor.occurrence && *t == editor.track,
                                        label,
                                    )
                                    .on_hover_text(
                                        "Select this occurrence and track in the tracker",
                                    )
                                    .clicked()
                                {
                                    self.result(editor.select(i, *t));
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
    }
}
