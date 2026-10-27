//! Optional, source-addressed view of ordinary tracker cells. No audio engine,
//! scheduling state or persistent step sequence is owned by this UI.
use crate::{
    drum_steps::{self, StepDivision, StepEdit, StepEvent, StepProjection, StepSlot, StepView},
    editor::{Editor, note_label},
    song::{InstrumentId, Note, PadId, Retrigger},
    tracker_ui::{AQUA, grid_id},
};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Stroke};
use serde_json::{Value, json};
use std::path::Path;
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};
const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const AMBER: Color32 = Color32::from_rgb(229, 185, 112);
const LINE: Color32 = Color32::from_rgb(47, 62, 70);
const BACK: Color32 = Color32::from_rgb(17, 23, 28);
#[derive(Clone, PartialEq, Eq)]
struct Key {
    revision: u64,
    occurrence: usize,
    view: StepView,
}
struct Pending {
    epoch: u64,
    key: Key,
    receiver: Receiver<Result<StepProjection, String>>,
}
struct Cached {
    key: Key,
    value: Arc<StepProjection>,
}
#[derive(Clone)]
struct Draft {
    key: Key,
    event: StepEvent,
    velocity: Option<f32>,
    delay_enabled: bool,
    delay: String,
    retrigger: Option<Retrigger>,
    retrigger_spacing: String,
}
pub(crate) struct DrumStepUi {
    pub open: bool,
    division: StepDivision,
    steps: u8,
    page: u32,
    toggle_mode: bool,
    epoch: u64,
    pending: Option<Pending>,
    cache: Option<Cached>,
    failed: Option<(Key, String)>,
    context: Option<(
        usize,
        crate::song::PatternId,
        crate::song::TrackId,
        InstrumentId,
    )>,
    selected: Option<(PadId, u8)>,
    draft: Option<Draft>,
    pad_selection: Option<(InstrumentId, PadId)>,
    error: Option<String>,
}
/// Alias authority is exposed separately by `read_only`; it is not a rhythm conflict.
/// Match only the shared model's exact write-protection diagnostic, never arbitrary warnings.
fn content_conflict(slot: &StepSlot, authority: Option<&str>) -> bool {
    slot.events.len() > 1
        || slot
            .conflicts
            .iter()
            .any(|reason| Some(reason.as_str()) != authority)
}
impl Default for DrumStepUi {
    fn default() -> Self {
        Self {
            open: false,
            division: StepDivision::Sixteenth,
            steps: 16,
            page: 0,
            toggle_mode: false,
            epoch: 0,
            pending: None,
            cache: None,
            failed: None,
            context: None,
            selected: None,
            draft: None,
            pad_selection: None,
            error: None,
        }
    }
}
impl DrumStepUi {
    pub(crate) fn agent_state(&self, editor: &Editor) -> Value {
        let projection = self.cache.as_ref().map(|c| &c.value);
        let draft=self.draft.as_ref().map(|d|json!({"pad":d.event.pad.0,"row":d.event.row,"column":d.event.column,"instrument":d.event.instrument.0,"velocity":d.velocity,"delay_enabled":d.delay_enabled,"delay_text":d.delay,"retrigger":d.retrigger.as_ref().map(|r|json!({"repeats":r.repeats,"spacing":r.spacing})),"retrigger_spacing_text":d.retrigger_spacing}));
        json!({"component":"steps","open":self.open,"edit_mode":editor.edit_mode,"busy":self.busy(),
            "division":format!("{:?}",self.division),"steps":self.steps,"page":self.page,"toggle_mode":self.toggle_mode,
            "context":self.context.map(|(o,p,t,i)|json!({"occurrence":o,"pattern":p.0,"track":t.0,"instrument":i.0})),
            "selected":self.selected.map(|(p,s)|json!({"pad":p.0,"step":s})),"draft":draft,
            "projection":projection.map(|p|json!({"view":p.view,"source":p.source,"read_only":p.read_only,"total_steps":p.total_steps,"total_pages":p.total_pages,"lanes":p.lanes,"issues":p.issues})),
            "failed":self.failed.as_ref().map(|(_,error)|error),"error":self.error,"actions":{"configure":{"instrument_id":"integer","division":"Quarter|Eighth|Sixteenth|ThirtySecond|SixtyFourth|OneTwentyEighth","steps":"16|32","page":"integer","toggle_mode":"boolean"},"navigate":{"occurrence":"integer","track_id":"integer"},"close":{},"select_step":{"pad_id":"integer","step":"integer"},"select_event":{"pad_id":"integer","step":"integer","event_index":"integer"},"toggle_selected":{},"patch_draft":{"velocity":"number|null","delay_enabled":"boolean","delay_text":"string","retrigger_enabled":"boolean","repeats":"1..16","retrigger_spacing_text":"string"},"apply_velocity":{},"apply_delay":{},"apply_retrigger":{},"jump_to_event":{},"jump_issue":{"issue_index":"integer"},"clear_selection":{}},
            "policy":{"selection_does_not_edit_song":true,"edits_require_edit_mode_and_local_writable_track":true,"projection_is_bounded":true}})
    }
    pub(crate) fn agent_action(
        &mut self,
        editor: &mut Editor,
        _base: Option<&Path>,
        action: Value,
    ) -> Result<Value, String> {
        let o = action.as_object().ok_or("action must be an object")?;
        let name = o
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action name required")?;
        let fields: &[&str] = match name {
            "configure" => &[
                "action",
                "instrument_id",
                "division",
                "steps",
                "page",
                "toggle_mode",
            ],
            "navigate" => &["action", "occurrence", "track_id"],
            "close" => &["action"],
            "select_step" => &["action", "pad_id", "step"],
            "select_event" => &["action", "pad_id", "step", "event_index"],
            "jump_issue" => &["action", "issue_index"],
            "patch_draft" => &[
                "action",
                "velocity",
                "delay_enabled",
                "delay_text",
                "retrigger_enabled",
                "repeats",
                "retrigger_spacing_text",
            ],
            "toggle_selected" | "apply_velocity" | "apply_delay" | "apply_retrigger"
            | "jump_to_event" | "clear_selection" => &["action"],
            _ => return Err(format!("unknown steps action: {name}")),
        };
        if let Some(k) = o.keys().find(|k| !fields.contains(&k.as_str())) {
            return Err(format!("field {k} is not accepted for {name}"));
        }
        let allowed = [
            "action",
            "instrument_id",
            "division",
            "steps",
            "page",
            "toggle_mode",
            "occurrence",
            "track_id",
            "pad_id",
            "step",
            "event_index",
            "issue_index",
            "velocity",
            "delay_enabled",
            "delay_text",
            "retrigger_enabled",
            "repeats",
            "retrigger_spacing_text",
        ];
        if let Some(k) = o.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(format!("unknown action field: {k}"));
        }
        let n = |k: &str| {
            o.get(k)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{k} must be unsigned integer"))
        };
        match name {
            "close" => {
                self.open = false;
                self.draft = None;
            }
            "navigate" => {
                let (_, _, _, instrument) = self.context.ok_or("configure a projection first")?;
                let occurrence = n("occurrence")? as usize;
                let track = crate::song::TrackId(n("track_id")?);
                editor.select(occurrence, track)?;
                let view = StepView {
                    pattern: editor.pattern_id(),
                    track,
                    instrument,
                    division: self.division,
                    steps: self.steps,
                    page: self.page,
                };
                let projection = drum_steps::project(&editor.song, &view)?;
                let key = Key {
                    revision: editor.revision(),
                    occurrence,
                    view,
                };
                self.context = Some((occurrence, key.view.pattern, track, instrument));
                self.cache = Some(Cached {
                    key,
                    value: Arc::new(projection),
                });
                self.selected = None;
                self.draft = None;
            }
            "configure" => {
                let instrument = InstrumentId(n("instrument_id")?);
                let division = match o.get("division").and_then(Value::as_str) {
                    Some("Quarter") => StepDivision::Quarter,
                    Some("Eighth") => StepDivision::Eighth,
                    Some("Sixteenth") => StepDivision::Sixteenth,
                    Some("ThirtySecond") => StepDivision::ThirtySecond,
                    Some("SixtyFourth") => StepDivision::SixtyFourth,
                    Some("OneTwentyEighth") => StepDivision::OneTwentyEighth,
                    _ => return Err("invalid division".into()),
                };
                let steps = n("steps")?;
                if !matches!(steps, 16 | 32) {
                    return Err("steps must be 16 or 32".into());
                }
                let page = n("page")? as u32;
                let pattern = editor.pattern_id();
                let track = editor.track;
                let view = StepView {
                    pattern,
                    track,
                    instrument,
                    division,
                    steps: steps as u8,
                    page,
                };
                let projected = drum_steps::project(&editor.song, &view)?;
                self.division = division;
                self.steps = steps as u8;
                self.page = page;
                self.toggle_mode = o
                    .get("toggle_mode")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let key = Key {
                    revision: editor.revision(),
                    occurrence: editor.occurrence,
                    view,
                };
                self.context = Some((editor.occurrence, pattern, track, instrument));
                self.cache = Some(Cached {
                    key,
                    value: Arc::new(projected),
                });
                self.pending = None;
                self.failed = None;
                self.selected = None;
                self.draft = None;
                self.open = true;
            }
            "select_step" | "select_event" => {
                let pad = PadId(n("pad_id")?);
                let step = n("step")?;
                if step >= u64::from(self.steps) {
                    return Err("step outside current page".into());
                }
                let (occ, pattern, track, instrument) =
                    self.context.ok_or("configure a projection first")?;
                let view = StepView {
                    pattern,
                    track,
                    instrument,
                    division: self.division,
                    steps: self.steps,
                    page: self.page,
                };
                let proj = drum_steps::project(&editor.song, &view)?;
                let slot = proj
                    .lanes
                    .iter()
                    .find(|l| l.pad == pad)
                    .and_then(|l| l.steps.get(step as usize))
                    .ok_or("unknown pad or step")?;
                self.selected = Some((pad, step as u8));
                self.draft = None;
                if name == "select_event" {
                    let i = n("event_index")? as usize;
                    let event = slot.events.get(i).ok_or("event index out of range")?;
                    self.select_event(
                        editor,
                        &Key {
                            revision: editor.revision(),
                            occurrence: occ,
                            view: view.clone(),
                        },
                        event,
                    );
                }
                self.cache = Some(Cached {
                    key: Key {
                        revision: editor.revision(),
                        occurrence: occ,
                        view,
                    },
                    value: Arc::new(proj),
                });
            }
            "toggle_selected" => {
                let (occ, pattern, track, instrument) =
                    self.context.ok_or("configure a projection first")?;
                let (pad, step) = self.selected.ok_or("select a step first")?;
                let view = StepView {
                    pattern,
                    track,
                    instrument,
                    division: self.division,
                    steps: self.steps,
                    page: self.page,
                };
                let projection = drum_steps::project(&editor.song, &view)?;
                let key = Key {
                    revision: editor.revision(),
                    occurrence: occ,
                    view,
                };
                self.edit(editor, &key, &projection, StepEdit::Toggle { pad, step });
                if let Some(e) = &self.error {
                    return Err(e.clone());
                }
                let refreshed = Arc::new(drum_steps::project(&editor.song, &key.view)?);
                self.cache = Some(Cached {
                    key,
                    value: refreshed,
                });
            }
            "patch_draft" => {
                let mut d = self
                    .draft
                    .as_ref()
                    .cloned()
                    .ok_or("select an exact event first")?;
                for k in o.keys() {
                    if !allowed.contains(&k.as_str()) {
                        return Err(format!("unknown draft field: {k}"));
                    }
                }
                if let Some(v) = o.get("velocity") {
                    d.velocity = if v.is_null() {
                        None
                    } else {
                        Some(
                            v.as_f64()
                                .filter(|x| (0.0..=1.0).contains(x))
                                .ok_or("velocity must be 0..1 or null")?
                                as f32,
                        )
                    }
                }
                if let Some(v) = o.get("delay_enabled") {
                    d.delay_enabled = v.as_bool().ok_or("delay_enabled must be boolean")?
                }
                if let Some(v) = o.get("delay_text") {
                    d.delay = v.as_str().ok_or("delay_text must be string")?.into()
                }
                if let Some(v) = o.get("retrigger_enabled") {
                    let yes = v.as_bool().ok_or("retrigger_enabled must be boolean")?;
                    d.retrigger = if yes {
                        Some(d.retrigger.clone().unwrap_or(Retrigger {
                            repeats: 1,
                            spacing: 0.25,
                        }))
                    } else {
                        None
                    }
                }
                if let Some(v) = o.get("repeats") {
                    let r = v
                        .as_u64()
                        .filter(|x| (1..=16).contains(x))
                        .ok_or("repeats must be 1..16")? as u8;
                    d.retrigger = Some(Retrigger {
                        repeats: r,
                        spacing: d.retrigger.as_ref().map_or(0.25, |v| v.spacing),
                    })
                }
                if let Some(v) = o.get("retrigger_spacing_text") {
                    d.retrigger_spacing = v.as_str().ok_or("spacing text must be string")?.into()
                }
                self.draft = Some(d);
            }
            "apply_velocity" | "apply_delay" | "apply_retrigger" => {
                let (occ, _, _, _) = self.context.ok_or("configure a projection first")?;
                let d = self.draft.take().ok_or("select an exact event first")?;
                let view = d.key.view.clone();
                if d.key.revision != editor.revision() {
                    self.draft = Some(d);
                    return Err("event draft is stale".into());
                }
                let projection = drum_steps::project(&editor.song, &view)?;
                let edit = match name {
                    "apply_velocity" => StepEdit::SetVelocity {
                        event: d.event.clone(),
                        value: d.velocity,
                    },
                    "apply_delay" => {
                        let value = if d.delay_enabled {
                            Some(
                                d.delay
                                    .trim()
                                    .parse::<f64>()
                                    .ok()
                                    .filter(|v| v.is_finite() && *v >= 0.0 && *v < 1.0)
                                    .ok_or("delay must be finite and 0 <= value < 1")?,
                            )
                        } else {
                            None
                        };
                        StepEdit::SetDelay {
                            event: d.event.clone(),
                            value,
                        }
                    }
                    _ => {
                        let value = d
                            .retrigger
                            .as_ref()
                            .map(|r| {
                                let spacing = d
                                    .retrigger_spacing
                                    .trim()
                                    .parse::<f64>()
                                    .ok()
                                    .filter(|v| v.is_finite() && *v > 0.0)
                                    .ok_or("spacing must be finite and positive")?;
                                Ok::<_, String>(Retrigger {
                                    repeats: r.repeats,
                                    spacing,
                                })
                            })
                            .transpose()?;
                        StepEdit::SetRetrigger {
                            event: d.event.clone(),
                            value,
                        }
                    }
                };
                self.edit(editor, &d.key, &projection, edit);
                if let Some(e) = &self.error {
                    self.draft = Some(d);
                    return Err(e.clone());
                }
                let key = Key {
                    revision: editor.revision(),
                    occurrence: occ,
                    view,
                };
                let refreshed = Arc::new(drum_steps::project(&editor.song, &key.view)?);
                self.cache = Some(Cached {
                    key,
                    value: refreshed,
                });
            }
            "jump_to_event" => {
                let d = self.draft.as_ref().ok_or("select an exact event first")?;
                editor.row = d.event.row;
                editor.column = d.event.column;
                self.open = false;
            }
            "jump_issue" => {
                let (_, _, _, instrument) = self.context.ok_or("configure a projection first")?;
                let issue_index = n("issue_index")? as usize;
                let view = StepView {
                    pattern: editor.pattern_id(),
                    track: editor.track,
                    instrument,
                    division: self.division,
                    steps: self.steps,
                    page: self.page,
                };
                let projection = drum_steps::project(&editor.song, &view)?;
                let issue = projection
                    .issues
                    .get(issue_index)
                    .ok_or("issue index out of range")?;
                editor.row = issue.row;
                editor.column = issue.column;
                self.open = false;
            }
            "clear_selection" => {
                self.selected = None;
                self.draft = None;
            }
            _ => return Err(format!("unknown steps action: {name}")),
        }
        Ok(self.agent_state(editor))
    }
    #[cfg(feature = "ui-capture")]
    pub(crate) fn capture_projection(&self, editor: &Editor) -> Option<&StepProjection> {
        self.cache
            .as_ref()
            .filter(|cache| {
                self.open
                    && editor.has_grid()
                    && cache.key.revision == editor.revision()
                    && cache.key.occurrence == editor.occurrence
                    && cache.key.view.pattern == editor.pattern_id()
                    && cache.key.view.track == editor.track
                    && cache.value.is_current(&editor.song)
            })
            .map(|cache| cache.value.as_ref())
    }
    pub(crate) fn take_pad_selection(&mut self) -> Option<(InstrumentId, PadId)> {
        self.pad_selection.take()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn reset_document(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.open = false;
        self.cache = None;
        self.failed = None;
        self.context = None;
        self.selected = None;
        self.draft = None;
        self.pad_selection = None;
        self.error = None;
        self.page = 0;
        self.toggle_mode = false;
    }
    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let value = match pending.receiver.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Step projection worker disconnected".into()),
        };
        let pending = self.pending.take().unwrap();
        if pending.epoch != self.epoch {
            return;
        }
        match value {
            Ok(value) => {
                self.cache = Some(Cached {
                    key: pending.key,
                    value: Arc::new(value),
                })
            }
            Err(error) => self.failed = Some((pending.key, error)),
        }
    }
    fn ensure(&mut self, editor: &Editor, key: &Key) {
        if self.cache.as_ref().is_some_and(|cache| cache.key != *key) {
            self.cache = None;
        }
        if self.draft.as_ref().is_some_and(|draft| draft.key != *key) {
            self.draft = None;
        }
        if self
            .failed
            .as_ref()
            .is_some_and(|(failed, _)| failed != key)
        {
            self.failed = None;
        }
        if self.pending.is_some() || self.cache.is_some() || self.failed.is_some() {
            return;
        }
        let song = editor.song.clone();
        let view = key.view.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-step-projection".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    drum_steps::project(&song, &view)
                }))
                .unwrap_or_else(|_| {
                    Err("Step projection worker panicked; no document changed".into())
                });
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    key: key.clone(),
                    receiver,
                })
            }
            Err(error) => self.failed = Some((key.clone(), error.to_string())),
        }
    }
    fn edit(
        &mut self,
        editor: &mut Editor,
        key: &Key,
        projection: &StepProjection,
        edit: StepEdit,
    ) {
        self.error = editor.edit_drum_step(key.revision, projection, &edit).err();
        if self.error.is_none() {
            self.cache = None;
            self.draft = None;
        }
    }
    fn select_event(&mut self, editor: &mut Editor, key: &Key, event: &StepEvent) {
        self.pad_selection = Some((event.instrument, event.pad));
        editor.row = event.row;
        editor.column = event.column;
        self.draft = Some(Draft {
            key: key.clone(),
            event: event.clone(),
            velocity: event.cell.velocity,
            delay_enabled: event.cell.delay.is_some(),
            delay: event.cell.delay.unwrap_or(0.0).to_string(),
            retrigger: event.cell.effects.retrigger.clone(),
            retrigger_spacing: event
                .cell
                .effects
                .retrigger
                .as_ref()
                .map_or_else(|| "0.25".into(), |value| value.spacing.to_string()),
        });
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        instrument: Option<InstrumentId>,
        viewport_height: f32,
    ) {
        self.poll();
        ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        let writable =
            ui.is_enabled() && editor.edit_mode && !crate::tracker_ui::cell_is_read_only(editor);
        if !writable {
            self.toggle_mode = false;
        }
        let Some(instrument) = instrument else {
            ui.label(
                RichText::new("DRUM STEPS / NO INSTRUMENT")
                    .strong()
                    .color(AQUA),
            );
            ui.label("Choose an explicit drum-kit instrument in the pad bank. This view never guesses incoming selection.");
            if ui.button("< Tracker").clicked() {
                self.open = false;
            }
            return;
        };
        let context = (
            editor.occurrence,
            editor.pattern_id(),
            editor.track,
            instrument,
        );
        if self.context != Some(context) {
            self.context = Some(context);
            self.page = 0;
            self.selected = None;
            self.draft = None;
            self.error = None;
        }
        let uses = editor
            .song
            .order
            .iter()
            .filter(|order| order.pattern == context.1)
            .count();
        ui.horizontal_wrapped(|ui| {
            if ui.button("< Tracker").clicked() {
                self.open = false;
                self.draft = None;
            }
            ui.label(RichText::new("DRUM STEPS").strong().color(AQUA));
            ui.monospace(format!(
                "I{} · P{} shared {uses}×",
                instrument.0, context.1.0
            ));
            egui::ComboBox::from_id_salt("step-position")
                .selected_text(format!("Position {}", editor.occurrence + 1))
                .show_ui(ui, |ui| {
                    let positions: Vec<_> = editor
                        .song
                        .order
                        .iter()
                        .enumerate()
                        .map(|(i, o)| (i, o.pattern))
                        .collect();
                    for (i, pattern) in positions {
                        if ui
                            .selectable_label(
                                editor.occurrence == i,
                                format!("{} / P{}", i + 1, pattern.0),
                            )
                            .clicked()
                        {
                            self.error = editor.select(i, editor.track).err();
                        }
                    }
                });
            egui::ComboBox::from_id_salt("step-track")
                .selected_text(format!("Track {}", editor.track.0))
                .show_ui(ui, |ui| {
                    for track in editor.song.track_order.clone() {
                        if ui
                            .selectable_label(editor.track == track, format!("Track {}", track.0))
                            .clicked()
                        {
                            self.error = editor.select(editor.occurrence, track).err();
                        }
                    }
                });
        });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("step-length")
                .selected_text(format!("{} steps", self.steps))
                .show_ui(ui, |ui| {
                    for count in [16, 32] {
                        if ui
                            .selectable_value(&mut self.steps, count, format!("{count} steps"))
                            .changed()
                        {
                            self.page = 0;
                        }
                    }
                });
            egui::ComboBox::from_id_salt("step-division")
                .selected_text(division_label(self.division))
                .show_ui(ui, |ui| {
                    for division in [
                        StepDivision::Quarter,
                        StepDivision::Eighth,
                        StepDivision::Sixteenth,
                        StepDivision::ThirtySecond,
                        StepDivision::SixtyFourth,
                        StepDivision::OneTwentyEighth,
                    ] {
                        if ui
                            .selectable_value(
                                &mut self.division,
                                division,
                                division_label(division),
                            )
                            .changed()
                        {
                            self.page = 0;
                        }
                    }
                });
            let pages = (editor.pattern().rows * self.division.steps_per_beat())
                .div_ceil(u32::from(editor.song.lines_per_beat))
                .div_ceil(u32::from(self.steps));
            if ui
                .add_enabled(self.page > 0, egui::Button::new("<"))
                .clicked()
            {
                self.page -= 1;
            }
            let mut page = self.page.saturating_add(1);
            ui.label("Page");
            if ui
                .add(egui::DragValue::new(&mut page).range(1..=pages.max(1)))
                .changed()
            {
                self.page = page - 1;
            }
            ui.label(format!("/ {pages}"));
            if ui
                .add_enabled(self.page + 1 < pages, egui::Button::new(">"))
                .clicked()
            {
                self.page += 1;
            }
            ui.selectable_value(&mut self.toggle_mode, false, "Select");
            ui.add_enabled_ui(writable, |ui| {
                ui.selectable_value(&mut self.toggle_mode, true, "Toggle");
            });
        });
        if (
            editor.occurrence,
            editor.pattern_id(),
            editor.track,
            instrument,
        ) != context
        {
            ui.ctx().request_repaint();
            return;
        }
        let key = Key {
            revision: editor.revision(),
            occurrence: editor.occurrence,
            view: StepView {
                pattern: editor.pattern_id(),
                track: editor.track,
                instrument,
                division: self.division,
                steps: self.steps,
                page: self.page,
            },
        };
        self.ensure(editor, &key);
        ui.label(
            RichText::new(format!(
                "Page origin = absolute step {} · {} LPB · no quantizing / no audition",
                self.page * u32::from(self.steps),
                editor.song.lines_per_beat
            ))
            .small()
            .color(MUTED),
        );
        if let Some((_, error)) = &self.failed {
            ui.colored_label(AMBER, error);
            if ui.button("Reset page / retry").clicked() {
                self.page = 0;
                self.failed = None;
            }
            return;
        }
        let Some(projection) = self
            .cache
            .as_ref()
            .filter(|cache| cache.key == key)
            .map(|cache| Arc::clone(&cache.value))
        else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Projecting ordinary cells on worker… edits disabled until current");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(20));
            return;
        };
        if projection.read_only {
            ui.label(RichText::new(format!("ALIAS / READ ONLY · P{}/T{} displays P{}/T{}. Use Edit source or Make unique above.",key.view.pattern.0,key.view.track.0,projection.source.pattern.0,projection.source.track.0)).small().color(AMBER));
        }
        if editor.song.order[editor.occurrence]
            .muted_tracks
            .contains(&editor.track)
        {
            ui.label(
                RichText::new(
                    "X · muted in this occurrence only; these pattern cells remain shared",
                )
                .small()
                .color(AMBER),
            );
        }
        if projection.lanes.is_empty() {
            ui.label("No pads in this kit. Create mappings in the pad bank first.");
            return;
        }
        let authority_reason = if projection.read_only {
            editor
                .song
                .require_local_track(key.view.pattern, key.view.track)
                .err()
        } else {
            None
        };
        let mut chosen = None;
        let mut toggle = None;
        let grid_height = (viewport_height - 160.0).clamp(120.0, 410.0);
        egui::ScrollArea::both().id_salt("drum-step-grid").max_height(grid_height).auto_shrink([false,false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show_viewport(ui,|ui,viewport|{
            let label_width=116.0;let step_width=29.0;let row_height=25.0;
            let (rect,_)=ui.allocate_exact_size(egui::vec2(label_width+step_width*f32::from(self.steps),row_height*(projection.lanes.len()+1) as f32),egui::Sense::hover());
            let screen=egui::Rect::from_min_size(rect.min+viewport.min.to_vec2(),viewport.size());
            let body=egui::Rect::from_min_max(screen.min+egui::vec2(label_width,row_height),screen.max).intersect(ui.clip_rect());
            let gutter=egui::Rect::from_min_max(screen.min+egui::vec2(0.0,row_height),egui::pos2(screen.left()+label_width,screen.bottom())).intersect(ui.clip_rect());
            ui.painter().rect_filled(rect,0.0,BACK);
            let painter=ui.painter().with_clip_rect(body);
            for (row,lane) in projection.lanes.iter().enumerate(){let y=rect.top()+row_height*(row+1) as f32;
                let label=egui::Rect::from_min_size(egui::pos2(screen.left(),y),egui::vec2(label_width-4.0,row_height));
                ui.painter().with_clip_rect(label.intersect(gutter)).text(label.left_center()+egui::vec2(3.0,0.0),Align2::LEFT_CENTER,format!("{:02} {} {}",lane.slot+1,note_label(Note::Note(lane.note)),lane.name),FontId::monospace(11.0),if lane.mapped{AQUA}else{MUTED});
                for slot in &lane.steps {let x=rect.left()+label_width+f32::from(slot.step)*step_width;let cell=egui::Rect::from_min_size(egui::pos2(x+1.0,y+2.0),egui::vec2(step_width-3.0,row_height-4.0));
                    let conflict=content_conflict(slot,authority_reason.as_deref());let filled=!slot.events.is_empty();let selected=self.selected==Some((lane.pad,slot.step));
                    let color=if !slot.in_bounds||!lane.mapped{Color32::from_rgb(26,33,38)}else if conflict{Color32::from_rgb(85,63,37)}else if filled{Color32::from_rgb(41,106,102)}else{Color32::from_rgb(30,43,50)};
                    painter.rect_filled(cell,0.0,color);painter.rect_stroke(cell,0.0,Stroke::new(1.0_f32,if selected{AQUA}else{LINE}),egui::StrokeKind::Inside);
                    if conflict {painter.text(cell.center(),Align2::CENTER_CENTER,if slot.events.len()>1{slot.events.len().to_string()}else{"!".into()},FontId::monospace(12.0),AMBER);}else if filled&&slot.events[0].cell.effects.retrigger.is_some(){painter.text(cell.center(),Align2::CENTER_CENTER,"R",FontId::monospace(12.0),AQUA);}
                    if u32::from(slot.step)%self.division.steps_per_beat()==0 {painter.vline(x,y..=y+row_height,Stroke::new(1.0_f32,MUTED));}
                    let response=ui.interact(cell.intersect(body),ui.id().with(("step-hit",lane.pad.0,slot.step)),egui::Sense::click());
                    let hint=format!("Pad {} · local step {} · absolute step {}\nAnchor row {} + {:?}\n{} exact source event(s)\n{}",lane.pad.0,slot.step+1,self.page*u32::from(self.steps)+u32::from(slot.step),slot.row,slot.delay,slot.events.len(),slot.conflicts.join("\n"));
                    if response.on_hover_text(hint).clicked(){chosen=Some((lane.pad,slot.step));if writable&&self.toggle_mode&&slot.in_bounds&&lane.mapped&&slot.conflicts.is_empty(){toggle=Some(StepEdit::Toggle {pad:lane.pad,step:slot.step});}}
                }
            }
            let header=egui::Rect::from_min_size(screen.min,egui::vec2(screen.width(),row_height));
            ui.painter().rect_filled(header,0.0,BACK);
            ui.painter().text(screen.min+egui::vec2(3.0,12.0),Align2::LEFT_CENTER,"PAD / SELECTOR",FontId::monospace(10.0),MUTED);
            let header_painter=ui.painter().with_clip_rect(egui::Rect::from_min_max(screen.min+egui::vec2(label_width,0.0),header.max).intersect(ui.clip_rect()));
            for step in 0..self.steps {let x=rect.left()+label_width+f32::from(step)*step_width;header_painter.text(egui::pos2(x+step_width*0.5,screen.top()+12.0),Align2::CENTER_CENTER,format!("{:02}",step+1),FontId::monospace(11.0),MUTED);}
        });
        if let Some(selected) = chosen {
            self.selected = Some(selected);
            self.draft = None;
            if let Some(event) = projection
                .lanes
                .iter()
                .find(|lane| lane.pad == selected.0)
                .and_then(|lane| lane.steps.get(usize::from(selected.1)))
                .and_then(|slot| (slot.events.len() == 1).then(|| &slot.events[0]))
            {
                self.select_event(editor, &key, event);
            }
        }
        if let Some(edit) = toggle {
            self.edit(editor, &key, &projection, edit);
            ui.ctx().request_repaint();
            return;
        }
        let off_grid = projection
            .lanes
            .iter()
            .flat_map(|lane| &lane.steps)
            .flat_map(|slot| &slot.events)
            .filter(|event| !event.on_grid)
            .count();
        let conflicts = projection
            .lanes
            .iter()
            .flat_map(|lane| &lane.steps)
            .filter(|slot| content_conflict(slot, authority_reason.as_deref()))
            .count();
        let conflict_label = if projection.read_only {
            "source conflict bins"
        } else {
            "blocked bins"
        };
        ui.label(RichText::new(format!("{off_grid} off-grid hits · {conflicts} {conflict_label} · {} preserved issues · R = one retrigger parent",projection.issues.len())).small().color(if off_grid+conflicts+projection.issues.len()>0{AMBER}else{MUTED}));
        ui.label(RichText::new("Toggle removes note + retrigger only; selection/scalars remain. Conflicts require exact selection, never snapping/deleting a whole bin.").small().color(MUTED));
        if let Some((pad, step)) = self.selected {
            if let Some(slot) = projection
                .lanes
                .iter()
                .find(|lane| lane.pad == pad)
                .and_then(|lane| lane.steps.get(usize::from(step)))
            {
                for reason in slot
                    .conflicts
                    .iter()
                    .filter(|reason| Some(reason.as_str()) != authority_reason.as_deref())
                {
                    ui.label(RichText::new(reason).small().color(AMBER));
                }
                if slot.events.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        for event in &slot.events {
                            if ui
                                .button(format!("Select row {} / N{}", event.row, event.column + 1))
                                .clicked()
                            {
                                self.select_event(editor, &key, event);
                            }
                        }
                    });
                }
            }
        }
        self.inspector(ui, editor, &key, &projection, writable);
        egui::CollapsingHeader::new(format!("Unrepresented source cells ({})",projection.issues.len())).id_salt("step-issues").show(ui,|ui|{
            egui::ScrollArea::vertical().max_height(100.0).show_rows(ui,24.0,projection.issues.len(),|ui,range|{for index in range {let issue=&projection.issues[index];ui.horizontal(|ui|{if ui.small_button(format!("Row {} / N{}",issue.row,issue.column+1)).clicked(){editor.row=issue.row;editor.column=issue.column;self.open=false;}ui.add(egui::Label::new(RichText::new(&issue.reason).small().color(AMBER)).truncate()).on_hover_text(format!("{}\nNote {} · velocity {:?} · delay {:?} · retrigger {:?}\nJump selects the exact tracker cell; no content changes",issue.reason,note_label(issue.cell.note),issue.cell.velocity,issue.cell.delay,issue.cell.effects.retrigger));});}});
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
    fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        key: &Key,
        projection: &StepProjection,
        writable: bool,
    ) {
        let Some(mut draft) = self.draft.take() else {
            ui.label(
                RichText::new(
                    "Select one exact source event to inspect velocity, row delay and retrigger.",
                )
                .small()
                .color(MUTED),
            );
            return;
        };
        if draft.key != *key {
            return;
        }
        let mut action = None;
        let mut jump = false;
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!(
                "P{} / T{} / row {} / N{} · {}",
                key.view.pattern.0,
                key.view.track.0,
                draft.event.row,
                draft.event.column + 1,
                if draft.event.on_grid {
                    "ON GRID"
                } else {
                    "OFF GRID — exact source"
                }
            ));
            if ui.button("Show tracker cell").clicked() {
                jump = true;
            }
        });
        if projection.read_only {
            ui.label(
                RichText::new(format!(
                    "Resolved source P{} / T{} · inspection only",
                    projection.source.pattern.0, projection.source.track.0
                ))
                .small()
                .color(AMBER),
            );
        }
        ui.add_enabled_ui(writable && !projection.read_only, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut enabled = draft.velocity.is_some();
                if ui.checkbox(&mut enabled, "Velocity").changed() {
                    draft.velocity = enabled.then_some(1.0);
                }
                if let Some(value) = &mut draft.velocity {
                    ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01));
                } else {
                    ui.label(RichText::new("inherit memory").small().color(MUTED));
                }
                if ui.button("Apply velocity").clicked() {
                    action = Some(StepEdit::SetVelocity {
                        event: draft.event.clone(),
                        value: draft.velocity,
                    });
                }
                ui.checkbox(&mut draft.delay_enabled, "Row delay");
                ui.add_enabled(
                    draft.delay_enabled,
                    egui::TextEdit::singleline(&mut draft.delay).desired_width(90.0),
                )
                .on_hover_text(format!(
                    "Exact stored row fraction: {:?}; 0 <= value < 1, not a step fraction",
                    draft.event.cell.delay
                ));
                if !draft.delay_enabled { ui.label(RichText::new("unset / default 0").small().color(MUTED)); }
                if ui.button("Apply delay").clicked() {
                    match if draft.delay_enabled {
                        draft
                            .delay
                            .parse::<f64>()
                            .map(Some)
                            .map_err(|_| "Enter a decimal row fraction".to_string())
                    } else {
                        Ok(None)
                    } {
                        Ok(value) => {
                            action = Some(StepEdit::SetDelay {
                                event: draft.event.clone(),
                                value,
                            })
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                let mut enabled = draft.retrigger.is_some();
                if ui.checkbox(&mut enabled, "Retrigger").changed() {
                    draft.retrigger = enabled.then_some(Retrigger {
                        repeats: 1,
                        spacing: 0.25,
                    });
                }
                if let Some(value) = &mut draft.retrigger {
                    ui.add(egui::DragValue::new(&mut value.repeats).range(1..=16));
                    ui.label("extra starts, spacing");
                    ui.add(egui::TextEdit::singleline(&mut draft.retrigger_spacing).desired_width(90.0)).on_hover_text(format!("Exact stored spacing: {:?}; finite positive row fraction, no display rounding", draft.event.cell.effects.retrigger.as_ref().map(|v| v.spacing)));
                    ui.label("row fraction");
                }
                if ui.button("Apply retrigger").clicked() {
                    match draft.retrigger.as_ref().map(|value| draft.retrigger_spacing.parse::<f64>().map(|spacing| Retrigger { repeats: value.repeats, spacing })).transpose() {
                        Ok(value) => action = Some(StepEdit::SetRetrigger { event: draft.event.clone(), value }),
                        Err(_) => self.error = Some("Enter an exact decimal retrigger spacing".into()),
                    }
                }
            });
        });
        ui.label(RichText::new("Exact field edits preserve other fields/commands. Delay + repeats × spacing must remain <1 row; no generated child hit is independently edited.").small().color(MUTED));
        if jump {
            editor.row = draft.event.row;
            editor.column = draft.event.column;
            self.open = false;
        } else if let Some(action) = action {
            self.edit(editor, key, projection, action);
            ui.ctx().request_repaint();
        } else {
            self.draft = Some(draft);
        }
    }
}
fn division_label(division: StepDivision) -> &'static str {
    match division {
        StepDivision::Quarter => "1/4",
        StepDivision::Eighth => "1/8",
        StepDivision::Sixteenth => "1/16",
        StepDivision::ThirtySecond => "1/32",
        StepDivision::SixtyFourth => "1/64",
        StepDivision::OneTwentyEighth => "1/128",
    }
}

#[cfg(test)]
mod agent_api_tests {
    use super::*;

    #[test]
    fn agent_state_advertises_actions_and_rejects_missing_projection_without_edit() {
        let mut editor = Editor::default();
        let before = editor.song.clone();
        let mut ui = DrumStepUi::default();
        let state = ui.agent_state(&editor);
        assert!(state["actions"]["configure"].is_object());
        assert!(
            ui.agent_action(&mut editor, None, json!({"action":"toggle_selected"}))
                .is_err()
        );
        assert_eq!(editor.song, before);
    }
}
