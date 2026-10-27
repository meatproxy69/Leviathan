//! Timing-only capture and transfer. Ordinary Song cells and shared preview plans are authoritative.
use crate::{
    break_compiler::Subdivision,
    editor::{Editor, Field},
    groove::{self, ApplyConfig, CaptureConfig, CapturePreview, GroovePreview},
    song::{GrooveId, MarkerSetId, PatternId, TrackId},
    tracker_ui::{grid_id, AQUA},
    transforms::TransformScope,
};
use eframe::egui::{self, Color32, RichText};
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Capture,
    Apply,
}
#[derive(Clone, PartialEq)]
struct Draft {
    context: Option<Context>,
    revision: u64,
    tab: Tab,
    marker_set: Option<MarkerSetId>,
    name: String,
    start_marker: String,
    end_marker: String,
    include_start: bool,
    division: Subdivision,
    cycle: String,
    groove: Option<GrooveId>,
    pattern: PatternId,
    track: TrackId,
    columns: Vec<u16>,
    row_start: String,
    row_end: String,
    strength: String,
}
fn integer(text: &str, min: u64, max: u64, label: &str) -> Result<u64, String> {
    let value = text.parse::<u64>().map_err(|_| {
        format!("{label} must be an integer in {min}..{max}; no rounding or clamping")
    })?;
    if !(min..=max).contains(&value) {
        return Err(format!("{label} must be in {min}..{max}; no clamping"));
    }
    Ok(value)
}
impl Draft {
    fn capture_config(&self) -> Result<CaptureConfig, String> {
        Ok(CaptureConfig {
            version: groove::VERSION,
            name: self.name.clone(),
            marker_set: self.marker_set.ok_or("Choose an accepted marker set")?,
            start_marker: integer(&self.start_marker, 0, 256, "Start marker index")? as u16,
            end_marker: integer(&self.end_marker, 0, 256, "End marker index")? as u16,
            include_start_anchor: self.include_start,
            division: self.division,
            cycle_steps: integer(&self.cycle, 1, 1024, "Cycle steps")? as u16,
        })
    }
    fn apply_config(&self) -> Result<ApplyConfig, String> {
        Ok(ApplyConfig {
            groove: self.groove.ok_or("Choose a saved timing template")?,
            scope: TransformScope {
                pattern: self.pattern,
                track: self.track,
                row_start: integer(&self.row_start, 0, u32::MAX as u64, "Row start")? as u32,
                row_end: integer(&self.row_end, 0, u32::MAX as u64, "End exclusive")? as u32,
                columns: self.columns.clone(),
            },
            strength_percent: integer(&self.strength, 0, 100, "Strength percent")? as u8,
        })
    }
}
enum Plan {
    Capture(Box<CapturePreview>),
    Apply(Box<GroovePreview>),
}
struct Pending {
    epoch: u64,
    revision: u64,
    receiver: Receiver<Result<Plan, String>>,
}
struct Cached {
    revision: u64,
    plan: Plan,
    reveal: bool,
}
#[derive(Default)]
pub(crate) struct GrooveUi {
    draft: Option<Draft>,
    pending: Option<Pending>,
    cached: Option<Cached>,
    epoch: u64,
    error: Option<String>,
    notice: Option<String>,
}
impl GrooveUi {
    pub(crate) fn open(&mut self, editor: &Editor) {
        self.reset_document();
        self.draft = Some(Draft {
            context: Context::of(editor),
            revision: editor.revision(),
            tab: if editor.song.grooves.is_empty() {
                Tab::Capture
            } else {
                Tab::Apply
            },
            marker_set: None,
            name: "Captured timing".into(),
            start_marker: "0".into(),
            end_marker: "1".into(),
            include_start: false,
            division: Subdivision::Sixteenth,
            cycle: "16".into(),
            groove: None,
            pattern: if editor.has_grid() {
                editor.pattern_id()
            } else {
                PatternId(0)
            },
            track: editor.track,
            columns: vec![editor.column],
            row_start: editor.row.to_string(),
            row_end: editor.row.saturating_add(1).to_string(),
            strength: "100".into(),
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
    }
    fn invalidate(&mut self, notice: &str) {
        self.epoch = self.epoch.wrapping_add(1);
        self.cached = None;
        self.error = None;
        self.notice = Some(notice.into());
    }
    pub(crate) fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Timing worker disconnected; no document changed".into())
            }
        };
        let pending = self.pending.take().unwrap();
        if pending.epoch != self.epoch || self.draft.is_none() {
            return;
        }
        match result {
            Ok(plan) => {
                self.cached = Some(Cached {
                    revision: pending.revision,
                    plan,
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
        let Some(draft) = &self.draft else { return };
        enum Recipe {
            Capture(GrooveId, CaptureConfig),
            Apply(ApplyConfig),
        }
        let recipe = match draft.tab {
            Tab::Capture => draft.capture_config().and_then(|config| {
                editor
                    .song
                    .next_groove_id()
                    .map(|id| Recipe::Capture(id, config))
            }),
            Tab::Apply => draft.apply_config().map(Recipe::Apply),
        };
        let recipe = match recipe {
            Ok(recipe) => recipe,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let song = editor.song.clone();
        let revision = editor.revision();
        self.cached = None;
        self.error = None;
        self.notice = None;
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-groove-preview".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match recipe {
                        Recipe::Capture(id, config) => groove::capture_preview(&song, id, config)
                            .map(|p| Plan::Capture(Box::new(p))),
                        Recipe::Apply(config) => {
                            groove::preview(&song, config).map(|p| Plan::Apply(Box::new(p)))
                        }
                    }))
                    .unwrap_or_else(|_| Err("Timing worker panicked; no document changed".into()));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    revision,
                    receiver,
                })
            }
            Err(error) => self.error = Some(format!("Could not start timing preview: {error}")),
        }
    }
    fn current_for(&self, editor: &Editor, draft: &Draft) -> bool {
        let Some(cached) = &self.cached else {
            return false;
        };
        cached.revision == editor.revision()
            && draft.context == Context::of(editor)
            && match &cached.plan {
                Plan::Capture(plan) => {
                    draft.tab == Tab::Capture
                        && plan.base() == &editor.song
                        && draft.capture_config().is_ok_and(|c| &c == plan.config())
                }
                Plan::Apply(plan) => {
                    draft.tab == Tab::Apply
                        && plan.base() == &editor.song
                        && draft.apply_config().is_ok_and(|c| &c == plan.config())
                }
            }
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_capture_preview(&self, editor: &Editor) -> Option<&CapturePreview> {
        let draft = self.draft.as_ref()?;
        if !self.current_for(editor, draft) {
            return None;
        }
        match &self.cached.as_ref()?.plan {
            Plan::Capture(p) => Some(p),
            _ => None,
        }
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_apply_preview(&self, editor: &Editor) -> Option<&GroovePreview> {
        let draft = self.draft.as_ref()?;
        if !self.current_for(editor, draft) {
            return None;
        }
        match &self.cached.as_ref()?.plan {
            Plan::Apply(p) => Some(p),
            _ => None,
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        if draft.context != Context::of(editor) {
            self.reset_document();
            return;
        }
        if draft.revision != editor.revision() {
            draft.revision = editor.revision();
            self.invalidate("Document changed: preview again before committing.");
        }
        if draft
            .groove
            .is_some_and(|id| !editor.song.grooves.contains_key(&id))
        {
            draft.groove = None;
            self.invalidate("Selected template no longer exists; choose a saved template.");
        }
        if draft
            .marker_set
            .is_some_and(|id| !editor.song.marker_sets.contains_key(&id))
        {
            draft.marker_set = None;
            self.invalidate("Selected marker set no longer exists; choose accepted markers.");
        }
        let old = draft.clone();
        let allowed = ui.is_enabled();
        let writable = allowed && editor.edit_mode;
        let (mut open, mut preview, mut commit, mut remove, mut close) =
            (true, false, false, false, false);
        let ctx = ui.ctx().clone();
        let body_height = (ctx.content_rect().height() - 390.0).max(140.0);
        egui::Window::new("Groove timing transfer").id(egui::Id::new("groove-timing-transfer"))
            .default_pos(egui::pos2(210.0,100.0)).default_width(620.0).min_width(510.0).resizable(false).collapsible(false).open(&mut open).show(&ctx,|ui|{
                ui.set_max_width(620.0);
                ui.label(RichText::new("TIMING ONLY · ORDINARY CELLS · EXPLICIT PREVIEW / COMMIT").strong().color(AQUA));
                ui.horizontal(|ui|{ui.add_enabled_ui(allowed,|ui|{ui.selectable_value(&mut draft.tab,Tab::Capture,"Capture template");ui.selectable_value(&mut draft.tab,Tab::Apply,"Apply timing");});ui.label(RichText::new(format!("v{} · revision {}",groove::VERSION,draft.revision)).small().color(MUTED));});
                egui::ScrollArea::vertical().id_salt("groove-body").max_height(body_height).auto_shrink([false,true]).show(ui,|ui|{
                    ui.add_enabled_ui(allowed,|ui| match draft.tab {Tab::Capture=>capture_fields(ui,editor,&mut draft),Tab::Apply=>apply_fields(ui,editor,&mut draft)});
                    if let Some(cached)=&mut self.cached {preview_details(ui,&cached.plan,cached.reveal);cached.reveal=false;}
                });
                ui.separator();
                if let Some(cached)=&self.cached {ready_summary(ui,&cached.plan);}
                if let Some(error)=&self.error{ui.colored_label(Color32::LIGHT_RED,error);}
                if let Some(notice)=&self.notice{ui.label(RichText::new(notice).small().color(AMBER));}
                if self.busy(){ui.label(RichText::new("Preview worker running · no document mutation").small().color(MUTED));}
                if !editor.edit_mode{ui.label(RichText::new("READ MODE · preview allowed; enable Edit mode to commit").small().color(AMBER));}
                let current=self.current_for(editor,&draft);
                let has_changes=self.cached.as_ref().is_some_and(|c|match &c.plan{Plan::Capture(_)=>true,Plan::Apply(p)=>p.summary().changed_cells>0});
                ui.horizontal(|ui|{
                    preview=ui.add_enabled(allowed&&!self.busy(),egui::Button::new(match draft.tab{Tab::Capture=>"Preview capture",Tab::Apply=>"Preview timing"})).clicked();
                    commit=ui.add_enabled(writable&&current&&has_changes&&!self.busy(),egui::Button::new(match draft.tab{Tab::Capture=>"Save template",Tab::Apply=>"Commit timing"})).clicked();
                    if draft.tab==Tab::Apply{remove=ui.add_enabled(writable&&!self.busy()&&draft.groove.is_some(),egui::Button::new("Remove template")).on_hover_text("Explicitly remove this saved template, not any already edited pattern cells. One Undo.").clicked();}
                    close=ui.button("Cancel / close").clicked();
                });
                ui.label(RichText::new("No accents, BPM inference, time-stretch or runtime groove. Each successful commit is one Undo.").small().color(MUTED));
            });
        ui.memory_mut(|m| m.surrender_focus(grid_id()));
        if !open || close {
            self.reset_document();
            return;
        }
        if old != draft {
            self.invalidate("Settings changed: preview this exact recipe again.");
        }
        self.draft = Some(draft);
        if preview && allowed {
            self.start_preview(editor);
        }
        if remove && writable {
            if let Some(id) = self.draft.as_ref().and_then(|d| d.groove) {
                match editor.remove_groove(id) {
                    Ok(()) => {
                        self.invalidate("Template removed (one Undo); existing cells unchanged.");
                        if let Some(d) = &mut self.draft {
                            d.groove = None;
                            d.revision = editor.revision();
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if commit
            && writable
            && self
                .draft
                .as_ref()
                .is_some_and(|d| self.current_for(editor, d))
        {
            let cached = self.cached.as_ref().unwrap();
            let created = match &cached.plan {
                Plan::Capture(p) => Some(p.summary().groove),
                Plan::Apply(_) => None,
            };
            let result = match &cached.plan {
                Plan::Capture(p) => editor.commit_groove_capture_preview(cached.revision, p),
                Plan::Apply(p) => editor.commit_groove_preview(cached.revision, p),
            };
            match result {
                Ok(()) => {
                    self.invalidate(if created.is_some(){"Timing template saved (one Undo). Choose Apply timing to edit ordinary notes."}else{"Timing committed once. Stored cells remain freely editable."});
                    if let Some(d) = &mut self.draft {
                        d.revision = editor.revision();
                        if let Some(id) = created {
                            d.groove = Some(id);
                        }
                    }
                }
                Err(e) => self.error = Some(e),
            }
        }
    }
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .id(egui::Id::new(("groove-number", label)))
            .desired_width(width),
    );
}
fn capture_fields(ui: &mut egui::Ui, editor: &Editor, draft: &mut Draft) {
    ui.horizontal(|ui| {
        field(ui, "Template name", &mut draft.name, 230.0);
    });
    let before = draft.marker_set;
    egui::ComboBox::from_id_salt("groove-marker-set")
        .width(360.0)
        .selected_text(
            draft
                .marker_set
                .map_or("Choose accepted marker set".into(), |id| {
                    format!("Marker set {}", id.0)
                }),
        )
        .show_ui(ui, |ui| {
            for (id, set) in &editor.song.marker_sets {
                ui.selectable_value(
                    &mut draft.marker_set,
                    Some(*id),
                    format!(
                        "M{} · {} · {} boundaries",
                        id.0,
                        set.name,
                        set.markers.len()
                    ),
                );
            }
        });
    if before != draft.marker_set {
        if let Some(set) = draft
            .marker_set
            .and_then(|id| editor.song.marker_sets.get(&id))
        {
            draft.start_marker = "0".into();
            draft.end_marker = set.markers.len().saturating_sub(1).to_string();
        }
    }
    if let Some(set) = draft
        .marker_set
        .and_then(|id| editor.song.marker_sets.get(&id))
    {
        ui.label(
            RichText::new(format!(
                "Accepted source {} · source BPM {} · beat origin = selected start boundary",
                set.sample.0,
                set.source_bpm.map_or(
                    "UNKNOWN — set explicitly in Samples / Break workflow".into(),
                    |bpm| bpm.to_string()
                )
            ))
            .small()
            .color(if set.source_bpm.is_some() {
                MUTED
            } else {
                AMBER
            }),
        );
        ui.horizontal(|ui| {
            field(ui, "Start marker", &mut draft.start_marker, 48.0);
            field(ui, "End marker (excluded)", &mut draft.end_marker, 48.0);
        });
        let start = draft
            .start_marker
            .parse::<usize>()
            .ok()
            .and_then(|i| set.markers.get(i));
        let end = draft
            .end_marker
            .parse::<usize>()
            .ok()
            .and_then(|i| set.markers.get(i));
        ui.label(
            RichText::new(format!(
                "SOURCE frame window [{}, {}) · zero-based marker indices",
                start.map_or("?".into(), u64::to_string),
                end.map_or("?".into(), u64::to_string)
            ))
            .small()
            .color(AQUA),
        );
        ui.checkbox(
            &mut draft.include_start,
            "Include start boundary as an onset anchor (explicit opt-in)",
        );
        ui.label(RichText::new("End boundary is structural and always excluded. No audio analysis or automatic onset inference.").small().color(MUTED));
    } else {
        ui.label(RichText::new("Save reviewed markers and an explicit source BPM in Samples / Break workflow first.").small().color(AMBER));
    }
    ui.horizontal(|ui| {
        ui.label("Grid");
        egui::ComboBox::from_id_salt("groove-division")
            .selected_text(format!("1/{}", draft.division.denominator()))
            .show_ui(ui, |ui| {
                for division in [
                    Subdivision::Sixteenth,
                    Subdivision::ThirtySecond,
                    Subdivision::SixtyFourth,
                    Subdivision::OneTwentyEighth,
                ] {
                    ui.selectable_value(
                        &mut draft.division,
                        division,
                        format!("1/{}", division.denominator()),
                    );
                }
            });
        field(ui, "Cycle steps", &mut draft.cycle, 65.0);
        ui.label(RichText::new("1..1024").small().color(MUTED));
    });
    ui.label(RichText::new("Nearest grid slots use half-up ties; duplicate/out-of-cycle anchors are refused, not averaged. Fresh immutable template; no overwrite. Stored SHA provenance is historical, not a fresh audio verification.").small().color(AMBER));
}
fn apply_fields(ui: &mut egui::Ui, editor: &Editor, draft: &mut Draft) {
    ui.label(
        RichText::new(format!(
            "Target Song {} BPM / {} LPB · unchanged by this timing edit",
            editor.song.bpm, editor.song.lines_per_beat
        ))
        .small()
        .color(AQUA),
    );
    egui::ComboBox::from_id_salt("groove-template")
        .width(360.0)
        .selected_text(
            draft
                .groove
                .map_or("Choose saved timing template".into(), |id| {
                    format!("Groove {}", id.0)
                }),
        )
        .show_ui(ui, |ui| {
            for (id, template) in &editor.song.grooves {
                ui.selectable_value(
                    &mut draft.groove,
                    Some(*id),
                    format!(
                        "G{} · {} · {} anchors",
                        id.0,
                        template.name,
                        template.anchors.len()
                    ),
                );
            }
        });
    if let Some(template) = draft.groove.and_then(|id| editor.song.grooves.get(&id)) {
        ui.label(
            RichText::new(format!(
                "1/{} · {}-step cycle · source BPM {} (declared) · {} anchors",
                template.division.denominator(),
                template.cycle_steps,
                template.source_bpm,
                template.anchors.len()
            ))
            .small()
            .color(AQUA),
        );
        template_details(ui, template);
    }
    ui.horizontal(|ui| {
        ui.label("Pattern");
        egui::ComboBox::from_id_salt("groove-pattern")
            .selected_text(format!("P{}", draft.pattern.0))
            .width(150.0)
            .show_ui(ui, |ui| {
                for (id, p) in &editor.song.patterns {
                    ui.selectable_value(
                        &mut draft.pattern,
                        *id,
                        format!("P{} · {} · {} rows", id.0, p.name, p.rows),
                    );
                }
            });
        ui.label("Track");
        egui::ComboBox::from_id_salt("groove-track")
            .selected_text(format!("T{}", draft.track.0))
            .width(140.0)
            .show_ui(ui, |ui| {
                for id in &editor.song.track_order {
                    let track = &editor.song.tracks[id];
                    ui.selectable_value(
                        &mut draft.track,
                        *id,
                        format!("T{} · {}", id.0, track.name),
                    );
                }
            });
    });
    ui.horizontal(|ui| {
        field(ui, "Row start", &mut draft.row_start, 55.0);
        field(ui, "End exclusive", &mut draft.row_end, 55.0);
        if ui.small_button("Whole pattern").clicked() {
            if let Some(p) = editor.song.patterns.get(&draft.pattern) {
                draft.row_start = "0".into();
                draft.row_end = p.rows.to_string();
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Columns");
        if let Some(track) = editor.song.tracks.get(&draft.track) {
            for column in 0..track.note_columns {
                let mut selected = draft.columns.contains(&column);
                if ui
                    .checkbox(&mut selected, format!("N{}", column + 1))
                    .changed()
                {
                    if selected {
                        draft.columns.push(column);
                        draft.columns.sort_unstable();
                    } else {
                        draft.columns.retain(|c| *c != column);
                    }
                }
            }
        }
    });
    ui.label(
        RichText::new(format!(
            "Rows [start,end), origin row_start · zero-based columns {:?}",
            draft.columns
        ))
        .small()
        .color(MUTED),
    );
    ui.horizontal(|ui| {
        field(ui, "Strength %", &mut draft.strength, 65.0);
        if ui.small_button("0% identity").clicked() {
            draft.strength = "0".into();
        }
        if ui.small_button("100% template").clicked() {
            draft.strength = "100".into();
        }
    });
    ui.label(RichText::new("Nearest half-up grid slots relative to row_start. Blend existing timing toward template phase, not an additive nudge. 0% is exact identity; 100% targets rounded template timing. Sparse unmatched slots stay unchanged.").small().color(AMBER));
    ui.label(RichText::new("Stored note values/velocity unchanged; complete cells move with commands. Bounds, collisions, changed phrase timing, changed mute eligibility and unrepresentable repeated/alias timing are refused. Shared source edits affect its repeats/aliases. Re-timing can change RR choices, choke order and overlap.").small().color(MUTED));
    if let Err(error) = editor.song.require_local_track(draft.pattern, draft.track) {
        ui.colored_label(AMBER, error);
    }
}
fn template_details(ui: &mut egui::Ui, template: &groove::GrooveTemplate) {
    let response=ui.collapsing("Template anchors / stored provenance",|ui|{
        ui.label(RichText::new(format!("Frozen source {} Hz / {} BPM · cycle {} × 1/{}",template.source_rate,template.source_bpm,template.cycle_steps,template.division.denominator())).small().color(AQUA));
        ui.label(RichText::new("Historical immutable recipe; no current source-byte or audio-quality claim. Approximate beat offsets are display-only; exact source-frame deltas remain authoritative.").small().color(MUTED));
        match groove::inspect(template){Ok(anchors)=>{egui::ScrollArea::vertical().id_salt("groove-anchor-list").max_height(120.0).show_rows(ui,19.0,anchors.len(),|ui,range|{for index in range{let a=&anchors[index];ui.monospace(format!("step {:04} · delta {} SOURCE frames · offset ≈{:+.8} beats",a.step,a.source_frame_delta,a.offset_beats)).on_hover_text(format!("{a:#?}"));}});},Err(error)=>{ui.colored_label(Color32::LIGHT_RED,error);}}
        let p=&template.provenance;
        ui.label(RichText::new(format!("Historical sample S{} · {} real SOURCE frames · marker set M{}",p.sample.0,p.source_frames,p.marker_set.0)).small().color(MUTED));
        ui.label(RichText::new(format!("Marker indices [{},{}) · SOURCE window [{},{}) · start anchor {}",p.start_marker,p.end_marker,p.window_start_frame,p.window_end_frame,if p.include_start_anchor {"included"}else{"excluded"})).small().color(MUTED));
        ui.label(RichText::new(format!("Source SHA {}",p.source_sha256)).small().monospace());
        ui.label(RichText::new(format!("Markers SHA {}",p.marker_set_sha256)).small().monospace());
    });
    if response.header_response.clicked() {
        response.header_response.request_focus();
    }
}
fn ready_summary(ui: &mut egui::Ui, plan: &Plan) {
    match plan {
        Plan::Capture(p) => {
            let s = p.summary();
            ui.label(
                RichText::new(format!(
                    "READY · new G{} · {} anchors / {} steps · SOURCE [{},{})",
                    s.groove.0,
                    s.anchor_count,
                    s.cycle_steps,
                    s.window_start_frame,
                    s.window_end_frame
                ))
                .small()
                .color(AQUA),
            );
            ui.label(
                RichText::new(format!(
                    "Declared source BPM {} · timing-only capture",
                    s.source_bpm
                ))
                .small()
                .color(MUTED),
            );
        }
        Plan::Apply(p) => {
            let s = p.summary();
            let c = p.config();
            ui.label(
                RichText::new(format!(
                    "READY · G{} · {}% blend · {} changed / {} matched · {} unmatched",
                    s.groove.0,
                    c.strength_percent,
                    s.changed_cells,
                    s.matched_notes,
                    s.unmatched_notes
                ))
                .small()
                .color(AQUA),
            );
            ui.label(
                RichText::new(format!(
                    "P{} / T{} · rows [{},{}) · {} columns · {} moved",
                    c.scope.pattern.0,
                    c.scope.track.0,
                    c.scope.row_start,
                    c.scope.row_end,
                    c.scope.columns.len(),
                    s.moved_cells
                ))
                .small()
                .color(MUTED),
            )
            .on_hover_ui(|ui| {
                ui.label("Exact selected note columns (one-based, scroll all)");
                egui::ScrollArea::vertical()
                    .id_salt("groove-ready-columns")
                    .max_height(150.0)
                    .show_rows(ui, 18.0, c.scope.columns.len(), |ui, range| {
                        for i in range {
                            ui.monospace(format!("N{}", c.scope.columns[i] + 1));
                        }
                    });
            });
        }
    }
}
fn preview_details(ui: &mut egui::Ui, plan: &Plan, reveal: bool) {
    ui.separator();
    let heading = ui.label(
        RichText::new("PREVIEW · NO DOCUMENT CHANGED")
            .strong()
            .color(AQUA),
    );
    if reveal {
        heading
            .scroll_to_me_animation(Some(egui::Align::TOP), egui::style::ScrollAnimation::none());
    }
    match plan {
        Plan::Capture(p) => {
            ready_summary(ui, plan);
            template_details(ui, p.template());
            warnings(ui, &p.summary().warnings);
            ui.label(
                RichText::new(format!("Plan {}", p.id()))
                    .small()
                    .color(MUTED),
            )
            .on_hover_text(format!("Effective capture config:\n{:#?}", p.config()));
        }
        Plan::Apply(p) => {
            let s = p.summary();
            ui.label(
                RichText::new(format!(
                    "{} selected notes · {} zero-frame moves · {} POS / {} alias targets",
                    s.selected_notes,
                    s.zero_frame_moves,
                    s.affected_occurrences.len(),
                    s.affected_aliases.len()
                ))
                .small()
                .color(MUTED),
            )
            .on_hover_ui(|ui| {
                ui.set_max_width(420.0);
                ui.label("Exact affected destinations (scroll all)");
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.label("Occurrences");
                        egui::ScrollArea::vertical()
                            .id_salt("groove-affected-pos")
                            .max_height(120.0)
                            .show_rows(ui, 18.0, s.affected_occurrences.len(), |ui, range| {
                                for i in range {
                                    ui.monospace(format!("POS {:02}", s.affected_occurrences[i]));
                                }
                            });
                    });
                    ui.vertical(|ui| {
                        ui.label("Alias targets");
                        egui::ScrollArea::vertical()
                            .id_salt("groove-affected-aliases")
                            .max_height(120.0)
                            .show_rows(ui, 18.0, s.affected_aliases.len(), |ui, range| {
                                for i in range {
                                    let a = s.affected_aliases[i];
                                    ui.monospace(format!("P{} / T{}", a.pattern.0, a.track.0));
                                }
                            });
                    });
                });
                ui.label(format!("Stored template SHA {}", s.template_sha256));
            });
            warnings(ui, &s.warnings);
            if p.changes().is_empty() {
                ui.label("No stored changes; no commit or history entry needed.");
            } else {
                ui.label(
                    RichText::new(
                        "Exact row / column and delay changes · hover for full stored cells",
                    )
                    .small()
                    .color(MUTED),
                );
                egui::ScrollArea::vertical()
                    .id_salt("groove-diff")
                    .max_height(150.0)
                    .show_rows(ui, 21.0, p.changes().len(), |ui, range| {
                        for i in range {
                            let c = &p.changes()[i];
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!(
                                        "{:03}->{:03} N{} · DLY {:?} -> {:?}",
                                        c.from_row,
                                        c.to_row,
                                        c.column + 1,
                                        c.before.delay,
                                        c.after.delay
                                    ))
                                    .monospace(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(format!(
                                "Source row {} / N{} -> row {} / N{}\nBEFORE {:?}\nAFTER {:?}",
                                c.from_row,
                                c.column + 1,
                                c.to_row,
                                c.column + 1,
                                c.before,
                                c.after
                            ));
                        }
                    });
            }
            ui.label(
                RichText::new(format!("Plan {}", p.id()))
                    .small()
                    .color(MUTED),
            )
            .on_hover_text(format!("Effective timing config:\n{:#?}", p.config()));
        }
    }
}
fn warnings(ui: &mut egui::Ui, warnings: &[String]) {
    if !warnings.is_empty() {
        let r = ui.collapsing(
            format!("Full timing constraints / warnings ({})", warnings.len()),
            |ui| {
                for warning in warnings {
                    ui.colored_label(AMBER, warning);
                }
            },
        );
        if r.header_response.clicked() {
            r.header_response.request_focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_numeric_text_rejects_instead_of_clamping() {
        for value in [
            "-1",
            "101",
            "NaN",
            "inf",
            "1.5",
            "1e2",
            "18446744073709551616",
        ] {
            assert!(integer(value, 0, 100, "Strength").is_err(), "{value}");
        }
        assert_eq!(integer("0", 0, 100, "Strength").unwrap(), 0);
        assert_eq!(integer("100", 0, 100, "Strength").unwrap(), 100);
        assert_eq!(integer("1024", 1, 1024, "Cycle").unwrap(), 1024);
        for value in ["0", "1025", "-3"] {
            assert!(integer(value, 1, 1024, "Cycle").is_err());
        }
    }
    #[test]
    fn canceled_worker_is_tracked_until_completion_and_never_reinstalled() {
        let editor = Editor::default();
        let mut ui = GrooveUi::default();
        ui.open(&editor);
        let (sender, receiver) = mpsc::sync_channel(1);
        ui.pending = Some(Pending {
            epoch: ui.epoch,
            revision: editor.revision(),
            receiver,
        });
        ui.reset_document();
        ui.open(&editor);
        let newer = ui.epoch;
        ui.start_preview(&editor);
        assert!(ui.busy());
        assert_ne!(ui.pending.as_ref().unwrap().epoch, newer);
        sender.send(Err("Canceled old result".into())).unwrap();
        ui.poll();
        assert!(!ui.busy());
        assert!(ui.cached.is_none());
        assert!(ui.error.is_none());
        assert!(ui.is_open());
    }
    #[test]
    fn empty_document_capture_context_does_not_require_a_pattern_or_alias_authority() {
        let mut editor = Editor::default();
        let mut song = editor.song.clone();
        song.order.clear();
        song.patterns.clear();
        song.tracks.clear();
        song.track_order.clear();
        editor.replace_document(song).unwrap();
        let mut ui = GrooveUi::default();
        ui.open(&editor);
        let draft = ui.draft.as_ref().unwrap();
        assert!(draft.context.is_none());
        assert!(!draft.include_start);
        assert!(draft.marker_set.is_none());
        assert!(draft.groove.is_none());
        assert_eq!(draft.pattern, PatternId(0));
    }
    #[test]
    fn maximum_256_column_preview_keeps_fixed_footer_inside_both_viewports() {
        use crate::song::*;
        for (width, height) in [(900.0, 620.0), (1180.0, 780.0)] {
            let mut editor = Editor::default();
            let mut song = editor.song.clone();
            song.order.truncate(1);
            song.bpm = 120.0;
            song.lines_per_beat = 4;
            for p in song.patterns.values_mut() {
                p.rows = 8;
                p.tracks.clear();
            }
            for i in song.instruments.values_mut() {
                i.sample = Some("/tmp/groove-ui-layout.wav".into());
            }
            song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
            song.register_sample(
                SampleId(1),
                SampleAsset {
                    path: "/tmp/groove-ui-layout.wav".into(),
                    sha256: "b".repeat(64),
                    sample_rate: 48000,
                    channels: 1,
                    frames: 24000,
                },
            )
            .unwrap();
            song.upsert_marker_set(
                MarkerSetId(1),
                MarkerSet {
                    name: "Timing".into(),
                    sample: SampleId(1),
                    source_bpm: Some(120.0),
                    markers: vec![0, 6600, 12000, 18600, 24000],
                    locked_frames: vec![],
                },
            )
            .unwrap();
            song.patterns
                .get_mut(&PatternId(1))
                .unwrap()
                .tracks
                .entry(TrackId(1))
                .or_default()
                .entry(1)
                .or_default()
                .insert(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        ..Default::default()
                    },
                );
            let captured = groove::capture_preview(
                &song,
                GrooveId(1),
                CaptureConfig {
                    version: 1,
                    name: "Timing".into(),
                    marker_set: MarkerSetId(1),
                    start_marker: 0,
                    end_marker: 4,
                    include_start_anchor: false,
                    division: Subdivision::Sixteenth,
                    cycle_steps: 4,
                },
            )
            .unwrap();
            editor
                .replace_document(captured.candidate().clone())
                .unwrap();
            editor.edit_mode = true;
            let mut view = GrooveUi::default();
            view.open(&editor);
            let draft = view.draft.as_mut().unwrap();
            draft.groove = Some(GrooveId(1));
            draft.columns = (0..256).collect();
            draft.row_end = "8".into();
            let preview = groove::preview(&editor.song, draft.apply_config().unwrap()).unwrap();
            assert_eq!(preview.summary().changed_cells, 1);
            view.cached = Some(Cached {
                revision: editor.revision(),
                plan: Plan::Apply(Box::new(preview)),
                reveal: false,
            });
            let ctx = egui::Context::default();
            let mut output = None;
            for frame in 0..25 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        time: Some(f64::from(frame) / 60.0),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| view.show(ui, &mut editor));
                    },
                ));
            }
            let output = output.unwrap();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            for label in ["Commit timing", "Remove template", "Cancel / close"] {
                assert!(
                    output.shapes.iter().any(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                            let r = egui::Rect::from_min_size(text.pos, text.galley.size());
                            shape.clip_rect.contains_rect(r) && screen.contains_rect(r)
                        }
                        _ => false,
                    }),
                    "{label} clipped at {width}x{height}"
                );
            }
            assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text.contains("256 columns"))));
        }
    }
}
