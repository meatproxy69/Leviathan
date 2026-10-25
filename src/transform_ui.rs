//! Explicit preview/apply protects shared content from ambiguous scope changes.
use crate::{
    editor::Editor,
    transform::{Preview, Scope, Transform},
};
use eframe::egui::{self, Color32};
#[derive(Debug, Clone, PartialEq)]
struct Settings {
    scope: Scope,
    start: u32,
    end: u32,
    kind: usize,
    transpose: i16,
    velocity: f32,
    pan: f32,
    groove: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            scope: Scope::Track,
            start: 0,
            end: 16,
            kind: 3,
            transpose: 12,
            velocity: 0.8,
            pan: 0.1,
            groove: "0 0.25".into(),
        }
    }
}
#[derive(Default)]
pub struct TransformUi {
    pub open: bool,
    settings: Settings,
    preview: Option<Preview>,
    error: Option<String>,
    seen: Option<(usize, crate::song::PatternId, crate::song::TrackId, u16)>,
}
impl TransformUi {
    pub fn preview_track_groove(&mut self, editor: &Editor) -> Result<(), String> {
        self.open = true;
        self.seen = Some((
            editor.occurrence,
            editor.pattern_id(),
            editor.track,
            editor.column,
        ));
        self.settings.scope = Scope::Track;
        self.settings.start = 0;
        self.settings.end = editor.pattern().rows;
        self.settings.kind = 3;
        self.settings.groove = "0 0 0.375 0".into();
        self.preview = Some(editor.preview_transform(
            Scope::Track,
            0..editor.pattern().rows,
            &self.operation()?,
        )?);
        Ok(())
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
        egui::CollapsingHeader::new("Groove & scoped transformations")
            .default_open(self.open)
            .show(ui, |ui| self.contents(ui, editor))
            .body_response
            .is_some()
    }
    fn operation(&self) -> Result<Transform, String> {
        Ok(match self.settings.kind {
            0 => Transform::Transpose(self.settings.transpose),
            1 => Transform::ScaleVelocity(self.settings.velocity),
            2 => Transform::OffsetPan(self.settings.pan),
            3 => Transform::Groove(
                self.settings
                    .groove
                    .split_whitespace()
                    .map(|v| {
                        v.parse::<f64>()
                            .map_err(|_| "Enter space-separated row delays".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            _ => Transform::ReverseRows,
        })
    }
    pub(crate) fn contents(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if !editor.has_grid() {
            ui.label("Open a song with ordered patterns and tracks.");
            return;
        }
        let at = (
            editor.occurrence,
            editor.pattern_id(),
            editor.track,
            editor.column,
        );
        if self.seen != Some(at) {
            self.seen = Some(at);
            self.settings.end = editor.pattern().rows;
            self.settings.start = 0;
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
                "Pos {} · P{} · T{} · column {}",
                editor.occurrence,
                editor.pattern_id().0,
                editor.track.0,
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
            ui.label("Scope");
            egui::ComboBox::from_id_salt("transform-scope")
                .selected_text(self.settings.scope.label())
                .show_ui(ui, |ui| {
                    for scope in Scope::ALL {
                        ui.selectable_value(&mut self.settings.scope, scope, scope.label());
                    }
                });
            ui.add_enabled_ui(
                !matches!(self.settings.scope, Scope::Section | Scope::Song),
                |ui| {
                    ui.label("Rows from");
                    ui.add(egui::DragValue::new(&mut self.settings.start).range(0..=65535));
                    ui.label("to (exclusive)");
                    ui.add(egui::DragValue::new(&mut self.settings.end).range(1..=65536));
                },
            );
        });
        let names = [
            "Transpose notes",
            "Scale explicit velocities",
            "Offset explicit pans",
            "Apply groove delays",
            "Reverse row positions",
        ];
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("transform-operation")
                .selected_text(names[self.settings.kind])
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.settings.kind, i, *name);
                    }
                });
            match self.settings.kind {
                0 => {
                    ui.add(
                        egui::DragValue::new(&mut self.settings.transpose)
                            .range(-127..=127)
                            .suffix(" semitones"),
                    );
                }
                1 => {
                    ui.add(
                        egui::DragValue::new(&mut self.settings.velocity)
                            .range(0.0..=4.0)
                            .speed(0.01)
                            .prefix("× "),
                    );
                }
                2 => {
                    ui.add(
                        egui::DragValue::new(&mut self.settings.pan)
                            .range(-2.0..=2.0)
                            .speed(0.01),
                    );
                }
                3 => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.groove).desired_width(170.0),
                    );
                    if ui.button("Straight").clicked() {
                        self.settings.groove = "0 0".into();
                    }
                    if ui.button("Swing").clicked() {
                        self.settings.groove = "0 0.25".into();
                    }
                }
                _ => {
                    ui.label("Mirrors selected rows; preserves delay and FX.");
                }
            }
        });
        if before != self.settings {
            self.preview = None;
            self.error = None;
        }
        ui.small("Section/song uses every row of each distinct ordered pattern. Phrase definitions and unused patterns are not directly edited.");
        if self.settings.kind == 3 {
            ui.label("Groove: 1–16 delays in 0..<1, repeated from pattern row 0. Replaces delays on populated cells; zero clears delay. No hidden timing state.");
        }
        if self.settings.kind == 1 || self.settings.kind == 2 {
            ui.label("Only explicit values change. Omitted values retain column memory. Out-of-range results reject the whole edit.");
        }
        ui.label("Aliases detach when their cells change. Repeated patterns and downstream aliases can affect other positions; preview lists them. Create an independent variation first for occurrence-local changes.");
        ui.horizontal(|ui| {
            if ui.button("Preview").clicked() {
                match self.operation().and_then(|op| {
                    editor.preview_transform(
                        self.settings.scope,
                        self.settings.start..self.settings.end,
                        &op,
                    )
                }) {
                    Ok(p) => {
                        self.preview = Some(p);
                        self.error = None;
                    }
                    Err(e) => {
                        self.preview = None;
                        self.error = Some(e);
                    }
                }
            }
            let can_apply = editor.edit_mode
                && self
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.is_current(&editor.song) && p.summary.changed_cells > 0);
            if ui
                .add_enabled(
                    can_apply,
                    egui::Button::new("Apply preview · one undo step"),
                )
                .clicked()
            {
                self.error = editor.apply_transform(self.preview.take().unwrap()).err();
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Some(preview) = &self.preview {
            let s = &preview.summary;
            ui.strong(format!(
                "{} changed cell positions · {} blocks · {} aliases detached",
                s.changed_cells, s.changed_blocks, s.detached_aliases
            ));
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 26.0).clamp(30.0, 150.0))
                .show(ui, |ui| {
                    ui.label(format!("Affected positions: {:?}", s.affected_positions));
                    if !s.outside_scope_positions.is_empty() {
                        ui.colored_label(
                            Color32::YELLOW,
                            format!(
                                "Shared content outside scope: {:?}",
                                s.outside_scope_positions
                            ),
                        );
                    }
                    for cell in &s.examples {
                        let label = |c: &crate::song::Cell| {
                            format!(
                                "{} v:{} p:{} d:{}",
                                crate::editor::note_label(c.note),
                                c.velocity.map_or("·".into(), |v| format!("{v:.3}")),
                                c.pan.map_or("·".into(), |v| format!("{v:.3}")),
                                c.delay.map_or("·".into(), |v| format!("{v:.3}"))
                            )
                        };
                        ui.monospace(format!(
                            "P{} T{} row {} col {}: {} → {}",
                            cell.pattern.0,
                            cell.track.0,
                            cell.row,
                            cell.column + 1,
                            label(&cell.before),
                            label(&cell.after)
                        ));
                    }
                });
        }
        ui.small("Select Tracker to inspect committed cells.");
    }
}
