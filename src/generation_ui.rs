//! Recipes are reusable; generated cells remain ordinary tracker content.
use crate::{
    editor::{note_label, Editor},
    generation::{self, Preview, Recipe, WriteMode},
};
use eframe::egui::{self, Color32};
#[derive(Clone, PartialEq)]
struct Settings {
    name: String,
    recipe: Recipe,
    sequence: String,
    seed: String,
    mode: WriteMode,
}
impl Default for Settings {
    fn default() -> Self {
        let recipe = Recipe::default();
        Self {
            name: "New recipe".into(),
            sequence: generation::sequence_text(&recipe.sequence),
            seed: recipe.seed.to_string(),
            recipe,
            mode: WriteMode::Merge,
        }
    }
}
#[derive(Default)]
pub struct GenerationUi {
    pub open: bool,
    settings: Settings,
    preview: Option<Preview>,
    error: Option<String>,
    loaded: String,
    seen: Option<(
        usize,
        crate::song::PatternId,
        crate::song::TrackId,
        u16,
        u32,
    )>,
}
impl GenerationUi {
    pub fn preview_demo(&mut self, editor: &Editor) -> Result<(), String> {
        self.open = true;
        self.seen = Some((
            editor.occurrence,
            editor.pattern_id(),
            editor.track,
            editor.column,
            editor.row,
        ));
        self.settings.name = "Native seed preview".into();
        self.settings.mode = WriteMode::Replace;
        self.settings.recipe = Recipe {
            rows: (editor.pattern().rows - editor.row).min(32),
            instrument: crate::song::InstrumentId(4),
            seed: 42,
            probability: 0.75,
            ..Default::default()
        };
        self.settings.sequence = generation::sequence_text(&self.settings.recipe.sequence);
        self.settings.seed = self.settings.recipe.seed.to_string();
        self.preview = Some(editor.preview_generation(
            &self.settings.name,
            &self.settings.recipe,
            self.settings.mode,
        )?);
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
        egui::CollapsingHeader::new("pattrns · generate editable cells")
            .default_open(self.open)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("generation-workspace")
                    .show(ui, |ui| self.contents(ui, editor));
            })
            .body_response
            .is_some()
    }
    pub(crate) fn contents(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
        if !editor.has_grid() {
            ui.label("Open a song with ordered patterns and tracks.");
            return;
        }
        let at = (
            editor.occurrence,
            editor.pattern_id(),
            editor.track,
            editor.column,
            editor.row,
        );
        if self.seen != Some(at) {
            self.seen = Some(at);
            self.preview = None;
        }
        if self
            .preview
            .as_ref()
            .is_some_and(|p| !p.is_current(&editor.song))
        {
            self.preview = None;
        }
        let before = self.settings.clone();
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut editor.edit_mode, "Edit Mode");
            ui.label(format!(
                "Target: pos {} · P{} · T{} · row {} · col {}",
                editor.occurrence,
                editor.pattern_id().0,
                editor.track.0,
                editor.row,
                editor.column + 1
            ));
            if ui
                .add_enabled(editor.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                editor.undo();
                self.preview = None;
            }
            if ui
                .add_enabled(editor.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                editor.redo();
                self.preview = None;
            }
        });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("generation-recipes")
                .selected_text(if self.loaded.is_empty() {
                    "Saved recipes"
                } else {
                    &self.loaded
                })
                .show_ui(ui, |ui| {
                    for name in editor.song.generation_recipes.keys() {
                        ui.selectable_value(&mut self.loaded, name.clone(), name);
                    }
                });
            if ui
                .add_enabled(
                    editor.song.generation_recipes.contains_key(&self.loaded),
                    egui::Button::new("Load recipe"),
                )
                .clicked()
            {
                let recipe = editor.song.generation_recipes[&self.loaded].clone();
                self.settings = Settings {
                    name: self.loaded.clone(),
                    sequence: generation::sequence_text(&recipe.sequence),
                    seed: recipe.seed.to_string(),
                    recipe,
                    mode: self.settings.mode,
                };
            }
            if ui
                .add_enabled(
                    editor.edit_mode && editor.song.generation_recipes.contains_key(&self.loaded),
                    egui::Button::new("Remove saved recipe"),
                )
                .on_hover_text("Removes the recipe only. Committed cells stay editable.")
                .clicked()
            {
                self.error = editor.remove_generation_recipe(&self.loaded).err();
                self.preview = None;
            }
            ui.label("Save as");
            ui.add(egui::TextEdit::singleline(&mut self.settings.name).desired_width(150.0));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Sequence");
            ui.add(egui::TextEdit::singleline(&mut self.settings.sequence).desired_width(300.0).hint_text("60,64,67 off ."))
                .on_hover_text("MIDI pitches 12–127. Commas form chords, spaces advance steps; off releases and . sustains.");
        });
        let recipe = &mut self.settings.recipe;
        ui.horizontal_wrapped(|ui| {
            ui.label("Rows");
            ui.add(egui::DragValue::new(&mut recipe.rows).range(1..=1024));
            ui.label("Step rows");
            ui.add(egui::DragValue::new(&mut recipe.step_rows).range(1..=64));
            ui.label("Cycle steps");
            ui.add(egui::DragValue::new(&mut recipe.cycle_steps).range(1..=64));
            ui.label("Hits");
            ui.add(egui::DragValue::new(&mut recipe.hits).range(0..=64));
            ui.label("Rotate");
            ui.add(egui::DragValue::new(&mut recipe.rotation).range(-63..=63));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Probability");
            ui.add(
                egui::DragValue::new(&mut recipe.probability)
                    .range(0.0..=1.0)
                    .speed(0.01),
            );
            ui.label("Seed");
            ui.add(egui::TextEdit::singleline(&mut self.settings.seed).desired_width(90.0))
                .on_hover_text("Exact unsigned 64-bit seed, in decimal");
            egui::ComboBox::from_id_salt("generation-instrument")
                .selected_text(format!("Instrument {}", recipe.instrument.0))
                .show_ui(ui, |ui| {
                    for (id, i) in &editor.song.instruments {
                        ui.selectable_value(
                            &mut recipe.instrument,
                            *id,
                            format!("{} · {}", id.0, i.name),
                        );
                    }
                });
            ui.label("Velocity");
            ui.add(
                egui::DragValue::new(&mut recipe.velocity)
                    .range(0.0..=1.0)
                    .speed(0.01),
            );
            ui.label("Pan");
            ui.add(
                egui::DragValue::new(&mut recipe.pan)
                    .range(-1.0..=1.0)
                    .speed(0.01),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.settings.mode,
                WriteMode::Merge,
                "Merge · refuse collisions",
            );
            ui.selectable_value(
                &mut self.settings.mode,
                WriteMode::Replace,
                "Replace selected rectangle",
            );
        });
        if before != self.settings {
            self.preview = None;
            self.error = None;
        }
        ui.small("Starts at cursor · seed resets · empty rows sustain · no automatic end OFF.")
            .on_hover_text(
                "The note/chord sequence advances only on accepted Euclidean/probability pulses.",
            );
        ui.small(
            "Aliases detach; repeats/downstream aliases can share edits. Preview lists positions.",
        )
        .on_hover_text("Use an independent section variation first for occurrence-local changes.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Generate preview").clicked() {
                match generation::parse_sequence(&self.settings.sequence).and_then(|sequence| {
                    self.settings.recipe.sequence = sequence;
                    self.settings.recipe.seed = self
                        .settings
                        .seed
                        .trim()
                        .parse()
                        .map_err(|_| "Enter an unsigned 64-bit seed in decimal".to_owned())?;
                    editor.preview_generation(
                        &self.settings.name,
                        &self.settings.recipe,
                        self.settings.mode,
                    )
                }) {
                    Ok(preview) => {
                        self.preview = Some(preview);
                        self.error = None;
                    }
                    Err(e) => {
                        self.preview = None;
                        self.error = Some(e);
                    }
                }
            }
            let can_apply = editor.edit_mode
                && self.preview.as_ref().is_some_and(|p| {
                    p.is_current(&editor.song) && (p.recipe_changed || p.summary.changed_cells > 0)
                });
            if ui
                .add_enabled(can_apply, egui::Button::new("Commit cells + save recipe"))
                .clicked()
            {
                self.error = editor.apply_generation(self.preview.take().unwrap()).err();
                if self.error.is_none() {
                    self.loaded = self.settings.name.trim().into();
                }
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Some(preview) = &self.preview {
            let s = &preview.summary;
            ui.strong(format!(
                "{} notes · {} OFFs · {} changed cells · {} aliases detached",
                preview.generated.notes,
                preview.generated.offs,
                s.changed_cells,
                s.detached_aliases
            ));
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 24.0).clamp(25.0, 150.0))
                .show(ui, |ui| {
                    ui.label(format!(
                        "Affected positions: {:?} · recipe {}",
                        s.affected_positions,
                        if preview.recipe_changed {
                            "will be saved"
                        } else {
                            "unchanged"
                        }
                    ));
                    if !s.outside_scope_positions.is_empty() {
                        ui.colored_label(
                            Color32::YELLOW,
                            format!(
                                "Shared changes outside selected occurrence: {:?}",
                                s.outside_scope_positions
                            ),
                        );
                    }
                    for c in &s.examples {
                        let a = &c.before;
                        let b = &c.after;
                        let mut changes = Vec::new();
                        if a.note != b.note {
                            changes.push(format!(
                                "{} → {}",
                                note_label(a.note),
                                note_label(b.note)
                            ));
                        }
                        if a.instrument != b.instrument {
                            changes.push(format!(
                                "I:{} → {}",
                                a.instrument.map_or("·".into(), |n| n.0.to_string()),
                                b.instrument.map_or("·".into(), |n| n.0.to_string())
                            ));
                        }
                        if a.velocity != b.velocity {
                            changes.push(format!(
                                "V:{} → {}",
                                a.velocity.map_or("·".into(), |v| format!("{v:.3}")),
                                b.velocity.map_or("·".into(), |v| format!("{v:.3}"))
                            ));
                        }
                        if a.pan != b.pan {
                            changes.push(format!(
                                "Pan:{} → {}",
                                a.pan.map_or("·".into(), |v| format!("{v:.3}")),
                                b.pan.map_or("·".into(), |v| format!("{v:.3}"))
                            ));
                        }
                        if a.delay != b.delay {
                            changes.push(format!(
                                "Delay:{} → {}",
                                a.delay.map_or("·".into(), |v| format!("{v:.3}")),
                                b.delay.map_or("·".into(), |v| format!("{v:.3}"))
                            ));
                        }
                        if a.effects != b.effects {
                            changes.push(format!(
                                "FX:[{}] → [{}]",
                                crate::editor::effects_label(&a.effects),
                                crate::editor::effects_label(&b.effects)
                            ));
                        }
                        if a.phrase != b.phrase {
                            changes.push(format!(
                                "Phr:{} → {}",
                                a.phrase.map_or("·".into(), |n| n.0.to_string()),
                                b.phrase.map_or("·".into(), |n| n.0.to_string())
                            ));
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "row {} col {}: {}",
                                    c.row,
                                    c.column + 1,
                                    changes.join(" · ")
                                ))
                                .monospace(),
                            )
                            .wrap(),
                        );
                    }
                });
        }
        ui.small("Commit once, then select Tracker to edit cells. Playback uses cells; it never reruns the generator.");
    }
}
