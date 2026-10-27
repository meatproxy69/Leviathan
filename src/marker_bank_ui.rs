//! Explicit reviewed marker intervals become a new ordinary pad bank. Preview is
//! metadata-only; source identity is checked on the existing Samples worker
//! immediately before the one guarded document transaction.
use crate::{
    editor::Editor,
    marker_bank::{self, MarkerBankConfig, MarkerBankPreview},
    sample_asset,
    song::{InstrumentId, MarkerSetId, PadId, SampleId, Song},
    tracker_ui::{grid_id, AQUA},
};
use eframe::egui::{self, Color32, RichText};
use std::{path::PathBuf, sync::Arc};

const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const WARN: Color32 = Color32::from_rgb(229, 185, 112);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Draft {
    marker_set: MarkerSetId,
    source: SampleId,
    name: String,
    intervals: String,
}
impl Draft {
    fn config(&self) -> Result<MarkerBankConfig, String> {
        let text = self.intervals.trim();
        if text.len() > 2048 {
            return Err("Interval text exceeds 2048 bytes; use at most 16 explicit indices. Text was not truncated".into());
        }
        if text.is_empty() {
            return Err(
                "Choose 1 to 16 adjacent marker intervals; nothing is selected automatically"
                    .into(),
            );
        }
        let mut indices = Vec::new();
        for field in text.split(',') {
            let field = field.trim();
            if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Intervals: use comma-separated whole indices in strictly ascending source order; no rounding or sorting of typed input".into());
            }
            let index = field
                .parse::<u16>()
                .map_err(|_| "Interval index is outside the u16 range; no clamping")?;
            if indices.last().is_some_and(|previous| *previous >= index) {
                return Err("Intervals must be unique and strictly ascending; typed order was not silently changed".into());
            }
            indices.push(index);
        }
        if indices.len() > marker_bank::MAX_INTERVALS {
            return Err(format!(
                "{} intervals selected; at most 16 are allowed. No intervals were truncated.",
                indices.len()
            ));
        }
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(
                "Bank name must be nonblank and at most 256 UTF-8 bytes; text was not truncated"
                    .into(),
            );
        }
        Ok(MarkerBankConfig {
            version: marker_bank::VERSION,
            marker_set: self.marker_set,
            interval_indices: indices,
            name: self.name.clone(),
        })
    }
}

pub(crate) struct PreviewRequest {
    song: Box<Song>,
    revision: u64,
    epoch: u64,
    draft: Draft,
    path: PathBuf,
}
#[derive(Clone)]
pub(crate) struct Prepared {
    revision: u64,
    epoch: u64,
    draft: Draft,
    path: PathBuf,
    preview: Arc<MarkerBankPreview>,
}
pub(crate) enum WorkRequest {
    Preview(Box<PreviewRequest>),
    Verify(Prepared),
}
pub(crate) enum WorkResult {
    Preview(Prepared),
    Verified(Prepared),
}
pub(crate) fn execute(request: WorkRequest) -> Result<WorkResult, String> {
    match request {
        WorkRequest::Preview(request) => {
            let preview = marker_bank::preview(&request.song, &request.draft.config()?)?;
            Ok(WorkResult::Preview(Prepared {
                revision: request.revision,
                epoch: request.epoch,
                draft: request.draft,
                path: request.path,
                preview: Arc::new(preview),
            }))
        }
        WorkRequest::Verify(prepared) => {
            if !prepared.path.is_absolute() {
                return Err("Bank source has no absolute project-resolved path; reopen the project or reload its registered source. No working-directory fallback.".into());
            }
            sample_asset::verify_identity(prepared.preview.source(), &prepared.path)?;
            Ok(WorkResult::Verified(prepared))
        }
    }
}

pub(crate) struct Context {
    pub marker_set: Option<MarkerSetId>,
    pub source: Option<SampleId>,
    pub markers_saved: bool,
    pub path: PathBuf,
    pub busy: bool,
}

#[derive(Default)]
pub(crate) struct MarkerBankUi {
    draft: Option<Draft>,
    epoch: u64,
    prepared: Option<Prepared>,
    verified: Option<Prepared>,
    authorized: bool,
    committed: bool,
    open_kit: Option<(InstrumentId, PadId)>,
    error: Option<String>,
    notice: Option<String>,
    reveal_error: bool,
    reveal_preview: bool,
    #[cfg(any(test, feature = "ui-capture"))]
    body_layout: Option<(egui::Rect, f32)>,
}
impl MarkerBankUi {
    pub(crate) fn agent_action(
        &mut self,
        action: &serde_json::Value,
        editor: &mut Editor,
        context: &Context,
    ) -> Result<Option<WorkRequest>, String> {
        let kind = action
            .get("action")
            .and_then(serde_json::Value::as_str)
            .ok_or("action.action must be string")?;
        match kind {
            "open" => {
                let marker = action
                    .get("marker_set")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("marker_set must be unsigned")?;
                let source = action
                    .get("source")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("source must be unsigned")?;
                self.open(MarkerSetId(marker), SampleId(source));
                Ok(None)
            }
            "patch" => {
                let mut draft = self.draft.clone().ok_or("open a marker bank draft first")?;
                let fields = action
                    .get("fields")
                    .and_then(serde_json::Value::as_object)
                    .ok_or("fields must be object")?;
                for (k, v) in fields {
                    match k.as_str() {
                        "name" => draft.name = v.as_str().ok_or("name must be text")?.into(),
                        "intervals" => {
                            draft.intervals = v.as_str().ok_or("intervals must be text")?.into()
                        }
                        _ => return Err(format!("unknown marker bank draft field: {k}")),
                    }
                }
                self.draft = Some(draft);
                self.cancel_commit();
                self.epoch = self.epoch.wrapping_add(1);
                self.prepared = None;
                self.error = None;
                Ok(None)
            }
            "all_intervals" | "clear_intervals" => {
                let draft = self
                    .draft
                    .as_mut()
                    .ok_or("open a marker bank draft first")?;
                if kind == "clear_intervals" {
                    draft.intervals.clear()
                } else {
                    let set = editor
                        .song
                        .marker_sets
                        .get(&draft.marker_set)
                        .ok_or("marker set no longer exists")?;
                    draft.intervals = (0..set.markers.len().saturating_sub(1))
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
                self.cancel_commit();
                self.epoch = self.epoch.wrapping_add(1);
                self.prepared = None;
                Ok(None)
            }
            "preview" => {
                let draft = self
                    .draft
                    .as_ref()
                    .ok_or("open a marker bank draft first")?
                    .clone();
                if context.busy
                    || !context.markers_saved
                    || context.marker_set != Some(draft.marker_set)
                    || context.source != Some(draft.source)
                {
                    return Err(
                        "bank preview requires the current saved marker set and idle source worker"
                            .into(),
                    );
                };
                draft.config()?;
                crate::project::check_serialized_size(&editor.song)?;
                self.cancel_commit();
                self.committed = false;
                self.prepared = None;
                self.notice = None;
                Ok(Some(WorkRequest::Preview(Box::new(PreviewRequest {
                    song: Box::new(editor.song.clone()),
                    revision: editor.revision(),
                    epoch: self.epoch,
                    draft,
                    path: context.path.clone(),
                }))))
            }
            "create" => {
                let draft = self
                    .draft
                    .as_ref()
                    .ok_or("open a marker bank draft first")?;
                if !editor.edit_mode || context.busy || !self.current(editor, context, draft) {
                    return Err(
                        "create requires edit mode and a current reviewed bank preview".into(),
                    );
                };
                self.authorized = true;
                self.error = None;
                Ok(Some(WorkRequest::Verify(
                    self.prepared
                        .as_ref()
                        .ok_or("bank preview missing")?
                        .clone(),
                )))
            }
            "open_kit" => {
                if !self.created_exists(editor) {
                    return Err("created kit is missing or bank has not been committed".into());
                }
                let summary = &self
                    .prepared
                    .as_ref()
                    .ok_or("bank preview missing")?
                    .preview
                    .summary();
                let pad = summary.pads.first().ok_or("created bank has no pads")?.pad;
                self.open_kit = Some((summary.instrument, pad));
                Ok(None)
            }
            "commit_verified" => {
                let verified = self
                    .verified
                    .take()
                    .ok_or("no verified create result is waiting")?;
                let draft = self.draft.as_ref().ok_or("bank dialog is closed")?;
                let valid = self.authorized
                    && editor.edit_mode
                    && context.markers_saved
                    && context.marker_set == Some(draft.marker_set)
                    && context.source == Some(draft.source)
                    && self.current(editor, context, draft)
                    && self
                        .prepared
                        .as_ref()
                        .is_some_and(|p| p.preview.id() == verified.preview.id());
                self.authorized = false;
                if !valid {
                    return Err("verified bank result became stale or was canceled".into());
                };
                editor.commit_marker_bank_preview(verified.revision, &verified.preview)?;
                self.committed = true;
                self.notice = Some(
                    "Bank created (one undo); source, markers and arrangement are unchanged."
                        .into(),
                );
                Ok(None)
            }
            "cancel" => {
                self.close();
                Ok(None)
            }
            _ => Err(format!("unsupported marker bank action: {kind}")),
        }
    }
    pub(crate) fn open(&mut self, marker_set: MarkerSetId, source: SampleId) {
        self.reset();
        self.draft = Some(Draft {
            marker_set,
            source,
            name: "Marker pad bank".into(),
            intervals: String::new(),
        });
    }
    pub(crate) fn reset(&mut self) {
        let epoch = self.epoch.wrapping_add(1);
        *self = Self {
            epoch,
            ..Self::default()
        };
    }
    pub(crate) fn cancel_commit(&mut self) {
        self.authorized = false;
        self.verified = None;
    }
    pub(crate) fn close(&mut self) {
        self.reset();
    }
    pub(crate) fn take_open_kit(&mut self) -> Option<(InstrumentId, PadId)> {
        self.open_kit.take()
    }
    pub(crate) fn fail(&mut self, error: String) {
        self.cancel_commit();
        self.error = Some(error);
        self.notice = None;
        self.reveal_error = true;
    }
    pub(crate) fn finish(&mut self, result: WorkResult) {
        let (prepared, verified) = match result {
            WorkResult::Preview(p) => (p, false),
            WorkResult::Verified(p) => (p, true),
        };
        if prepared.epoch != self.epoch || self.draft.as_ref() != Some(&prepared.draft) {
            return;
        }
        if verified {
            if self.authorized {
                self.verified = Some(prepared);
            }
        } else {
            self.cancel_commit();
            self.prepared = Some(prepared);
            self.reveal_preview = true;
        }
    }
    fn current(&self, editor: &Editor, context: &Context, draft: &Draft) -> bool {
        !self.committed
            && context.markers_saved
            && context.marker_set == Some(draft.marker_set)
            && context.source == Some(draft.source)
            && self.prepared.as_ref().is_some_and(|p| {
                p.epoch == self.epoch
                    && p.revision == editor.revision()
                    && &p.draft == draft
                    && draft.config().is_ok_and(|c| &c == p.preview.config())
                    && p.path == context.path
                    && editor.song.marker_sets.get(&draft.marker_set)
                        == p.preview.base().marker_sets.get(&draft.marker_set)
                    && editor.song.samples.get(&draft.source) == Some(p.preview.source())
            })
    }
    pub(crate) fn show(
        &mut self,
        parent: &mut egui::Ui,
        editor: &mut Editor,
        context: Context,
    ) -> Option<WorkRequest> {
        let mut draft = self.draft.take()?;
        if context.marker_set != Some(draft.marker_set) || context.source != Some(draft.source) {
            self.reset();
            return None;
        }
        let allowed = parent.is_enabled();
        let focused = parent.ctx().input(|i| {
            i.focused
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if !allowed || !focused || !editor.edit_mode || !context.markers_saved {
            self.cancel_commit();
        }
        let before = draft.clone();
        let ctx = parent.ctx().clone();
        let mut open = true;
        let (mut preview, mut create, mut cancel, mut open_kit) = (false, false, false, false);
        let extra_footer_lines = usize::from(self.error.is_some())
            + usize::from(self.notice.is_some())
            + usize::from(context.busy)
            + usize::from(!editor.edit_mode);
        let body_height = (ctx.content_rect().height() - 365.0 - 22.0 * extra_footer_lines as f32)
            .clamp(150.0, 420.0);
        egui::Window::new("Pad bank from accepted markers")
            .id(egui::Id::new("marker-bank-window"))
            .default_pos(egui::pos2(205.0, 75.0)).default_width(640.0).min_width(500.0)
            .resizable(false).collapsible(false).open(&mut open).show(&ctx, |ui| {
                ui.set_max_width(640.0);
                ui.label(RichText::new(format!("SAVED M{} / SOURCE S{} · version {}", draft.marker_set.0, draft.source.0, marker_bank::VERSION)).strong().color(AQUA));
                ui.label(RichText::new("Exact adjacent source-frame intervals; no BPM required, extraction, marker acceptance or pattern rewrite.").small().color(MUTED));
                let body = egui::ScrollArea::vertical().id_salt("marker-bank-body").max_height(body_height).auto_shrink([false,true]).show(ui, |ui| {
                    ui.add_enabled_ui(allowed && !context.busy, |ui| {
                        ui.horizontal(|ui| { ui.label("Bank name"); ui.add(egui::TextEdit::singleline(&mut draft.name).id(egui::Id::new("marker-bank-name")).desired_width(340.0)).on_hover_text("Nonblank, at most 256 UTF-8 bytes; invalid text is retained, never truncated."); });
                        ui.horizontal(|ui| { ui.label("Interval indices"); ui.add(egui::TextEdit::singleline(&mut draft.intervals).id(egui::Id::new("marker-bank-intervals")).hint_text("0, 1, 3").desired_width(340.0)); });
                        let entered=draft.intervals.split(',').filter(|part|!part.trim().is_empty()).count();
                        ui.label(RichText::new(format!("{entered} entries typed · maximum 16 · validity checked by Preview")).small().color(if entered>16{WARN}else{MUTED}));
                        ui.label(RichText::new("Zero-based indices, comma-separated and strictly increasing. Sparse choices stay in source order. Select 1–16; excess selection is refused, not truncated.").small().color(MUTED));
                        if let Some(set) = editor.song.marker_sets.get(&draft.marker_set) {
                            ui.horizontal(|ui| {
                                if ui.button("All intervals").clicked() { draft.intervals = (0..set.markers.len().saturating_sub(1)).map(|i| i.to_string()).collect::<Vec<_>>().join(", "); }
                                if ui.button("Clear selection").clicked() { draft.intervals.clear(); }
                                ui.label(format!("{} source intervals", set.markers.len().saturating_sub(1)));
                            });
                            egui::CollapsingHeader::new("Accepted interval boundaries").default_open(false).show(ui, |ui| {
                                egui::ScrollArea::vertical().id_salt("marker-bank-interval-list").max_height(130.0).show_rows(ui, 20.0, set.markers.len().saturating_sub(1), |ui, rows| {
                                    for i in rows { ui.monospace(format!("{:03}  [{}, {}) source frames", i, set.markers[i], set.markers[i+1])); }
                                });
                            });
                        }
                    });
                    if !context.markers_saved { ui.colored_label(WARN, "UNSAVED MARKER DRAFT · Save markers explicitly before Preview or Create. No draft boundaries are silently accepted."); }
                    if let Some(prepared) = &self.prepared {
                        ui.separator();
                        let current = self.current(editor, &context, &draft);
                        let response = ui.label(RichText::new(if self.committed {"CREATED · ordinary regions and pad mapping"} else if current {"REVIEWED SHARED PREVIEW"} else {"PREVIOUS PREVIEW · STALE / CREATE DISABLED"}).strong().color(if current || self.committed {AQUA} else {WARN}));
                        if self.reveal_preview { response.scroll_to_me(Some(egui::Align::Min)); self.reveal_preview=false; }
                        preview_body(ui, &prepared.preview);
                    }
                    if let Some(error)=&self.error { ui.separator(); let r=ui.colored_label(Color32::LIGHT_RED,error); if self.reveal_error {r.scroll_to_me(Some(egui::Align::Center)); self.reveal_error=false;} }
                });
                #[cfg(any(test, feature = "ui-capture"))]
                { self.body_layout = Some((body.inner_rect, body.state.offset.y)); }
                #[cfg(not(any(test, feature = "ui-capture")))]
                let _ = body;
                ui.separator();
                if let Some(p)=&self.prepared { let s=p.preview.summary(); ui.monospace(format!("{} · {} pads / new kit K{} / instrument I{}", if self.committed {"CREATED"} else {"PREVIEW"},s.pads.len(),s.kit.0,s.instrument.0)); }
                if self.error.is_some() {ui.label(RichText::new("REFUSED · full error above; no document changes").small().color(Color32::LIGHT_RED));}
                if let Some(notice)=&self.notice {ui.label(RichText::new(notice).small().color(MUTED));}
                if context.busy {ui.label(RichText::new(if self.authorized {"Verifying source SHA; Create remains authorized only while this exact review/context stays current."} else {"Source task running; no Create is authorized."}).small().color(WARN));}
                if !editor.edit_mode {ui.label(RichText::new("READ MODE · preview allowed; Create protected").small().color(WARN));}
                let current=self.current(editor,&context,&draft);
                ui.horizontal(|ui| {
                    preview=ui.add_enabled(allowed && !context.busy && context.markers_saved,egui::Button::new("Preview bank")).clicked();
                    create=ui.add_enabled(allowed && focused && editor.edit_mode && !context.busy && current,egui::Button::new("Create bank")).clicked();
                    if self.committed {open_kit=ui.add_enabled(allowed && self.created_exists(editor),egui::Button::new("Open new kit")).clicked();}
                    cancel=ui.button("Cancel / close").clicked();
                });
                ui.label(RichText::new("Create is one undo. Opening a kit is separate; never automatic audition, note insertion or cursor movement.").small().color(MUTED));
            });
        parent.memory_mut(|m| m.surrender_focus(grid_id()));
        if !open || cancel {
            self.reset();
            return None;
        }
        if before != draft {
            self.cancel_commit();
            self.epoch = self.epoch.wrapping_add(1);
            self.notice = None;
            self.error = None;
        }
        self.draft = Some(draft.clone());
        if open_kit && self.created_exists(editor) {
            let s = self.prepared.as_ref().unwrap().preview.summary();
            self.open_kit = Some((s.instrument, s.pads[0].pad));
        }
        if preview && allowed && !context.busy && context.markers_saved {
            match draft
                .config()
                .and_then(|_| crate::project::check_serialized_size(&editor.song))
            {
                Ok(_) => {
                    self.cancel_commit();
                    self.committed = false;
                    self.error = None;
                    self.notice = None;
                    return Some(WorkRequest::Preview(Box::new(PreviewRequest {
                        song: Box::new(editor.song.clone()),
                        revision: editor.revision(),
                        epoch: self.epoch,
                        draft,
                        path: context.path,
                    })));
                }
                Err(error) => self.fail(error),
            }
        }
        if create
            && allowed
            && focused
            && editor.edit_mode
            && !context.busy
            && self.current(editor, &context, &draft)
        {
            self.authorized = true;
            self.error = None;
            self.notice = None;
            return Some(WorkRequest::Verify(self.prepared.as_ref().unwrap().clone()));
        }
        if let Some(verified) = self.verified.take() {
            let valid = self.authorized
                && allowed
                && focused
                && editor.edit_mode
                && self.current(editor, &context, &draft)
                && self
                    .prepared
                    .as_ref()
                    .is_some_and(|p| p.preview.id() == verified.preview.id());
            self.authorized = false;
            if valid {
                match editor.commit_marker_bank_preview(verified.revision, &verified.preview) {
                    Ok(()) => {
                        self.committed = true;
                        self.error = None;
                        self.notice=Some("Bank created (one undo); original audio, markers and arrangement are unchanged.".into());
                    }
                    Err(error) => self.fail(error),
                }
            } else {
                self.notice = Some("Create canceled or stale; nothing was applied.".into());
            }
        }
        None
    }
    fn created_exists(&self, editor: &Editor) -> bool {
        self.committed && self.prepared.as_ref().is_some_and(|p| {let s=p.preview.summary(); matches!(editor.song.instruments.get(&s.instrument).map(|i| &i.kind),Some(crate::song::InstrumentKind::DrumKit{kit}) if *kit==s.kit) && editor.song.drum_kits.contains_key(&s.kit)})
    }
    pub(crate) fn capture_state(&self) -> serde_json::Value {
        serde_json::json!({"open":self.draft.is_some(),"epoch":self.epoch,"authorized":self.authorized,"verified_waiting":self.verified.is_some(),"committed":self.committed,"open_kit_pending":self.open_kit,"error":self.error,"notice":self.notice,"draft":self.draft.as_ref().map(|d|serde_json::json!({"marker_set":d.marker_set,"source":d.source,"name":d.name,"intervals":d.intervals,"parsed":d.config().ok()})),"preview":self.prepared.as_ref().map(|p|serde_json::json!({"id":p.preview.id(),"revision":p.revision,"config":p.preview.config(),"summary":p.preview.summary(),"base_sha256":p.preview.base_sha256(),"candidate_sha256":p.preview.candidate_sha256(),"candidate":p.preview.candidate()}))})
    }
}

fn preview_body(ui: &mut egui::Ui, preview: &MarkerBankPreview) {
    let s = preview.summary();
    ui.label(
        RichText::new(format!(
            "Source S{} · {} Hz / {} ch · {} native frames",
            s.source.0, s.source_sample_rate, s.source_channels, s.source_frames
        ))
        .small()
        .color(MUTED),
    );
    ui.label(RichText::new("FIXED DEFAULTS · root MIDI 60 (native reference, not inferred), tune 0, gain 1, pan 0, OneShot / Overlap. No roles, choke groups, velocity layers, envelope or loop are assigned.").small().color(WARN));
    ui.label(RichText::new("Slot is zero-based; selector is a drum key, not audible pitch. Bounds are exact [start,end) source frames; no audio is extracted or rewritten.").small().color(MUTED));
    egui::Grid::new("marker-bank-preview-rows")
        .striped(true)
        .spacing(egui::vec2(12.0, 5.0))
        .show(ui, |ui| {
            for text in ["Interval", "Pad / slot / key", "Region / source frames"] {
                ui.label(RichText::new(text).small().strong());
            }
            ui.end_row();
            for row in &s.pads {
                ui.monospace(row.interval_index.to_string());
                ui.monospace(format!("{} / {} / {}", row.pad.0, row.slot, row.selector));
                ui.monospace(format!(
                    "R{}  [{}, {})",
                    row.region.0, row.start_frame, row.end_frame
                ));
                ui.end_row();
                ui.label("");
                ui.label("Name / root");
                ui.label(format!("{} · MIDI {}", row.name, row.root_note));
                ui.end_row();
            }
        });
    egui::CollapsingHeader::new("Source identity and full bank details").show(ui,|ui|{
        ui.label(format!("Bank name: {}",s.name));
        ui.label(format!("Algorithm: {} · version {}",s.algorithm,s.version));
        ui.monospace(format!("Preview {}",preview.id()));
        ui.monospace(format!("Base SHA-256: {}",preview.base_sha256()));
        ui.monospace(format!("Candidate SHA-256: {}",preview.candidate_sha256()));
        ui.label(format!("Stored source SHA-256: {}",s.source_sha256));
        ui.label(format!("Preview source_identity_verified: {} · model metadata only. Create rechecks the selected source's SHA on the worker; subsequent external file changes are not prevented.",s.source_identity_verified));
        let w=&s.work;
        ui.monospace(format!("Ordered blocks / rows / cells: {} / {} / {}",w.ordered_track_blocks,w.ordered_rows,w.ordered_cells));
        ui.monospace(format!("Definition rows / cells / phrase cells: {} / {} / {}",w.definition_rows,w.definition_cells,w.phrase_cells));
        ui.monospace(format!("Prospective instruments / validation units: {} / {}",w.prospective_instruments,w.validation_units));
        if ui.button("Copy complete bank summary JSON").clicked() {
            if let Ok(exact)=serde_json::to_string_pretty(s){ui.ctx().copy_text(exact);}
        }
    });
    egui::CollapsingHeader::new("Creation caveats").show(ui, |ui| {
        for warning in &s.warnings {
            ui.label(RichText::new(warning).small().color(WARN));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::*;

    fn fixture(count: u16) -> (Editor, MarkerBankUi) {
        let mut editor = Editor::default();
        let mut song = example_song();
        for pattern in song.patterns.values_mut() {
            pattern.tracks.clear();
        }
        song.register_sample(
            SampleId(1),
            SampleAsset {
                path: PathBuf::from("/missing-marker-bank-ui-owned.wav"),
                sha256: "0".repeat(64),
                sample_rate: 48000,
                channels: 1,
                frames: u64::from(count) * 1000,
            },
        )
        .unwrap();
        song.upsert_marker_set(
            MarkerSetId(1),
            MarkerSet {
                name: "Accepted source without BPM".into(),
                sample: SampleId(1),
                source_bpm: None,
                markers: (0..=count).map(|i| u64::from(i) * 1000).collect(),
                locked_frames: vec![],
            },
        )
        .unwrap();
        editor.replace_document(song).unwrap();
        editor.edit_mode = true;
        let mut bank = MarkerBankUi::default();
        bank.open(MarkerSetId(1), SampleId(1));
        bank.draft.as_mut().unwrap().intervals = (0..count.min(16))
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        (editor, bank)
    }
    fn prepare(editor: &Editor, bank: &MarkerBankUi) -> Prepared {
        let draft = bank.draft.as_ref().unwrap().clone();
        Prepared {
            revision: editor.revision(),
            epoch: bank.epoch,
            path: PathBuf::from("/missing-marker-bank-ui-owned.wav"),
            preview: Arc::new(
                marker_bank::preview(&editor.song, &draft.config().unwrap()).unwrap(),
            ),
            draft,
        }
    }
    fn context() -> Context {
        Context {
            marker_set: Some(MarkerSetId(1)),
            source: Some(SampleId(1)),
            markers_saved: true,
            path: PathBuf::from("/missing-marker-bank-ui-owned.wav"),
            busy: false,
        }
    }
    #[test]
    fn marker_bank_exact_interval_parser_never_sorts_clamps_or_truncates() {
        let (_, mut bank) = fixture(17);
        let draft = bank.draft.as_mut().unwrap();
        for input in [
            "", "1, 0", "0, 0", "-1", "1.0", "NaN", "65536", "0,", "0,,1",
        ] {
            draft.intervals = input.into();
            assert!(draft.config().is_err(), "{input}");
            assert_eq!(draft.intervals, input);
        }
        draft.intervals = (0..17).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        assert!(draft
            .config()
            .unwrap_err()
            .contains("No intervals were truncated"));
        draft.intervals = "0, 2, 16".into();
        assert_eq!(draft.config().unwrap().interval_indices, vec![0, 2, 16]);
        draft.name = "界".repeat(86);
        assert!(draft.config().is_err());
        assert_eq!(draft.name.len(), 258);
        draft.name = "x".repeat(256);
        assert_eq!(draft.config().unwrap().name.len(), 256);
    }
    #[test]
    fn marker_bank_late_verification_never_installs_without_current_create_intent() {
        let (editor, mut bank) = fixture(3);
        let prepared = prepare(&editor, &bank);
        bank.finish(WorkResult::Preview(prepared.clone()));
        assert!(!bank.authorized);
        assert!(bank.prepared.is_some());
        bank.finish(WorkResult::Verified(prepared.clone()));
        assert!(bank.verified.is_none());
        bank.authorized = true;
        bank.cancel_commit();
        bank.finish(WorkResult::Verified(prepared.clone()));
        assert!(bank.verified.is_none());
        bank.authorized = true;
        bank.draft.as_mut().unwrap().name = "Changed".into();
        bank.finish(WorkResult::Verified(prepared.clone()));
        assert!(bank.verified.is_none());
        bank.close();
        bank.finish(WorkResult::Preview(prepared.clone()));
        assert!(bank.prepared.is_none());
        bank.finish(WorkResult::Verified(prepared));
        assert!(bank.verified.is_none());
    }
    fn frame(
        ctx: &egui::Context,
        bank: &mut MarkerBankUi,
        editor: &mut Editor,
        size: egui::Vec2,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        assert!(bank.show(ui, editor, context()).is_none());
                    });
                });
            },
        )
    }
    fn visible(out: &egui::FullOutput, text: &str, size: egui::Vec2) -> Option<egui::Rect> {
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text == text => {
                let r = t.visual_bounding_rect();
                (s.clip_rect.contains_rect(r)
                    && egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(r)
                    && !t.galley.elided)
                    .then_some(r)
            }
            _ => None,
        })
    }
    #[test]
    fn marker_bank_verified_commit_one_undo_preserves_cursor_and_open_is_separate() {
        let (mut editor, mut bank) = fixture(3);
        editor.row = 4;
        editor.column = 1;
        let before = editor.song.clone();
        let cursor = (
            editor.occurrence,
            editor.track,
            editor.row,
            editor.column,
            editor.field,
        );
        let p = prepare(&editor, &bank);
        let candidate = p.preview.candidate().clone();
        bank.finish(WorkResult::Preview(p.clone()));
        bank.authorized = true;
        bank.finish(WorkResult::Verified(p));
        let ctx = egui::Context::default();
        frame(
            &ctx,
            &mut bank,
            &mut editor,
            egui::vec2(900., 620.),
            vec![],
            true,
        );
        assert_eq!(editor.song, candidate);
        assert!(bank.committed);
        assert!(bank.take_open_kit().is_none());
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.row,
                editor.column,
                editor.field
            ),
            cursor
        );
        for _ in 0..4 {
            frame(
                &ctx,
                &mut bank,
                &mut editor,
                egui::vec2(900., 620.),
                vec![],
                true,
            );
        }
        editor.undo();
        assert_eq!(editor.song, before);
        assert!(!bank.created_exists(&editor));
        editor.redo();
        assert_eq!(editor.song, candidate);
        assert!(bank.created_exists(&editor));
    }
    #[test]
    fn marker_bank_verified_result_refuses_focus_pair_read_mode_disabled_and_exact_base_drift() {
        for guard in 0..5 {
            let (mut editor, mut bank) = fixture(3);
            let p = prepare(&editor, &bank);
            bank.finish(WorkResult::Preview(p.clone()));
            bank.authorized = true;
            bank.finish(WorkResult::Verified(p));
            let mut events = vec![];
            match guard {
                0 => editor.edit_mode = false,
                1 => {
                    events = vec![
                        egui::Event::WindowFocused(false),
                        egui::Event::WindowFocused(true),
                    ]
                }
                2 => editor.song.title = "Unchecked base change".into(),
                3 => {
                    editor.set_timing(140., 4).unwrap();
                    editor.undo();
                }
                _ => {}
            }
            let before = editor.song.clone();
            let revision = editor.revision();
            frame(
                &egui::Context::default(),
                &mut bank,
                &mut editor,
                egui::vec2(900., 620.),
                events,
                guard != 4,
            );
            assert_eq!(editor.song, before);
            assert_eq!(editor.revision(), revision);
            assert!(!bank.committed);
            assert!(!bank.authorized);
        }
    }
    #[test]
    fn marker_bank_sixteen_rows_and_footer_remain_reachable_at_both_sizes() {
        for size in [egui::vec2(900., 620.), egui::vec2(1180., 780.)] {
            let (mut editor, mut bank) = fixture(16);
            let p = prepare(&editor, &bank);
            bank.finish(WorkResult::Preview(p));
            let ctx = egui::Context::default();
            let mut last = false;
            for i in 0..90 {
                let events = if i > 6 {
                    let r = bank.body_layout.unwrap().0;
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0., -65.),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                };
                let out = frame(&ctx, &mut bank, &mut editor, size, events, true);
                if i > 3 {
                    for action in ["Preview bank", "Create bank", "Cancel / close"] {
                        assert!(
                            visible(&out, action, size).is_some(),
                            "{action} at {size:?}"
                        );
                    }
                }
                if out.shapes.iter().any(|s| match &s.shape {
                    egui::epaint::Shape::Text(t)
                        if t.galley.job.text.contains("[15000, 16000)") =>
                    {
                        s.clip_rect.contains_rect(t.visual_bounding_rect())
                    }
                    _ => false,
                }) {
                    last = true;
                    break;
                }
            }
            assert!(last, "Last exact interval was never visible at {size:?}");
        }
    }
    #[test]
    fn marker_bank_worst_footer_is_bounded_with_max_ids_name_error_busy_and_read_mode() {
        for size in [egui::vec2(900., 620.), egui::vec2(1180., 780.)] {
            let (mut editor, mut bank) = fixture(16);
            editor.song.instruments.insert(
                InstrumentId(u64::MAX - 1),
                Instrument {
                    name: "Unused high ID".into(),
                    kind: InstrumentKind::Pitched,
                    sample: None,
                    region: None,
                },
            );
            editor.song.drum_kits.insert(
                DrumKitId(u64::MAX - 1),
                DrumKitInstrument {
                    name: "Unused high kit".into(),
                    pads: Default::default(),
                },
            );
            bank.draft.as_mut().unwrap().name = "界".repeat(85) + "x";
            let p = prepare(&editor, &bank);
            assert_eq!(p.preview.summary().instrument.0, u64::MAX);
            assert_eq!(p.preview.summary().kit.0, u64::MAX);
            bank.finish(WorkResult::Preview(p));
            bank.error = Some("Long retained rejection. ".repeat(100));
            bank.notice = Some(
                "Prior review remains read-only; current source task does not authorize Create."
                    .into(),
            );
            editor.edit_mode = false;
            let ctx = egui::Context::default();
            for i in 0..10 {
                let out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            assert!(bank
                                .show(
                                    ui,
                                    &mut editor,
                                    Context {
                                        busy: true,
                                        ..context()
                                    }
                                )
                                .is_none());
                        });
                    },
                );
                if i > 3 {
                    for label in ["Preview bank", "Create bank", "Cancel / close"] {
                        assert!(visible(&out, label, size).is_some(), "{label} at {size:?}");
                    }
                }
            }
            assert_eq!(
                bank.prepared.as_ref().unwrap().preview.summary().name.len(),
                256
            );
        }
    }
}
