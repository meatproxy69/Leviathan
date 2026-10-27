//! Reviewed, persisted track balance. Draft controls never touch the audio owner.
//! The desktop submits an explicit successful Apply separately; its acknowledgement
//! describes worker-enqueued PCM, never what a device or listener has heard.
use crate::{
    editor::{DocumentOwner, DocumentTrackMixPreview, Editor},
    song::{Song, TrackId},
    track_mix::{self, MixLayoutStatus, TrackMixConfig, TrackMixSetting},
    tracker_ui::AQUA,
};
use eframe::egui::{self, Color32, RichText};
use serde_json::{json, Value};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

mod meters;

const WARN: Color32 = Color32::from_rgb(229, 185, 112);

fn number(text: &str, low: f64, high: f64, label: &str) -> Result<f64, String> {
    if text.is_empty() || text.trim() != text {
        return Err(format!(
            "{label} requires a finite number in {low}..{high}; no clamping"
        ));
    }
    let value = text
        .parse::<f64>()
        .map_err(|_| format!("{label} is not a finite number"))?;
    if !value.is_finite() || !(low..=high).contains(&value) {
        return Err(format!(
            "{label} must be finite and in {low}..{high} before f32 conversion; no clamping"
        ));
    }
    Ok(value)
}
fn float_text(value: f32) -> String {
    let decimal = value.to_string();
    let scientific = format!("{value:e}");
    if scientific.len() < decimal.len() {
        scientific
    } else {
        decimal
    }
}
fn short_name(value: &str) -> String {
    let mut name: String = value.chars().take(28).collect();
    if value.chars().nth(28).is_some() {
        name.push_str("...");
    }
    name
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RowDraft {
    track: TrackId,
    name: String,
    gain: String,
    balance: String,
    mute: bool,
    solo: bool,
    stored: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Draft {
    rows: Vec<RowDraft>,
    defined_tracks: usize,
    neutral_reset: bool,
}
impl Draft {
    fn from_song(song: &Song) -> Self {
        let ids: Vec<_> = if song.tracks.len() <= track_mix::MAX_TRACKS {
            song.track_order.clone()
        } else {
            song.track_mix.keys().copied().collect()
        };
        Self {
            rows: ids
                .into_iter()
                .map(|track| {
                    let mix = song.track_mix.get(&track).copied().unwrap_or_default();
                    RowDraft {
                        track,
                        name: song
                            .tracks
                            .get(&track)
                            .map(|t| short_name(&t.name))
                            .unwrap_or_default(),
                        gain: float_text(mix.gain),
                        balance: float_text(mix.balance),
                        mute: mix.mute,
                        solo: mix.solo,
                        stored: song.track_mix.contains_key(&track),
                    }
                })
                .collect(),
            defined_tracks: song.tracks.len(),
            neutral_reset: false,
        }
    }
    fn reset_neutral(&mut self) {
        for row in &mut self.rows {
            row.gain = "1".into();
            row.balance = "0".into();
            row.mute = false;
            row.solo = false;
            row.stored = false;
        }
        self.neutral_reset = true;
    }
    fn config(&self) -> Result<TrackMixConfig, String> {
        if self.neutral_reset {
            return Ok(TrackMixConfig {
                version: track_mix::VERSION,
                settings: vec![],
            });
        }
        let mut settings = Vec::new();
        for row in &self.rows {
            let gain = number(&row.gain, 0.0, 1.0, &format!("T{} gain", row.track.0))?;
            let balance = number(
                &row.balance,
                -1.0,
                1.0,
                &format!("T{} balance", row.track.0),
            )?;
            // Opening controls must not insert implicit defaults. Explicitly stored
            // neutral entries and negative zero remain distinct document metadata.
            if row.stored || gain != 1.0 || balance.to_bits() != 0 || row.mute || row.solo {
                settings.push(TrackMixSetting {
                    track: row.track,
                    gain,
                    balance,
                    mute: row.mute,
                    solo: row.solo,
                });
            }
        }
        Ok(TrackMixConfig {
            version: track_mix::VERSION,
            settings,
        })
    }
}
struct Pending {
    epoch: u64,
    revision: u64,
    draft: Draft,
    receiver: Receiver<Result<Box<DocumentTrackMixPreview>, String>>,
}
struct Ready {
    epoch: u64,
    draft: Draft,
    plan: DocumentTrackMixPreview,
}

/// UI-only reporting. The transport's own status is authoritative for accepted
/// and enqueued revisions. A document commit is not a project-file save.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct Delivery {
    pub document_revision: Option<u64>,
    pub requested_revision: Option<u64>,
    pub accepted_revision: u64,
    pub enqueued_revision: u64,
    pub available: bool,
    pub message: String,
}

#[derive(Default)]
pub(crate) struct TrackMixUi {
    open: bool,
    meters: meters::MeterView,
    owner: Option<DocumentOwner>,
    revision: Option<u64>,
    epoch: u64,
    draft: Option<Draft>,
    pending: Option<Pending>,
    completed: Option<Result<Box<DocumentTrackMixPreview>, String>>,
    ready: Option<Ready>,
    error: Option<String>,
    notice: Option<String>,
    reveal_error: bool,
    applied: bool,
    #[cfg(any(test, feature = "ui-capture"))]
    body_layout: Option<(egui::Rect, f32)>,
}
impl TrackMixUi {
    pub(crate) fn agent_state(&self, e: &Editor) -> Value {
        json!({"component":"mix","open":self.open,"busy":self.busy(),"current":self.current(e),"phase":if self.busy(){"busy"}else if self.current(e){"preview_ready"}else if self.open{"draft"}else{"closed"},"actions":["open","set_draft","reload","preview","poll","apply","cancel_authorization","close","meter_snapshot_latest","meter_follow_latest"],
        "draft_fields":{"rows":"array of {track:u64,gain:string 0..1,balance:string -1..1,mute:bool,solo:bool,stored:bool}","neutral_reset":"bool; explicit empty stored map"},
        "draft":self.draft.as_ref().map(|d|json!({"defined_tracks":d.defined_tracks,"neutral_reset":d.neutral_reset,"rows":d.rows.iter().map(|r|json!({"track":r.track,"name":r.name,"gain":r.gain,"balance":r.balance,"mute":r.mute,"solo":r.solo,"stored":r.stored})).collect::<Vec<_>>(),"parsed_config":d.config().ok()})),
        "preview":self.ready.as_ref().map(|r|json!({"revision":r.plan.revision(),"current":self.current(e),"config":r.plan.preview().config(),"summary":r.plan.preview().summary(),"content_sha256":r.plan.preview().summary().content_sha256,"candidate_settings":r.plan.preview().candidate().track_mix})),"applied":self.applied,"error":self.error,"notice":self.notice,"edit_mode":e.edit_mode,"meters":self.meters.agent_state()})
    }

    pub(crate) fn agent_action(
        &mut self,
        e: &mut Editor,
        _base: Option<&std::path::Path>,
        action: Value,
    ) -> Result<Value, String> {
        let object = action.as_object().ok_or("Action must be a JSON object")?;
        let verb = object
            .get("action")
            .and_then(Value::as_str)
            .ok_or("Action requires string action")?;
        match verb {
            "open" => {
                mix_agent_fields(object, &["action"])?;
                self.open_for(e);
            }
            "close" => {
                mix_agent_fields(object, &["action"])?;
                self.close();
            }
            "reload" => {
                mix_agent_fields(object, &["action"])?;
                if self.busy() {
                    return Err("Wait for the active mix preview before reloading".into());
                }
                self.sync(e);
                self.draft = Some(Draft::from_song(&e.song));
                self.ready = None;
                self.error = None;
                self.notice = Some("Reloaded stored document settings; no change applied.".into());
            }
            "set_draft" => {
                mix_agent_fields(object, &["action", "values"])?;
                if self.busy() {
                    return Err("Wait for the active mix preview before editing settings".into());
                }
                // A new document owner can reuse the same revision. Synchronize before
                // cloning the draft so a stale draft cannot be installed on that owner.
                self.sync(e);
                let values = object
                    .get("values")
                    .and_then(Value::as_object)
                    .ok_or("set_draft requires object values")?;
                mix_agent_fields(values, &["rows", "neutral_reset"])?;
                let mut d = self.draft.clone().ok_or("Open track mix first")?;
                if let Some(v) = values.get("neutral_reset") {
                    let reset = v.as_bool().ok_or("neutral_reset must be boolean")?;
                    if reset {
                        d.reset_neutral()
                    } else {
                        d.neutral_reset = false;
                    }
                }
                if let Some(rows) = values.get("rows") {
                    let rows = rows.as_array().ok_or("rows must be an array")?;
                    let mut seen = std::collections::BTreeSet::new();
                    for value in rows {
                        let row = value.as_object().ok_or("each row must be an object")?;
                        mix_agent_fields(
                            row,
                            &["track", "gain", "balance", "mute", "solo", "stored"],
                        )?;
                        let id = TrackId(
                            row.get("track")
                                .and_then(Value::as_u64)
                                .ok_or("row.track must be u64")?,
                        );
                        if !seen.insert(id) {
                            return Err(format!("Duplicate track {}", id.0));
                        }
                        let target = d
                            .rows
                            .iter_mut()
                            .find(|r| r.track == id)
                            .ok_or_else(|| format!("Unknown track {}", id.0))?;
                        if let Some(v) = row.get("gain") {
                            target.gain = v.as_str().ok_or("gain must remain text")?.into();
                        }
                        if let Some(v) = row.get("balance") {
                            target.balance = v.as_str().ok_or("balance must remain text")?.into();
                        }
                        if let Some(v) = row.get("mute") {
                            target.mute = v.as_bool().ok_or("mute must be boolean")?;
                        }
                        if let Some(v) = row.get("solo") {
                            target.solo = v.as_bool().ok_or("solo must be boolean")?;
                        }
                        if let Some(v) = row.get("stored") {
                            target.stored = v.as_bool().ok_or("stored must be boolean")?;
                        }
                    }
                    d.neutral_reset = false;
                }
                if self.draft.as_ref() != Some(&d) {
                    self.invalidate("Mix draft changed; preview the exact complete set again.");
                }
                self.draft = Some(d);
            }
            "preview" => {
                mix_agent_fields(object, &["action"])?;
                self.start(e);
            }
            "poll" => {
                mix_agent_fields(object, &["action"])?;
                self.sync(e);
                self.poll();
                self.finish(e);
            }
            "apply" => {
                mix_agent_fields(object, &["action"])?;
                if !e.edit_mode {
                    return Err("Enable Edit Mode to apply track mix settings".into());
                }
                self.apply(e);
            }
            "cancel_authorization" => {
                mix_agent_fields(object, &["action"])?;
                self.cancel_authorization();
            }
            "meter_snapshot_latest" | "meter_follow_latest" => {
                mix_agent_fields(object, &["action"])?;
                self.meters.agent_action(&json!({"action":if verb=="meter_snapshot_latest"{"snapshot_latest"}else{"follow_latest"}}))?;
            }
            _ => return Err(format!("Unknown track-mix action `{verb}`")),
        }
        Ok(self.agent_state(e))
    }
    pub fn accept_meters(&mut self, poll: crate::transport::MeterPoll, now: f64) {
        self.meters.accept(poll, now);
    }
    pub fn clear_meters(&mut self, reason: impl Into<String>) {
        self.meters.clear(reason);
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn open_for(&mut self, e: &Editor) {
        self.close();
        self.owner = Some(e.document_owner());
        self.revision = Some(e.revision());
        self.draft = Some(Draft::from_song(&e.song));
        self.open = true;
    }
    pub fn close(&mut self) {
        self.open = false;
        self.meters.clear("Mixer closed");
        self.epoch = self.epoch.wrapping_add(1);
        self.ready = None;
        self.completed = None;
        self.draft = None;
        self.error = None;
        self.notice = None;
        self.applied = false;
        // A canceled worker remains tracked until its actual terminal result.
    }
    pub fn reset_document(&mut self) {
        self.close();
        self.owner = None;
        self.revision = None;
    }
    pub fn take_applied(&mut self) -> bool {
        std::mem::take(&mut self.applied)
    }
    pub fn cancel_authorization(&mut self) {
        if self.open && (self.ready.is_some() || self.pending.is_some()) {
            self.invalidate("Interaction interrupted; preview again. Already accepted audio settings are not retracted.");
        }
    }
    fn invalidate(&mut self, message: &str) {
        self.epoch = self.epoch.wrapping_add(1);
        self.notice = Some(message.into());
        self.error = None;
    }
    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.notice = None;
        self.reveal_error = true;
    }
    fn sync(&mut self, e: &Editor) {
        if self.owner.as_ref().is_some_and(|o| !e.owns_document(o)) {
            self.reset_document();
            return;
        }
        if self.revision.is_some_and(|r| r != e.revision()) {
            self.invalidate("Document changed; previous review is stale. Reload stored settings or preview the retained draft.");
        }
        self.revision = Some(e.revision());
    }
    pub fn poll(&mut self) {
        if self.completed.is_some() {
            return;
        }
        let Some(p) = &self.pending else {
            return;
        };
        match p.receiver.try_recv() {
            Ok(result) => self.completed = Some(result),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.completed = Some(Err(
                    "Mix preview worker disconnected; document unchanged".into()
                ))
            }
        }
        if self.completed.is_some() && self.pending.as_ref().is_some_and(|p| p.epoch != self.epoch)
        {
            self.completed = None;
            self.pending = None;
        }
    }
    fn finish(&mut self, e: &Editor) {
        let Some(result) = self.completed.take() else {
            return;
        };
        let Some(p) = self.pending.take() else {
            return;
        };
        if !self.open
            || p.epoch != self.epoch
            || p.revision != e.revision()
            || self.draft.as_ref() != Some(&p.draft)
            || !self.owner.as_ref().is_some_and(|o| e.owns_document(o))
        {
            return;
        }
        match result {
            Ok(plan) => match e.validate_track_mix_preview(&plan) {
                Ok(()) => {
                    self.ready = Some(Ready {
                        epoch: p.epoch,
                        draft: p.draft,
                        plan: *plan,
                    });
                    self.error = None;
                    self.notice = None;
                }
                Err(error) => self.fail(error),
            },
            Err(error) => self.fail(error),
        }
    }
    fn start(&mut self, e: &Editor) {
        if self.busy() {
            return;
        }
        self.sync(e);
        let Some(draft) = self.draft.clone() else {
            return;
        };
        let config = match draft.config() {
            Ok(c) => c,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let capture = match e.capture_track_mix_document() {
            Ok(c) => c,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let revision = capture.revision();
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-track-mix".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    capture.preview_track_mix(config).map(Box::new)
                }))
                .unwrap_or_else(|_| Err("Mix preview worker panicked; document unchanged".into()));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    revision,
                    draft,
                    receiver,
                });
                self.error = None;
                self.notice = None;
            }
            Err(error) => self.fail(format!("Could not start mix preview worker: {error}")),
        }
    }
    fn current(&self, e: &Editor) -> bool {
        self.owner.as_ref().is_some_and(|o| e.owns_document(o))
            && self.ready.as_ref().is_some_and(|r| {
                r.epoch == self.epoch
                    && r.plan.revision() == e.revision()
                    && self.draft.as_ref() == Some(&r.draft)
            })
    }
    fn apply(&mut self, e: &mut Editor) {
        if self.busy() || !self.current(e) {
            self.fail("Preview the exact current draft before Apply".into());
            return;
        }
        let before = e.revision();
        let result = e.commit_track_mix_preview(&self.ready.as_ref().unwrap().plan);
        match result {
            Ok(()) => {
                self.applied = true;
                self.ready = None;
                self.revision = Some(e.revision());
                self.draft = Some(Draft::from_song(&e.song));
                self.error = None;
                self.notice = Some(if e.revision() == before {
                    "Document unchanged; no history entry. Explicit live retry is separate."
                } else { "Mix stored in document (one Undo). Save the project separately; live delivery is separate." }.into());
            }
            Err(error) => self.fail(error),
        }
    }
}

fn mix_agent_fields(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unknown action field `{key}`"));
    }
    Ok(())
}

fn scalar(
    ui: &mut egui::Ui,
    track: TrackId,
    text: &mut String,
    range: std::ops::RangeInclusive<f64>,
    fallback: f64,
    label: &str,
) -> bool {
    let mut changed = ui
        .add(
            egui::TextEdit::singleline(text)
                .id(egui::Id::new(("track-mix-value", track.0, label)))
                .desired_width(86.0),
        )
        .changed();
    let mut value = text
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && range.contains(v))
        .unwrap_or(fallback);
    let response = ui.add_sized(
        [78.0, 18.0],
        egui::Slider::new(&mut value, range)
            .show_value(false)
            .clamping(egui::SliderClamping::Edits),
    );
    #[cfg(any(test, feature = "ui-capture"))]
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::new(("track-mix-slider", track.0, label)),
            response.rect,
        )
    });
    response.clone().on_hover_text(format!(
        "{label}: draft only. No audio or history changes until explicit Preview and Apply."
    ));
    if response.changed() {
        *text = value.to_string();
        changed = true;
    }
    changed
}
fn compact_delivery(delivery: &Delivery) -> String {
    if let Some(request) = delivery.requested_revision {
        if delivery.enqueued_revision >= request {
            format!("Stored · Enqueued r{request}")
        } else if delivery.available {
            format!("Stored · Queued r{request}")
        } else {
            format!("Stored · request r{request} ended before acknowledgement")
        }
    } else if delivery.document_revision.is_some() {
        "Stored · not live-applied (details below)".into()
    } else if delivery.available {
        "Stored settings · playing".into()
    } else {
        "Stored settings · no live request".into()
    }
}
fn layout_label(status: &MixLayoutStatus) -> String {
    match status {
        MixLayoutStatus::Available { contributing_tracks, legacy_groups, live_ready } => format!("Model layout: {contributing_tracks} contributing tracks / {legacy_groups} legacy groups{}", if *live_ready { "" } else { " / no live contributions" }),
        MixLayoutStatus::Unavailable { reason } => format!("Model layout unavailable: {reason}. Document settings may still be stored; not live-ready."),
        MixLayoutStatus::Unsupported { reason, .. } => format!("Model layout unsupported: {reason}. No live-ready claim."),
    }
}
impl TrackMixUi {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        e: &mut Editor,
        allowed: bool,
        delivery: &Delivery,
    ) {
        self.sync(e);
        if !self.open {
            self.finish(e);
            return;
        }
        let lost_focus = ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|ev| matches!(ev, egui::Event::WindowFocused(false)))
        });
        if !allowed || lost_focus {
            self.cancel_authorization();
        }
        let current = self.current(e);
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        let original = draft.clone();
        let mut open = true;
        let mut preview = false;
        let mut apply = false;
        let mut reset = false;
        let mut reload = false;
        let mut close = false;
        let reserve = 270.0
            + if self.error.is_some() { 20.0 } else { 0.0 }
            + if !e.edit_mode { 20.0 } else { 0.0 }
            + if self.busy() { 20.0 } else { 0.0 };
        let body_height = (ctx.content_rect().height() - reserve).max(110.0);
        egui::Window::new("Track mix").id(egui::Id::new("track-mix-window"))
            .default_pos(egui::pos2(136.0, 76.0)).default_width(740.0).min_width(650.0)
            .resizable(false).collapsible(false).open(&mut open).enabled(allowed)
            .show(ctx, |ui| {
                ui.label(RichText::new("TRACK GAIN / STEREO BALANCE").strong().color(AQUA));
                ui.label("Draft controls are silent. Preview, then Apply.");
                let body = egui::ScrollArea::vertical().id_salt("track-mix-body").max_height(body_height).auto_shrink([false, false]).show(ui, |ui| {
                    self.meters.master(ui);
                    if draft.defined_tracks > track_mix::MAX_TRACKS {
                        ui.colored_label(WARN,format!("This native full-table editor supports at most 64 defined tracks (document has {}). Showing all configured entries read-only; no truncation or implicit deletion. Inspect or Reset to neutral remains available. This UI limit differs from the shared API's configured/contributing-track limits; neutral Song playback is unaffected. Headless metadata edits remain available where shared validation permits.",draft.defined_tracks));
                    }
                    ui.horizontal(|ui| {
                        ui.add_sized([168.0,18.0],egui::Label::new("Track · stored ID"));
                        ui.add_sized([174.0,18.0],egui::Label::new("Gain 0..1"));
                        ui.add_sized([174.0,18.0],egui::Label::new("Balance -1..1 (L/R)"));
                        ui.label("Mute / Solo");
                    });
                    let mut table_changed=false;
                    ui.add_enabled_ui(draft.defined_tracks <= track_mix::MAX_TRACKS, |ui| {
                        for row in &mut draft.rows {
                            ui.push_id(row.track.0,|ui| ui.horizontal(|ui| {
                                let name=ui.add_sized([168.0,34.0],egui::Label::new(RichText::new(format!("T{} · {}",row.track.0,row.name)).small()).truncate());
                                self.meters.row(ui,name,row.track,&format!("Track {} · {}\n{}",row.track.0,row.name,if row.stored{"Explicit stored entry"}else{"Implicit neutral; opening does not create an entry"}));
                                let changed = scalar(ui,row.track,&mut row.gain,0.0..=1.0,1.0,"Gain") | scalar(ui,row.track,&mut row.balance,-1.0..=1.0,0.0,"Stereo balance");
                                let mute=ui.checkbox(&mut row.mute,"M").on_hover_text(format!("Mute T{} · draft only",row.track.0));
                                let solo=ui.checkbox(&mut row.solo,"S").on_hover_text(format!("Solo T{} · draft only; even an unused track can exclude other tracks",row.track.0));
                                #[cfg(any(test,feature="ui-capture"))]
                                ui.ctx().data_mut(|d|{d.insert_temp(egui::Id::new(("track-mix-mute",row.track.0)),mute.rect);d.insert_temp(egui::Id::new(("track-mix-solo",row.track.0)),solo.rect);});
                                table_changed |= changed | mute.changed() | solo.changed();
                            }));
                        }
                    });
                    if table_changed {draft.neutral_reset=false;}
                    if draft.rows.is_empty() { ui.weak("No track rows. Empty neutral settings may still be reviewed."); }
                    ui.weak("M = mute · S = solo · balance is not equal-power pan");
                    if draft.defined_tracks > track_mix::MAX_TRACKS {self.meters.contributing(ui);}
                    if ui.button("Reload stored settings").clicked() { reload = true; }
                    if draft.neutral_reset { ui.colored_label(AQUA,"Neutral reset staged: the entire stored mix map will be empty only after Preview and Apply."); }
                    if let Some(r) = &self.ready {
                        let p = r.plan.preview(); let s = p.summary();
                        ui.separator();
                        ui.label(RichText::new(if current {"CURRENT REVIEW"} else {"PREVIOUS / STALE REVIEW"}).strong().color(AQUA));
                        ui.colored_label(if matches!(s.layout_status,MixLayoutStatus::Available{..}) { AQUA } else { WARN },layout_label(&s.layout_status));
                        ui.label(format!("{} changed stored tracks / {} configured / any solo {}",s.changed_tracks,s.configured_tracks,s.any_solo));
                        for row in &r.draft.rows {
                            let old=p.base().track_mix.get(&row.track);
                            let new=p.candidate().track_mix.get(&row.track);
                            if old!=new || old.zip(new).is_some_and(|(a,b)|a.gain.to_bits()!=b.gain.to_bits()||a.balance.to_bits()!=b.balance.to_bits()) {
                                let a=old.copied().unwrap_or_default();let b=new.copied().unwrap_or_default();
                                ui.label(RichText::new(format!("T{} · gain {} -> {} · balance {} -> {} · M {} -> {} · S {} -> {}",row.track.0,float_text(a.gain),float_text(b.gain),float_text(a.balance),float_text(b.balance),a.mute,b.mute,a.solo,b.solo)).small());
                            }
                        }
                    }
                    egui::CollapsingHeader::new("Exact settings, identity and full caveats").id_salt("track-mix-exact-details").show(ui,|ui| {
                        ui.label("Exact text is range-checked as f64 before shared f32 storage; no silent clamping. Stored signed-zero bits are preserved. Stereo balance is not equal-power pan/crossfeed. Solo includes unused tracks; mute wins.");
                        if let Some(r)=&self.ready {
                            let p=r.plan.preview();let s=p.summary();
                        for row in &r.draft.rows {
                            let old_stored = p.base().track_mix.get(&row.track);
                            let new_stored = p.candidate().track_mix.get(&row.track);
                            let old = old_stored.copied().unwrap_or_default();
                            let new = new_stored.copied().unwrap_or_default();
                            ui.label(RichText::new(format!("T{}  gain {} -> {}  balance {} -> {}  M {} -> {}  S {} -> {}  stored {} -> {}", row.track.0,
                                float_text(old.gain),float_text(new.gain),float_text(old.balance),float_text(new.balance),old.mute,new.mute,old.solo,new.solo,
                                old_stored.is_some(),new_stored.is_some())).monospace().small());
                            if let Some(item)=s.tracks.iter().find(|item|item.track==row.track) {
                                ui.label(format!("Effective L {} / R {}",float_text(item.coefficients.left),float_text(item.coefficients.right)));
                                if !item.shared_legacy_tracks.is_empty() {
                                    egui::CollapsingHeader::new(format!("Shared legacy group: {} peer tracks / {} instruments",item.shared_legacy_tracks.len(),item.shared_legacy_instruments.len()))
                                        .id_salt(("mix-legacy",row.track.0)).show(ui,|ui| {
                                            ui.label("Exact peer track IDs");
                                            for id in &item.shared_legacy_tracks {ui.monospace(format!("T{}",id.0));}
                                            ui.label("Exact shared legacy instrument IDs");
                                            egui::ScrollArea::vertical().id_salt(("mix-legacy-instruments",row.track.0)).max_height(100.0).show_rows(ui,18.0,item.shared_legacy_instruments.len(),|ui,range|{
                                                for index in range {ui.monospace(format!("I{}",item.shared_legacy_instruments[index].0));}
                                            });
                                        });
                                }
                            } else {
                                ui.weak("No compiled contribution in this model layout; stored Solo can still exclude other tracks.");
                            }
                        }
                            ui.label(format!("Plan {}",p.id())); ui.label(format!("Playing content SHA-256 {}",s.content_sha256));
                            for (&track,mix) in &p.candidate().track_mix {
                                ui.label(RichText::new(format!("Stored T{}: gain {} ({:08x}), balance {} ({:08x}), mute {}, solo {}",track.0,float_text(mix.gain),mix.gain.to_bits(),float_text(mix.balance),mix.balance.to_bits(),mix.mute,mix.solo)).monospace().small());
                            }
                            if p.candidate().track_mix.is_empty(){ui.label("Stored map: empty (all implicit neutral)");}
                            for warning in &s.warnings { ui.colored_label(WARN,warning); }

                        } else {
                            for (&track,mix) in &e.song.track_mix {
                                ui.label(RichText::new(format!("Stored T{}: gain {} ({:08x}), balance {} ({:08x}), mute {}, solo {}",track.0,float_text(mix.gain),mix.gain.to_bits(),float_text(mix.balance),mix.balance.to_bits(),mix.mute,mix.solo)).monospace().small());
                            }
                            if e.song.track_mix.is_empty(){ui.label("Stored map: empty (all implicit neutral)");}
                        }
                        if let Some(notice)=&self.notice{ui.label(notice);}
                        ui.separator();
                        ui.label(RichText::new("Runtime delivery · separate from document history").color(AQUA));
                        ui.label(&delivery.message);
                        if let Some(revision)=delivery.document_revision {ui.label(format!("Last explicit delivery attempt: document revision {revision}, requested live revision {:?}",delivery.requested_revision));}
                        ui.label(format!("Mailbox accepted r{} / first-block enqueued r{} · available {}",delivery.accepted_revision,delivery.enqueued_revision,delivery.available));
                        ui.weak("Enqueued means the first nonempty rendered block after applying the target was wholly enqueued; its 128-frame ramp may be incomplete. It is not callback consumption, device playback or heard output. Already queued samples are not retracted; Apply does not seek or flush. Unchanged targets do not restart ongoing ramps.");
                        egui::CollapsingHeader::new("Raw meter window and attribution").id_salt("track-mix-meter-details").show(ui,|ui|self.meters.details(ui));
                    });
                    if let Some(error) = &self.error {let r=ui.colored_label(Color32::LIGHT_RED,error);if self.reveal_error{r.scroll_to_me(Some(egui::Align::BOTTOM));self.reveal_error=false;}}

                });
                #[cfg(any(test,feature="ui-capture"))] { self.body_layout=Some((body.inner_rect,body.state.offset.y)); }
                #[cfg(not(any(test,feature="ui-capture")))] let _ = body;
                ui.separator();
                ui.label(RichText::new(if current {"READY · Apply reviewed settings"}else{"DRAFT · Preview to review changes"}).strong());
                if self.busy(){ui.colored_label(WARN,"Preparing review · no automatic Apply");}
                if !e.edit_mode{ui.weak("READ MODE · Apply unavailable");}
                if self.error.is_some(){ui.colored_label(Color32::LIGHT_RED,"Review refused · exact error in the scrollable body");}
                ui.label(RichText::new(compact_delivery(delivery)).small());
                ui.weak("Worker render-ahead · not heard level");
                // Stable action identities survive busy/read/error label reflow, so a
                // Close arriving with the first worker completion still revokes it.
                ui.scope_builder(egui::UiBuilder::new().id("track-mix-fixed-actions"), |ui| ui.horizontal(|ui| {
                    if ui.add_enabled(!self.busy(),egui::Button::new("Preview")).clicked(){preview=true;}
                    if ui.add_enabled(!self.busy()&&current&&e.edit_mode,egui::Button::new("Apply")).clicked(){apply=true;}
                    if ui.add_enabled(!self.busy(),egui::Button::new("Reset to neutral")).clicked(){reset=true;}
                    let close_response=ui.button("Close");
                    if close_response.clicked(){close=true;}
                    ui.weak("One Undo · live request separate");
                }));
            });
        self.draft = Some(draft);
        if self.draft.as_ref() != Some(&original) {
            self.invalidate("Draft changed; preview again. No audio or history change.");
        }
        if !open || close {
            self.close();
            return;
        }
        if !allowed || lost_focus {
            self.finish(e);
            return;
        }
        self.finish(e);
        if reload {
            self.invalidate("Stored settings reloaded; preview again.");
            self.draft = Some(Draft::from_song(&e.song));
        }
        if reset {
            self.invalidate("Neutral reset is a draft only; Preview and Apply are required.");
            if let Some(d) = &mut self.draft {
                d.reset_neutral();
            }
        }
        if preview {
            self.start(e);
        }
        if apply {
            self.apply(e);
        }
        ctx.memory_mut(|m| m.surrender_focus(crate::tracker_ui::grid_id()));
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub fn capture_state(&self, e: &Editor) -> serde_json::Value {
        serde_json::json!({"open":self.open,"busy":self.busy(),"completion_waiting":self.completed.is_some(),"current":self.current(e),"epoch":self.epoch,"error":self.error,"notice":self.notice,
            "draft":self.draft.as_ref().map(|d|serde_json::json!({"defined_tracks":d.defined_tracks,"neutral_reset":d.neutral_reset,"rows":d.rows.iter().map(|r|serde_json::json!({"track":r.track,"name":r.name,"gain_text":r.gain,"balance_text":r.balance,"mute":r.mute,"solo":r.solo,"stored":r.stored})).collect::<Vec<_>>(),"parsed_config":d.config().ok(),"parse_error":d.config().err()})),
            "preview":self.ready.as_ref().map(|r|{let p=r.plan.preview();serde_json::json!({"id":p.id(),"revision":r.plan.revision(),"config":p.config(),"summary":p.summary(),"candidate":p.candidate(),"base_json":serde_json::to_string(p.base()).unwrap(),"candidate_json":serde_json::to_string(p.candidate()).unwrap()})}),
            "meters":self.meters.capture(),
            "body":self.body_layout.map(|(r,o)|serde_json::json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y],"offset":o}))})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{song::Track, track_mix::TrackMix};
    #[test]
    fn agent_state_discloses_phase_and_rejects_unknown_actions() {
        let mut e = Editor::default();
        let mut ui = TrackMixUi::default();
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
        ui.agent_action(&mut e, None, json!({"action":"close"}))
            .unwrap();
    }
    fn editor() -> Editor {
        Editor {
            edit_mode: true,
            ..Default::default()
        }
    }
    fn wait(ui: &mut TrackMixUi, e: &Editor) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while ui.busy() {
            ui.poll();
            ui.finish(e);
            assert!(std::time::Instant::now() < until);
            thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn mix_exact_text_rejects_before_narrowing_and_preserves_signed_zero() {
        for text in [
            "NaN",
            "inf",
            "-inf",
            "1.0000000000000002",
            "-0.00000000001",
            "1e400",
            " 0.5",
            "",
        ] {
            assert!(number(text, 0.0, 1.0, "Gain").is_err(), "{text}");
        }
        assert!(number("-1.0000000000000002", -1.0, 1.0, "Balance").is_err());
        assert_eq!(
            number("-0", -1.0, 1.0, "Balance").unwrap().to_bits(),
            (-0.0f64).to_bits()
        );
        for value in [
            0.0f32,
            -0.0,
            1.0,
            f32::from_bits(1),
            f32::from_bits(0x3f7fffff),
            0.3,
            -0.3,
            -1.0,
        ] {
            assert_eq!(
                (float_text(value).parse::<f64>().unwrap() as f32).to_bits(),
                value.to_bits()
            );
        }
    }
    #[test]
    fn mix_opening_preserves_implicit_neutral_and_explicit_f32_bits() {
        let mut e = editor();
        assert!(Draft::from_song(&e.song)
            .config()
            .unwrap()
            .settings
            .is_empty());
        e.song.track_mix.insert(
            TrackId(1),
            TrackMix {
                gain: f32::from_bits(1),
                balance: -0.0,
                mute: false,
                solo: false,
            },
        );
        e.song.track_mix.insert(TrackId(2), TrackMix::default());
        let before = serde_json::to_vec(&e.song).unwrap();
        let draft = Draft::from_song(&e.song);
        let plan = track_mix::preview(&e.song, draft.config().unwrap()).unwrap();
        assert_eq!(serde_json::to_vec(plan.candidate()).unwrap(), before);
        let mut reset = draft;
        reset.reset_neutral();
        assert!(reset.config().unwrap().settings.is_empty());
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
    }
    #[test]
    fn mix_preview_commit_noop_retry_and_history_are_separate_from_live_delivery() {
        let mut e = editor();
        let cursor = (e.occurrence, e.track, e.row, e.column, e.field);
        let before = serde_json::to_vec(&e.song).unwrap();
        let mut ui = TrackMixUi::default();
        ui.open_for(&e);
        ui.draft.as_mut().unwrap().rows[0].gain = "0.5".into();
        ui.start(&e);
        wait(&mut ui, &e);
        assert!(ui.current(&e));
        assert!(
            matches!(
                ui.ready
                    .as_ref()
                    .unwrap()
                    .plan
                    .preview()
                    .summary()
                    .layout_status,
                MixLayoutStatus::Unavailable { .. }
            ),
            "Unassigned source is an explicit model status, not a fabricated playable fixture"
        );
        let candidate =
            serde_json::to_vec(ui.ready.as_ref().unwrap().plan.preview().candidate()).unwrap();
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        ui.apply(&mut e);
        assert!(ui.take_applied());
        assert!(!ui.take_applied());
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), candidate);
        assert_eq!((e.occurrence, e.track, e.row, e.column, e.field), cursor);
        let revision = e.revision();
        ui.start(&e);
        wait(&mut ui, &e);
        ui.apply(&mut e);
        assert!(
            ui.take_applied(),
            "Explicit no-op Apply can request a live retry"
        );
        assert_eq!(e.revision(), revision);
        e.undo();
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        e.redo();
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), candidate);
    }
    #[test]
    fn mix_failed_drafts_retain_previous_review_but_cannot_apply_it() {
        let mut e = editor();
        let before = serde_json::to_vec(&e.song).unwrap();
        let mut ui = TrackMixUi::default();
        ui.open_for(&e);
        ui.start(&e);
        wait(&mut ui, &e);
        let id = ui.ready.as_ref().unwrap().plan.preview().id().to_owned();
        for raw in ["1.0000000000000002", "NaN", "-1"] {
            ui.draft.as_mut().unwrap().rows[0].gain = raw.into();
            ui.invalidate("changed");
            ui.start(&e);
            assert_eq!(ui.ready.as_ref().unwrap().plan.preview().id(), id);
            assert!(!ui.current(&e));
            assert!(ui.error.is_some());
            ui.apply(&mut e);
            assert!(!ui.take_applied());
            assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        }
    }
    #[test]
    fn mix_late_preview_close_revision_and_equal_document_owner_are_inert() {
        for guard in 0..4 {
            let mut e = editor();
            let mut ui = TrackMixUi::default();
            ui.open_for(&e);
            ui.draft.as_mut().unwrap().rows[0].gain = "0.5".into();
            ui.start(&e);
            match guard {
                0 => ui.close(),
                1 => ui.cancel_authorization(),
                2 => e.set_timing(121.0, 4).unwrap(),
                _ => {
                    let same = e.song.clone();
                    e.replace_document(same).unwrap();
                }
            }
            let before = serde_json::to_vec(&e.song).unwrap();
            ui.sync(&e);
            wait(&mut ui, &e);
            assert!(!ui.current(&e));
            assert!(!ui.take_applied());
            assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        }
    }
    #[test]
    fn mix_native_threshold_retains_all_configured_entries_and_explicit_reset() {
        let mut e = editor();
        for id in 5..=65 {
            e.song.tracks.insert(
                TrackId(id),
                Track {
                    name: "wide 界 name".repeat(100),
                    note_columns: 1,
                },
            );
            e.song.track_order.push(TrackId(id));
        }
        assert_eq!(e.song.tracks.len(), 65);
        e.song.track_mix.insert(
            TrackId(65),
            TrackMix {
                gain: 0.25,
                balance: -0.0,
                mute: false,
                solo: true,
            },
        );
        let mut draft = Draft::from_song(&e.song);
        assert_eq!(draft.rows.len(), 1);
        assert_eq!(draft.rows[0].track, TrackId(65));
        let unchanged = track_mix::preview(&e.song, draft.config().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_vec(unchanged.candidate()).unwrap(),
            serde_json::to_vec(&e.song).unwrap()
        );
        draft.reset_neutral();
        assert!(draft.config().unwrap().settings.is_empty());
        assert!(e.song.track_mix.contains_key(&TrackId(65)));
        e.song.tracks.remove(&TrackId(65));
        e.song.track_order.retain(|t| t.0 != 65);
        e.song.track_mix.clear();
        assert_eq!(Draft::from_song(&e.song).rows.len(), 64);
    }
    fn visible(out: &egui::FullOutput, size: egui::Vec2, wanted: &str) -> bool {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        out.shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == wanted => {
                s.clip_rect.contains_rect(t.visual_bounding_rect())
                    && viewport.contains_rect(t.visual_bounding_rect())
                    && !t.galley.elided
            }
            _ => false,
        })
    }
    #[test]
    fn mix_sixty_four_rows_long_runtime_error_read_busy_keep_fixed_actions() {
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let mut e = editor();
            for id in 5..=64 {
                e.song.tracks.insert(
                    TrackId(id),
                    Track {
                        name: "界".repeat(4096),
                        note_columns: 1,
                    },
                );
                e.song.track_order.push(TrackId(id));
            }
            let last = e.song.tracks.remove(&TrackId(64)).unwrap();
            e.song.tracks.insert(TrackId(u64::MAX), last);
            *e.song.track_order.last_mut().unwrap() = TrackId(u64::MAX);
            let mut ui = TrackMixUi::default();
            ui.open_for(&e);
            ui.start(&e);
            wait(&mut ui, &e);
            ui.invalidate("Previous review retained");
            ui.error = Some("Long bounded validation message. ".repeat(70));
            let (_held, receiver) = mpsc::sync_channel(1);
            ui.pending = Some(Pending {
                epoch: ui.epoch,
                revision: e.revision(),
                draft: ui.draft.clone().unwrap(),
                receiver,
            });
            e.edit_mode = false;
            let delivery = Delivery {
                message: "Full runtime refusal stays scrollable. ".repeat(150),
                accepted_revision: u64::MAX,
                enqueued_revision: u64::MAX,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            crate::tracker_ui::install_theme(&ctx);
            for i in 0..10 {
                let out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(i as f64 / 60.0),
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| ui.show(ctx, &mut e, true, &delivery),
                );
                if i == 9 {
                    for label in ["Preview", "Apply", "Reset to neutral", "Close"] {
                        assert!(visible(&out, size, label), "{size:?} {label}");
                    }
                }
            }
            assert!(e.song.track_mix.is_empty());
        }
    }
}
