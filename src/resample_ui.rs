//! Control-side isolated-occurrence printing. A published file is not an undoable
//! document edit; its retained receipt deliberately outlives the current document.
use crate::{
    editor::{Editor, Field},
    resample::{
        self, ResampleConfig, ResamplePlan, ResamplePreview, ResampleReceipt, ResampleSummary,
        TailPolicy,
    },
    song::{SampleAsset, TrackId},
    tracker_ui::{grid_id, AQUA},
};
use eframe::egui::{self, Color32, RichText};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
};
const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const AMBER: Color32 = Color32::from_rgb(229, 185, 112);
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    occurrence: usize,
    track: TrackId,
    row: u32,
    column: u16,
    field: Field,
}
impl Context {
    fn of(e: &Editor) -> Self {
        Self {
            occurrence: e.occurrence,
            track: e.track,
            row: e.row,
            column: e.column,
            field: e.field,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Draft {
    context: Context,
    revision: u64,
    occurrence: String,
    name: String,
    root: String,
    tail: TailPolicy,
    output: String,
}
fn integer(text: &str, max: u64, label: &str) -> Result<u64, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "{label} must be an integer in 0..{max}; no clamping"
        ));
    }
    let value = text
        .parse::<u64>()
        .map_err(|_| format!("{label} must be in 0..{max}"))?;
    if value > max {
        return Err(format!("{label} must be in 0..{max}; no clamping"));
    }
    Ok(value)
}
impl Draft {
    fn config(&self) -> Result<ResampleConfig, String> {
        Ok(ResampleConfig {
            version: resample::VERSION,
            occurrence: integer(&self.occurrence, u64::from(u32::MAX), "Occurrence POS")? as u32,
            name: self.name.clone(),
            root_note: integer(&self.root, 127, "Native root MIDI")? as u8,
            tail: self.tail,
            output: PathBuf::from(&self.output),
        })
    }
}
struct Cached<T> {
    revision: u64,
    context: Context,
    value: Arc<T>,
}
enum Completed {
    Prepared(Arc<ResamplePlan>),
    Printed(Arc<ResamplePreview>),
    Verified(Arc<ResamplePreview>),
}
struct Pending {
    epoch: u64,
    revision: u64,
    context: Context,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<Completed, String>>,
    label: &'static str,
}
struct Retained {
    config: ResampleConfig,
    receipt: ResampleReceipt,
    asset: SampleAsset,
    registered_once: bool,
}
#[derive(Default)]
pub(crate) struct ResampleUi {
    pub open: bool,
    draft: Option<Draft>,
    pending: Option<Pending>,
    prepared: Option<Cached<ResamplePlan>>,
    printed: Option<Cached<ResamplePreview>>,
    verified: Option<Cached<ResamplePreview>>,
    retained: Option<Retained>,
    epoch: u64,
    commit_authorized: bool,
    agent_registration_requested: bool,
    error: Option<String>,
    notice: String,
    event: Option<String>,
}
#[cfg(test)]
mod agent_tests {
    use super::*;
    #[test]
    fn verified_commit_requires_explicit_agent_registration_readiness() {
        let mut e = Editor::default();
        let mut ui = ResampleUi::default();
        assert_eq!(ui.agent_state(&e)["phase"], "closed");
        assert!(ui
            .agent_action(&mut e, None, json!({"action":"commit_verified"}))
            .is_err());
        ui.open = true;
        ui.agent_registration_requested = true;
        ui.finish_registration(&mut e, false);
        ui.cancel_commit();
        assert!(ui.agent_registration_requested);
    }
}
impl ResampleUi {
    pub(crate) fn agent_state(&self, e: &Editor) -> Value {
        json!({"component":"resample","open":self.open,"busy":self.busy(),"phase":if self.busy(){"busy"}else if self.verified.is_some(){"verified_ready"}else if self.printed.as_ref().is_some_and(|_|self.current_print(e)){"printed_ready"}else if self.prepared.as_ref().is_some_and(|_|self.current_plan(e)){"prepared_ready"}else if self.open{"draft"}else{"closed"},"actions":["open","set_draft","prepare","poll","print","register","commit_verified","cancel_task","cancel_registration","forget_receipt","close","reset"],
        "draft_fields":{"occurrence":"integer text 0..u32::MAX","name":"string","root":"integer text 0..127","tail":"MusicalOnly|ReleaseTail","output":"absolute .wav path within session base"},
        "draft":self.draft.as_ref().map(|d|json!({"revision":d.revision,"cursor":{"occurrence":d.context.occurrence,"track":d.context.track,"row":d.context.row,"column":d.context.column,"field":d.context.field},"occurrence":d.occurrence,"name":d.name,"root":d.root,"tail":d.tail,"output":d.output,"parsed":d.config().ok()})),
        "pending":self.pending.as_ref().map(|p|json!({"label":p.label,"revision":p.revision,"cancel_requested":p.cancel.load(Ordering::Acquire)})),"prepared":self.prepared.as_ref().map(|p|json!({"revision":p.revision,"context_current":self.current_plan(e),"config":p.value.config(),"summary":p.value.summary()})),"printed":self.printed.as_ref().map(|p|json!({"revision":p.revision,"current":self.current_print(e),"config":p.value.config(),"summary":p.value.summary(),"receipt":p.value.receipt()})),"verified":self.verified.as_ref().map(|p|json!({"revision":p.revision,"current":self.current_print(e),"ready_for_agent_commit":self.agent_registration_requested,"summary":p.value.summary(),"receipt":p.value.receipt()})),"retained":self.retained.as_ref().map(|r|json!({"config":r.config,"receipt":r.receipt,"asset":r.asset,"registered_once":r.registered_once})),"registration_authorized":self.commit_authorized,"agent_registration_requested":self.agent_registration_requested,"error":self.error,"notice":self.notice,"event":self.event,"edit_mode":e.edit_mode,"publication_supported":resample::PUBLICATION_SUPPORTED})
    }

    pub(crate) fn agent_action(
        &mut self,
        e: &mut Editor,
        base: Option<&std::path::Path>,
        action: Value,
    ) -> Result<Value, String> {
        let object = action.as_object().ok_or("Action must be a JSON object")?;
        let verb = object
            .get("action")
            .and_then(Value::as_str)
            .ok_or("Action requires string action")?;
        match verb {
            "open" => {
                agent_fields(object, &["action", "output"])?;
                let base = base.ok_or("Resampling needs the current project directory as base")?;
                let output = object
                    .get("output")
                    .and_then(Value::as_str)
                    .map(|v| agent_output(base, v))
                    .transpose()?
                    .unwrap_or_else(|| base.join("resampled.leviathan.wav"));
                self.open_for(e, output);
            }
            "close" | "cancel" => {
                agent_fields(object, &["action"])?;
                self.hide();
            }
            "reset" => {
                agent_fields(object, &["action"])?;
                self.reset_document();
            }
            "set_draft" => {
                agent_fields(object, &["action", "values"])?;
                if self.busy() || self.retained.is_some() {
                    return Err("Wait for pending work and explicitly forget the retained receipt before editing options".into());
                }
                let values = object
                    .get("values")
                    .and_then(Value::as_object)
                    .ok_or("set_draft requires object values")?;
                let d = self.draft.as_mut().ok_or("Open resampling first")?;
                for (field, v) in values {
                    match field.as_str() {
                        "occurrence" => {
                            d.occurrence = v.as_str().ok_or("occurrence must remain text")?.into()
                        }
                        "name" => d.name = v.as_str().ok_or("name must be text")?.into(),
                        "root" => d.root = v.as_str().ok_or("root must remain text")?.into(),
                        "tail" => {
                            d.tail = match v.as_str() {
                                Some("MusicalOnly") => TailPolicy::MusicalOnly,
                                Some("ReleaseTail") => TailPolicy::ReleaseTail,
                                _ => return Err("tail must be MusicalOnly or ReleaseTail".into()),
                            }
                        }
                        "output" => {
                            d.output = agent_output(
                                base.ok_or(
                                    "Resampling needs the current project directory as base",
                                )?,
                                v.as_str().ok_or("output must be a path string")?,
                            )?
                            .display()
                            .to_string()
                        }
                        _ => return Err(format!("Unknown resample draft field `{field}`")),
                    }
                }
                self.invalidate();
                self.error = None;
                self.notice = "Options changed; prepare a new resample plan.".into();
            }
            "prepare" => {
                agent_fields(object, &["action"])?;
                let r = self.prepare(e);
                self.result(r);
            }
            "poll" => {
                agent_fields(object, &["action"])?;
                self.sync(e);
                self.poll();
            }
            "print" => {
                agent_fields(object, &["action"])?;
                let r = self.print(e);
                self.result(r);
            }
            "register" => {
                agent_fields(object, &["action"])?;
                let r = self.register_for_agent(e);
                self.result(r);
            }
            "commit_verified" => {
                agent_fields(object, &["action"])?;
                self.commit_verified_for_agent(e)?;
            }
            "cancel_task" => {
                agent_fields(object, &["action"])?;
                self.cancel_task();
            }
            "cancel_registration" => {
                agent_fields(object, &["action"])?;
                self.agent_registration_requested = false;
                self.cancel_commit();
            }
            "forget_receipt" => {
                agent_fields(object, &["action"])?;
                let r = self.forget();
                self.result(r);
            }
            _ => return Err(format!("Unknown resample action `{verb}`")),
        }
        Ok(self.agent_state(e))
    }
    pub fn open_for(&mut self, editor: &Editor, output: PathBuf) {
        if self.draft.is_none() {
            self.draft = Some(Draft {
                context: Context::of(editor),
                revision: editor.revision(),
                occurrence: editor.occurrence.to_string(),
                name: "Resampled occurrence".into(),
                root: "60".into(),
                tail: TailPolicy::ReleaseTail,
                output: output.display().to_string(),
            });
        }
        self.open = true;
        self.sync(editor);
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn take_event(&mut self) -> Option<String> {
        self.event.take()
    }
    pub fn cancel_commit(&mut self) {
        self.commit_authorized = false;
        if !self.agent_registration_requested {
            self.verified = None;
        }
    }
    fn cancel_agent_registration(&mut self) {
        self.agent_registration_requested = false;
        self.verified = None;
    }
    /// Hiding the inspector does not cancel a file publication. It does cancel
    /// registration intent; the result/receipt remains available on reopening.
    pub fn hide(&mut self) {
        self.open = false;
        self.cancel_commit();
        self.cancel_agent_registration();
    }
    fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(p) = &self.pending {
            p.cancel.store(true, Ordering::Release);
        }
        self.prepared = None;
        self.printed = None;
        self.verified = None;
        self.commit_authorized = false;
        self.agent_registration_requested = false;
    }
    pub fn reset_document(&mut self) {
        self.invalidate();
        self.draft = None;
        self.open = false;
        self.error = None;
        self.notice =
            "Document replaced. Any published file remains; its receipt is retained separately."
                .into();
    }
    fn sync(&mut self, e: &Editor) {
        let changed = self
            .draft
            .as_ref()
            .is_some_and(|d| d.revision != e.revision() || d.context != Context::of(e));
        if changed {
            self.invalidate();
            if let Some(d) = &mut self.draft {
                d.revision = e.revision();
                d.context = Context::of(e);
            }
            self.error = None;
            self.notice="Document or cursor changed. Review the explicit target again; previous registration authority was discarded. Published files remain.".into();
        }
    }
    pub fn cancel_task(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| !p.cancel.load(Ordering::Acquire))
        {
            self.invalidate();
            self.notice="Cancellation requested before publication. If publication already won the race, the file and receipt will remain; nothing is registered.".into();
        }
    }
    fn current_plan(&self, e: &Editor) -> bool {
        let (Some(d), Some(p)) = (&self.draft, &self.prepared) else {
            return false;
        };
        p.revision == e.revision()
            && p.context == Context::of(e)
            && p.value.base() == &e.song
            && d.config().is_ok_and(|c| &c == p.value.config())
    }
    fn current_print(&self, e: &Editor) -> bool {
        let (Some(d), Some(p)) = (&self.draft, &self.printed) else {
            return false;
        };
        p.revision == e.revision()
            && p.context == Context::of(e)
            && p.value.base() == &e.song
            && d.config().is_ok_and(|c| &c == p.value.config())
    }
    fn spawn(
        &mut self,
        revision: u64,
        context: Context,
        label: &'static str,
        work: impl FnOnce(Arc<AtomicBool>) -> Result<Completed, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("A resample task is still running; wait for its result".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, rx) = mpsc::sync_channel(1);
        thread::Builder::new().name("leviathan-resample".into()).spawn(move||{
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||work(worker_cancel))).unwrap_or_else(|_|Err("Resample worker panicked. Inspect the declared output path; no automatic document registration occurred.".into()));
            let _=tx.send(result);
        }).map_err(|e|format!("Could not start resample worker: {e}"))?;
        self.pending = Some(Pending {
            epoch: self.epoch,
            revision,
            context,
            cancel,
            receiver: rx,
            label,
        });
        self.error = None;
        self.notice.clear();
        Ok(())
    }
    fn prepare(&mut self, e: &Editor) -> Result<(), String> {
        if self.retained.is_some() {
            return Err("Forget the retained receipt explicitly before preparing another file; the old file stays on disk".into());
        }
        let d = self.draft.as_ref().ok_or("Open resampling first")?;
        let config = d.config()?;
        let revision = e.revision();
        let context = Context::of(e);
        let song = e.song.clone();
        self.prepared = None;
        self.printed = None;
        self.spawn(
            revision,
            context,
            "Preparing isolated occurrence…",
            move |_| resample::prepare(&song, config).map(|p| Completed::Prepared(Arc::new(p))),
        )
    }
    fn print(&mut self, e: &Editor) -> Result<(), String> {
        if !resample::PUBLICATION_SUPPORTED {
            return Err(
                "WAV publication v1 requires Unix file identity; unavailable on this platform"
                    .into(),
            );
        }
        if self.retained.is_some() {
            return Err("A published artifact receipt must be explicitly forgotten before another publication".into());
        }
        if !self.current_plan(e) {
            return Err("Preview is stale; prepare it again before printing".into());
        }
        let p = self.prepared.as_ref().unwrap();
        let plan = Arc::clone(&p.value);
        self.spawn(
            p.revision,
            p.context.clone(),
            "Printing new WAV…",
            move |cancel| {
                resample::render(&plan, |_| Ok(()), || cancel.load(Ordering::Acquire))
                    .map(|p| Completed::Printed(Arc::new(p)))
            },
        )
    }
    fn register(&mut self, e: &Editor) -> Result<(), String> {
        if !e.edit_mode {
            return Err("Enable Edit mode to register a published asset".into());
        }
        if !self.current_print(e) {
            return Err("Registration is stale. The published WAV remains; no document changes were applied".into());
        }
        let p = self.printed.as_ref().unwrap();
        let preview = Arc::clone(&p.value);
        self.spawn(
            p.revision,
            p.context.clone(),
            "Checking artifact + prospective project sources/storage…",
            move |_| {
                resample::verify_artifact(&preview, |_| Ok(()))?;
                Ok(Completed::Verified(preview))
            },
        )?;
        self.commit_authorized = true;
        Ok(())
    }
    fn register_for_agent(&mut self, e: &Editor) -> Result<(), String> {
        if !e.edit_mode {
            return Err("Enable Edit mode to register a published asset".into());
        }
        if !self.current_print(e) {
            return Err("Registration is stale. The published WAV remains; no document changes were applied".into());
        }
        let p = self.printed.as_ref().unwrap();
        let preview = Arc::clone(&p.value);
        self.spawn(
            p.revision,
            p.context.clone(),
            "Checking artifact + prospective project sources/storage…",
            move |_| {
                resample::verify_artifact(&preview, |_| Ok(()))?;
                Ok(Completed::Verified(preview))
            },
        )?;
        self.agent_registration_requested = true;
        Ok(())
    }
    fn commit_verified_for_agent(&mut self, e: &mut Editor) -> Result<(), String> {
        if self.busy() {
            return Err("Poll until artifact verification is complete before committing".into());
        }
        if !self.agent_registration_requested {
            return Err("No agent registration request is awaiting commit".into());
        }
        let p = self
            .verified
            .take()
            .ok_or("No verified registration is ready")?;
        self.agent_registration_requested = false;
        if !self.open || !e.edit_mode || !self.current_print(e) {
            return Err("Verified registration is stale or Edit Mode is off; published WAV remains and no document edit was applied".into());
        }
        e.commit_resample_preview(p.revision, &p.value)?;
        if let Some(r) = &mut self.retained {
            r.registered_once = true;
        }
        if let Some(d) = &mut self.draft {
            d.revision = e.revision();
        }
        self.prepared = None;
        self.printed = None;
        self.error = None;
        self.notice = "Registered source + full region + new instrument in one undo. Existing notes/cursor unchanged. Save the Song separately; Undo never deletes the WAV.".into();
        self.event = Some(self.notice.clone());
        Ok(())
    }
    pub fn poll(&mut self) {
        let Some(p) = &self.pending else {
            return;
        };
        let result=match p.receiver.try_recv(){Ok(r)=>r,Err(TryRecvError::Empty)=>return,Err(TryRecvError::Disconnected)=>Err("Resample worker disconnected. No automatic registration; inspect the declared output path.".into())};
        let p = self.pending.take().unwrap();
        let current = p.epoch == self.epoch && self.draft.is_some();
        match result {
            Ok(Completed::Printed(value)) => {
                // This branch is deliberately before the generation guard: once
                // published, a late completion must never silently lose its receipt.
                self.retained = Some(Retained {
                    config: value.config().clone(),
                    receipt: value.receipt().clone(),
                    asset: value.asset().clone(),
                    registered_once: false,
                });
                self.event = Some(format!(
                    "Published WAV retained: {}. Not registered in the Song.",
                    value.summary().output.display()
                ));
                self.notice=if current{"WAV published. Review the receipt, then explicitly register it (one undo)."}else{"WAV publication completed after cancellation/document change. File retained; registration discarded."}.into();
                if current {
                    self.printed = Some(Cached {
                        revision: p.revision,
                        context: p.context,
                        value,
                    });
                    self.prepared = None;
                }
            }
            Ok(Completed::Prepared(value)) if current => {
                self.prepared = Some(Cached {
                    revision: p.revision,
                    context: p.context,
                    value,
                });
                self.notice="Preview only: no file or document has been changed. Print publishes a new external WAV.".into();
            }
            Ok(Completed::Verified(value))
                if current && (self.commit_authorized || self.agent_registration_requested) =>
            {
                self.verified = Some(Cached {
                    revision: p.revision,
                    context: p.context,
                    value,
                });
            }
            Ok(_) => {}
            Err(error) => {
                self.commit_authorized = false;
                self.agent_registration_requested = false;
                self.error = Some(error.clone());
                self.event = Some(error);
            }
        }
    }
    fn finish_registration(&mut self, e: &mut Editor, enabled: bool) {
        // Agent registration is completed only by its explicit commit_verified action.
        // A GUI frame must not consume or cancel that readiness based on keyboard focus.
        if self.agent_registration_requested {
            return;
        }
        let Some(p) = self.verified.take() else {
            return;
        };
        let authorized = self.commit_authorized;
        self.commit_authorized = false;
        if !authorized || !enabled || !self.open || !e.edit_mode || !self.current_print(e) {
            self.error=Some("Registration canceled or stale; published WAV retained and no document edit applied".into());
            return;
        }
        match e.commit_resample_preview(p.revision, &p.value) {
            Ok(()) => {
                if let Some(r) = &mut self.retained {
                    r.registered_once = true;
                }
                if let Some(d) = &mut self.draft {
                    d.revision = e.revision();
                }
                self.prepared = None;
                self.printed = None;
                self.error = None;
                self.notice="Registered source + full region + new instrument in one undo. Existing notes/cursor unchanged. Save the Song separately; Undo never deletes the WAV.".into();
                self.event = Some(self.notice.clone());
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn forget(&mut self) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the tracked task before forgetting its receipt".into());
        }
        let path = self
            .retained
            .as_ref()
            .map(|r| r.receipt.summary.output.clone());
        self.invalidate();
        self.retained = None;
        self.error = None;
        self.notice = path.map_or(String::new(), |p| {
            format!("Receipt forgotten only. File remains at {}", p.display())
        });
        Ok(())
    }
    fn result(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.notice.clear();
            self.error = Some(error);
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, e: &mut Editor) {
        if !self.open {
            return;
        }
        self.sync(e);
        let enabled = ui.is_enabled();
        self.finish_registration(e, enabled);
        ui.memory_mut(|m| m.surrender_focus(grid_id()));
        let mut open = true;
        let mut close = false;
        egui::Window::new("Resample occurrence").id(egui::Id::new("resample-occurrence")).default_pos(egui::pos2(180.0,100.0)).default_width(680.0).min_width(560.0).resizable(false).collapsible(false).open(&mut open).show(ui.ctx(),|ui|{
            if !enabled{ui.disable();}
            ui.label(RichText::new("ISOLATED OCCURRENCE · ALL TRACKS / MUTES · NEW WAV").small().color(AQUA));
            ui.label(RichText::new("Cold start: no incoming parameter memory, voices or tails; phrases/RR restart. Not an arrangement crop. No audition.").small().color(AMBER));
            if !resample::PUBLICATION_SUPPORTED { ui.colored_label(AMBER,"Printing unavailable: v1 publication requires Unix file identity. Preparation remains available."); }
            egui::ScrollArea::vertical().id_salt("resample-body").max_height((ui.ctx().content_rect().height()-380.0).max(150.0)).auto_shrink([false,false]).show(ui,|ui|{
                let before=self.draft.clone();let editable=!self.busy() && self.retained.is_none();
                if let Some(d)=&mut self.draft{
                    ui.add_enabled_ui(editable,|ui|{
                        ui.horizontal_wrapped(|ui|{number(ui,"Occurrence POS (0-based)",&mut d.occurrence);number(ui,"Native root MIDI",&mut d.root);});
                        ui.horizontal(|ui|{ui.label("New asset / instrument name");ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(280.0).id_salt("resample-name"));});
                        ui.label("Absolute .wav in an existing real directory · no overwrite");
                        ui.add(egui::TextEdit::singleline(&mut d.output).desired_width(f32::INFINITY).id_salt("resample-output"));
                        ui.horizontal_wrapped(|ui|{ui.label("End policy");ui.selectable_value(&mut d.tail,TailPolicy::MusicalOnly,"Musical frames only").on_hover_text("Truncates at the musical boundary and may click; no added fade.");ui.selectable_value(&mut d.tail,TailPolicy::ReleaseTail,"+ 500 ms release tail");});
                        ui.label(RichText::new("Musical-only may click. The optional 500 ms tail is baked into the full new region.").small().color(AMBER));
                    });
                }
                if self.draft!=before{self.invalidate();self.error=None;self.notice="Options changed; prepare a new preview. No implicit clamping or file replacement.".into();}
                ui.label(RichText::new("Native root is declared, not detected pitch. Pre-master ×2 is not normalization. Neutral replay must hold the whole printed region; early OFF/replacement/SongEnd changes baked tails. No limiter: float peaks above ±1 may clip externally.").small().color(AMBER));
                ui.label(RichText::new("Used legacy sample paths must first be imported/registered as regions in Samples. Preparation does not read/decode source files.").small().color(MUTED));
                if let Some(p)=&self.prepared{summary(ui,p.value.summary());
                    egui::CollapsingHeader::new("Used registered sources · stored identities, not freshly verified").show(ui,|ui|{
                        egui::ScrollArea::vertical().id_salt("resample-planned-sources").max_height(130.0).show_rows(ui,18.0,p.value.summary().source_samples.len(),|ui,range|{for index in range{let id=p.value.summary().source_samples[index];let asset=&p.value.base().samples[&id];ui.label(RichText::new(format!("S{} · {}",id.0,asset.sha256)).small().monospace()).on_hover_text(asset.path.display().to_string());}});
                    });warnings(ui,&p.value.summary().warnings);
                }
                if let Some(r)=&self.retained{
                    ui.separator();ui.label(RichText::new("PUBLISHED ARTIFACT · RECEIPT RETAINED").color(AQUA));
                    summary(ui,&r.receipt.summary);
                    ui.label(RichText::new(r.receipt.summary.output.display().to_string()).monospace()).on_hover_text(r.receipt.summary.output.display().to_string());
                    ui.label(RichText::new(format!("SHA-256 {}",r.receipt.sha256)).small().monospace()).on_hover_text(&r.receipt.sha256);
                    ui.label(format!("{} encoded bytes · Engine peak {:.6} · printed asset peak {:.6}",r.receipt.encoded_bytes,r.receipt.post_master_peak,r.receipt.asset_peak)).on_hover_text(format!("Exact post-master peak {:?}; asset peak {:?}; post-master energy {:?}",r.receipt.post_master_peak,r.receipt.asset_peak,r.receipt.post_master_energy));
                    if r.receipt.asset_peak>1.0{ui.colored_label(AMBER,"Printed peak exceeds ±1: float WAV retained without normalization; external playback may clip.");}
                    let summary=&r.receipt.summary;
                    let source_present=e.song.samples.get(&summary.sample)==Some(&r.asset);
                    let region_link=e.song.regions.get(&summary.region).is_some_and(|region|region.sample==summary.sample);
                    let instrument_link=e.song.instruments.get(&summary.instrument).is_some_and(|instrument|instrument.region==Some(summary.region));
                    ui.label(RichText::new(format!("Current document links: S{} {} · R{} {} · I{} {}. Not a current filesystem verification.",summary.sample.0,if source_present{"matches"}else{"absent/changed"},summary.region.0,if region_link{"linked"}else{"absent/changed"},summary.instrument.0,if instrument_link{"linked"}else{"absent/changed"})).small().color(AMBER));
                    ui.label(RichText::new(if r.registered_once{"Artifact and project storage were checked for registration. Undo/Redo affects references only; external deletion/tampering is not repaired."}else{"Register checks the output plus all prospective project sources/storage off-thread. Unused missing/tampered sources can block registration; the printed file remains."}).small().color(MUTED));
                    egui::CollapsingHeader::new("Captured source identities / exact receipt").show(ui,|ui|{
                        ui.label(format!("Base Song SHA-256 {}",r.receipt.base_sha256));
                        ui.label(format!("Recipe v{} · POS{} · root MIDI{} · {:?}",r.config.version,r.config.occurrence,r.config.root_note,r.config.tail));
                        egui::ScrollArea::vertical().id_salt("resample-source-identities").max_height(130.0).show_rows(ui,18.0,r.receipt.source_identities.len(),|ui,range|{for index in range{let s=&r.receipt.source_identities[index];ui.label(RichText::new(format!("S{} · {}",s.sample.0,s.asset.sha256)).small().monospace()).on_hover_text(s.asset.path.display().to_string());}});
                    });warnings(ui,&r.receipt.summary.warnings);
                }
                if self.error.is_some() || !self.notice.is_empty() { egui::CollapsingHeader::new("Full task status / error").show(ui,|ui|{if let Some(error)=&self.error{ui.colored_label(Color32::LIGHT_RED,error);}
                    if !self.notice.is_empty(){ui.label(&self.notice);}}); }
                ui.label(RichText::new("Print publishes a file outside Song history. Cancel only stops before publication. Close/New/stale results/Undo never delete a published file. Forget removes this receipt only; import a retained file through Samples if registration was discarded.").small().color(AMBER));
            });
            ui.separator();
            if let Some(p)=&self.pending{ui.label(RichText::new(p.label).color(AQUA));}
            else if self.current_print(e){ui.label(RichText::new("PRINTED · awaiting explicit verified registration").color(AQUA));}
            else if self.current_plan(e){ui.label(RichText::new("PREPARED · no file written · ready to print").color(AQUA));}
            else if self.retained.is_some(){ui.label(RichText::new("RECEIPT RETAINED · Forget explicitly before another print; WAV stays on disk").color(AMBER));}
            if let Some(r)=&self.retained{footer_summary(ui,&r.receipt.summary,"Artifact");}
            else if let Some(p)=&self.prepared{footer_summary(ui,p.value.summary(),"Preview");}
            if let Some(error)=&self.error{status_line(ui,error,Color32::LIGHT_RED);}
            if !self.notice.is_empty(){status_line(ui,&self.notice,MUTED);}
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!self.busy()&&self.retained.is_none(),egui::Button::new("Prepare preview")).clicked(){let r=self.prepare(e);self.result(r);}
                if ui.add_enabled(resample::PUBLICATION_SUPPORTED&&!self.busy()&&self.retained.is_none()&&self.current_plan(e),egui::Button::new("Print new WAV")).clicked(){let r=self.print(e);self.result(r);}
                if ui.add_enabled(!self.busy()&&e.edit_mode&&self.current_print(e),egui::Button::new("Register + new instrument")).clicked(){let r=self.register(e);self.result(r);}
            });
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(self.busy(),egui::Button::new("Cancel task")).clicked(){self.cancel_task();}
                if ui.add_enabled(!self.busy()&&self.retained.is_some(),egui::Button::new("Forget receipt (file stays)")).clicked(){let r=self.forget();self.result(r);}
                if ui.button("Close").clicked(){close=true;}
                if !e.edit_mode{ui.label(RichText::new("READ: printing allowed; registration requires Edit mode").small().color(AMBER));}
            });
        });
        if !open || close {
            self.hide();
        }
    }
    #[cfg(any(all(test, unix), feature = "ui-capture"))]
    pub(crate) fn capture_state(&self, e: &Editor) -> serde_json::Value {
        serde_json::json!({"open":self.open,"busy":self.busy(),"pending":self.pending.as_ref().map(|p|serde_json::json!({"label":p.label,"cancel_requested":p.cancel.load(Ordering::Acquire)})),"registration_authorized":self.commit_authorized,"error":self.error,"notice":self.notice,"draft":self.draft.as_ref().map(|d|serde_json::json!({"occurrence":d.occurrence,"root":d.root,"name":d.name,"output":d.output,"tail":d.tail})),"prepared":self.prepared.as_ref().map(|p|serde_json::json!({"id":p.value.id(),"config":p.value.config(),"summary":p.value.summary(),"current":self.current_plan(e)})),"printed":self.printed.as_ref().map(|p|serde_json::json!({"id":p.value.id(),"config":p.value.config(),"summary":p.value.summary(),"receipt":p.value.receipt(),"candidate":p.value.candidate(),"current":self.current_print(e)})),"retained":self.retained.as_ref().map(|r|serde_json::json!({"config":r.config,"receipt":r.receipt,"asset":r.asset,"registered_once":r.registered_once}))})
    }
}
fn agent_fields(object: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unknown action field `{key}`"));
    }
    Ok(())
}
fn agent_output(base: &std::path::Path, value: &str) -> Result<PathBuf, String> {
    let entered = PathBuf::from(value);
    let output = if entered.is_absolute() {
        entered
    } else {
        base.join(entered)
    };
    if !output.is_absolute()
        || !output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        || output.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("output must be an absolute .wav path under the project base, without . or .. components".into());
    }
    let base = base
        .canonicalize()
        .map_err(|e| format!("Cannot resolve project base: {e}"))?;
    let parent = output
        .parent()
        .ok_or("output has no parent directory")?
        .canonicalize()
        .map_err(|e| format!("Cannot resolve output directory: {e}"))?;
    if !parent.starts_with(&base) {
        return Err("output directory must stay under the project base".into());
    }
    Ok(output)
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(68.0)
            .id(egui::Id::new(("resample-number", label))),
    );
}
fn summary(ui: &mut egui::Ui, s: &ResampleSummary) {
    ui.label(
        RichText::new(format!(
            "POS{} / P{} · {} BPM · {} LPB · {} source(s) · {} starts",
            s.occurrence,
            s.pattern.0,
            s.bpm,
            s.lines_per_beat,
            s.source_samples.len(),
            s.starts
        ))
        .monospace()
        .color(AQUA),
    );
    ui.label(format!(
        "{} musical + {} tail = {} frames · {} Hz / {} ch · new S{} / R{} / I{}",
        s.musical_frames,
        s.tail_frames,
        s.printed_frames,
        s.sample_rate,
        s.channels,
        s.sample.0,
        s.region.0,
        s.instrument.0
    ));
    ui.label(
        RichText::new(format!(
            "{} · {} bounded voice-frames",
            s.algorithm, s.voice_frames
        ))
        .small()
        .color(MUTED),
    );
}
fn warnings(ui: &mut egui::Ui, items: &[String]) {
    egui::CollapsingHeader::new(format!(
        "Full shared preparation / playback details ({})",
        items.len()
    ))
    .show(ui, |ui| {
        for warning in items {
            ui.label(RichText::new(warning).small().color(AMBER));
        }
    });
}

fn status_line(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).small().color(color)).truncate())
        .on_hover_ui(|ui| {
            ui.set_max_width(600.0);
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    ui.label(text);
                });
        });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{
        sample_asset,
        song::{Cell, InstrumentId, Note, PatternId, RegionId, SampleId, SampleRegion},
    };
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Files(PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Files, Editor, ResampleUi) {
        let dir = std::env::temp_dir().join(format!(
            "resample-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let source = dir.join("source.wav");
        let mut wav = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..4800 {
            wav.write_sample(if i % 100 < 50 { 0.125_f32 } else { -0.125_f32 })
                .unwrap();
        }
        wav.finalize().unwrap();
        let imported = sample_asset::import(&source).unwrap();
        let mut editor = Editor::default();
        let mut song = editor.song.clone();
        song.order.truncate(1);
        for p in song.patterns.values_mut() {
            p.rows = 4;
            p.tracks.clear();
            p.track_aliases.clear();
        }
        song.register_sample(SampleId(1), imported.asset).unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                name: "Owned source".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 4800,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        song.assign_region(InstrumentId(1), Some(RegionId(1)))
            .unwrap();
        song.patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(0)
            .or_default()
            .insert(
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(0.5),
                    ..Default::default()
                },
            );
        editor.replace_document(song).unwrap();
        editor.edit_mode = true;
        let mut ui = ResampleUi::default();
        ui.open_for(&editor, dir.join("printed.wav"));
        (Files(dir), editor, ui)
    }
    fn wait(ui: &mut ResampleUi) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while ui.busy() {
            ui.poll();
            assert!(std::time::Instant::now() < end, "worker timeout");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn printed(ui: &mut ResampleUi, e: &Editor) {
        ui.prepare(e).unwrap();
        wait(ui);
        assert!(ui.current_plan(e), "{:?}", ui.error);
        ui.print(e).unwrap();
        wait(ui);
        assert!(ui.current_print(e), "{:?}", ui.error);
    }
    #[test]
    fn exact_resample_numeric_inputs_reject_without_clamping() {
        let (_files, e, mut ui) = fixture();
        let original = e.song.clone();
        for value in [
            "-1",
            "128",
            "1.5",
            "NaN",
            "1e1",
            "+1",
            "",
            "18446744073709551616",
        ] {
            ui.draft.as_mut().unwrap().root = value.into();
            assert!(ui.prepare(&e).is_err(), "{value}");
            assert_eq!(ui.draft.as_ref().unwrap().root, value);
            assert!(!ui.busy());
            assert_eq!(e.song, original);
        }
        ui.draft.as_mut().unwrap().root = "127".into();
        assert_eq!(ui.draft.as_ref().unwrap().config().unwrap().root_note, 127);
        ui.draft.as_mut().unwrap().occurrence = "4294967296".into();
        assert!(ui.prepare(&e).is_err());
        ui.draft.as_mut().unwrap().occurrence = "0".into();
        ui.draft.as_mut().unwrap().output = "relative.wav".into();
        ui.prepare(&e).unwrap();
        wait(&mut ui);
        assert!(ui.error.as_ref().unwrap().contains("absolute"));
        assert_eq!(e.song, original);
    }
    #[test]
    fn prepare_print_register_are_separate_and_undo_only_removes_references() {
        let (files, mut e, mut ui) = fixture();
        let before = e.song.clone();
        let revision = e.revision();
        let cursor = Context::of(&e);
        ui.prepare(&e).unwrap();
        wait(&mut ui);
        assert!(ui.current_plan(&e));
        assert!(!files.0.join("printed.wav").exists());
        assert_eq!(e.song, before);
        assert_eq!(e.revision(), revision);
        ui.print(&e).unwrap();
        wait(&mut ui);
        assert_eq!(e.song, before);
        let artifact = std::fs::read(files.0.join("printed.wav")).unwrap();
        let candidate = ui.printed.as_ref().unwrap().value.candidate().clone();
        assert!(ui.prepare(&e).unwrap_err().contains("receipt"));
        assert!(ui.print(&e).unwrap_err().contains("receipt"));
        ui.register(&e).unwrap();
        wait(&mut ui);
        assert_eq!(e.song, before);
        ui.finish_registration(&mut e, true);
        assert_eq!(e.song, candidate);
        assert_eq!(Context::of(&e), cursor);
        assert!(ui.retained.as_ref().unwrap().registered_once);
        e.undo();
        ui.sync(&e);
        assert_eq!(e.song, before);
        assert!(ui.retained.is_some());
        assert_eq!(
            std::fs::read(files.0.join("printed.wav")).unwrap(),
            artifact
        );
        e.redo();
        ui.sync(&e);
        assert_eq!(e.song, candidate);
        assert_eq!(
            std::fs::read(files.0.join("printed.wav")).unwrap(),
            artifact
        );
        ui.forget().unwrap();
        assert!(ui.retained.is_none());
        assert_eq!(
            std::fs::read(files.0.join("printed.wav")).unwrap(),
            artifact
        );
    }
    #[test]
    fn late_published_receipt_survives_new_cancel_and_hidden_inspector() {
        for mode in 0..3 {
            let (files, mut e, mut ui) = fixture();
            let plan =
                resample::prepare(&e.song, ui.draft.as_ref().unwrap().config().unwrap()).unwrap();
            let value = Arc::new(resample::render(&plan, |_| Ok(()), || false).unwrap());
            let (tx, rx) = mpsc::sync_channel(1);
            ui.pending = Some(Pending {
                epoch: ui.epoch,
                revision: e.revision(),
                context: Context::of(&e),
                cancel: Arc::new(AtomicBool::new(false)),
                receiver: rx,
                label: "test already-published completion",
            });
            assert!(ui.forget().is_err());
            match mode {
                0 => {
                    ui.reset_document();
                    let song = e.song.clone();
                    e.replace_document(song).unwrap();
                }
                1 => ui.cancel_task(),
                _ => ui.hide(),
            };
            tx.send(Ok(Completed::Printed(value))).unwrap();
            ui.poll();
            assert!(ui.retained.is_some());
            assert!(files.0.join("printed.wav").exists());
            assert!(!ui.busy());
            assert!(ui.take_event().unwrap().contains("Not registered"));
            if mode < 2 {
                assert!(ui.printed.is_none());
            } else {
                assert!(ui.printed.is_some());
            }
            ui.open_for(&e, files.0.join("another.wav"));
            assert!(ui.prepare(&e).is_err());
            assert!(ui.capture_state(&e)["retained"].is_object());
        }
    }
    #[test]
    fn verified_registration_rechecks_mode_modal_generation_and_explicit_close() {
        for guard in 0..5 {
            let (files, mut e, mut ui) = fixture();
            printed(&mut ui, &e);
            let before = e.song.clone();
            ui.register(&e).unwrap();
            wait(&mut ui);
            match guard {
                0 => e.edit_mode = false,
                1 => {}
                2 => {
                    let song = e.song.clone();
                    e.replace_document(song).unwrap();
                    e.edit_mode = true;
                }
                3 => ui.hide(),
                _ => {
                    ui.cancel_commit();
                }
            }
            ui.finish_registration(&mut e, guard != 1);
            assert_eq!(e.song, before);
            assert!(ui.retained.is_some());
            assert!(files.0.join("printed.wav").exists());
        }
    }
    #[test]
    fn tampered_published_file_and_changed_options_never_register() {
        let (files, mut e, mut ui) = fixture();
        printed(&mut ui, &e);
        let before = e.song.clone();
        let path = files.0.join("printed.wav");
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        ui.register(&e).unwrap();
        wait(&mut ui);
        assert!(ui.error.is_some());
        ui.finish_registration(&mut e, true);
        assert_eq!(e.song, before);
        assert!(ui.retained.is_some());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        ui.draft.as_mut().unwrap().root = "61".into();
        assert!(!ui.current_print(&e));
        assert!(ui.register(&e).is_err());
    }
    #[test]
    fn preparation_is_read_mode_safe_and_used_legacy_paths_remain_actionable() {
        let (_files, mut e, mut ui) = fixture();
        e.edit_mode = false;
        printed(&mut ui, &e);
        assert!(ui.register(&e).unwrap_err().contains("Edit mode"));
        ui.forget().unwrap();
        e.edit_mode = true;
        e.assign_sample(InstrumentId(1), Some(_files.0.join("source.wav")))
            .unwrap();
        ui.sync(&e);
        ui.prepare(&e).unwrap();
        wait(&mut ui);
        assert!(ui
            .error
            .as_ref()
            .unwrap()
            .contains("import/register a region"));
    }
    #[test]
    fn prospective_unused_source_failure_retains_print_without_registration() {
        let (files, mut e, mut ui) = fixture();
        let mut song = e.song.clone();
        let mut unused = song.samples[&SampleId(1)].clone();
        unused.path = files.0.join("missing-unused.wav");
        song.register_sample(SampleId(2), unused).unwrap();
        e.replace_document(song).unwrap();
        e.edit_mode = true;
        ui.sync(&e);
        let before = e.song.clone();
        let revision = e.revision();
        printed(&mut ui, &e);
        assert!(files.0.join("printed.wav").exists());
        ui.register(&e).unwrap();
        wait(&mut ui);
        assert!(ui.error.is_some());
        ui.finish_registration(&mut e, true);
        assert_eq!(e.song, before);
        assert_eq!(e.revision(), revision);
        assert!(ui.retained.is_some());
        assert!(files.0.join("printed.wav").exists());
    }
    #[test]
    fn actual_cancel_click_keeps_a_late_already_published_receipt() {
        let (files, mut editor, mut inspector) = fixture();
        let before = editor.song.clone();
        let plan = resample::prepare(
            &editor.song,
            inspector.draft.as_ref().unwrap().config().unwrap(),
        )
        .unwrap();
        let preview = Arc::new(resample::render(&plan, |_| Ok(()), || false).unwrap());
        // Deterministically hold delivery after genuine core publication, so the
        // actual Cancel click exercises the late-publication race without sleeps.
        let (tx, rx) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        inspector.pending = Some(Pending {
            epoch: inspector.epoch,
            revision: editor.revision(),
            context: Context::of(&editor),
            cancel: Arc::clone(&cancel),
            receiver: rx,
            label: "Held test completion after real publication",
        });
        let ctx = egui::Context::default();
        crate::tracker_ui::install_theme(&ctx);
        let mut frame = 0_u32;
        let mut render = |inspector: &mut ResampleUi, editor: &mut Editor, events| {
            frame += 1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 620.0),
                    )),
                    time: Some(f64::from(frame) / 60.0),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| inspector.show(ui, editor));
                },
            )
        };
        for _ in 0..8 {
            render(&mut inspector, &mut editor, vec![]);
        }
        let out = render(&mut inspector, &mut editor, vec![]);
        let pos = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(t)
                    if t.galley.job.text == "Cancel task"
                        && shape.clip_rect.contains_rect(t.visual_bounding_rect()) =>
                {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
            .unwrap();
        render(
            &mut inspector,
            &mut editor,
            vec![egui::Event::PointerMoved(pos)],
        );
        for pressed in [true, false] {
            render(
                &mut inspector,
                &mut editor,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert!(cancel.load(Ordering::Acquire));
        assert!(inspector.busy());
        assert!(inspector.forget().is_err());
        tx.send(Ok(Completed::Printed(preview))).unwrap();
        inspector.poll();
        assert!(inspector.retained.is_some());
        assert!(inspector.printed.is_none());
        assert_eq!(editor.song, before);
        assert!(files.0.join("printed.wav").exists());
    }
    #[test]
    fn native_window_footer_is_bounded_even_for_long_errors_at_both_sizes() {
        for (width, height) in [(900.0, 620.0), (1180.0, 780.0)] {
            let (_files, mut editor, mut inspector) = fixture();
            printed(&mut inspector, &editor);
            inspector.error = Some("Long explicit error path: ".to_owned() + &"a".repeat(4096));
            inspector.notice = "An artifact is external to document history. ".repeat(100);
            let ctx = egui::Context::default();
            crate::tracker_ui::install_theme(&ctx);
            let mut output = None;
            for frame in 0..8 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        time: Some(f64::from(frame) / 60.0),
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .show(ctx, |ui| inspector.show(ui, &mut editor));
                    },
                ));
            }
            let output = output.unwrap();
            for label in [
                "Prepare preview",
                "Print new WAV",
                "Register + new instrument",
                "Cancel task",
                "Forget receipt (file stays)",
                "Close",
            ] {
                let found=output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(t) if t.galley.job.text==label && shape.clip_rect.contains_rect(t.visual_bounding_rect()) && t.visual_bounding_rect().bottom()<height));
                assert!(found, "{label} must be fully painted at{width}x{height}");
            }
        }
    }
}

fn footer_summary(ui: &mut egui::Ui, s: &ResampleSummary, label: &str) {
    let name = s.output.file_name().map_or_else(
        || s.output.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    status_line(
        ui,
        &format!(
            "{label} POS{} / P{} · {} total frames (tail {}) · S{} / R{} / I{} · {name}",
            s.occurrence,
            s.pattern.0,
            s.printed_frames,
            s.tail_frames,
            s.sample.0,
            s.region.0,
            s.instrument.0
        ),
        AQUA,
    );
}
