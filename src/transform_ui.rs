//! Explicit, non-destructive preview of bounded edits to ordinary pattern cells.
//! This view owns no musical events, runtime groove state, or audio capability.
use crate::{
    break_compiler::Subdivision,
    editor::{note_label, Editor, Field},
    song::{Cell, Note, PatternId, TrackId},
    tracker_ui::{grid_id, AQUA},
    transforms::{
        self, TransformConfig, TransformFields, TransformPreview, TransformScope, TransformSwing,
    },
};
use eframe::egui::{self, Color32, RichText};
use serde_json::{json, Value};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const AMBER: Color32 = Color32::from_rgb(229, 185, 112);

#[derive(Clone, PartialEq, Eq)]
struct Context {
    occurrence: usize,
    pattern: PatternId,
    track: TrackId,
    row: u32,
    column: u16,
    field: Field,
}
impl Context {
    fn of(editor: &Editor) -> Option<Self> {
        editor.has_grid().then(|| Self {
            occurrence: editor.occurrence,
            pattern: editor.pattern_id(),
            track: editor.track,
            row: editor.row,
            column: editor.column,
            field: editor.field,
        })
    }
}
#[derive(Clone, PartialEq)]
struct Draft {
    context: Context,
    revision: u64,
    config: TransformConfig,
    velocity_scale: String,
    row_start: String,
    row_end: String,
    transpose: String,
    nudge: String,
    swing_amount: String,
}
impl Draft {
    fn effective_config(&self) -> Result<TransformConfig, String> {
        let mut config = self.config.clone();
        let scale = self
            .velocity_scale
            .parse::<f64>()
            .map_err(|_| "Velocity scale must be a finite decimal in 0..4".to_owned())?;
        if !scale.is_finite() || !(0.0..=4.0).contains(&scale) {
            return Err("Velocity scale must be a finite decimal in 0..4; no clamping".into());
        }
        config.velocity_scale = scale as f32;
        config.scope.row_start = self
            .row_start
            .parse::<u32>()
            .map_err(|_| "Row start must be a nonnegative integer; no clamping".to_owned())?;
        config.scope.row_end = self.row_end.parse::<u32>().map_err(|_| {
            "Row end must be a nonnegative integer (exclusive); no clamping".to_owned()
        })?;
        config.transpose_semitones =
            parse_integer(&self.transpose, -48, 48, "Transpose (semitones)")? as i8;
        config.nudge_frames =
            parse_integer(&self.nudge, -48000, 48000, "Nudge (output frames)")? as i32;
        if let Some(swing) = &mut config.swing {
            swing.amount_percent = parse_integer(&self.swing_amount, 0, 50, "Swing percent")? as u8;
        }
        Ok(config)
    }
}
fn parse_integer(text: &str, min: i64, max: i64, label: &str) -> Result<i64, String> {
    let value = text.parse::<i64>().map_err(|_| {
        format!("{label} must be an integer in {min}..{max}; no rounding or clamping")
    })?;
    if !(min..=max).contains(&value) {
        return Err(format!("{label} must be in {min}..{max}; no clamping"));
    }
    Ok(value)
}
struct Pending {
    epoch: u64,
    revision: u64,
    receiver: Receiver<Result<TransformPreview, String>>,
}
struct Cached {
    revision: u64,
    preview: TransformPreview,
    reveal: bool,
}
#[derive(Default)]
pub(crate) struct TransformUi {
    draft: Option<Draft>,
    pending: Option<Pending>,
    cached: Option<Cached>,
    epoch: u64,
    error: Option<String>,
    notice: Option<String>,
}
impl TransformUi {
    pub(crate) fn agent_state(&self, editor: &Editor) -> Value {
        let draft = self.draft.as_ref();
        let current = draft.is_some_and(|_| self.current(editor));
        json!({
            "component":"transform","open":self.is_open(),"busy":self.busy(),"current":current,"phase":if self.busy(){"busy"}else if current&&self.cached.is_some(){"preview_ready"}else if self.is_open(){"draft"}else{"closed"},
            "actions":["open","set_draft","preview","poll","commit","cancel","close","reset"],
            "draft_fields":{"pattern":"u64","track":"u64","columns":"array<u16>","row_start":"integer text, inclusive","row_end":"integer text, exclusive","fields":{"notes":"bool","glide_targets":"bool","velocity":"bool","timing":"bool"},"transpose":"integer text -48..48","velocity_scale":"decimal text 0..4","nudge":"integer text -48000..48000","swing_division":"null|Sixteenth|ThirtySecond|SixtyFourth|OneTwentyEighth","swing_amount":"integer text 0..50"},
            "draft":draft.map(|d|json!({"revision":d.revision,"cursor":{"occurrence":d.context.occurrence,"pattern":d.context.pattern,"track":d.context.track,"row":d.context.row,"column":d.context.column,"field":d.context.field},"pattern":d.config.scope.pattern,"track":d.config.scope.track,"columns":d.config.scope.columns,"row_start":d.row_start,"row_end":d.row_end,"fields":d.config.fields,"transpose":d.transpose,"velocity_scale":d.velocity_scale,"nudge":d.nudge,"swing_division":d.config.swing.map(|s|s.division),"swing_amount":d.swing_amount,"parsed_config":d.effective_config().ok()})),
            "preview":self.cached.as_ref().map(|c|json!({"revision":c.revision,"current":current,"config":c.preview.config(),"summary":c.preview.summary(),"changes":c.preview.changes()})),"error":self.error,"notice":self.notice,"edit_mode":editor.edit_mode
        })
    }

    pub(crate) fn agent_action(
        &mut self,
        editor: &mut Editor,
        _base: Option<&std::path::Path>,
        action: Value,
    ) -> Result<Value, String> {
        let object = action.as_object().ok_or("Action must be a JSON object")?;
        let verb = object
            .get("action")
            .and_then(Value::as_str)
            .ok_or("Action requires a string `action`")?;
        match verb {
            "open" => {
                ensure_agent_fields(object, &["action"])?;
                self.open(editor);
            }
            "cancel" | "close" | "reset" => {
                ensure_agent_fields(object, &["action"])?;
                self.reset_document();
            }
            "set_draft" => {
                ensure_agent_fields(object, &["action", "values"])?;
                let values = object
                    .get("values")
                    .and_then(Value::as_object)
                    .ok_or("set_draft requires an object `values`")?;
                let mut draft = self
                    .draft
                    .clone()
                    .ok_or("Open transform before setting its draft")?;
                for (field, value) in values {
                    match field.as_str() {
                        "pattern" => {
                            draft.config.scope.pattern =
                                PatternId(value.as_u64().ok_or("pattern must be u64")?)
                        }
                        "track" => {
                            draft.config.scope.track =
                                TrackId(value.as_u64().ok_or("track must be u64")?)
                        }
                        "columns" => {
                            draft.config.scope.columns = serde_json::from_value(value.clone())
                                .map_err(|_| "columns must be an array of u16")?
                        }
                        "row_start" => {
                            draft.row_start =
                                value.as_str().ok_or("row_start must remain text")?.into()
                        }
                        "row_end" => {
                            draft.row_end = value.as_str().ok_or("row_end must remain text")?.into()
                        }
                        "transpose" => {
                            draft.transpose =
                                value.as_str().ok_or("transpose must remain text")?.into()
                        }
                        "velocity_scale" => {
                            draft.velocity_scale = value
                                .as_str()
                                .ok_or("velocity_scale must remain text")?
                                .into()
                        }
                        "nudge" => {
                            draft.nudge = value.as_str().ok_or("nudge must remain text")?.into()
                        }
                        "swing_amount" => {
                            draft.swing_amount = value
                                .as_str()
                                .ok_or("swing_amount must remain text")?
                                .into()
                        }
                        "fields" => {
                            let fields = value.as_object().ok_or("fields must be an object")?;
                            ensure_agent_fields(
                                fields,
                                &["notes", "glide_targets", "velocity", "timing"],
                            )?;
                            if let Some(v) = fields.get("notes") {
                                draft.config.fields.notes =
                                    v.as_bool().ok_or("notes must be boolean")?;
                            }
                            if let Some(v) = fields.get("glide_targets") {
                                draft.config.fields.glide_targets =
                                    v.as_bool().ok_or("glide_targets must be boolean")?;
                            }
                            if let Some(v) = fields.get("velocity") {
                                draft.config.fields.velocity =
                                    v.as_bool().ok_or("velocity must be boolean")?;
                            }
                            if let Some(v) = fields.get("timing") {
                                draft.config.fields.timing =
                                    v.as_bool().ok_or("timing must be boolean")?;
                            }
                        }
                        "swing_division" => {
                            draft.config.swing = match value.as_str() {
                                None | Some("none") => None,
                                Some("Sixteenth") => Some(TransformSwing {
                                    division: Subdivision::Sixteenth,
                                    amount_percent: 0,
                                }),
                                Some("ThirtySecond") => Some(TransformSwing {
                                    division: Subdivision::ThirtySecond,
                                    amount_percent: 0,
                                }),
                                Some("SixtyFourth") => Some(TransformSwing {
                                    division: Subdivision::SixtyFourth,
                                    amount_percent: 0,
                                }),
                                Some("OneTwentyEighth") => Some(TransformSwing {
                                    division: Subdivision::OneTwentyEighth,
                                    amount_percent: 0,
                                }),
                                Some(_) => return Err("Unknown swing division".into()),
                            };
                        }
                        _ => return Err(format!("Unknown transform draft field `{field}`")),
                    }
                }
                if let Some(swing) = &mut draft.config.swing {
                    swing.amount_percent =
                        parse_integer(&draft.swing_amount, 0, 50, "Swing percent")? as u8;
                }
                if self.draft.as_ref() != Some(&draft) {
                    self.invalidate("Settings changed: preview the exact new transform.");
                }
                self.draft = Some(draft);
            }
            "preview" => {
                ensure_agent_fields(object, &["action"])?;
                if self.draft.is_none() {
                    return Err("Open transform before previewing".into());
                }
                self.start_preview(editor);
            }
            "poll" => {
                ensure_agent_fields(object, &["action"])?;
                self.poll();
            }
            "commit" => {
                ensure_agent_fields(object, &["action"])?;
                if !editor.edit_mode {
                    return Err("Enable Edit Mode to commit transform".into());
                }
                if !self.current(editor) {
                    return Err("Transform preview is stale or missing".into());
                }
                let c = self.cached.as_ref().unwrap();
                if c.preview.summary().changed_cells == 0 {
                    return Err("Transform preview has no changes to commit".into());
                }
                editor.commit_transform_preview(c.revision, &c.preview)?;
                self.reset_document();
            }
            _ => return Err(format!("Unknown transform action `{verb}`")),
        }
        Ok(self.agent_state(editor))
    }
    pub(crate) fn open(&mut self, editor: &Editor) {
        let Some(context) = Context::of(editor) else {
            return;
        };
        self.reset_document();
        self.draft = Some(Draft {
            context,
            revision: editor.revision(),
            config: TransformConfig {
                scope: TransformScope {
                    pattern: editor.pattern_id(),
                    track: editor.track,
                    row_start: editor.row,
                    row_end: editor.row + 1,
                    columns: vec![editor.column],
                },
                fields: TransformFields {
                    notes: true,
                    glide_targets: false,
                    velocity: false,
                    timing: false,
                },
                transpose_semitones: 0,
                velocity_scale: 1.0,
                nudge_frames: 0,
                swing: None,
            },
            velocity_scale: "1".into(),
            row_start: editor.row.to_string(),
            row_end: (editor.row + 1).to_string(),
            transpose: "0".into(),
            nudge: "0".into(),
            swing_amount: "0".into(),
        });
    }
    pub(crate) fn is_open(&self) -> bool {
        self.draft.is_some()
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn reset_document(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.draft = None;
        self.cached = None;
        self.error = None;
        self.notice = None;
        // A canceled worker remains tracked until completion; never spawn an unbounded replacement.
    }
    fn invalidate(&mut self, notice: &str) {
        self.epoch = self.epoch.wrapping_add(1);
        self.cached = None;
        self.error = None;
        self.notice = Some(notice.into());
    }
    pub(crate) fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Transform worker disconnected; no document changed".into())
            }
        };
        let pending = self.pending.take().unwrap();
        if pending.epoch != self.epoch || self.draft.is_none() {
            return;
        }
        match result {
            Ok(preview) => {
                self.cached = Some(Cached {
                    revision: pending.revision,
                    preview,
                    reveal: true,
                });
                self.notice = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn start_preview(&mut self, editor: &Editor) {
        if self.pending.is_some() {
            return;
        }
        let Some(draft) = &self.draft else {
            return;
        };
        let config = match draft.effective_config() {
            Ok(config) => config,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let revision = editor.revision();
        let song = editor.song.clone();
        self.cached = None;
        self.error = None;
        self.notice = None;
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-transform-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    transforms::preview(&song, config)
                }))
                .unwrap_or_else(|_| Err("Transform worker panicked; no document changed".into()));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    revision,
                    receiver,
                })
            }
            Err(error) => self.error = Some(format!("Could not start transform preview: {error}")),
        }
    }
    fn current(&self, editor: &Editor) -> bool {
        let (Some(draft), Some(cached)) = (&self.draft, &self.cached) else {
            return false;
        };
        Context::of(editor).as_ref() == Some(&draft.context)
            && cached.revision == editor.revision()
            && cached.preview.base() == &editor.song
            && draft
                .effective_config()
                .is_ok_and(|config| &config == cached.preview.config())
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_preview(&self, editor: &Editor) -> Option<&TransformPreview> {
        self.current(editor)
            .then(|| &self.cached.as_ref().unwrap().preview)
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        if Context::of(editor).as_ref() != Some(&draft.context) {
            self.reset_document();
            return;
        }
        if draft.revision != editor.revision() {
            draft.revision = editor.revision();
            self.invalidate("Document changed: prepare a new preview before committing.");
        }
        let allowed = ui.is_enabled();
        let writable = allowed && editor.edit_mode;
        let old_draft = draft.clone();
        let mut open = true;
        let mut preview = false;
        let mut commit = false;
        let mut close = false;
        let ctx = ui.ctx().clone();
        let body_height = (ctx.content_rect().height() - 390.0).max(140.0);
        egui::Window::new("Scoped transform")
            .id(egui::Id::new("scoped-transform"))
            .default_pos(egui::pos2(220.0, 145.0))
            .default_width(620.0)
            .min_width(500.0)
            .resizable(false)
            .collapsible(false)
            .open(&mut open)
            .show(&ctx, |ui| {
                ui.set_max_width(620.0);
                ui.label(RichText::new("ORDINARY PATTERN CELLS · PREVIEW BEFORE COMMIT").color(AQUA).strong());
                ui.label(RichText::new(format!("Document revision {} · cursor POS {:02} / P{} / T{}", draft.revision, draft.context.occurrence, draft.context.pattern.0, draft.context.track.0)).small().color(MUTED));
                egui::ScrollArea::vertical().id_salt("transform-body").max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add_enabled_ui(allowed, |ui| scope_fields(ui, editor, &mut draft));
                    ui.separator();
                    ui.add_enabled_ui(allowed, |ui| operation_fields(ui, &mut draft));
                    ui.separator();
                    if let Err(error) = editor.song.require_local_track(draft.config.scope.pattern, draft.config.scope.track) {
                        ui.colored_label(AMBER, error);
                    }
                    ui.label(RichText::new("Edits affect this shared pattern block, its ordered occurrences and aliases. No runtime groove, snapping, clamping or audition.").small().color(MUTED));
                    if let Some(cached) = &mut self.cached { preview_details(ui, &cached.preview, cached.reveal); cached.reveal = false; }
                });
                ui.separator();
                if let Some(cached) = &self.cached {
                    let scope = &cached.preview.config().scope;
                    ui.label(RichText::new(format!("READY · {} changed · P{} / T{} · rows [{},{}) · N {:?}", cached.preview.summary().changed_cells, scope.pattern.0, scope.track.0, scope.row_start, scope.row_end, scope.columns.iter().map(|column| column + 1).collect::<Vec<_>>())).small().color(AQUA));
                    ui.label(RichText::new(operation_summary(&cached.preview)).small().color(MUTED));
                }
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
                if let Some(notice) = &self.notice { ui.label(RichText::new(notice).small().color(AMBER)); }
                if self.pending.is_some() { ui.label(RichText::new("Preview worker running · no song mutation").small().color(MUTED)); }
                if !editor.edit_mode { ui.label(RichText::new("READ MODE · preview is allowed; enable Edit mode to commit").small().color(AMBER)); }
                let current = self.cached.as_ref().is_some_and(|cached| cached.revision == editor.revision() && cached.preview.base() == &editor.song && draft.effective_config().is_ok_and(|config| &config == cached.preview.config()));
                let changed = self.cached.as_ref().map_or(0, |cached| cached.preview.summary().changed_cells);
                ui.horizontal(|ui| {
                    preview = ui.add_enabled(allowed && self.pending.is_none(), egui::Button::new("Preview transform")).clicked();
                    commit = ui.add_enabled(writable && current && changed > 0 && self.pending.is_none(), egui::Button::new("Commit transform")).clicked();
                    close = ui.button("Cancel / close").clicked();
                });
                ui.label(RichText::new("Commit changes existing cells once; Undo restores the whole transform. Changing scope/settings discards its preview.").small().color(MUTED));
            });
        ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        if !open || close {
            self.reset_document();
            return;
        }
        if old_draft != draft {
            self.invalidate("Settings changed: preview the exact new operation.");
        }
        self.draft = Some(draft);
        if preview && allowed {
            self.start_preview(editor);
        }
        if commit && writable && self.current(editor) {
            let cached = self.cached.as_ref().unwrap();
            match editor.commit_transform_preview(cached.revision, &cached.preview) {
                Ok(()) => self.reset_document(),
                Err(error) => self.error = Some(error),
            }
        }
    }
}
fn ensure_agent_fields(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unknown action field `{key}`"));
    }
    Ok(())
}

fn scope_fields(ui: &mut egui::Ui, editor: &Editor, draft: &mut Draft) {
    ui.label(RichText::new("SCOPE · stored rows [start, end), zero-based").strong());
    ui.horizontal(|ui| {
        ui.label("Pattern");
        egui::ComboBox::from_id_salt("transform-pattern")
            .selected_text(format!("P{}", draft.config.scope.pattern.0))
            .width(150.0)
            .show_ui(ui, |ui| {
                for (id, pattern) in &editor.song.patterns {
                    ui.selectable_value(
                        &mut draft.config.scope.pattern,
                        *id,
                        format!("P{} · {} · {} rows", id.0, pattern.name, pattern.rows),
                    );
                }
            });
        ui.label("Track");
        egui::ComboBox::from_id_salt("transform-track")
            .selected_text(format!("T{}", draft.config.scope.track.0))
            .width(150.0)
            .show_ui(ui, |ui| {
                for id in &editor.song.track_order {
                    let track = &editor.song.tracks[id];
                    ui.selectable_value(
                        &mut draft.config.scope.track,
                        *id,
                        format!(
                            "T{} · {} · {} columns",
                            id.0, track.name, track.note_columns
                        ),
                    );
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Row start");
        ui.add(egui::TextEdit::singleline(&mut draft.row_start).desired_width(50.0));
        ui.label("End exclusive");
        ui.add(egui::TextEdit::singleline(&mut draft.row_end).desired_width(55.0));
        if let Some(pattern) = editor.song.patterns.get(&draft.config.scope.pattern) {
            if ui.small_button("Whole pattern").clicked() {
                draft.row_start = "0".into();
                draft.row_end = pattern.rows.to_string();
            }
            ui.label(
                RichText::new(format!("/ {} rows", pattern.rows))
                    .small()
                    .color(MUTED),
            );
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Note columns");
        if let Some(track) = editor.song.tracks.get(&draft.config.scope.track) {
            for column in 0..track.note_columns {
                let mut selected = draft.config.scope.columns.contains(&column);
                if ui
                    .checkbox(&mut selected, format!("N{}", column + 1))
                    .changed()
                {
                    if selected {
                        draft.config.scope.columns.push(column);
                        draft.config.scope.columns.sort_unstable();
                    } else {
                        draft.config.scope.columns.retain(|value| *value != column);
                    }
                }
            }
        }
    });
    ui.label(
        RichText::new(format!(
            "Selected columns (zero-based): {:?}",
            draft.config.scope.columns
        ))
        .small()
        .color(MUTED),
    );
}
fn operation_fields(ui: &mut egui::Ui, draft: &mut Draft) {
    ui.label(RichText::new("FIELD MASK / OPERATION").strong());
    ui.horizontal(|ui| {
        ui.checkbox(&mut draft.config.fields.notes, "Notes");
        ui.checkbox(&mut draft.config.fields.glide_targets, "Glide targets");
        ui.add_enabled_ui(
            draft.config.fields.notes || draft.config.fields.glide_targets,
            |ui| {
                ui.label("Transpose");
                ui.add(egui::TextEdit::singleline(&mut draft.transpose).desired_width(50.0));
                ui.label("semitones (−48..48)");
            },
        );
    });
    ui.label(RichText::new("Pitched notes only, never kit selectors. Notes and absolute glide targets are independent masks; unproven voice context is refused.").small().color(MUTED));
    ui.horizontal(|ui| {
        ui.checkbox(&mut draft.config.fields.velocity, "Stored velocity");
        ui.add_enabled_ui(draft.config.fields.velocity, |ui| {
            ui.label("Scale ×");
            ui.add(egui::TextEdit::singleline(&mut draft.velocity_scale).desired_width(75.0));
            ui.label(
                RichText::new("0..4 · inherited stays unset · overflow refused")
                    .small()
                    .color(MUTED),
            );
        });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut draft.config.fields.timing, "Whole-event timing");
        ui.add_enabled_ui(draft.config.fields.timing, |ui| {
            ui.label("Nudge");
            ui.add(egui::TextEdit::singleline(&mut draft.nudge).desired_width(65.0));
            ui.label("output frames (±48,000)");
        });
    });
    ui.add_enabled_ui(draft.config.fields.timing, |ui| {
        ui.horizontal(|ui| {
            let mut swing = draft.config.swing.is_some();
            if ui.checkbox(&mut swing, "Swing odd bins").changed() {
                draft.config.swing = swing.then_some(TransformSwing {
                    division: Subdivision::Sixteenth,
                    amount_percent: 0,
                });
            }
            if let Some(swing) = &mut draft.config.swing {
                egui::ComboBox::from_id_salt("transform-swing-division")
                    .selected_text(format!("1/{}", swing.division.denominator()))
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for division in [
                            Subdivision::Sixteenth,
                            Subdivision::ThirtySecond,
                            Subdivision::SixtyFourth,
                            Subdivision::OneTwentyEighth,
                        ] {
                            ui.selectable_value(
                                &mut swing.division,
                                division,
                                format!("1/{}", division.denominator()),
                            );
                        }
                    });
                ui.add(egui::TextEdit::singleline(&mut draft.swing_amount).desired_width(45.0));
                ui.label("% (0..50)");
            }
        });
    });
    ui.label(RichText::new("Timing moves the complete cell with its commands; only within scope, no collisions/overtaking. Swing delays odd pattern-local bins by a percentage of bin duration, keeps off-grid phase, then applies nudge. Phrase-trigger timing is unsupported.").small().color(MUTED));
}
fn preview_details(ui: &mut egui::Ui, preview: &TransformPreview, reveal: bool) {
    let summary = preview.summary();
    ui.separator();
    let heading = ui.label(
        RichText::new(format!(
            "PREVIEW · {} changed / {} selected · {} moved",
            summary.changed_cells, summary.selected_cells, summary.moved_cells
        ))
        .strong()
        .color(AQUA),
    );
    if reveal && preview.changes().is_empty() {
        heading
            .scroll_to_me_animation(Some(egui::Align::TOP), egui::style::ScrollAnimation::none());
    }
    ui.label(RichText::new(format!("Notes {} · glide targets {} · stored velocity {} · timing {} · inherited velocity skipped {}", summary.note_changes, summary.glide_changes, summary.velocity_changes, summary.timing_changes, summary.skipped_unspecified_velocity)).small());
    let positions = if summary.affected_occurrences.len() <= 8 {
        format!("{:?}", summary.affected_occurrences)
    } else {
        format!("{} positions", summary.affected_occurrences.len())
    };
    ui.label(
        RichText::new(format!(
            "Affected POS {positions} · alias blocks {} · swing displacement {} output frames",
            summary.affected_aliases.len(),
            summary.swing_frames
        ))
        .small()
        .color(MUTED),
    )
    .on_hover_ui(|ui| {
        ui.set_max_width(380.0);
        ui.label("Exact shared destinations · zero-based POS (scroll all)");
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(format!(
                    "Occurrences ({})",
                    summary.affected_occurrences.len()
                ));
                egui::ScrollArea::vertical()
                    .id_salt("transform-affected-pos")
                    .max_height(120.0)
                    .show_rows(ui, 18.0, summary.affected_occurrences.len(), |ui, range| {
                        for index in range {
                            ui.monospace(format!("POS {:02}", summary.affected_occurrences[index]));
                        }
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.label(format!(
                    "Alias targets ({})",
                    summary.affected_aliases.len()
                ));
                egui::ScrollArea::vertical()
                    .id_salt("transform-affected-aliases")
                    .max_height(120.0)
                    .show_rows(ui, 18.0, summary.affected_aliases.len(), |ui, range| {
                        for index in range {
                            let alias = summary.affected_aliases[index];
                            ui.monospace(format!("P{} / T{}", alias.pattern.0, alias.track.0));
                        }
                    });
            });
        });
    });
    for warning in &summary.warnings {
        ui.colored_label(AMBER, warning);
    }
    ui.label(
        RichText::new(format!("Plan {}", preview.id()))
            .small()
            .color(MUTED),
    )
    .on_hover_text(format!("Effective configuration:\n{:#?}", preview.config()));
    let changes = preview.changes();
    if changes.is_empty() {
        ui.label("No stored cell changes; no commit or history entry needed.");
    } else {
        ui.label(
            RichText::new(
                "Exact cell changes · row/N · before -> after · hover for full stored fields",
            )
            .small()
            .color(MUTED),
        );
        let output = egui::ScrollArea::vertical()
            .id_salt("transform-diff")
            .max_height(150.0)
            .auto_shrink([false, true])
            .show_rows(ui, 20.0, changes.len(), |ui, range| {
                for index in range {
                    let change = &changes[index];
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{:03}->{:03} N{}  {} -> {}",
                                change.from_row,
                                change.to_row,
                                change.column + 1,
                                cell_summary(&change.before),
                                cell_summary(&change.after)
                            ))
                            .monospace(),
                        )
                        .truncate(),
                    )
                    .on_hover_text(format!(
                        "Source row {} / N{} -> row {} / N{}\nBEFORE {:?}\nAFTER {:?}",
                        change.from_row,
                        change.column + 1,
                        change.to_row,
                        change.column + 1,
                        change.before,
                        change.after
                    ));
                }
            });
        if reveal {
            ui.scroll_to_rect_animation(
                output.inner_rect,
                Some(egui::Align::BOTTOM),
                egui::style::ScrollAnimation::none(),
            );
        }
    }
}
fn operation_summary(preview: &TransformPreview) -> String {
    let config = preview.config();
    let pitch = |enabled: bool| {
        if enabled {
            format!("{:+} st", config.transpose_semitones)
        } else {
            "off".into()
        }
    };
    let velocity = if config.fields.velocity {
        format!("×{}", config.velocity_scale)
    } else {
        "off".into()
    };
    let timing = if config.fields.timing {
        format!("{:+} output frames", config.nudge_frames)
    } else {
        "off".into()
    };
    let swing = if config.fields.timing {
        config.swing.map_or_else(
            || "off".into(),
            |value| {
                format!(
                    "1/{} {}% / {} frames",
                    value.division.denominator(),
                    value.amount_percent,
                    preview.summary().swing_frames
                )
            },
        )
    } else {
        "off".into()
    };
    format!(
        "Notes {} · Glide {} · Velocity {velocity} · Nudge {timing} · Swing {swing}",
        pitch(config.fields.notes),
        pitch(config.fields.glide_targets)
    )
}
fn cell_summary(cell: &Cell) -> String {
    let note = match cell.note {
        Note::Empty => "---".into(),
        Note::Off => "OFF".into(),
        Note::Note(note) => note_label(Note::Note(note)),
    };
    let velocity = cell
        .velocity
        .map_or_else(|| "inherit".into(), |value| value.to_string());
    let delay = cell
        .delay
        .map_or_else(|| "unset".into(), |value| value.to_string());
    let glide = cell
        .effects
        .glide
        .as_ref()
        .map_or_else(|| "-".into(), |value| value.target.to_string());
    format!("{note} V{velocity} D{delay} G{glide}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_state_discloses_phase_and_rejects_unknown_actions() {
        let mut e = Editor::default();
        let mut ui = TransformUi::default();
        assert_eq!(ui.agent_state(&e)["phase"], "closed");
        assert!(ui
            .agent_action(&mut e, None, json!({"action":"unlisted"}))
            .is_err());
        ui.agent_action(&mut e, None, json!({"action":"open"}))
            .unwrap();
        assert!(ui.agent_state(&e)["open"].as_bool().unwrap());
        assert!(ui
            .agent_action(
                &mut e,
                None,
                json!({"action":"set_draft","values":{"unknown":true}})
            )
            .is_err());
        ui.agent_action(&mut e, None, json!({"action":"cancel"}))
            .unwrap();
        assert!(!ui.agent_state(&e)["open"].as_bool().unwrap());
    }
    #[test]
    fn canceled_preview_is_tracked_until_completion_and_never_reinstalled() {
        let editor = Editor::default();
        let mut ui = TransformUi::default();
        ui.open(&editor);
        let config = ui.draft.as_ref().unwrap().effective_config().unwrap();
        let preview = transforms::preview(&editor.song, config).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        ui.pending = Some(Pending {
            epoch: ui.epoch,
            revision: editor.revision(),
            receiver,
        });
        ui.reset_document();
        ui.open(&editor);
        let newer_epoch = ui.epoch;
        ui.start_preview(&editor);
        assert!(ui.busy());
        assert_ne!(ui.pending.as_ref().unwrap().epoch, newer_epoch);
        sender.send(Ok(preview)).unwrap();
        ui.poll();
        assert!(!ui.busy());
        assert!(ui.cached.is_none());
        assert!(ui.is_open());
    }
}
