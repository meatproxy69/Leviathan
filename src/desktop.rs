use crate::{
    jobs::{Completed, Jobs, Request, SampleInfo},
    pattern_manifest, Event, Manifest, RATE,
};
use eframe::egui::{self, Color32, RichText, Vec2};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};
const MINT: Color32 = crate::tracker_ui::AQUA;
struct MeterBinding {
    shared: std::sync::Arc<crate::transport::Shared>,
    revision: u64,
    reader: Result<crate::transport::MeterReader, String>,
}
#[derive(Clone, Copy)]
enum NativePickerTarget {
    LegacySample,
    InstrumentSample,
}
struct NativePicker {
    target: NativePickerTarget,
    receiver: Receiver<Option<PathBuf>>,
    owner: crate::editor::DocumentOwner,
    instrument_id: u64,
}

pub struct Leviathan {
    tracker: crate::tracker_ui::TrackerUi,
    legacy_tools: bool,
    sample_view: bool,
    sample_editor: crate::sample_ui::SampleUi,
    multisample_editor: crate::multisample_ui::MultisampleUi,
    return_to_multisample: bool,
    resample_editor: crate::resample_ui::ResampleUi,
    mix_editor: crate::track_mix_ui::TrackMixUi,
    mix_delivery: crate::track_mix_ui::Delivery,
    mix_play_owner: Option<crate::editor::DocumentOwner>,
    mix_seen_owner: Option<crate::editor::DocumentOwner>,
    mix_seen_revision: Option<u64>,
    mix_content_cache: Option<(u64, bool)>,
    meter_binding: Option<MeterBinding>,
    timing_source: Option<(u64, f64, u16, bool)>,
    timing_draft: (f64, u16),
    session: crate::session::Session,
    project_path: String,
    pending_save_path: String,
    instrument_id: u64,
    asset_path: String,
    asset_source: Option<(u64, Option<PathBuf>)>,
    song_output: String,
    #[cfg(feature = "realtime")]
    transport: crate::transport::host::Transport,
    audition: crate::audition_ui::AuditionUi,
    #[cfg(any(test, feature = "ui-capture"))]
    diagnostic_audio: crate::audition_ui::diagnostic::Driver,
    follow: bool,
    loop_song: bool,
    #[cfg(feature = "realtime")]
    transport_smoke: bool,
    jobs: Jobs,
    sample: Option<SampleInfo>,
    sample_path: String,
    native_picker: Option<NativePicker>,
    output_path: String,
    bpm: f64,
    bars: u32,
    manifest: Manifest,
    status: String,
    error: Option<String>,
    capture: Option<PathBuf>,
    capture_requested: bool,
    capture_frames: u8,
    export_finished: bool,
}
impl Leviathan {
    fn open_native_sample_picker(&mut self, target: NativePickerTarget, ctx: &egui::Context) {
        if self.native_picker.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        let owner = self.tracker.editor.document_owner();
        let instrument_id = self.instrument_id;
        match std::thread::Builder::new()
            .name("leviathan-file-picker".into())
            .spawn(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("Audio source", &["wav", "flac"])
                    .pick_file();
                let _ = sender.send(path);
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.native_picker = Some(NativePicker {
                    target,
                    receiver,
                    owner,
                    instrument_id,
                })
            }
            Err(error) => self.error = Some(format!("Could not start file picker: {error}")),
        }
    }
    fn poll_native_sample_picker(&mut self) {
        let Some(picker) = &self.native_picker else {
            return;
        };
        let target = picker.target;
        let owner = picker.owner.clone();
        let instrument_id = picker.instrument_id;
        let result = match picker.receiver.try_recv() {
            Ok(path) => Some(path),
            Err(TryRecvError::Disconnected) => {
                self.native_picker = None;
                self.error = Some("Native file picker disconnected".into());
                return;
            }
            Err(TryRecvError::Empty) => return,
        };
        self.native_picker = None;
        if !self.tracker.editor.owns_document(&owner)
            || (matches!(target, NativePickerTarget::InstrumentSample)
                && self.instrument_id != instrument_id)
        {
            self.status =
                "File selection discarded because the project or instrument changed".into();
            return;
        }
        if let Some(path) = result.flatten() {
            match target {
                NativePickerTarget::LegacySample => self.sample_path = path.display().to_string(),
                NativePickerTarget::InstrumentSample => {
                    self.asset_path = path.display().to_string()
                }
            }
        }
    }
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::tracker_ui::install_theme(&cc.egui_ctx);
        let root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent()?.parent()?.parent().map(|x| x.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let mut app = Self {
            tracker: crate::tracker_ui::TrackerUi::default(),
            legacy_tools: false,
            sample_view: false,
            sample_editor: crate::sample_ui::SampleUi::default(),
            multisample_editor: crate::multisample_ui::MultisampleUi::default(),
            return_to_multisample: false,
            resample_editor: crate::resample_ui::ResampleUi::default(),
            mix_editor: crate::track_mix_ui::TrackMixUi::default(),
            mix_delivery: crate::track_mix_ui::Delivery {
                message:
                    "No live mix request. Offline export uses the committed document settings."
                        .into(),
                ..Default::default()
            },
            mix_play_owner: None,
            mix_seen_owner: None,
            mix_seen_revision: None,
            mix_content_cache: None,
            meter_binding: None,
            timing_source: None,
            timing_draft: (120.0, 4),
            session: crate::session::Session::default(),
            project_path: String::new(),
            pending_save_path: String::new(),
            #[cfg(feature = "realtime")]
            transport: crate::transport::host::Transport::default(),
            audition: crate::audition_ui::AuditionUi::default(),
            #[cfg(any(test, feature = "ui-capture"))]
            diagnostic_audio: crate::audition_ui::diagnostic::Driver::default(),
            follow: true,
            loop_song: false,
            #[cfg(feature = "realtime")]
            transport_smoke: false,
            instrument_id: 1,
            asset_path: String::new(),
            asset_source: None,
            song_output: root.join("artifacts/song.wav").display().to_string(),
            jobs: Jobs::default(),
            sample: None,
            sample_path: String::new(),
            native_picker: None,
            output_path: root.join("artifacts/session.wav").display().to_string(),
            bpm: 120.0,
            bars: 2,
            manifest: pattern_manifest(120.0, 2).expect("valid defaults"),
            status: "Song editor ready · enable Edit Mode to enter notes".into(),
            error: None,
            capture: None,
            capture_requested: false,
            capture_frames: 0,
            export_finished: false,
        };
        app.tracker
            .editor
            .replace_document(crate::song::startup_song())
            .expect("valid startup song");
        let args: Vec<_> = std::env::args().collect();
        if args.len() == 3 && args[1] == "--project" {
            let path = PathBuf::from(&args[2]);
            let result = app.session.request(
                &mut app.tracker.editor,
                crate::session::Action::Open(path.clone()),
            );
            app.project_result(result);
            if app.error.is_none() {
                if app.tracker.editor.has_grid() {
                    let song = &app.tracker.editor.song;
                    let pattern = song.order[0].pattern;
                    let first = song.track_order.iter().copied().find(|track| {
                        song.resolved_track(pattern, *track)
                            .ok()
                            .flatten()
                            .and_then(|rows| rows.get(&0))
                            .is_some_and(|columns| {
                                columns
                                    .values()
                                    .any(|cell| matches!(cell.note, crate::song::Note::Note(_)))
                            })
                    });
                    if let Some(track) = first {
                        let _ = app.tracker.editor.select(0, track);
                    }
                }
                app.song_output = path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("next-export.wav")
                    .display()
                    .to_string();
                app.status = format!("Loaded {} · press Play song", app.tracker.editor.song.title);
                eprintln!("{}", app.status);
            }
        }
        if args.len() == 5 && args[1] == "--smoke-test" {
            app.sample_path = args[2].clone();
            app.output_path = args[3].clone();
            app.capture = Some(PathBuf::from(&args[4]));
            app.load();
        }
        if args.len() == 3 && args[1] == "--tracker-smoke" {
            app.capture = Some(PathBuf::from(&args[2]));
            app.export_finished = true;
            app.tracker.editor.edit_mode = true;
            app.tracker
                .editor
                .enter_note(crate::song::Note::Note(60))
                .expect("smoke edit");
            app.tracker
                .editor
                .select(2, crate::song::TrackId(1))
                .expect("smoke selection");
            app.tracker.editor.row = 0;
            assert_eq!(app.tracker.editor.cell().note, crate::song::Note::Note(60));
            eprintln!("Tracker smoke: edited A, verified shared A at occurrence 2");
        }
        if args.len() == 4 && args[1] == "--project-smoke" {
            let path = PathBuf::from(&args[2]);
            app.tracker.editor.edit_mode = true;
            app.tracker
                .editor
                .enter_note(crate::song::Note::Note(65))
                .expect("smoke edit");
            let expected = app.tracker.editor.song.clone();
            app.session
                .save_request(&mut app.tracker.editor, path.clone(), false)
                .expect("smoke save");
            assert!(
                app.session.overwrite.is_none(),
                "smoke requires a NEW project path"
            );
            app.tracker.editor.undo();
            assert!(app.tracker.editor.dirty);
            app.tracker.editor.redo();
            assert!(!app.tracker.editor.dirty);
            app.session
                .request(&mut app.tracker.editor, crate::session::Action::New)
                .expect("smoke new");
            app.session
                .request(
                    &mut app.tracker.editor,
                    crate::session::Action::Open(path.clone()),
                )
                .expect("smoke open");
            assert_eq!(app.tracker.editor.song, expected);
            app.project_path = path.display().to_string();
            app.status = "Project saved, undo/redo checked, then reopened successfully".into();
            app.capture = Some(PathBuf::from(&args[3]));
            app.export_finished = true;
            eprintln!("Project smoke: save, undo/redo saved-state, new/open roundtrip verified");
        }
        if args.len() == 5 && args[1] == "--song-smoke" {
            app.session
                .request(
                    &mut app.tracker.editor,
                    crate::session::Action::Open(PathBuf::from(&args[2])),
                )
                .expect("smoke open song");
            app.project_path = args[2].clone();
            app.song_output = args[3].clone();
            app.capture = Some(PathBuf::from(&args[4]));
            app.export_song();
        }
        #[cfg(feature = "realtime")]
        if args.len() == 4 && args[1] == "--transport-smoke" {
            app.session
                .request(
                    &mut app.tracker.editor,
                    crate::session::Action::Open(PathBuf::from(&args[2])),
                )
                .expect("open transport smoke song");
            app.project_path = args[2].clone();
            app.capture = Some(PathBuf::from(&args[3]));
            app.transport_smoke = true;
            app.play_song();
        }
        app
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_step_projection(&self) -> Option<&crate::drum_steps::StepProjection> {
        self.tracker.steps.capture_projection(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_thin_preview(&self) -> Option<&crate::mutations::ThinPreview> {
        self.tracker.thin.capture_preview(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_thin_state(&self) -> serde_json::Value {
        self.tracker.thin.capture_state(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_transform_preview(&self) -> Option<&crate::transforms::TransformPreview> {
        self.tracker
            .transforms
            .capture_preview(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_groove_capture_preview(&self) -> Option<&crate::groove::CapturePreview> {
        self.tracker
            .grooves
            .capture_capture_preview(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_groove_preview(&self) -> Option<&crate::groove::GroovePreview> {
        self.tracker
            .grooves
            .capture_apply_preview(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_generation_preview(&self) -> Option<&crate::generation::GenerationPreview> {
        self.tracker
            .generation
            .capture_preview(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_onset_analysis(&self) -> Option<&crate::onset_service::SourceAnalysis> {
        self.sample_editor
            .capture_experimental_analysis(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_catalog_state(&self) -> serde_json::Value {
        self.sample_editor
            .capture_catalog_state(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_tempo_state(&self) -> serde_json::Value {
        self.sample_editor.capture_tempo_state(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_resample_state(&self) -> serde_json::Value {
        self.resample_editor.capture_state(&self.tracker.editor)
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_editor_mut(&mut self) -> &mut crate::editor::Editor {
        &mut self.tracker.editor
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub fn capture_track_mix_state(&self) -> serde_json::Value {
        serde_json::json!({"ui":self.mix_editor.capture_state(&self.tracker.editor),"delivery":self.mix_delivery,
            "content_current":self.mix_content_cache,"owner_current":self.mix_play_owner.as_ref().is_some_and(|o|self.tracker.editor.owns_document(o))})
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_track_mix_poll(&mut self) {
        self.mix_editor.poll();
    }
    fn open_mix(&mut self) {
        self.tracker.cell_blocks.close();
        self.tracker.transforms.reset_document();
        self.tracker.thin.reset_document();
        self.tracker.grooves.reset_document();
        self.tracker.reset_generation_document();
        self.sample_editor.cancel_commit();
        self.multisample_editor.open = false;
        self.resample_editor.hide();
        self.mix_editor.open_for(&self.tracker.editor);
    }
    fn mix_button(&mut self, ui: &mut egui::Ui) {
        if ui.add_enabled(!self.session.close_ready && self.session.pending.is_none() && self.session.overwrite.is_none(),egui::Button::new("Mix…"))
            .on_hover_text("Reviewed track gain, stereo balance, mute and solo. Draft controls are silent; Apply stores one document edit and separately requests live delivery when authorized.").clicked() {
            self.open_mix();
        }
    }
    fn revoke_mix_owner(&mut self) {
        if let Some(shared) = self.transport_shared() {
            shared.revoke_live_mix();
        }
        self.mix_play_owner = None;
        self.mix_content_cache = None;
        self.meter_binding = None;
        self.mix_editor.clear_meters("Document owner changed");
        self.mix_delivery.available = false;
        self.mix_delivery.requested_revision = None;
        self.mix_delivery.message="Document replaced: prior live mix control is revoked. Stored settings apply to future export/Play; any existing buffered audio/current ramp is not retracted.".into();
    }
    fn guard_mix_owner(&mut self) {
        if self
            .mix_play_owner
            .as_ref()
            .is_some_and(|o| !self.tracker.editor.owns_document(o))
        {
            self.revoke_mix_owner();
        }
        if self
            .mix_seen_owner
            .as_ref()
            .is_none_or(|o| !self.tracker.editor.owns_document(o))
        {
            self.mix_seen_owner = Some(self.tracker.editor.document_owner());
            self.mix_seen_revision = Some(self.tracker.editor.revision());
        }
    }
    fn submit_mix(&mut self) {
        let revision = self.tracker.editor.revision();
        let result = if self.audition.owned {
            Err("Audition does not accept Song mix updates; committed settings remain in the document.".into())
        } else if !self.audio_available() {
            Err("Live audio is unavailable in this build; committed settings apply to offline export and future playback.".into())
        } else if !self.transport_busy() {
            Err("No active Song playback; committed settings remain stored for export or the next explicit Play.".into())
        } else if let Some(shared) = self.transport_shared() {
            shared.submit_live_mix(
                &self.tracker.editor.document_owner(),
                &self.tracker.editor.song,
            )
        } else {
            Err("No Song transport session is available; document settings remain stored.".into())
        };
        self.mix_delivery.document_revision = Some(revision);
        match result {
            Ok(request) => {
                self.mix_delivery.requested_revision = Some(request);
                self.mix_delivery.message=format!("Document revision {revision} is stored; live request {request} accepted by the coalescing mailbox. Waiting for worker-enqueued acknowledgement; not heard output.");
                self.mix_content_cache = Some((revision, true));
            }
            Err(error) => {
                self.mix_delivery.requested_revision = None;
                self.mix_delivery.message = format!(
                    "Document revision {revision} remains stored. NOT LIVE-APPLIED: {error} Earlier accepted audio settings are not rolled back."
                );
            }
        }
    }
    fn refresh_mix_status(&mut self) {
        let status = self.transport_shared().map(|s| s.live_mix_status());
        if let Some(status) = status {
            self.mix_delivery.accepted_revision = status.accepted_revision;
            self.mix_delivery.enqueued_revision = status.applied_revision;
            self.mix_delivery.available = status.available
                && !self.audition.owned
                && self
                    .mix_play_owner
                    .as_ref()
                    .is_some_and(|o| self.tracker.editor.owns_document(o))
                && self.mix_content_cache == Some((self.tracker.editor.revision(), true));
            if let Some(request) = self.mix_delivery.requested_revision {
                if status.applied_revision >= request {
                    self.mix_delivery.message=format!("Live request {request}: the first nonempty rendered block after applying the target was wholly enqueued; the ramp may be incomplete. This is not device playback or heard output; already queued samples are not retracted.");
                } else if !status.available {
                    self.mix_delivery.message=format!("Live request {request} was accepted, but the session is no longer available before enqueued acknowledgement. Document settings remain stored; no heard-output claim.");
                }
            }
        } else {
            self.mix_delivery.available = false;
        }
    }
    fn refresh_meters(&mut self, ctx: &egui::Context) {
        if !self.mix_editor.is_open() {
            self.meter_binding = None;
            self.mix_editor.clear_meters("Mixer closed");
            return;
        }
        let unavailable = if !self.audio_available() {
            Some("Live meters unavailable in this build")
        } else if self.audition.owned {
            Some("Audition is not metered by the Song mixer")
        } else if !self.transport_busy() {
            Some("Song transport stopped")
        } else if !self
            .mix_play_owner
            .as_ref()
            .is_some_and(|o| self.tracker.editor.owns_document(o))
        {
            Some("No current Song-owner meter session")
        } else {
            None
        };
        if let Some(reason) = unavailable {
            self.meter_binding = None;
            self.mix_editor.clear_meters(reason);
            return;
        }
        let Some(shared) = self.transport_shared().cloned() else {
            self.meter_binding = None;
            self.mix_editor.clear_meters("No Song transport session");
            return;
        };
        let revision = self.tracker.editor.revision();
        let changed_session = self
            .meter_binding
            .as_ref()
            .is_none_or(|b| !std::sync::Arc::ptr_eq(&b.shared, &shared));
        if changed_session
            || self
                .meter_binding
                .as_ref()
                .is_none_or(|b| b.revision != revision)
        {
            if changed_session {
                self.mix_editor
                    .clear_meters("Waiting for current-session window");
            }
            let reader = shared.meter_reader(
                &self.tracker.editor.document_owner(),
                &self.tracker.editor.song,
            );
            self.meter_binding = Some(MeterBinding {
                shared: shared.clone(),
                revision,
                reader,
            });
        }
        let result = match &self.meter_binding.as_ref().unwrap().reader {
            Ok(reader) => shared.poll_meters(reader),
            Err(error) => Err(error.clone()),
        };
        match result {
            Ok(poll) => self.mix_editor.accept_meters(poll, ctx.input(|i| i.time)),
            Err(error) => self.mix_editor.clear_meters(error),
        }
        // Presentation-only hold/decay; this neither consumes device frames nor
        // submits any transport/mix command. The worker already owns observation.
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    fn finish_mix_frame(&mut self, explicit_apply: bool) {
        self.guard_mix_owner();
        let revision = self.tracker.editor.revision();
        let changed = self.mix_seen_revision != Some(revision);
        if explicit_apply
            || (changed && self.tracker.editor.track_mix_change_revision() == Some(revision))
        {
            self.submit_mix();
        }
        self.mix_seen_revision = Some(revision);
        if self.transport_busy()
            && !self.audition.owned
            && self
                .mix_play_owner
                .as_ref()
                .is_some_and(|o| self.tracker.editor.owns_document(o))
            && self.mix_content_cache.is_none_or(|(r, _)| r != revision)
        {
            let current = self.transport_snapshot().is_some_and(|song| {
                crate::track_mix::same_playing_content(&self.tracker.editor.song, song)
                    .unwrap_or(false)
            });
            self.mix_content_cache = Some((revision, current));
        }
        self.refresh_mix_status();
    }
    fn diagnostic_enabled(&self) -> bool {
        #[cfg(any(test, feature = "ui-capture"))]
        {
            self.diagnostic_audio.enabled
        }
        #[cfg(not(any(test, feature = "ui-capture")))]
        {
            false
        }
    }
    fn audio_available(&self) -> bool {
        self.diagnostic_enabled() || cfg!(feature = "realtime")
    }
    fn transport_busy(&self) -> bool {
        #[cfg(any(test, feature = "ui-capture"))]
        if self.diagnostic_audio.enabled {
            return self.diagnostic_audio.busy();
        }
        #[cfg(feature = "realtime")]
        {
            self.transport.busy()
        }
        #[cfg(not(feature = "realtime"))]
        {
            false
        }
    }
    fn transport_shared(&self) -> Option<&std::sync::Arc<crate::transport::Shared>> {
        #[cfg(any(test, feature = "ui-capture"))]
        if self.diagnostic_audio.enabled {
            return self.diagnostic_audio.shared.as_ref();
        }
        #[cfg(feature = "realtime")]
        {
            self.transport.shared.as_ref()
        }
        #[cfg(not(feature = "realtime"))]
        {
            None
        }
    }
    fn transport_snapshot(&self) -> Option<&crate::song::Song> {
        #[cfg(any(test, feature = "ui-capture"))]
        if self.diagnostic_audio.enabled {
            return self.diagnostic_audio.snapshot.as_ref();
        }
        #[cfg(feature = "realtime")]
        {
            self.transport.snapshot.as_ref()
        }
        #[cfg(not(feature = "realtime"))]
        {
            None
        }
    }
    fn transport_error(&self) -> Option<&String> {
        #[cfg(any(test, feature = "ui-capture"))]
        if self.diagnostic_audio.enabled {
            return self.diagnostic_audio.error.as_ref();
        }
        #[cfg(feature = "realtime")]
        {
            self.transport.error.as_ref()
        }
        #[cfg(not(feature = "realtime"))]
        {
            None
        }
    }
    fn start_transport(
        &mut self,
        song: crate::song::Song,
        base: Option<PathBuf>,
        looping: bool,
    ) -> Result<(), String> {
        if self.transport_busy() {
            return Err("Transport is active or still stopping; no takeover".into());
        }
        #[cfg(any(test, feature = "ui-capture"))]
        if self.diagnostic_audio.enabled {
            if looping {
                return Err("Device-free UI diagnostic does not loop".into());
            }
            let _ = base;
            return self.diagnostic_audio.start(song);
        }
        #[cfg(feature = "realtime")]
        {
            self.transport.start_with_loop(song, base, looping)
        }
        #[cfg(not(feature = "realtime"))]
        {
            let _ = (song, base, looping);
            Err("Realtime audio is unavailable in this build".into())
        }
    }
    fn stop_transport(&mut self) {
        self.meter_binding = None;
        self.mix_editor.clear_meters("Stop requested");
        if let Some(shared) = self.transport_shared() {
            shared.stop();
        }
        self.audition.cancel(
            "Hard Stop requested; it may click. Submitted hardware buffers cannot be retracted.",
        );
    }
    fn poll_transport(&mut self) {
        let was_busy = self.transport_busy();
        #[cfg(any(test, feature = "ui-capture"))]
        if self.diagnostic_audio.enabled {
            self.diagnostic_audio.poll();
        }
        #[cfg(feature = "realtime")]
        if !self.diagnostic_enabled() {
            self.transport.poll();
        }
        if self.audition.owned {
            if self.transport_shared().is_some_and(|s| s.is_ready()) {
                self.audition.engine_ready = true;
            }
            if was_busy && !self.transport_busy() {
                self.audition.finish(self.transport_error().cloned());
            }
        }
    }
    fn audition_request(&self) -> Option<crate::audition_ui::Request> {
        if self.legacy_tools
            || self.multisample_editor.open
            || self.resample_editor.open
            || self.tracker.is_phrase_authoring()
            || self.tracker.transforms.is_open()
            || self.tracker.grooves.is_open()
            || self.tracker.thin.is_open()
            || self.tracker.generation.is_open()
        {
            return None;
        }
        if self.sample_view {
            self.sample_editor.audition_request().ok()
        } else {
            self.tracker.pads.audition_request()
        }
    }
    fn audition_blocked(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        }) || ctx.input(|i| i.viewport().close_requested())
            || self.session.close_ready
            || self.session.pending.is_some()
            || self.session.overwrite.is_some()
    }
    fn guard_audition(&mut self, ctx: &egui::Context) {
        if self.audition.owned
            && self.transport_busy()
            && !self.audition.canceled
            && (self.audition_blocked(ctx)
                || !self
                    .audition
                    .current(&self.tracker.editor, self.audition_request().as_ref()))
        {
            self.stop_transport();
            self.audition.notice=Some("Audition stopped: focus, document, draft, selection or workspace changed. Click again explicitly after stopping completes.".into());
        }
    }
    fn audition_panel_state(&self, ctx: &egui::Context) -> crate::audition_ui::PanelState {
        use std::sync::atomic::Ordering;
        let state = self.transport_shared();
        crate::audition_ui::PanelState {
            available: self.audio_available(),
            blocked: self.audition_blocked(ctx),
            busy: self.transport_busy(),
            owned: self.audition.owned,
            ready: state.is_some_and(|s| s.is_ready()),
            diagnostic: self.diagnostic_enabled(),
            ended: state.is_some_and(|s| s.has_ended()),
            stopping: self.transport_busy() && state.is_some_and(|s| s.quit.load(Ordering::SeqCst)),
            error: self.audition.error.clone(),
            notice: self.audition.notice.clone(),
            summary: self.audition.last.as_ref().map(|p| p.summary().clone()),
        }
    }
    fn audition_actions(&mut self, ctx: &egui::Context) {
        // Native clicks only. Recheck against the final same-frame UI context;
        // no queued Start survives a panel/selection/document/focus transition.
        let actions = [
            self.sample_editor.take_audition_action(),
            self.tracker.pads.take_audition_action(),
        ];
        for action in actions.into_iter().flatten() {
            match action {
                crate::audition_ui::Action::Stop => self.stop_transport(),
                crate::audition_ui::Action::Invalid(error) => {
                    self.audition.error = Some(error);
                    self.audition.notice = None;
                }
                crate::audition_ui::Action::Start(request) => {
                    let result = (|| {
                        if !self.audio_available() {
                            return Err("Realtime audio is unavailable in this build".into());
                        }
                        if self.audition_blocked(ctx) {
                            return Err(
                                "Audition is blocked by focus, project confirmation or closing"
                                    .into(),
                            );
                        }
                        if self.transport_busy() {
                            return Err(
                                "Transport is active or still stopping; no mixing or takeover"
                                    .into(),
                            );
                        }
                        if self.audition_request().as_ref() != Some(&request) {
                            return Err(
                                "Audition request changed before Start; select it again".into()
                            );
                        }
                        let prepared =
                            crate::audition_ui::AuditionUi::prepare(&self.tracker.editor, request)?;
                        self.start_transport(prepared.preview.song().clone(), None, false)?;
                        self.audition.install(prepared);
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.audition.error = Some(error);
                        self.audition.notice = None;
                    }
                }
            }
        }
        self.guard_audition(ctx);
        if self.transport_busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_enable_audition_diagnostic(
        &mut self,
        held: bool,
        auto_advance: bool,
    ) -> Result<(), String> {
        if self.transport_busy() {
            return Err("Cannot change audio driver during a session".into());
        }
        self.diagnostic_audio.enabled = true;
        self.diagnostic_audio.held = held;
        self.diagnostic_audio.auto_advance = auto_advance;
        Ok(())
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_audition_advance(&mut self) {
        self.diagnostic_audio.advance();
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_audition_release_prepare(&self) {
        self.diagnostic_audio.release();
    }
    #[cfg(feature = "ui-capture")]
    pub fn capture_audition_pcm(&self) -> &[f32] {
        &self.diagnostic_audio.pcm
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub fn capture_audition_state(&self) -> serde_json::Value {
        use std::sync::atomic::Ordering;
        let shared = self.transport_shared();
        serde_json::json!({"diagnostic":self.diagnostic_enabled(),"available":self.audio_available(),"busy":self.transport_busy(),"owned":self.audition.owned,
            "epoch":self.audition.epoch,"canceled":self.audition.canceled,"engine_ready_this_run":self.audition.engine_ready,"starts":self.audition.starts,
            "error":self.audition.error,"notice":self.audition.notice,"request":self.audition_request(),
            "preview":self.audition.last.as_ref().map(|p|serde_json::json!({"id":p.id(),"summary":p.summary(),"snapshot":p.song()})),
            "position":shared.map(|s|s.position()),"ready":shared.is_some_and(|s|s.is_ready()),"ended":shared.is_some_and(|s|s.has_ended()),
            "callbacks":shared.map(|s|s.callbacks.load(Ordering::SeqCst)),"underrun_frames":shared.map(|s|s.underruns.load(Ordering::SeqCst)),
            "diagnostic_prepared":self.diagnostic_audio.prepared,"diagnostic_starts":self.diagnostic_audio.starts,
            "diagnostic_pcm_samples":self.diagnostic_audio.pcm.len(),"post_terminal_callback_silent":self.diagnostic_audio.after_stop_silent})
    }
    /// Capture-only drain of a real bounded metadata worker. No UI frame,
    /// clipboard installation, synthetic result, or document commit occurs.
    #[cfg(feature = "ui-capture")]
    pub fn capture_cell_blocks_poll(&mut self) {
        self.tracker.cell_blocks.poll();
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub fn capture_cell_blocks_state(&self) -> serde_json::Value {
        self.tracker.cell_blocks.capture_state(&self.tracker.editor)
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub fn capture_marker_bank_state(&self) -> serde_json::Value {
        self.sample_editor.capture_marker_bank_state()
    }
    /// Capture-only scheduling boundary: drain the actual source-worker result
    /// without running a UI frame. Never manufactures verification or commits.
    #[cfg(feature = "ui-capture")]
    pub fn capture_marker_bank_poll_source(&mut self) {
        self.sample_editor.poll();
    }
    fn unavailable_transport_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(false, egui::Button::new("▶ Play"));
            ui.add_enabled(false, egui::Button::new("■ Stop"));
            ui.add_enabled(false, egui::Checkbox::new(&mut self.loop_song, "Loop"));
            ui.add_enabled(false, egui::Checkbox::new(&mut self.follow, "Follow"));
            ui.separator();
            self.song_timing_menu(ui);
            self.mix_button(ui);
            ui.label(RichText::new("AUDIO UNAVAILABLE IN THIS BUILD").small().color(Color32::from_rgb(229, 185, 112)))
                .on_hover_text("This UI-only build supports editing and offline WAV export. Native audio requires the desktop-audio feature and host audio libraries.");
        });
    }
    fn song_timing_menu(&mut self, ui: &mut egui::Ui) {
        let current = (
            self.tracker.editor.song.bpm,
            self.tracker.editor.song.lines_per_beat,
        );
        let source = (
            self.tracker.editor.revision(),
            current.0,
            current.1,
            self.tracker.editor.edit_mode,
        );
        if self.timing_source != Some(source) {
            self.timing_source = Some(source);
            self.timing_draft = current;
        }
        let writable = ui.is_enabled()
            && self.tracker.editor.edit_mode
            && !self.session.close_ready
            && self.session.pending.is_none()
            && self.session.overwrite.is_none();
        let label =
            RichText::new(format!("{:.1} BPM  /  {} LPB", current.0, current.1)).monospace();
        let (_, popup) = egui::containers::menu::MenuButton::new(label)
            .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
                ui.set_max_width(360.0);
                ui.label(RichText::new("SONG TIMING").strong().color(MINT));
                ui.add_enabled_ui(writable, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("BPM");
                        ui.add(egui::DragValue::new(&mut self.timing_draft.0).range(20.0..=400.0).speed(0.25).max_decimals(3));
                        ui.label("LPB");
                        ui.add(egui::DragValue::new(&mut self.timing_draft.1).range(1..=256));
                    });
                });
                ui.label(RichText::new("Changes the real Song clock, not legacy motif tempo or source BPM metadata. Region playback stays at native pitch; phrases keep their own LPB. Restart playback to hear a changed snapshot.").small().weak());
                ui.horizontal(|ui| {
                    if ui.add_enabled(writable && self.timing_draft != current, egui::Button::new("Apply timing")).clicked() {
                        match self.tracker.editor.set_timing(self.timing_draft.0, self.timing_draft.1) {
                            Ok(()) => { self.error = None; self.status = "Song timing updated (undoable)".into(); }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if ui.button("Reset").clicked() { self.timing_draft = current; }
                    if ui.button("Close").clicked() { ui.close(); }
                });
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
            });
        if popup.is_some() {
            ui.memory_mut(|memory| memory.surrender_focus(crate::tracker_ui::grid_id()));
        }
    }
    fn project_result(&mut self, result: Result<(), String>) {
        match result {
            Err(error) => self.error = Some(error),
            Ok(()) => {
                self.error = None;
                self.asset_source = None;
                if !self.session.close_ready
                    && self.session.pending.is_none()
                    && self.session.overwrite.is_none()
                {
                    if matches!(self.session.notice.as_str(), "New song" | "Project opened") {
                        self.revoke_mix_owner();
                        self.mix_editor.reset_document();
                        self.sample_editor.reset_document();
                        self.multisample_editor.reset_document();
                        self.resample_editor.reset_document();
                        self.return_to_multisample = false;
                        self.tracker.pads.reset_document();
                        self.tracker.steps.reset_document();
                        self.tracker.thin.reset_document();
                        self.tracker.cell_blocks.reset_document();
                        self.tracker.transforms.reset_document();
                        self.tracker.grooves.reset_document();
                        self.tracker.reset_generation_document();
                        self.sample_view = false;
                    }
                    self.status = self.session.notice.clone();
                    self.project_path = self
                        .session
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                }
            }
        }
    }
    fn request_action(&mut self, action: crate::session::Action) {
        self.mix_editor.cancel_authorization();
        if self.audition.owned {
            self.stop_transport();
            self.audition.last = None;
        }
        self.pending_save_path = self
            .session
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let result = self.session.request(&mut self.tracker.editor, action);
        self.project_result(result);
    }
    fn project_controls(&mut self, ctx: &egui::Context) {
        let blocked = self.session.close_ready
            || self.session.pending.is_some()
            || self.session.overwrite.is_some();
        egui::TopBottomPanel::top("project-file").show(ctx, |ui| {
            ui.add_enabled_ui(!blocked, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("PROJECT").small().strong().color(MINT));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.project_path)
                            .desired_width((ui.available_width() - 375.0).clamp(160.0, 680.0))
                            .hint_text("Project path · .leviathan.json"),
                    );
                    if ui.button("New").clicked() {
                        self.request_action(crate::session::Action::New);
                    }
                    if ui
                        .add_enabled(
                            !self.project_path.trim().is_empty(),
                            egui::Button::new("Open"),
                        )
                        .clicked()
                    {
                        self.request_action(crate::session::Action::Open(PathBuf::from(
                            self.project_path.trim(),
                        )));
                    }
                    if ui
                        .add_enabled(
                            !self.project_path.trim().is_empty(),
                            egui::Button::new(
                                if self.session.path.as_ref().is_some_and(|path| {
                                    path.to_string_lossy() != self.project_path.trim()
                                }) {
                                    "Save as"
                                } else {
                                    "Save"
                                },
                            ),
                        )
                        .clicked()
                    {
                        let r = self.session.save_request(
                            &mut self.tracker.editor,
                            PathBuf::from(self.project_path.trim()),
                            false,
                        );
                        self.project_result(r);
                    }
                    if ui
                        .add_enabled(self.tracker.editor.can_undo(), egui::Button::new("Undo"))
                        .clicked()
                    {
                        self.tracker.editor.undo();
                        self.tracker.reveal_current_cursor();
                    }
                    if ui
                        .add_enabled(self.tracker.editor.can_redo(), egui::Button::new("Redo"))
                        .clicked()
                    {
                        self.tracker.editor.redo();
                        self.tracker.reveal_current_cursor();
                    }
                });
            });
        });
        if !blocked && ctx.memory(|m| m.has_focus(crate::tracker_ui::grid_id())) {
            for event in ctx.input(|i| i.events.clone()) {
                if self.session.close_ready
                    || self.session.pending.is_some()
                    || self.session.overwrite.is_some()
                {
                    break;
                }
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                {
                    if modifiers.command || modifiers.ctrl {
                        match key {
                            egui::Key::N => self.request_action(crate::session::Action::New),
                            egui::Key::O if !self.project_path.trim().is_empty() => self
                                .request_action(crate::session::Action::Open(PathBuf::from(
                                    self.project_path.trim(),
                                ))),
                            egui::Key::S if !self.project_path.trim().is_empty() => {
                                let r = self.session.save_request(
                                    &mut self.tracker.editor,
                                    PathBuf::from(self.project_path.trim()),
                                    false,
                                );
                                self.project_result(r);
                            }
                            _ => (),
                        }
                    }
                }
            }
        }
        if let Some(overwrite) = &self.session.overwrite {
            let path = overwrite.path.display().to_string();
            egui::Modal::new(egui::Id::new("confirm-overwrite")).show(ctx, |ui| {
                ui.heading("Replace existing project?"); ui.label(path);
                ui.label("Only this explicitly confirmed file will be replaced. Changed targets are rejected.");
                if let Some(e) = &self.error { ui.colored_label(Color32::LIGHT_RED, e); }
                ui.horizontal(|ui| {
                    if ui.button("Replace file").clicked() { let r = self.session.confirm_overwrite(&mut self.tracker.editor); self.project_result(r); }
                    if ui.button("Cancel overwrite").clicked() { self.session.overwrite = None; }
                });
            });
        } else if self.session.pending.is_some() {
            egui::Modal::new(egui::Id::new("unsaved-song")).show(ctx, |ui| {
                ui.heading("Save changes before continuing?");
                ui.label("Save current song to:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.pending_save_path).desired_width(420.0),
                );
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.pending_save_path.trim().is_empty(),
                            egui::Button::new("Save and continue"),
                        )
                        .clicked()
                    {
                        let r = self.session.save_request(
                            &mut self.tracker.editor,
                            PathBuf::from(self.pending_save_path.trim()),
                            true,
                        );
                        self.project_result(r);
                    }
                    if ui.button("Discard and continue").clicked() {
                        let r = self.session.discard(&mut self.tracker.editor);
                        self.project_result(r);
                    }
                    if ui.button("Cancel").clicked() {
                        self.session.cancel();
                        self.error = None;
                    }
                });
            });
        }
    }
    fn play_song(&mut self) {
        let base = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_owned());
        match self.start_transport(self.tracker.editor.song.clone(), base, self.loop_song) {
            Ok(()) => {
                self.error = None;
                self.audition.song_started();
                self.meter_binding = None;
                self.mix_editor.clear_meters("Preparing Song meters");
                let owner = self.tracker.editor.document_owner();
                let binding = self
                    .transport_shared()
                    .ok_or_else(|| "Song transport has no shared state".to_string())
                    .and_then(|shared| {
                        shared.bind_live_mix(owner.clone(), &self.tracker.editor.song)
                    });
                self.mix_play_owner = Some(owner);
                self.mix_content_cache = Some((self.tracker.editor.revision(), true));
                self.mix_delivery=crate::track_mix_ui::Delivery {
                    message:match binding {Ok(())=>"Song playback started with committed mix settings. No live update has been requested.".into(),Err(error)=>format!("Song playback started; live mix unavailable: {error}")},
                    ..Default::default()
                };
                self.mix_seen_owner = Some(self.tracker.editor.document_owner());
                self.mix_seen_revision = Some(self.tracker.editor.revision());
                self.status = "Transport preparing the immutable song snapshot".into();
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn transport_controls(&mut self, ui: &mut egui::Ui) {
        use std::sync::atomic::Ordering;
        if !self.audio_available() {
            self.unavailable_transport_controls(ui);
            return;
        }
        if self.diagnostic_enabled() {
            ui.label(
                RichText::new("DEVICE-FREE DIAGNOSTIC · Producer/Shared; no physical output")
                    .small()
                    .color(Color32::from_rgb(229, 185, 112)),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.transport_busy()
                        && !self.session.close_ready
                        && self.session.pending.is_none()
                        && self.session.overwrite.is_none(),
                    egui::Button::new("▶ Play"),
                )
                .clicked()
            {
                self.play_song();
            }
            if ui
                .add_enabled(self.transport_busy(), egui::Button::new("■ Stop"))
                .clicked()
            {
                self.stop_transport();
            }
            if ui
                .add_enabled(
                    !self.session.close_ready
                        && !(self.audition.owned && self.transport_busy())
                        && !self.diagnostic_enabled(),
                    egui::Checkbox::new(&mut self.loop_song, "Loop"),
                )
                .changed()
                && self.transport_busy()
            {
                if let (Some(state), Some(song)) =
                    (self.transport_shared(), self.transport_snapshot())
                {
                    let rows: u64 = song
                        .order
                        .iter()
                        .map(|o| song.patterns[&o.pattern].rows as u64)
                        .sum();
                    let end = (rows as f64 * RATE as f64 * 60.0
                        / (song.bpm * song.lines_per_beat as f64))
                        .round() as u64;
                    state.set_loop(self.loop_song, end);
                }
            }
            ui.add_enabled(
                !(self.audition.owned && self.transport_busy()),
                egui::Checkbox::new(&mut self.follow, "Follow"),
            )
            .on_hover_text(
                "Read-only playback snapshot. Turn off to edit; your cursor is preserved.",
            );
            ui.separator();
            self.song_timing_menu(ui);
            self.mix_button(ui);
            if ui
                .add_enabled(
                    !self.session.close_ready
                        && !self.audition.owned
                        && self.transport_busy()
                        && self.tracker.editor.has_grid(),
                    egui::Button::new("Seek to edit row"),
                )
                .on_hover_text("Cold seek: clears voices/tails; no reconstruction of earlier notes")
                .clicked()
            {
                if let (Some(state), Some(song)) =
                    (self.transport_shared(), self.transport_snapshot())
                {
                    let occurrence = self.tracker.editor.occurrence;
                    if !self
                        .mix_play_owner
                        .as_ref()
                        .is_some_and(|o| self.tracker.editor.owns_document(o))
                        || !crate::track_mix::same_playing_content(&self.tracker.editor.song, song)
                            .unwrap_or(false)
                        || occurrence >= song.order.len()
                        || song.order[occurrence].pattern != self.tracker.editor.pattern_id()
                        || self.tracker.editor.row
                            >= song.patterns[&song.order[occurrence].pattern].rows
                    {
                        self.error = Some(
                            "Edit position differs from playback snapshot; restart playback first"
                                .into(),
                        );
                    } else {
                        let rows: u64 = song.order[..occurrence]
                            .iter()
                            .map(|o| song.patterns[&o.pattern].rows as u64)
                            .sum();
                        let frame =
                            ((rows + self.tracker.editor.row as u64) as f64 * RATE as f64 * 60.0
                                / (song.bpm * song.lines_per_beat as f64))
                                .round() as u64;
                        state.seek(frame);
                    }
                }
            }
        });
        if self.audition.owned {
            let state = self.audition_panel_state(ui.ctx());
            let status = if state.stopping {
                "AUDITION STOPPING"
            } else if state.busy && state.ready {
                "AUDITION SNAPSHOT"
            } else if state.busy {
                "AUDITION PREPARING"
            } else if state.ended {
                "AUDITION ENDED"
            } else {
                "AUDITION STOPPED"
            };
            ui.label(
                RichText::new(format!(
                    "{status} · fixed 2s + 0.5s tail · no Song follow/seek/loop"
                ))
                .small()
                .color(MINT),
            );
            if let Some(error) = &self.audition.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            return;
        }
        if let (Some(state), Some(song)) = (self.transport_shared(), self.transport_snapshot()) {
            let frame = state.position();
            let (occ, row) = crate::transport::location(song, frame);
            let status = if self.transport_busy() {
                if state.quit.load(Ordering::Relaxed) {
                    "STOPPING"
                } else if state.is_ready() {
                    "PLAYING SNAPSHOT"
                } else {
                    "PREPARING / SEEKING"
                }
            } else if state.has_ended() {
                "ENDED"
            } else {
                "STOPPED"
            };
            ui.label(RichText::new(format!(
                "{status}  /  POS {:02}  ROW {:03}  /  {:.2}s  /  {} underrun frames",
                occ + 1,
                row,
                frame as f64 / RATE as f64,
                state.underruns.load(Ordering::Relaxed)
            )).monospace().small()).on_hover_text("Playback content uses the start snapshot. Only explicit authorized track-mix Apply can update its mix; other edits need restart. Enqueued mix acknowledgement is not heard output. Seek and loop-toggle clear voices. Loop restarts at row 0 without tail.");
        }

        if let Some(error) = self.transport_error() {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
    fn export_song(&mut self) {
        if let Err(error) = self.tracker.editor.song.validate() {
            self.error = Some(error);
            return;
        }
        let song = self.tracker.editor.song.clone();
        let project_dir = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_owned());
        self.start(
            Request::SongExport {
                song: Box::new(song),
                project_dir,
                output: PathBuf::from(self.song_output.trim()),
            },
            "Rendering song snapshot…",
        );
    }
    fn song_audio_controls(&mut self, ui: &mut egui::Ui) {
        let assigned = self
            .tracker
            .editor
            .song
            .instruments
            .get(&crate::song::InstrumentId(self.instrument_id))
            .and_then(|instrument| instrument.sample.clone());
        let source = (self.instrument_id, assigned);
        if self.asset_source.as_ref() != Some(&source) {
            self.asset_path = source
                .1
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            self.asset_source = Some(source);
        }
        egui::CollapsingHeader::new("Song audio · instruments & offline export").default_open(false).show(ui, |ui| {
            ui.add_enabled_ui(!self.session.close_ready && self.session.pending.is_none() && self.session.overwrite.is_none() && !self.multisample_editor.open && !self.resample_editor.open && !self.mix_editor.is_open(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    let before = self.instrument_id;
                    egui::ComboBox::from_id_salt("song-instrument").selected_text(format!("Instrument {}", self.instrument_id)).show_ui(ui, |ui| {
                        for (id, instrument) in &self.tracker.editor.song.instruments { ui.selectable_value(&mut self.instrument_id, id.0, format!("{} · {}", id.0, instrument.name)); }
                    });
                    if before != self.instrument_id { self.asset_path = self.tracker.editor.song.instruments.get(&crate::song::InstrumentId(self.instrument_id)).and_then(|i| i.sample.as_ref()).map(|p| p.display().to_string()).unwrap_or_default(); }
                    if ui.button("Melodic map…").clicked() {
                        self.tracker.thin.reset_document();
                        self.tracker.transforms.reset_document(); self.tracker.grooves.reset_document(); self.tracker.reset_generation_document();
                        self.multisample_editor.open_for(&self.tracker.editor, crate::song::InstrumentId(self.instrument_id));
                    }
                    if ui.button("Resample…").clicked() {
                        self.tracker.thin.reset_document();
                        self.tracker.transforms.reset_document(); self.tracker.grooves.reset_document(); self.tracker.reset_generation_document();
                        self.resample_editor.open_for(&self.tracker.editor, PathBuf::from(&self.song_output).with_file_name("resampled-occurrence.wav"));
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.asset_path).desired_width(230.0).hint_text("Sample path (blank unassigns)"));
                    if ui.add_enabled(self.native_picker.is_none(), egui::Button::new("Browse…")).clicked() {
                        self.open_native_sample_picker(
                            NativePickerTarget::InstrumentSample,
                            ui.ctx(),
                        );
                    }
                    if ui.add_enabled(self.tracker.editor.edit_mode, egui::Button::new("Assign sample")).clicked() {
                        let path = if self.asset_path.trim().is_empty() { None } else { Some(PathBuf::from(self.asset_path.trim())) };
                        let r = self.tracker.editor.assign_sample(crate::song::InstrumentId(self.instrument_id), path);
                        match r { Ok(()) => { self.error = None; self.status = "Instrument sample reference updated (undoable)".into(); }, Err(e) => self.error = Some(e) }
                    }
                });
                if ui.add_enabled(self.tracker.editor.has_grid() && !self.tracker.is_phrase_authoring(), egui::Button::new("Thin…")).on_hover_text("Preview seeded whole-cell deletion using explicitly reviewed region roles; no runtime recipe").clicked() {
                    self.tracker.transforms.reset_document(); self.tracker.grooves.reset_document(); self.tracker.reset_generation_document(); self.tracker.cell_blocks.close(); self.tracker.thin.open(&self.tracker.editor);
                }
                let assigned = self.tracker.editor.song.instruments.get(&crate::song::InstrumentId(self.instrument_id));
                let source = assigned.map_or("No source assigned".into(), |instrument| {
                    if let crate::song::InstrumentKind::DrumKit { kit } = &instrument.kind { format!("Assigned drum kit {} · edit in Pads (blank legacy path does not unassign it)", kit.0) }
                    else if let crate::song::InstrumentKind::Multisample { map } = &instrument.kind { format!("Assigned melodic map M{} · edit in Melodic map (blank legacy path does not detach it)", map.0) }
                    else if let Some(region) = instrument.region { format!("Assigned region {} · edit in Samples (blank legacy path does not unmap it)", region.0) }
                    else { instrument.sample.as_ref().map_or("No source assigned".into(), |path| format!("Assigned: {}", path.display())) }
                });
                ui.label(RichText::new(source).small().weak());
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.song_output).desired_width(300.0).hint_text("New song WAV path"));
                    if ui.add_enabled(!self.jobs.busy() && !self.song_output.trim().is_empty(), egui::Button::new("Export song WAV")).clicked() { self.export_song(); }
                });
                ui.label(RichText::new("Exports current order snapshot. Edits after click affect the next export; transport is independent.").small().weak());
            });
        });
    }
    fn start(&mut self, r: Request, status: &str) {
        if self.session.close_ready {
            return;
        }
        match self.jobs.start(r) {
            Ok(()) => {
                self.status = status.into();
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn load(&mut self) {
        let path = PathBuf::from(self.sample_path.trim());
        self.start(Request::Load(path), "Loading sample…");
    }
    fn export(&mut self) {
        if let Some(sample) = &self.sample {
            self.start(
                Request::Export {
                    sample: sample.path.clone(),
                    output: PathBuf::from(self.output_path.trim()),
                    bpm: self.bpm,
                    bars: self.bars,
                },
                "Rendering offline…",
            );
        }
    }
    fn timeline(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Pattern");
            ui.label(RichText::new("C major seventh motif · 16 steps per bar").weak());
        });
        ui.label("Read-only pattern • C4 / E4 / G4 / B4 • 50% gate • alternating pan");
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(
                ui.available_width(),
                (ui.available_height() - 170.0).clamp(110.0, 220.0),
            ),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(13, 17, 23));
        let left = rect.left() + 42.0;
        let width = rect.width() - 54.0;
        let total = self.manifest.musical_frames as f32;
        for i in 0..=16 {
            let x = left + width * i as f32 / 16.0;
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + 27.0),
                    egui::pos2(x, rect.bottom() - 12.0),
                ],
                egui::Stroke::new(
                    1.0_f32,
                    if i % 4 == 0 {
                        Color32::from_gray(55)
                    } else {
                        Color32::from_gray(30)
                    },
                ),
            );
        }
        painter.text(
            egui::pos2(left, rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            format!(
                "01                                      {} bars / {:.2} s",
                self.bars,
                total / RATE as f32
            ),
            egui::FontId::monospace(12.0),
            Color32::GRAY,
        );
        for (row, (note, label)) in [(71, "B4"), (67, "G4"), (64, "E4"), (60, "C4")]
            .iter()
            .enumerate()
        {
            let row_height = (rect.height() - 48.0) / 4.0;
            let y = rect.top() + 36.0 + row as f32 * row_height;
            painter.text(
                egui::pos2(rect.left() + 10.0, y + 10.0),
                egui::Align2::LEFT_CENTER,
                *label,
                egui::FontId::monospace(12.0),
                Color32::GRAY,
            );
            for e in &self.manifest.events {
                if let Event::NoteOn { midi_note, .. } = e.event {
                    if midi_note == *note {
                        let x = left + e.frame as f32 / total * width;
                        let length = (width / (self.bars * 32) as f32).max(1.0);
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(x, y),
                                Vec2::new(length, (row_height - 6.0).clamp(8.0, 22.0)),
                            ),
                            0.0,
                            MINT,
                        );
                    }
                }
            }
        }
    }
}
impl eframe::App for Leviathan {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_native_sample_picker();
        self.guard_mix_owner();
        self.mix_editor.poll();
        if self.session.close_ready
            || self.session.pending.is_some()
            || self.session.overwrite.is_some()
            || ctx.input(|i| {
                i.viewport().close_requested()
                    || !i.focused
                    || i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::WindowFocused(false)))
            })
        {
            self.mix_editor.cancel_authorization();
        }
        if self.mix_editor.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.refresh_mix_status();
        self.guard_audition(ctx);
        self.poll_transport();
        let audition = self.audition_panel_state(ctx);
        self.sample_editor.audition = audition.clone();
        self.tracker.pads.audition = audition;
        if !self.sample_view
            || self.mix_editor.is_open()
            || self.legacy_tools
            || self.session.close_ready
            || self.session.pending.is_some()
            || self.session.overwrite.is_some()
            || ctx.input(|input| input.viewport().close_requested())
        {
            self.sample_editor.cancel_commit();
        }
        if !self.resample_editor.open
            || self.sample_view
            || self.legacy_tools
            || self.session.close_ready
            || self.session.pending.is_some()
            || self.session.overwrite.is_some()
            || ctx.input(|i| i.viewport().close_requested())
        {
            self.resample_editor.cancel_commit();
        }
        if self.session.close_ready || ctx.input(|i| i.viewport().close_requested()) {
            self.resample_editor.cancel_task();
        }
        if self.sample_view || self.legacy_tools {
            self.resample_editor.hide();
        }
        self.resample_editor.poll();
        if let Some(notice) = self.resample_editor.take_event() {
            self.status = notice;
        }
        if self.resample_editor.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.sample_editor.poll();
        if self.legacy_tools {
            self.multisample_editor.open = false;
            self.resample_editor.hide();
            self.return_to_multisample = false;
        }
        self.tracker.steps.poll();
        self.tracker.transforms.poll();
        self.tracker.cell_blocks.poll();
        if (self.sample_view
            || self.legacy_tools
            || self.multisample_editor.open
            || self.resample_editor.open)
            && self.tracker.cell_blocks.is_open()
        {
            self.tracker.cell_blocks.close();
        }
        if self.tracker.cell_blocks.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.tracker.thin.poll();
        if (self.sample_view || self.legacy_tools) && self.tracker.thin.is_open() {
            self.tracker.thin.reset_document();
        }
        if self.tracker.thin.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.tracker.grooves.poll();
        if (self.sample_view || self.legacy_tools) && self.tracker.grooves.is_open() {
            self.tracker.grooves.reset_document();
        }
        if self.tracker.grooves.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.tracker.generation.poll();
        if (self.sample_view || self.legacy_tools) && self.tracker.generation.is_open() {
            self.tracker.generation.reset_document();
        }
        if self.tracker.generation.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        if (self.sample_view || self.legacy_tools) && self.tracker.transforms.is_open() {
            self.tracker.thin.reset_document();
            self.tracker.transforms.reset_document();
        }
        if self.tracker.transforms.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        if self.tracker.steps.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        if self.sample_editor.busy() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
        #[cfg(feature = "realtime")]
        {
            if self.transport_busy() {
                ctx.request_repaint_after(Duration::from_millis(20));
            }
            self.tracker.playhead = None;
            if self.transport_busy() && !self.audition.owned {
                if let (Some(state), Some(song)) =
                    (self.transport_shared(), self.transport_snapshot())
                {
                    let (occ, row) = crate::transport::location(song, state.position());
                    // IDs can be reused after New/Open, and editing can change timing.
                    // Only annotate the editor when it still matches the playing snapshot.
                    if self.mix_content_cache == Some((self.tracker.editor.revision(), true))
                        && self
                            .mix_play_owner
                            .as_ref()
                            .is_some_and(|o| self.tracker.editor.owns_document(o))
                        && state.is_running()
                    {
                        self.tracker.playhead = Some((occ, row));
                    }
                }
            }
            if self.transport_smoke {
                if let Some(error) = &self.transport.error {
                    panic!("Transport smoke failed: {error}");
                }
                if let Some(state) = &self.transport.shared {
                    if state
                        .audible_frames
                        .load(std::sync::atomic::Ordering::Relaxed)
                        > 24000
                        && !self.capture_requested
                    {
                        self.export_finished = true;
                    }
                }
                if self.capture_requested && self.capture.is_none() {
                    self.stop_transport();
                }
            }
        }
        if let Some(result) = self.jobs.poll() {
            match result {
                Ok(Completed::Loaded(info)) => {
                    self.status = format!(
                        "Loaded {}",
                        info.path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    self.sample = Some(info);
                    if self.capture.is_some() {
                        self.export();
                    }
                }
                Ok(Completed::Exported { path, stats }) => {
                    self.status = format!(
                        "Exported {} · {} frames · peak {:.3}",
                        path.display(),
                        stats.frames,
                        stats.peak
                    );
                    self.export_finished = true;
                }
                Err(error) => {
                    self.status = "Job failed".into();
                    eprintln!("Leviathan job error: {error}");
                    self.error = Some(error);
                    if self.capture.is_some() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = self.capture.take() {
                    let saved = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let file = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)?;
                        let pixels: Vec<u8> =
                            image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        let rgba = image::RgbaImage::from_raw(
                            image.width() as u32,
                            image.height() as u32,
                            pixels,
                        )
                        .ok_or("invalid screenshot")?;
                        image::DynamicImage::ImageRgba8(rgba).write_to(
                            &mut std::io::BufWriter::new(file),
                            image::ImageFormat::Png,
                        )?;
                        Ok(())
                    })();
                    eprintln!("UI smoke capture {}: {:?}", path.display(), saved);
                    self.stop_transport();
                    self.session.close_ready = true;
                }
            }
        }
        if self.export_finished && self.capture.is_some() && !self.capture_requested {
            self.capture_frames += 1;
            if self.capture_frames >= 3 {
                self.capture_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.request_repaint();
        }
        if self.jobs.busy() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.capture_requested
            && !self.session.close_ready
        {
            self.stop_transport();
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.session.close_ready
                && self.session.pending.is_none()
                && self.session.overwrite.is_none()
            {
                self.request_action(crate::session::Action::Close);
            }
        }
        if self.session.close_ready {
            if self.jobs.busy()
                || self.sample_editor.busy()
                || self.resample_editor.busy()
                || self.mix_editor.busy()
                || self.tracker.steps.busy()
                || self.tracker.transforms.busy()
                || self.tracker.thin.busy()
                || self.tracker.grooves.busy()
                || self.tracker.generation.busy()
                || self.transport_busy()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !self.jobs.busy() && !self.session.close_ready {
            if let Some(path) =
                ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()))
            {
                self.sample_path = path.display().to_string();
                self.load();
            }
        }
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("LEVIATHAN").size(19.0).strong().color(MINT));
                ui.separator();
                ui.add(
                    egui::Label::new(RichText::new(&self.tracker.editor.song.title).strong())
                        .truncate(),
                );
                if self.tracker.editor.dirty {
                    ui.label(
                        RichText::new("● UNSAVED")
                            .small()
                            .color(Color32::from_rgb(229, 185, 112)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .checkbox(&mut self.legacy_tools, "Legacy tools")
                        .changed()
                    {
                        self.mix_editor.close();
                    }
                    let mut steps = self.tracker.steps.open && !self.sample_view;
                    if ui.toggle_value(&mut steps, "Steps").clicked() {
                        self.mix_editor.close();
                        self.multisample_editor.open = false;
                        self.resample_editor.hide();
                        self.return_to_multisample = false;
                        self.legacy_tools = false;
                        self.sample_view = false;
                        self.tracker.set_step_view(steps);
                    }
                    let mut pads = self.tracker.pads.open && !self.sample_view;
                    if ui.toggle_value(&mut pads, "Pads").clicked() {
                        self.mix_editor.close();
                        self.multisample_editor.open = false;
                        self.resample_editor.hide();
                        self.return_to_multisample = false;
                        self.legacy_tools = false;
                        self.sample_view = false;
                        self.tracker.set_pad_view(pads);
                    }
                    if ui
                        .selectable_value(&mut self.sample_view, true, "Samples")
                        .clicked()
                    {
                        self.mix_editor.close();
                        self.legacy_tools = false;
                        self.multisample_editor.open = false;
                        self.resample_editor.hide();
                        self.return_to_multisample = false;
                    }
                    if self.sample_view
                        && ui
                            .selectable_value(&mut self.sample_view, false, "Song")
                            .clicked()
                    {
                        self.mix_editor.close();
                        if self.return_to_multisample {
                            self.multisample_editor.open = true;
                            self.return_to_multisample = false;
                        }
                    }
                });
            });
        });
        self.project_controls(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.jobs.busy()
                    || self.sample_editor.busy()
                    || self.resample_editor.busy()
                    || self.mix_editor.busy()
                    || self.tracker.steps.busy()
                    || self.tracker.transforms.busy()
                    || self.tracker.thin.busy()
                    || self.tracker.grooves.busy()
                    || self.tracker.generation.busy()
                    || self.transport_busy()
                {
                    ui.spinner();
                } else {
                    ui.colored_label(MINT, "Ready");
                }
                ui.add(egui::Label::new(&self.status).truncate());
            });
            if self.session.close_ready
                && (self.jobs.busy()
                    || self.sample_editor.busy()
                    || self.resample_editor.busy()
                    || self.mix_editor.busy()
                    || self.tracker.steps.busy()
                    || self.tracker.transforms.busy()
                    || self.tracker.thin.busy()
                    || self.tracker.grooves.busy()
                    || self.tracker.generation.busy())
            {
                ui.label("Finishing the active job before closing…");
            }
            if let Some(e) = &self.error {
                ui.colored_label(Color32::from_rgb(255, 145, 145), e);
            }
        });
        if self.legacy_tools {
            egui::SidePanel::left("source").exact_width(290.0).show(ctx,|ui|{
   ui.add_space(16.0);ui.heading("Fixed-motif source");ui.label("Drop an audio file here, or enter its path.");
   ui.add_enabled_ui(!self.jobs.busy(),|ui|{ui.add(egui::TextEdit::singleline(&mut self.sample_path).hint_text("/path/to/sample.wav").desired_width(f32::INFINITY));ui.horizontal(|ui|{if ui.add_enabled(self.native_picker.is_none(),egui::Button::new("Browse…")).clicked(){self.open_native_sample_picker(NativePickerTarget::LegacySample,ui.ctx());}if ui.add_enabled(!self.sample_path.trim().is_empty(),egui::Button::new("Load sample").min_size(Vec2::new(180.0,32.0))).clicked(){self.load();}});});
   if let Some(s)=&self.sample{ui.add_space(6.0);ui.colored_label(MINT,s.path.file_name().unwrap_or_default().to_string_lossy());ui.label(format!("{} Hz · {} channels · {:.2} s",s.rate,s.channels,s.frames as f64/s.rate as f64));ui.label(RichText::new(s.path.display().to_string()).small().weak());}else{ui.label(RichText::new("No sample loaded").weak());}
   ui.add_space(22.0);ui.separator();ui.heading("Legacy motif export");ui.colored_label(MINT,"Tracker edits are NOT rendered here.");ui.label("New WAV destination");ui.add_enabled_ui(!self.jobs.busy(),|ui|{ui.add(egui::TextEdit::singleline(&mut self.output_path).desired_width(f32::INFINITY));});
   ui.label(RichText::new("Writes WAV + event JSON. Existing files are never replaced.").small().weak());
   if ui.add_enabled(self.sample.is_some()&&!self.jobs.busy(),egui::Button::new(RichText::new("Export WAV").strong()).fill(Color32::from_rgb(35,107,89)).min_size(Vec2::new(260.0,40.0))).clicked(){self.export();}
   ui.add_space(24.0);ui.separator();ui.label(RichText::new("ENGINE").small().strong());ui.label("phonic 0.16  +  pattrns 0.9.3");ui.label(RichText::new("Sample playback and deterministic rendering are connected. MIDI, plugins and discovery are not installed features.").small().weak());
  });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.legacy_tools {
                self.transport_controls(ui);
                if self.sample_view {
                    let base = self.session.path.as_ref().and_then(|path| path.parent());
                    ui.add_enabled_ui(
                        !self.session.close_ready
                            && self.session.pending.is_none()
                            && self.session.overwrite.is_none()
                            && !self.mix_editor.is_open(),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("source-inspector")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    if self.sample_editor.show(ui, &mut self.tracker.editor, base) {
                                        self.sample_view = false;
                                        if self.return_to_multisample {
                                            self.multisample_editor.open = true;
                                            self.return_to_multisample = false;
                                        }
                                    }
                                    if let Some((occurrence, track)) =
                                        self.sample_editor.take_generated_position()
                                    {
                                        if let Err(error) =
                                            self.tracker.open_song_position(occurrence, track)
                                        {
                                            self.error = Some(error);
                                        }
                                    }
                                    if let Some((instrument, pad)) =
                                        self.sample_editor.take_marker_bank_kit()
                                    {
                                        self.sample_view = false;
                                        self.return_to_multisample = false;
                                        self.tracker.set_pad_view(true);
                                        self.tracker.pads.select_projected_pad(
                                            &self.tracker.editor,
                                            instrument,
                                            pad,
                                        );
                                    }
                                });
                        },
                    );
                    return;
                }
                self.song_audio_controls(ui);
                #[cfg(feature = "realtime")]
                if self.follow
                    && !self.audition.owned
                    && self.transport_busy()
                    && !self.multisample_editor.open
                    && !self.resample_editor.open
                {
                    if let (Some(state), Some(song)) =
                        (self.transport_shared(), self.transport_snapshot())
                    {
                        let (occ, row) = crate::transport::location(song, state.position());
                        crate::tracker_ui::playback_view(ui, song, occ, row);
                        return;
                    }
                }
                ui.add_enabled_ui(
                    !self.session.close_ready
                        && self.session.pending.is_none()
                        && self.session.overwrite.is_none(),
                    |ui| {
                        ui.add_enabled_ui(
                            !self.multisample_editor.open
                                && !self.resample_editor.open
                                && !self.mix_editor.is_open(),
                            |ui| self.tracker.show(ui),
                        );
                        self.multisample_editor.show(ui, &mut self.tracker.editor);
                        self.resample_editor.show(ui, &mut self.tracker.editor);
                        if self.multisample_editor.open {
                            if let Some(id) = self.multisample_editor.selected_instrument() {
                                self.instrument_id = id.0;
                            }
                        }
                    },
                );
                if let Some((region, instrument)) = self.multisample_editor.take_region_request() {
                    let base = self.session.path.as_ref().and_then(|path| path.parent());
                    match self.sample_editor.inspect_region(
                        &self.tracker.editor,
                        region,
                        instrument,
                        base,
                    ) {
                        Ok(()) => {
                            self.sample_view = true;
                            self.return_to_multisample = true;
                        }
                        Err(error) => {
                            self.error = Some(error);
                            self.multisample_editor.open = true;
                        }
                    }
                }
                if let Some((region, instrument)) = self.tracker.pads.take_region_request() {
                    let base = self.session.path.as_ref().and_then(|path| path.parent());
                    match self.sample_editor.inspect_region(
                        &self.tracker.editor,
                        region,
                        instrument,
                        base,
                    ) {
                        Ok(()) => self.sample_view = true,
                        Err(error) => self.error = Some(error),
                    }
                }
                return;
            }
            ui.colored_label(
                MINT,
                "LEGACY FIXED MOTIF — independent of the editable song",
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.heading("Transport");
                ui.label(
                    RichText::new(if self.jobs.busy() {
                        "WORKING"
                    } else {
                        "STOPPED"
                    })
                    .color(MINT),
                );
            });
            ui.horizontal(|ui| {
                ui.add_enabled(false, egui::Button::new("▶ Live play"))
                    .on_hover_text("Live pattern scheduling is not implemented");
                ui.label("Offline session · no live playback");
            });
            ui.add_space(6.0);
            let mut changed = false;
            ui.add_enabled_ui(!self.jobs.busy(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("BPM");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.bpm)
                                .range(20.0..=400.0)
                                .speed(0.25),
                        )
                        .changed();
                    ui.label("Bars");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.bars).range(1..=256))
                        .changed();
                    ui.label("4 / 4");
                    ui.separator();
                    ui.label(format!(
                        "{} frames + 0.5 s tail",
                        self.manifest.musical_frames
                    ));
                });
            });
            if changed {
                match pattern_manifest(self.bpm, self.bars) {
                    Ok(m) => self.manifest = m,
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            ui.add_space(18.0);
            self.timeline(ui);
            ui.add_space(15.0);
            ui.horizontal(|ui| {
                ui.heading("Event inspector");
                ui.label(format!(
                    "{} deterministic events",
                    self.manifest.events.len()
                ));
            });
            ui.label(
                RichText::new("FRAME          TIME        EVENT / NOTE / VOICE")
                    .monospace()
                    .weak(),
            );
            egui::ScrollArea::vertical()
                .id_salt("events")
                .auto_shrink([false, false])
                .show_rows(ui, 22.0, self.manifest.events.len(), |ui, range| {
                    for i in range {
                        let e = &self.manifest.events[i];
                        let text = match e.event {
                            Event::NoteOn { id, midi_note, .. } => {
                                format!("ON    MIDI {midi_note:<3}  voice {id}")
                            }
                            Event::NoteOff { id } => format!("OFF             voice {id}"),
                            Event::AllNotesOff => "ALL NOTES OFF".into(),
                        };
                        ui.label(
                            RichText::new(format!(
                                "{:>8}     {:>7.3}s     {}",
                                e.frame,
                                e.frame as f64 / RATE as f64,
                                text
                            ))
                            .monospace(),
                        );
                    }
                });
        });
        let mix_allowed = !self.legacy_tools
            && !self.session.close_ready
            && self.session.pending.is_none()
            && self.session.overwrite.is_none()
            && !ctx.input(|i| i.viewport().close_requested());
        self.refresh_meters(ctx);
        self.mix_editor.show(
            ctx,
            &mut self.tracker.editor,
            mix_allowed,
            &self.mix_delivery,
        );
        let explicit_mix_apply = self.mix_editor.take_applied();
        self.audition_actions(ctx);
        self.finish_mix_frame(explicit_mix_apply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    fn render(app: &mut Leviathan, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = render_output(app, ctx, events);
    }
    fn render_output(
        app: &mut Leviathan,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        )
    }
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn closing_and_modal_decisions_freeze_grid_keys_then_cancel_restores_editing() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        render(&mut app, &ctx, vec![]);
        ctx.memory_mut(|m| m.request_focus(crate::tracker_ui::grid_id()));
        let before = app.tracker.editor.song.clone();
        app.session.close_ready = true;
        render(
            &mut app,
            &ctx,
            vec![key(egui::Key::Z), key(egui::Key::ArrowDown)],
        );
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.row, 0);
        app.session.close_ready = false;
        app.tracker
            .editor
            .enter_note(crate::song::Note::Off)
            .unwrap();
        app.session
            .request(&mut app.tracker.editor, crate::session::Action::Close)
            .unwrap();
        let before = app.tracker.editor.song.clone();
        render(&mut app, &ctx, vec![key(egui::Key::X)]);
        assert_eq!(app.tracker.editor.song, before);
        app.session.cancel();
        render(&mut app, &ctx, vec![]);
        ctx.memory_mut(|m| m.request_focus(crate::tracker_ui::grid_id()));
        render(&mut app, &ctx, vec![key(egui::Key::X)]);
        assert_ne!(app.tracker.editor.song, before);
    }
    #[test]
    fn instrument_input_initializes_from_assignment_without_overwriting_typed_draft() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let sample = PathBuf::from("existing-sample.wav");
        app.tracker
            .editor
            .song
            .instruments
            .get_mut(&crate::song::InstrumentId(1))
            .unwrap()
            .sample = Some(sample);
        render(&mut app, &ctx, vec![]);
        assert_eq!(app.asset_path, "existing-sample.wav");
        app.asset_path = "draft-replacement.wav".into();
        render(&mut app, &ctx, vec![]);
        assert_eq!(app.asset_path, "draft-replacement.wav");
        app.tracker
            .editor
            .song
            .instruments
            .get_mut(&crate::song::InstrumentId(1))
            .unwrap()
            .sample = None;
        render(&mut app, &ctx, vec![]);
        assert!(app.asset_path.is_empty());
    }
    fn control(output: &egui::FullOutput, wanted: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                    let pos = text.pos + text.galley.size() * 0.5;
                    shape.clip_rect.contains(pos).then_some(pos)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Visible control missing: {wanted}"))
    }
    fn click_app(app: &mut Leviathan, ctx: &egui::Context, pos: egui::Pos2) {
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    #[test]
    fn actual_remove_initial_or_compiled_occurrence_preserves_content_and_undo_cursor() {
        use crate::{
            break_compiler::{self, CompileConfig, CompileTiming, TailPolicy},
            song::{MarkerSet, MarkerSetId, SampleAsset, SampleId},
        };
        for selected in [0, 1] {
            let ctx = egui::Context::default();
            let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
            app.tracker.editor.edit_mode = true;
            // Pure compiler fixture: no audio IO or claimed playback in this UI test.
            let mut base = app.tracker.editor.song.clone();
            base.order.truncate(1);
            for pattern in base.patterns.values_mut() {
                pattern.tracks.clear();
            }
            base.register_sample(
                SampleId(1),
                SampleAsset {
                    path: "ui-model-fixture.wav".into(),
                    sha256: "0".repeat(64),
                    sample_rate: 48_000,
                    channels: 1,
                    frames: 48_000,
                },
            )
            .unwrap();
            base.upsert_marker_set(
                MarkerSetId(1),
                MarkerSet {
                    name: "Reviewed markers".into(),
                    sample: SampleId(1),
                    source_bpm: Some(70.0),
                    locked_frames: vec![],
                    markers: vec![0, 24_000, 48_000],
                },
            )
            .unwrap();
            let plan = break_compiler::preview(
                &base,
                &CompileConfig {
                    marker_set: MarkerSetId(1),
                    target_bpm: 170.0,
                    timing: CompileTiming::SourceRhythm,
                    cycles: 1,
                    roll: None,
                    tail: TailPolicy::CutPrevious,
                    minimum_slice_frames: 1,
                    root_note: 60,
                    pad_tails: true,
                    name: "Generated".into(),
                },
            )
            .unwrap();
            app.tracker
                .editor
                .apply_document_edit(plan.candidate().clone())
                .unwrap();
            app.tracker
                .editor
                .select(selected, plan.summary().track)
                .unwrap();
            app.tracker.editor.row = if selected == 0 { 31 } else { 1 };
            let row = app.tracker.editor.row;
            let before = app.tracker.editor.song.clone();
            render(&mut app, &ctx, vec![]);
            let output = render_output(&mut app, &ctx, vec![]);
            click_app(&mut app, &ctx, control(&output, "Remove"));
            let mut expected = before.clone();
            expected.order.remove(selected);
            assert_eq!(
                app.tracker.editor.song, expected,
                "Only the order reference may change"
            );
            assert_eq!(app.tracker.editor.occurrence, 0);
            assert!(app.tracker.editor.row < app.tracker.editor.pattern().rows);
            let output = render_output(&mut app, &ctx, vec![]);
            click_app(&mut app, &ctx, control(&output, "Undo"));
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(app.tracker.editor.occurrence, selected);
            assert_eq!(app.tracker.editor.row, row);
            let output = render_output(&mut app, &ctx, vec![]);
            click_app(&mut app, &ctx, control(&output, "Redo"));
            assert_eq!(app.tracker.editor.song, expected);
            let revision = app.tracker.editor.revision();
            let output = render_output(&mut app, &ctx, vec![]);
            click_app(&mut app, &ctx, control(&output, "Remove"));
            assert_eq!(
                app.tracker.editor.song, expected,
                "Final occurrence must stay"
            );
            assert_eq!(app.tracker.editor.revision(), revision);
        }
    }
    #[test]
    fn actual_remove_repeated_reference_keeps_every_shared_pattern() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        let original = app.tracker.editor.song.clone();
        render(&mut app, &ctx, vec![]);
        let output = render_output(&mut app, &ctx, vec![]);
        click_app(&mut app, &ctx, control(&output, "+ Repeat"));
        let repeated = app.tracker.editor.song.clone();
        assert_eq!(repeated.order.len(), original.order.len() + 1);
        let output = render_output(&mut app, &ctx, vec![]);
        click_app(&mut app, &ctx, control(&output, "Remove"));
        assert_eq!(app.tracker.editor.song, original);
        let output = render_output(&mut app, &ctx, vec![]);
        click_app(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, repeated);
    }

    #[test]
    fn remove_occurrence_respects_read_mode_modal_close_and_empty_order() {
        for blocked in 0..3 {
            let ctx = egui::Context::default();
            let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
            app.tracker.editor.edit_mode = true;
            app.tracker.editor.append(false).unwrap();
            let before = app.tracker.editor.song.clone();
            if blocked == 0 {
                app.tracker.editor.edit_mode = false;
            }
            if blocked == 1 {
                app.session.close_ready = true;
            }
            if blocked == 2 {
                app.session
                    .request(&mut app.tracker.editor, crate::session::Action::Close)
                    .unwrap();
            }
            render(&mut app, &ctx, vec![]);
            let output = render_output(&mut app, &ctx, vec![]);
            click_app(&mut app, &ctx, control(&output, "Remove"));
            assert_eq!(app.tracker.editor.song, before);
        }
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.song.order.clear();
        let output = render_output(&mut app, &ctx, vec![]);
        control(&output, "No pattern selected");
        assert!(app.tracker.editor.song.order.is_empty());
    }

    #[test]
    fn actual_song_timing_popup_is_undoable_and_independent_of_legacy_tempo() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        render(&mut app, &ctx, vec![]);
        let output = render_output(&mut app, &ctx, vec![]);
        click_app(&mut app, &ctx, control(&output, "120.0 BPM  /  4 LPB"));
        let output = render_output(&mut app, &ctx, vec![]);
        control(&output, "SONG TIMING");
        let number = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text.parse::<f64>().ok() == Some(120.0) =>
                {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Real BPM numeric input");
        click_app(&mut app, &ctx, number);
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                },
                egui::Event::Text("170".into()),
                key(egui::Key::Enter),
            ],
        );
        assert_eq!(app.timing_draft.0, 170.0);
        assert_eq!(
            app.tracker.editor.song.bpm, 120.0,
            "Typing must remain a draft"
        );
        assert!(!ctx.memory(|memory| memory.has_focus(crate::tracker_ui::grid_id())));
        let before = app.tracker.editor.song.clone();
        let output = render_output(&mut app, &ctx, vec![]);
        click_app(&mut app, &ctx, control(&output, "Apply timing"));
        assert_eq!(app.tracker.editor.song.bpm, 170.0);
        assert_eq!(app.bpm, 120.0);
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn already_open_song_timing_popup_cannot_commit_after_close_or_modal() {
        for closing in [true, false] {
            let ctx = egui::Context::default();
            let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
            app.tracker.editor.edit_mode = true;
            render(&mut app, &ctx, vec![]);
            let output = render_output(&mut app, &ctx, vec![]);
            click_app(&mut app, &ctx, control(&output, "120.0 BPM  /  4 LPB"));
            render(&mut app, &ctx, vec![]);
            app.timing_draft = (170.0, 8);
            let output = render_output(&mut app, &ctx, vec![]);
            let apply = control(&output, "Apply timing");
            if closing {
                app.session.close_ready = true;
            } else {
                app.session.pending = Some(crate::session::Action::New);
            }
            let before = app.tracker.editor.song.clone();
            click_app(&mut app, &ctx, apply);
            assert_eq!(app.tracker.editor.song, before);
        }
    }
    struct PadFixture(std::path::PathBuf);
    impl Drop for PadFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn pad_app() -> (Leviathan, egui::Context, PadFixture) {
        use crate::song::*;
        let directory = std::env::temp_dir().join(format!(
            "leviathan-pad-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("owned.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for frame in 0..4800 {
            wav.write_sample(((frame % 80) as i16 - 40) * 100).unwrap();
        }
        wav.finalize().unwrap();
        let source = crate::sample_asset::import(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let mut song = app.tracker.editor.song.clone();
        song.order.truncate(1);
        for pattern in song.patterns.values_mut() {
            pattern.tracks.clear();
        }
        song.register_sample(SampleId(1), source.asset).unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                name: "Owned hit".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 4800,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        song.upsert_drum_kit(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Sixteen slots".into(),
                pads: (0..16)
                    .map(|slot| {
                        (
                            PadId(u64::from(slot) + 1),
                            DrumPad {
                                slot,
                                name: format!("Pad {:02}", slot + 1),
                                note: 36 + slot,
                                region: (slot != 15).then_some(RegionId(1)),
                                velocity_zones: vec![],
                                root_note: 60,
                                tune_semitones: 0,
                                gain: 1.0,
                                pan: 0.0,
                                mode: PadMode::OneShot,
                                new_note: PadNewNote::Overlap,
                                choke_group: None,
                            },
                        )
                    })
                    .collect(),
            },
        )
        .unwrap();
        song.assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.edit_step = 2;
        render(&mut app, &ctx, vec![]);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Pads"));
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Choose instrument"));
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "1 · Instrument 1"));
        (app, ctx, PadFixture(directory))
    }
    fn pad_click(app: &mut Leviathan, ctx: &egui::Context, pos: egui::Pos2) {
        render(app, ctx, vec![egui::Event::PointerMoved(pos)]);
        click_app(app, ctx, pos);
        render(app, ctx, vec![]);
    }
    fn pad_control(app: &mut Leviathan, ctx: &egui::Context, wanted: &str) -> egui::Pos2 {
        let mut previous = None;
        for _ in 0..80 {
            let output = render_output(app, ctx, vec![]);
            if let Some(pos) = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                        let pos = text.pos + text.galley.size() * 0.5;
                        shape.clip_rect.contains(pos).then_some(pos)
                    }
                    _ => None,
                })
            {
                if previous.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.05) {
                    return pos;
                }
                previous = Some(pos);
                continue;
            }
            previous = None;
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(90.0, 450.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -80.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let output = render_output(app, ctx, vec![]);
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    Some((text.galley.job.text.clone(), text.pos, shape.clip_rect))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        panic!("Pad control not reachable: {wanted}; {labels:?}");
    }
    fn pad_type(app: &mut Leviathan, ctx: &egui::Context, old: &str, value: &str) {
        let pos = pad_control(app, ctx, old);
        pad_click(app, ctx, pos);
        render(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                },
                egui::Event::Text(value.into()),
                key(egui::Key::Enter),
            ],
        );
    }
    #[test]
    fn actual_pad_selection_insertion_shared_events_undo_and_return_cursor() {
        use crate::song::{DrumKitId, InstrumentId, Note, PadId};
        let (mut app, ctx, _fixture) = pad_app();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let label = format!("01\n{}", crate::editor::note_label(Note::Note(36)));
        let pos = pad_control(&mut app, &ctx, &label);
        pad_click(&mut app, &ctx, pos);
        assert_eq!(
            app.tracker.editor.song, before,
            "Select must not write a note"
        );
        assert_eq!(app.tracker.editor.revision(), revision);
        let mut expected = app.tracker.editor.clone();
        expected.insert_pad_note(InstrumentId(1), PadId(1)).unwrap();
        let pos = pad_control(&mut app, &ctx, "Insert note");
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, expected.song);
        assert_eq!(app.tracker.editor.row, 2);
        assert_eq!(
            crate::song_render::evaluate(&app.tracker.editor.song, None)
                .unwrap()
                .events,
            crate::song_render::evaluate(&expected.song, None)
                .unwrap()
                .events
        );
        assert_eq!(
            app.tracker.editor.song.drum_kits[&DrumKitId(1)].pads.len(),
            16
        );
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.row, 0);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Redo"));
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
        );
        let pos = pad_control(&mut app, &ctx, "< Order");
        pad_click(&mut app, &ctx, pos);
        assert!(!app.tracker.pads.open);
        assert_eq!(
            cursor,
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column
            )
        );
    }
    #[test]
    fn held_pad_click_inserts_once_and_read_unmapped_and_close_do_not_insert() {
        use crate::song::Note;
        let (mut app, ctx, _fixture) = pad_app();
        let pos = pad_control(&mut app, &ctx, "Insert");
        pad_click(&mut app, &ctx, pos);
        let label = format!("01\n{}", crate::editor::note_label(Note::Note(36)));
        let pos = pad_control(&mut app, &ctx, &label);
        render(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]);
        let before = app.tracker.editor.song.clone();
        render(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..10 {
            render(&mut app, &ctx, vec![]);
        }
        assert_eq!(app.tracker.editor.song, before);
        render(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.tracker.editor.row, 2);
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.row, 4);
        let before = app.tracker.editor.song.clone();
        app.tracker.editor.edit_mode = false;
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, before);
        app.tracker.editor.edit_mode = true;
        let insert = pad_control(&mut app, &ctx, "Insert");
        pad_click(&mut app, &ctx, insert);
        let label = format!("16\n{}", crate::editor::note_label(Note::Note(51)));
        let unmapped = pad_control(&mut app, &ctx, &label);
        pad_click(&mut app, &ctx, unmapped);
        assert_eq!(app.tracker.editor.song, before);
        app.session.close_ready = true;
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn actual_mapping_draft_applies_once_and_is_frozen_after_modal_or_close() {
        use crate::song::{DrumKitId, PadId};
        for blocked in [0, 1, 2] {
            let (mut app, ctx, _fixture) = pad_app();
            let pos = pad_control(&mut app, &ctx, "01\nC2");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "Edit mapping");
            pad_click(&mut app, &ctx, pos);
            pad_type(&mut app, &ctx, "Pad 01", "Changed pad");
            assert!(!ctx.memory(|memory| memory.has_focus(crate::tracker_ui::grid_id())));
            let before = app.tracker.editor.song.clone();
            let apply = pad_control(&mut app, &ctx, "Apply mapping");
            if blocked == 1 {
                app.session.close_ready = true;
            } else if blocked == 2 {
                app.session.pending = Some(crate::session::Action::New);
            }
            pad_click(&mut app, &ctx, apply);
            if blocked == 0 {
                assert_eq!(
                    app.tracker.editor.song.drum_kits[&DrumKitId(1)].pads[&PadId(1)].name,
                    "Changed pad"
                );
                app.tracker.editor.undo();
                assert_eq!(app.tracker.editor.song, before);
            } else {
                assert_eq!(app.tracker.editor.song, before);
            }
        }
    }
    #[test]
    fn stale_mapping_after_undo_cannot_recreate_removed_kit_or_pad() {
        use crate::song::{DrumKitId, PadId};
        let (mut app, ctx, _fixture) = pad_app();
        let pos = pad_control(&mut app, &ctx, "01\nC2");
        pad_click(&mut app, &ctx, pos);
        let pos = pad_control(&mut app, &ctx, "Edit mapping");
        pad_click(&mut app, &ctx, pos);
        let apply = pad_control(&mut app, &ctx, "Apply mapping");
        app.tracker
            .editor
            .remove_drum_pad(DrumKitId(1), PadId(1))
            .unwrap();
        let before = app.tracker.editor.song.clone();
        pad_click(&mut app, &ctx, apply);
        assert_eq!(app.tracker.editor.song, before);
        assert!(!app.tracker.editor.song.drum_kits[&DrumKitId(1)]
            .pads
            .contains_key(&PadId(1)));
        app.tracker.editor.undo();
        render(&mut app, &ctx, vec![]);
        assert!(app.tracker.editor.song.drum_kits[&DrumKitId(1)]
            .pads
            .contains_key(&PadId(1)));
    }
    #[test]
    fn pad_region_inspector_reuses_worker_without_replacing_kit_or_cursor() {
        use crate::song::{DrumKitId, InstrumentId, InstrumentKind, RegionId};
        let (mut app, ctx, _fixture) = pad_app();
        let pos = pad_control(&mut app, &ctx, "01\nC2");
        pad_click(&mut app, &ctx, pos);
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
        );
        let before = app.tracker.editor.song.clone();
        let pos = pad_control(&mut app, &ctx, "Inspect region");
        pad_click(&mut app, &ctx, pos);
        assert!(
            app.sample_view,
            "Region inspect did not open: {:?}",
            app.error
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.sample_editor.busy() {
            assert!(
                std::time::Instant::now() < deadline,
                "Region worker timed out"
            );
            render(&mut app, &ctx, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        render(&mut app, &ctx, vec![]);
        assert_eq!(
            app.tracker.editor.song, before,
            "Inspect must not edit mappings"
        );
        pad_type(&mut app, &ctx, "Owned hit", "Reviewed region");
        let pos = pad_control(&mut app, &ctx, "Apply region");
        pad_click(&mut app, &ctx, pos);
        assert_eq!(
            app.tracker.editor.song.regions[&RegionId(1)].name,
            "Reviewed region"
        );
        assert_eq!(
            app.tracker.editor.song.instruments[&InstrumentId(1)].kind,
            InstrumentKind::DrumKit { kit: DrumKitId(1) }
        );
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
        audition_top(&mut app, &ctx);
        let pos = pad_control(&mut app, &ctx, "< Song");
        pad_click(&mut app, &ctx, pos);
        assert!(!app.sample_view);
        assert!(app.tracker.pads.open);
        assert_eq!(
            cursor,
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column
            )
        );
    }
    #[test]
    fn shared_region_playback_actual_inspector_preserves_kit_and_blocks_modal_close_read() {
        use crate::song::{AmpEnvelope, RegionId};
        for mode in 0..4 {
            let (mut app, ctx, _fixture) = pad_app();
            let pos = pad_control(&mut app, &ctx, "01\nC2");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "Inspect region");
            pad_click(&mut app, &ctx, pos);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.sample_editor.busy() {
                assert!(std::time::Instant::now() < deadline);
                render(&mut app, &ctx, vec![]);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let before = app.tracker.editor.song.clone();
            let pos = pad_control(&mut app, &ctx, "Sampler playback · off / legacy behavior");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "Amplitude envelope");
            pad_click(&mut app, &ctx, pos);
            let apply = pad_control(&mut app, &ctx, "Apply region");
            match mode {
                1 => app.tracker.editor.edit_mode = false,
                2 => app.session.close_ready = true,
                3 => {
                    app.tracker.editor.dirty = true;
                    app.session
                        .request(&mut app.tracker.editor, crate::session::Action::Close)
                        .unwrap();
                }
                _ => {}
            }
            pad_click(&mut app, &ctx, apply);
            if mode == 0 {
                let mut expected = before.clone();
                expected
                    .set_region_playback(RegionId(1), Some(AmpEnvelope::default()), None)
                    .unwrap();
                assert_eq!(app.tracker.editor.song, expected, "Only the shared region changes; all 16 pad references and kit kind remain intact");
                app.tracker.editor.undo();
                assert_eq!(app.tracker.editor.song, before);
            } else {
                assert_eq!(app.tracker.editor.song, before, "blocked mode {mode}");
            }
        }
    }
    #[test]
    fn velocity_source_mode_actual_mapping_apply_and_insert_share_undo_and_guards() {
        use crate::song::{DrumKitId, PadId, PadLayer, PadLayerId, RegionId, VelocityZone};
        for blocked in 0..4 {
            let (mut app, ctx, _fixture) = pad_app();
            let pos = pad_control(&mut app, &ctx, "01\nC2");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "Edit mapping");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "Sources");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "Velocity zones / RR");
            pad_click(&mut app, &ctx, pos);
            let before = app.tracker.editor.song.clone();
            let revision = app.tracker.editor.revision();
            let apply = pad_control(&mut app, &ctx, "Apply mapping");
            match blocked {
                1 => app.tracker.editor.edit_mode = false,
                2 => app.session.close_ready = true,
                3 => {
                    app.tracker.editor.dirty = true;
                    app.session
                        .request(&mut app.tracker.editor, crate::session::Action::Close)
                        .unwrap();
                }
                _ => {}
            }
            pad_click(&mut app, &ctx, apply);
            if blocked != 0 {
                assert_eq!(app.tracker.editor.song, before);
                assert_eq!(app.tracker.editor.revision(), revision);
                continue;
            }
            let mut expected = before.clone();
            expected
                .set_pad_sources(
                    DrumKitId(1),
                    PadId(1),
                    None,
                    vec![VelocityZone {
                        min_velocity: 0,
                        max_velocity: 127,
                        alternatives: vec![PadLayer {
                            id: PadLayerId(1),
                            region: RegionId(1),
                        }],
                    }],
                )
                .unwrap();
            assert_eq!(
                app.tracker.editor.song, expected,
                "Actual source-mode Apply equals shared model helper"
            );
            let pos = pad_control(&mut app, &ctx, "Insert note");
            pad_click(&mut app, &ctx, pos);
            assert_eq!(
                app.tracker.editor.song.patterns[&app.tracker.editor.pattern_id()].tracks
                    [&app.tracker.editor.track][&0][&0]
                    .note,
                crate::song::Note::Note(36)
            );
            app.tracker.editor.undo();
            assert_eq!(
                app.tracker.editor.song, expected,
                "Zoned pad uses ordinary note insertion history"
            );
            app.tracker.editor.undo();
            assert_eq!(
                app.tracker.editor.song, before,
                "Whole mapping was exactly one undo"
            );
        }
    }
    #[test]
    fn switching_pad_or_instrument_closes_the_old_mapping_draft() {
        let (mut app, ctx, _fixture) = pad_app();
        let before = app.tracker.editor.song.clone();
        let pos = pad_control(&mut app, &ctx, "01\nC2");
        pad_click(&mut app, &ctx, pos);
        let pos = pad_control(&mut app, &ctx, "Edit mapping");
        pad_click(&mut app, &ctx, pos);
        let pos = pad_control(&mut app, &ctx, "02\nC#2");
        pad_click(&mut app, &ctx, pos);
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="Pad mapping")));
        let pos = pad_control(&mut app, &ctx, "Edit mapping");
        pad_click(&mut app, &ctx, pos);
        let output = render_output(&mut app, &ctx, vec![]);
        control(&output, "KIT 1 · PAD 2 · slot 02");
        let pos = pad_control(&mut app, &ctx, "Instrument 1");
        pad_click(&mut app, &ctx, pos);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "2 · Instrument 2"));
        let output = render_output(&mut app, &ctx, vec![]);
        control(&output, "No kit assigned");
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="Pad mapping")));
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn ordinary_grid_cursor_follow_closes_a_different_pad_mapping_draft() {
        use crate::song::{InstrumentId, PadId, TrackId};
        let (mut app, ctx, _fixture) = pad_app();
        app.tracker.editor.select(0, TrackId(2)).unwrap();
        app.tracker.editor.row = 8;
        app.tracker
            .editor
            .insert_pad_note(InstrumentId(1), PadId(2))
            .unwrap();
        app.tracker.editor.select(0, TrackId(1)).unwrap();
        app.tracker.editor.row = 0;
        render(&mut app, &ctx, vec![]);
        let pos = pad_control(&mut app, &ctx, "01\nC2");
        pad_click(&mut app, &ctx, pos);
        let pos = pad_control(&mut app, &ctx, "Edit mapping");
        pad_click(&mut app, &ctx, pos);
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let output = render_output(&mut app, &ctx, vec![]);
        let note = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text == "C#2" && text.pos.x > 700.0 =>
                {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Actual Track 2 row8 selector field");
        pad_click(&mut app, &ctx, note);
        assert_eq!(app.tracker.editor.track, TrackId(2));
        assert_eq!(app.tracker.editor.row, 8);
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="Pad mapping")));
        let pos = pad_control(&mut app, &ctx, "Edit mapping");
        pad_click(&mut app, &ctx, pos);
        let output = render_output(&mut app, &ctx, vec![]);
        control(&output, "KIT 1 · PAD 2 · slot 02");
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
    }
    #[test]
    fn populated_startup_song_can_add_fresh_kit_instrument_atomically_without_replacing_notes() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        let before = app.tracker.editor.song.clone();
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
        );
        let mut expected = app.tracker.editor.clone();
        let ids = expected
            .create_drum_kit_instrument("New drum kit".into())
            .unwrap();
        assert_eq!(
            ids,
            (crate::song::InstrumentId(25), crate::song::DrumKitId(1))
        );
        render(&mut app, &ctx, vec![]);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Pads"));
        let pos = pad_control(&mut app, &ctx, "Kit…");
        pad_click(&mut app, &ctx, pos);
        let pos = pad_control(&mut app, &ctx, "New kit + new instrument");
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, expected.song);
        assert_eq!(app.tracker.editor.song.patterns, before.patterns);
        assert_eq!(
            cursor,
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column
            )
        );
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, before);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Redo"));
        assert_eq!(app.tracker.editor.song, expected.song);
    }
    #[test]
    fn open_new_instrument_action_respects_read_mode_modal_and_close() {
        for blocked in 0..3 {
            let ctx = egui::Context::default();
            let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
            app.tracker.editor.edit_mode = true;
            render(&mut app, &ctx, vec![]);
            let output = render_output(&mut app, &ctx, vec![]);
            pad_click(&mut app, &ctx, control(&output, "Pads"));
            let pos = pad_control(&mut app, &ctx, "Kit…");
            pad_click(&mut app, &ctx, pos);
            let pos = pad_control(&mut app, &ctx, "New kit + new instrument");
            let before = app.tracker.editor.song.clone();
            if blocked == 0 {
                app.tracker.editor.edit_mode = false;
            } else if blocked == 1 {
                app.session.close_ready = true;
            } else {
                app.session.pending = Some(crate::session::Action::New);
            }
            pad_click(&mut app, &ctx, pos);
            assert_eq!(app.tracker.editor.song, before);
        }
    }
    fn put_step_test_cell(
        song: &mut crate::song::Song,
        pattern: crate::song::PatternId,
        track: crate::song::TrackId,
        row: u32,
        column: u16,
        cell: crate::song::Cell,
    ) -> Result<(), String> {
        song.patterns
            .get_mut(&pattern)
            .ok_or("Missing fixture pattern")?
            .tracks
            .entry(track)
            .or_default()
            .entry(row)
            .or_default()
            .insert(column, cell);
        song.validate()
    }
    fn wait_step_grid(app: &mut Leviathan, ctx: &egui::Context) -> egui::FullOutput {
        render(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(880.0, 580.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 5000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..8 {
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(800.0, 380.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 5000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let output = render_output(app, ctx, vec![]);
            if !app.tracker.steps.busy() && output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="PAD / SELECTOR")) {return output;}
            assert!(
                std::time::Instant::now() < deadline,
                "Step projection/control did not become ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn step_point(output: &egui::FullOutput, pad: u8, step: u8) -> egui::Pos2 {
        let header = control(output, "PAD / SELECTOR");
        let x = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text == format!("{step:02}")
                        && text.pos.x > header.x + 60.0
                        && (text.pos.y + text.galley.size().y * 0.5 - header.y).abs() < 8.0 =>
                {
                    Some(text.pos.x + text.galley.size().x * 0.5)
                }
                _ => None,
            })
            .expect("Visible step header");
        let prefix = format!("{pad:02} ");
        let y = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text.starts_with(&prefix)
                        && text.pos.x > 210.0
                        && text.pos.x < 340.0 =>
                {
                    let y = text.pos.y + text.galley.size().y * 0.5;
                    shape
                        .clip_rect
                        .contains(egui::pos2(text.pos.x + 3.0, y))
                        .then_some(y)
                }
                _ => None,
            })
            .expect("Visible pad lane");
        egui::pos2(x, y)
    }
    fn step_control(app: &mut Leviathan, ctx: &egui::Context, wanted: &str) -> egui::Pos2 {
        let mut previous = None;
        for _ in 0..100 {
            let output = render_output(app, ctx, vec![]);
            if let Some(pos) = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                        let pos = text.pos + text.galley.size() * 0.5;
                        shape.clip_rect.contains(pos).then_some(pos)
                    }
                    _ => None,
                })
            {
                if previous.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.05) {
                    return pos;
                }
                previous = Some(pos);
                continue;
            }
            previous = None;
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(880.0, 500.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -70.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        panic!("Step control not reachable: {wanted}");
    }
    fn open_steps(app: &mut Leviathan, ctx: &egui::Context) -> egui::FullOutput {
        let output = render_output(app, ctx, vec![]);
        pad_click(app, ctx, control(&output, "Steps"));
        wait_step_grid(app, ctx)
    }
    #[test]
    fn actual_step_toggle_is_the_same_ordinary_cell_and_one_undo_including_residual_reclaim() {
        use crate::drum_steps::*;
        use crate::song::{InstrumentId, PadId};
        let (mut app, ctx, _fixture) = pad_app();
        let output = open_steps(&mut app, &ctx);
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        let view = StepView {
            pattern: app.tracker.editor.pattern_id(),
            track: app.tracker.editor.track,
            instrument: InstrumentId(1),
            division: StepDivision::Sixteenth,
            steps: 16,
            page: 0,
        };
        let mut expected = app.tracker.editor.clone();
        let projected = project(&expected.song, &view).unwrap();
        expected
            .edit_drum_step(
                expected.revision(),
                &projected,
                &StepEdit::Toggle {
                    pad: PadId(1),
                    step: 0,
                },
            )
            .unwrap();
        let pos = step_control(&mut app, &ctx, "Toggle");
        pad_click(&mut app, &ctx, pos);
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        wait_step_grid(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, expected.song);
        assert_eq!(
            crate::song_render::evaluate(&app.tracker.editor.song, None)
                .unwrap()
                .events,
            crate::song_render::evaluate(&expected.song, None)
                .unwrap()
                .events
        );
        let on = app.tracker.editor.song.clone();
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        wait_step_grid(&mut app, &ctx);
        assert_eq!(app.tracker.editor.cell().note, crate::song::Note::Empty);
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        wait_step_grid(&mut app, &ctx);
        assert_eq!(
            app.tracker.editor.song, on,
            "Compatible scalar residual is reclaimed, not stranded"
        );
        app.tracker.editor.undo();
        app.tracker.editor.undo();
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn exact_offgrid_step_event_edits_preserve_other_columns_commands_and_cursor_identity() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = pad_app();
        let mut song = app.tracker.editor.song.clone();
        put_step_test_cell(
            &mut song,
            app.tracker.editor.pattern_id(),
            app.tracker.editor.track,
            0,
            1,
            Cell {
                note: Note::Note(36),
                instrument: Some(InstrumentId(1)),
                velocity: None,
                pan: Some(0.2),
                delay: Some(0.125),
                effects: StepEffects {
                    direction: Some(Direction::Reverse),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        app.tracker.editor.apply_document_edit(song).unwrap();
        let output = open_steps(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        assert_eq!((app.tracker.editor.row, app.tracker.editor.column), (0, 1));
        let before = app.tracker.editor.song.clone();
        let pos = step_control(&mut app, &ctx, "Velocity");
        pad_click(&mut app, &ctx, pos);
        let pos = step_control(&mut app, &ctx, "Apply velocity");
        pad_click(&mut app, &ctx, pos);
        wait_step_grid(&mut app, &ctx);
        let cell = app.tracker.editor.cell();
        assert_eq!(cell.velocity, Some(1.0));
        assert_eq!(cell.pan, Some(0.2));
        assert_eq!(cell.delay, Some(0.125));
        assert_eq!(cell.effects.direction, Some(Direction::Reverse));
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        let pos = step_control(&mut app, &ctx, "Show tracker cell");
        pad_click(&mut app, &ctx, pos);
        assert!(!app.tracker.steps.open);
        assert_eq!((app.tracker.editor.row, app.tracker.editor.column), (0, 1));
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn step_grid_conflicts_read_mode_modal_and_close_never_toggle_hidden_events() {
        use crate::song::*;
        for blocked in 0..4 {
            let (mut app, ctx, _fixture) = pad_app();
            if blocked == 0 {
                let mut song = app.tracker.editor.song.clone();
                for column in 0..2 {
                    put_step_test_cell(
                        &mut song,
                        app.tracker.editor.pattern_id(),
                        app.tracker.editor.track,
                        0,
                        column,
                        Cell {
                            note: Note::Note(36),
                            instrument: Some(InstrumentId(1)),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                app.tracker.editor.apply_document_edit(song).unwrap();
            }
            open_steps(&mut app, &ctx);
            let pos = step_control(&mut app, &ctx, "Toggle");
            pad_click(&mut app, &ctx, pos);
            let output = wait_step_grid(&mut app, &ctx);
            let point = step_point(&output, 1, 1);
            let before = app.tracker.editor.song.clone();
            if blocked == 1 {
                app.tracker.editor.edit_mode = false;
            } else if blocked == 2 {
                app.session.close_ready = true;
            } else if blocked == 3 {
                app.session.pending = Some(crate::session::Action::New);
            }
            ctx.memory_mut(|memory| memory.request_focus(crate::tracker_ui::grid_id()));
            pad_click(&mut app, &ctx, point);
            render(&mut app, &ctx, vec![key(egui::Key::Z)]);
            assert_eq!(app.tracker.editor.song, before);
        }
    }
    #[test]
    fn new_document_discards_late_step_projection_without_recreating_kit_or_notes() {
        let (mut app, ctx, _fixture) = pad_app();
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Steps"));
        app.request_action(crate::session::Action::New);
        assert!(!app.tracker.steps.open);
        let before = app.tracker.editor.song.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tracker.steps.busy() {
            assert!(std::time::Instant::now() < deadline);
            render(&mut app, &ctx, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        render(&mut app, &ctx, vec![]);
        assert_eq!(app.tracker.editor.song, before);
        assert!(app.tracker.editor.song.drum_kits.is_empty());
    }
    #[test]
    fn step_retrigger_apply_unchanged_preserves_extreme_valid_spacing_exactly() {
        use crate::song::*;
        for spacing in [1e-8_f64, 0.9999999_f64] {
            let (mut app, ctx, _fixture) = pad_app();
            let mut song = app.tracker.editor.song.clone();
            put_step_test_cell(
                &mut song,
                app.tracker.editor.pattern_id(),
                app.tracker.editor.track,
                0,
                0,
                Cell {
                    note: Note::Note(36),
                    instrument: Some(InstrumentId(1)),
                    effects: StepEffects {
                        retrigger: Some(Retrigger {
                            repeats: 1,
                            spacing,
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            app.tracker.editor.apply_document_edit(song).unwrap();
            let output = open_steps(&mut app, &ctx);
            let before = app.tracker.editor.song.clone();
            let revision = app.tracker.editor.revision();
            pad_click(&mut app, &ctx, step_point(&output, 1, 1));
            let pos = step_control(&mut app, &ctx, "Apply retrigger");
            pad_click(&mut app, &ctx, pos);
            wait_step_grid(&mut app, &ctx);
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(
                app.tracker.editor.revision(),
                revision,
                "Unchanged exact text is a no-op"
            );
            assert_eq!(
                app.tracker
                    .editor
                    .cell()
                    .effects
                    .retrigger
                    .as_ref()
                    .unwrap()
                    .spacing
                    .to_bits(),
                spacing.to_bits()
            );
        }
    }
    #[test]
    fn step_32_right_edge_scroll_hit_and_held_pointer_resolve_exact_tracker_cell() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = pad_app();
        open_steps(&mut app, &ctx);
        for text in ["16 steps", "32 steps", "1/16", "1/32", "Toggle"] {
            let point = step_control(&mut app, &ctx, text);
            pad_click(&mut app, &ctx, point);
        }
        wait_step_grid(&mut app, &ctx);
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(800.0, 350.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-2000.0, 0.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..12 {
            render(&mut app, &ctx, vec![]);
        }
        let output = render_output(&mut app, &ctx, vec![]);
        let point = step_point(&output, 1, 32);
        let before = app.tracker.editor.song.clone();
        render(&mut app, &ctx, vec![egui::Event::PointerMoved(point)]);
        render(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..12 {
            render(&mut app, &ctx, vec![]);
        }
        assert_eq!(
            app.tracker.editor.song, before,
            "Holding a pad step is not an auto-repeat sequencer"
        );
        render(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        wait_step_grid(&mut app, &ctx);
        let cell = &app.tracker.editor.pattern().tracks[&app.tracker.editor.track][&15][&0];
        assert_eq!(cell.note, Note::Note(36));
        assert_eq!(cell.delay, Some(0.5));
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn exact_step_selection_follows_inherited_pad_without_mutating_cell_or_mapping_draft() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = pad_app();
        let mut song = app.tracker.editor.song.clone();
        let pattern = app.tracker.editor.pattern_id();
        let track = app.tracker.editor.track;
        put_step_test_cell(
            &mut song,
            pattern,
            track,
            0,
            0,
            Cell {
                note: Note::Note(36),
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        )
        .unwrap();
        put_step_test_cell(
            &mut song,
            pattern,
            track,
            15,
            0,
            Cell {
                note: Note::Note(37),
                instrument: None,
                ..Default::default()
            },
        )
        .unwrap();
        app.tracker.editor.apply_document_edit(song).unwrap();
        let output = open_steps(&mut app, &ctx);
        let pos = step_point(&output, 2, 16);
        let edit = pad_control(&mut app, &ctx, "Edit mapping");
        pad_click(&mut app, &ctx, edit);
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        pad_click(&mut app, &ctx, pos);
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="Pad 02")));
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="Apply mapping")));
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!((app.tracker.editor.row, app.tracker.editor.column), (15, 0));
        assert_eq!(app.tracker.editor.cell().instrument, None);
    }
    fn alias_app(linked: bool) -> (Leviathan, egui::Context, PadFixture) {
        use crate::song::*;
        let (mut app, ctx, fixture) = pad_app();
        let mut song = app.tracker.editor.song.clone();
        song.patterns.get_mut(&PatternId(2)).unwrap().rows = 32;
        song.order = vec![
            Occurrence {
                pattern: PatternId(1),
                muted_tracks: Default::default(),
                section: None,
            },
            Occurrence {
                pattern: PatternId(2),
                muted_tracks: Default::default(),
                section: None,
            },
            Occurrence {
                pattern: PatternId(1),
                muted_tracks: Default::default(),
                section: None,
            },
        ];
        put_step_test_cell(
            &mut song,
            PatternId(2),
            TrackId(2),
            0,
            0,
            Cell {
                note: Note::Note(36),
                instrument: Some(InstrumentId(1)),
                velocity: Some(0.7),
                effects: StepEffects {
                    retrigger: Some(Retrigger {
                        repeats: 2,
                        spacing: 0.125,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        put_step_test_cell(
            &mut song,
            PatternId(2),
            TrackId(2),
            4,
            1,
            Cell {
                note: Note::Note(37),
                instrument: Some(InstrumentId(1)),
                delay: Some(0.125),
                ..Default::default()
            },
        )
        .unwrap();
        if linked {
            song.set_track_alias(
                PatternId(1),
                TrackId(1),
                Some(TrackAlias {
                    pattern: PatternId(2),
                    track: TrackId(2),
                }),
            )
            .unwrap();
        }
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.octave = 2;
        app.tracker.editor.edit_step = 0;
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Pads"));
        (app, ctx, fixture)
    }
    fn matrix_control(app: &mut Leviathan, ctx: &egui::Context, wanted: &str) -> egui::Pos2 {
        let mut previous = None;
        for _ in 0..80 {
            let output = render_output(app, ctx, vec![]);
            if let Some(pos) = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => {
                        let p = text.pos + text.galley.size() * 0.5;
                        shape.clip_rect.contains(p).then_some(p)
                    }
                    _ => None,
                })
            {
                if previous.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.05) {
                    return pos;
                }
                previous = Some(pos);
                continue;
            }
            previous = None;
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(650.0, 430.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -65.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        panic!("Matrix control not reachable: {wanted}");
    }
    fn matrix_click(app: &mut Leviathan, ctx: &egui::Context, text: &str) {
        let point = matrix_control(app, ctx, text);
        pad_click(app, ctx, point);
    }
    fn link_fixture_block(app: &mut Leviathan, ctx: &egui::Context) {
        matrix_click(app, ctx, "Block…");
        matrix_click(app, ctx, "P1");
        matrix_click(app, ctx, "P2 · B · 32 rows");
        matrix_click(app, ctx, "Link source");
        matrix_click(app, ctx, "Close");
    }
    #[test]
    fn actual_matrix_link_source_edit_and_make_unique_preserve_shared_pattern_identity() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = alias_app(false);
        let before = app.tracker.editor.song.clone();
        let mut expected = before.clone();
        expected
            .set_track_alias(
                PatternId(1),
                TrackId(1),
                Some(TrackAlias {
                    pattern: PatternId(2),
                    track: TrackId(2),
                }),
            )
            .unwrap();
        link_fixture_block(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, expected);
        assert!(!app
            .tracker
            .editor
            .pattern()
            .tracks
            .contains_key(&TrackId(1)));
        matrix_click(&mut app, &ctx, "Edit source · POS 01");
        assert_eq!(
            (app.tracker.editor.occurrence, app.tracker.editor.track),
            (1, TrackId(2))
        );
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "C2"));
        render(&mut app, &ctx, vec![key(egui::Key::X)]);
        assert_eq!(app.tracker.editor.cell().note, Note::Note(38));
        assert_eq!(
            app.tracker
                .editor
                .song
                .resolved_track(PatternId(1), TrackId(1))
                .unwrap()
                .unwrap()[&0][&0]
                .note,
            Note::Note(38)
        );
        matrix_click(&mut app, &ctx, "L");
        assert_eq!(
            (app.tracker.editor.occurrence, app.tracker.editor.track),
            (2, TrackId(1))
        );
        matrix_click(&mut app, &ctx, "Make unique");
        let local = app.tracker.editor.song.clone();
        assert!(local.patterns[&PatternId(1)].track_aliases.is_empty());
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "D2"));
        render(&mut app, &ctx, vec![key(egui::Key::C)]);
        assert_eq!(app.tracker.editor.cell().note, Note::Note(40));
        assert_eq!(
            app.tracker.editor.song.patterns[&PatternId(2)].tracks[&TrackId(2)][&0][&0].note,
            Note::Note(38)
        );
        matrix_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, local);
        matrix_click(&mut app, &ctx, "Undo");
        assert!(crate::tracker_ui::cell_is_read_only(&app.tracker.editor));
        matrix_click(&mut app, &ctx, "Undo");
        matrix_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn actual_occurrence_x_mute_is_not_shared_pattern_or_global_track_state() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = alias_app(true);
        let before = app.tracker.editor.song.clone();
        matrix_click(&mut app, &ctx, "Block…");
        matrix_click(&mut app, &ctx, "X · mute this occurrence's track");
        let muted = app.tracker.editor.song.clone();
        assert!(muted.order[0].muted_tracks.contains(&TrackId(1)));
        assert!(muted.order[2].muted_tracks.is_empty());
        assert_eq!(muted.patterns, before.patterns);
        let mut expected = before.clone();
        expected
            .set_occurrence_track_mute(0, TrackId(1), true)
            .unwrap();
        assert_eq!(muted, expected);
        assert_eq!(
            crate::song_render::evaluate(&muted, None).unwrap().events,
            crate::song_render::evaluate(&expected, None)
                .unwrap()
                .events
        );
        matrix_click(&mut app, &ctx, "X · mute this occurrence's track");
        assert_eq!(app.tracker.editor.song, before);
        matrix_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, muted);
        matrix_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn aliased_tracker_commands_pad_insert_and_step_fields_are_inspection_only() {
        let (mut app, ctx, _fixture) = alias_app(true);
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "C2"));
        render(
            &mut app,
            &ctx,
            vec![key(egui::Key::X), key(egui::Key::Delete)],
        );
        assert_eq!(app.tracker.editor.song, before);
        matrix_click(&mut app, &ctx, "Commands (1)");
        matrix_click(&mut app, &ctx, "Clear commands");
        matrix_click(&mut app, &ctx, "Close");
        assert_eq!(app.tracker.editor.song, before);
        matrix_click(&mut app, &ctx, "Pads");
        let insert = pad_control(&mut app, &ctx, "Insert note");
        pad_click(&mut app, &ctx, insert);
        assert_eq!(app.tracker.editor.song, before);
        let output = open_steps(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        let toggle = step_control(&mut app, &ctx, "Toggle");
        pad_click(&mut app, &ctx, toggle);
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 2));
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 1));
        let apply = step_control(&mut app, &ctx, "Apply velocity");
        pad_click(&mut app, &ctx, apply);
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
    }
    #[test]
    fn matrix_window_rejects_modal_close_read_mode_and_revision_aba() {
        for blocked in 0..4 {
            let (mut app, ctx, _fixture) = alias_app(false);
            matrix_click(&mut app, &ctx, "Block…");
            let button = matrix_control(&mut app, &ctx, "Link source");
            let before = app.tracker.editor.song.clone();
            match blocked {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.close_ready = true,
                2 => app.session.pending = Some(crate::session::Action::New),
                _ => {
                    app.tracker.editor.set_timing(121.0, 4).unwrap();
                    app.tracker.editor.undo();
                }
            }
            pad_click(&mut app, &ctx, button);
            assert_eq!(app.tracker.editor.song, before);
        }
    }
    #[test]
    fn matrix_link_cycle_error_and_unordered_source_are_explicit_not_hidden_writes() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = alias_app(false);
        matrix_click(&mut app, &ctx, "Block…");
        matrix_click(&mut app, &ctx, "T2");
        matrix_click(&mut app, &ctx, "T1 · Track 1");
        let before = app.tracker.editor.song.clone();
        matrix_click(&mut app, &ctx, "Link source");
        assert_eq!(app.tracker.editor.song, before);
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text.contains("cycle"))));
        matrix_click(&mut app, &ctx, "Close");
        let mut song = before.clone();
        song.order.retain(|order| order.pattern == PatternId(1));
        song.set_track_alias(
            PatternId(1),
            TrackId(1),
            Some(TrackAlias {
                pattern: PatternId(2),
                track: TrackId(2),
            }),
        )
        .unwrap();
        app.tracker.editor.apply_document_edit(song).unwrap();
        let before = app.tracker.editor.song.clone();
        matrix_click(&mut app, &ctx, "Edit source unavailable");
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.occurrence, 0);
        matrix_click(&mut app, &ctx, "Make unique");
        assert!(!crate::tracker_ui::cell_is_read_only(&app.tracker.editor));
        matrix_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn all_four_alias_and_occurrence_mute_marks_remain_visible_in_narrow_matrix() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = alias_app(true);
        let mut song = app.tracker.editor.song.clone();
        // This test covers the four populated demo tracks; the 20 extra startup
        // slots are intentionally not alias sources in this narrow matrix case.
        let demo_tracks = song.track_order.iter().take(4).copied().collect::<Vec<_>>();
        for track in demo_tracks {
            song.set_track_alias(
                PatternId(1),
                track,
                Some(TrackAlias {
                    pattern: PatternId(2),
                    track: TrackId(2),
                }),
            )
            .unwrap();
            song.set_occurrence_track_mute(0, track, true).unwrap();
        }
        app.tracker.editor.apply_document_edit(song).unwrap();
        render(&mut app, &ctx, vec![]);
        let output = render_output(&mut app, &ctx, vec![]);
        let visible=output.shapes.iter().filter(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="XL" && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos,text.galley.size())))).count();
        assert_eq!(visible,4,"Every default track must disclose both independent matrix states without horizontal scrolling");
    }
    #[test]
    fn explicit_alias_source_navigation_uses_terminal_track_and_preserves_same_pattern_position() {
        use crate::song::*;
        for chained in [false, true] {
            let (mut app, ctx, _fixture) = alias_app(false);
            let mut song = app.tracker.editor.song.clone();
            if chained {
                song.set_track_alias(
                    PatternId(1),
                    TrackId(2),
                    Some(TrackAlias {
                        pattern: PatternId(2),
                        track: TrackId(2),
                    }),
                )
                .unwrap();
            } else {
                let rows = song.patterns[&PatternId(2)].tracks[&TrackId(2)].clone();
                song.patterns
                    .get_mut(&PatternId(1))
                    .unwrap()
                    .tracks
                    .insert(TrackId(2), rows);
            }
            song.set_track_alias(
                PatternId(1),
                TrackId(1),
                Some(TrackAlias {
                    pattern: PatternId(1),
                    track: TrackId(2),
                }),
            )
            .unwrap();
            app.tracker.editor.apply_document_edit(song).unwrap();
            app.tracker.editor.select(2, TrackId(1)).unwrap();
            let before = app.tracker.editor.song.clone();
            let revision = app.tracker.editor.revision();
            matrix_click(
                &mut app,
                &ctx,
                if chained {
                    "Edit source · POS 01"
                } else {
                    "Edit source · POS 02"
                },
            );
            assert_eq!(
                (app.tracker.editor.occurrence, app.tracker.editor.track),
                (if chained { 1 } else { 2 }, TrackId(2))
            );
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(app.tracker.editor.revision(), revision);
            assert_eq!(app.tracker.editor.cell().note, Note::Note(36));
        }
    }
    #[test]
    fn readonly_alias_steps_preserve_real_rhythm_conflicts_without_filling_empty_bins_with_warnings(
    ) {
        use crate::song::*;
        let (mut app, ctx, _fixture) = alias_app(true);
        let mut song = app.tracker.editor.song.clone();
        for column in 0..2 {
            put_step_test_cell(
                &mut song,
                PatternId(2),
                TrackId(2),
                2,
                column,
                Cell {
                    note: Note::Note(38),
                    instrument: Some(InstrumentId(1)),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        put_step_test_cell(
            &mut song,
            PatternId(2),
            TrackId(2),
            3,
            0,
            Cell {
                note: Note::Note(39),
                instrument: Some(InstrumentId(1)),
                effects: StepEffects {
                    direction: Some(Direction::Reverse),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        app.tracker.editor.apply_document_edit(song).unwrap();
        let output = open_steps(&mut app, &ctx);
        let warnings=output.shapes.iter().filter(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="!" && shape.clip_rect.contains(text.pos+text.galley.size()*0.5))).count();
        assert_eq!(warnings,2,"Only the off-grid hit and additional-FX hit are warning bins; empty read-only slots stay empty");
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="R")));
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text.contains("3 source conflict bins"))));
        let before = app.tracker.editor.song.clone();
        let toggle = step_control(&mut app, &ctx, "Toggle");
        pad_click(&mut app, &ctx, toggle);
        let output = wait_step_grid(&mut app, &ctx);
        pad_click(&mut app, &ctx, step_point(&output, 1, 2));
        assert_eq!(app.tracker.editor.song, before);
    }
    fn transform_app() -> (Leviathan, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.edit_step = 0;
        render(&mut app, &ctx, vec![]);
        (app, ctx)
    }
    fn transform_control(app: &mut Leviathan, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        let mut previous = None;
        for attempt in 0..50 {
            let output = render_output(app, ctx, vec![]);
            if let Some(point) = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                        let point = text.pos + text.galley.size() * 0.5;
                        shape.clip_rect.contains(point).then_some(point)
                    }
                    _ => None,
                })
            {
                if previous.is_some_and(|old: egui::Pos2| old.distance(point) < 0.05) {
                    return point;
                }
                previous = Some(point);
            } else {
                previous = None;
                let scroll_point = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some(
                            egui::pos2(shape.clip_rect.right() - 3.0, shape.clip_rect.center().y),
                        ),
                        _ => None,
                    })
                    .or_else(|| {
                        output.shapes.iter().rev().find_map(|shape| {
                            let rect = shape.clip_rect;
                            (rect.left() > 220.0
                                && rect.width() > 500.0
                                && rect.top() > 220.0
                                && rect.height() > 100.0
                                && rect.height() < 350.0)
                                .then_some(egui::pos2(rect.right() - 3.0, rect.center().y))
                        })
                    })
                    .unwrap_or(egui::pos2(835.0, 340.0));
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(scroll_point),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if attempt < 2 { 2000.0 } else { -60.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                for _ in 0..10 {
                    render(app, ctx, vec![]);
                }
            }
        }
        let output = render_output(app, ctx, vec![]);
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    Some((text.galley.job.text.clone(), text.pos, shape.clip_rect))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        panic!("Transform control not reachable: {label}; {labels:?}")
    }
    fn transform_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let point = transform_control(app, ctx, label);
        pad_click(app, ctx, point);
    }
    fn transform_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        let label_point = transform_control(app, ctx, label);
        let output = render_output(app, ctx, vec![]);
        let point = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    let p = text.pos + text.galley.size() * 0.5;
                    (shape.clip_rect.contains(p)
                        && (p.y - label_point.y).abs() < 3.0
                        && p.x > label_point.x
                        && text
                            .galley
                            .job
                            .text
                            .trim_end_matches('%')
                            .trim()
                            .parse::<f64>()
                            .is_ok())
                    .then_some(p)
                }
                _ => None,
            })
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap_or_else(|| panic!("Numeric control missing after {label}"));
        pad_click(app, ctx, point);
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            }],
        );
        render(app, ctx, vec![egui::Event::Text(value.into())]);
        render(app, ctx, vec![key(egui::Key::Enter)]);
    }
    fn wait_transform(app: &mut Leviathan, ctx: &egui::Context) -> egui::FullOutput {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let output = render_output(app, ctx, vec![]);
            if !app.tracker.transforms.busy() {
                return output;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Transform worker timeout"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn actual_scoped_transform_preview_commit_and_undo_match_shared_plan() {
        let (mut app, ctx) = transform_app();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
            app.tracker.editor.field,
        );
        transform_click(&mut app, &ctx, "Transform…");
        transform_number(&mut app, &ctx, "Transpose", "12");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        let plan = app
            .tracker
            .transforms
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(plan.config().transpose_semitones, 12);
        assert_eq!(plan.config().scope.row_start, 0);
        assert_eq!(plan.config().scope.row_end, 1);
        assert_eq!(plan.config().scope.columns, vec![0]);
        assert_eq!(plan.summary().changed_cells, 1);
        assert_eq!(plan.summary().affected_occurrences, vec![0, 2]);
        let expected = plan.candidate().clone();
        transform_click(&mut app, &ctx, "Commit transform");
        assert_eq!(app.tracker.editor.song, expected);
        assert_eq!(app.tracker.editor.cell().note, crate::song::Note::Note(61));
        assert_eq!(
            cursor,
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column,
                app.tracker.editor.field
            )
        );
        assert!(!app.tracker.transforms.is_open());
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, before);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Redo"));
        assert_eq!(app.tracker.editor.song, expected);
    }
    #[test]
    fn actual_transform_masks_leave_unselected_glide_and_inherited_velocity_untouched() {
        use crate::song::*;
        let (mut app, ctx) = transform_app();
        let mut song = app.tracker.editor.song.clone();
        let cell = song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap();
        cell.effects.glide = Some(Glide {
            target: 60,
            semitones_per_second: 2.0,
        });
        cell.velocity = None;
        app.tracker.editor.apply_document_edit(song).unwrap();
        let before = app.tracker.editor.song.clone();
        transform_click(&mut app, &ctx, "Transform…");
        transform_number(&mut app, &ctx, "Transpose", "12");
        transform_click(&mut app, &ctx, "Stored velocity");
        transform_number(&mut app, &ctx, "Scale ×", "0.5");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        let plan = app
            .tracker
            .transforms
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(plan.config().velocity_scale, 0.5);
        assert_eq!(plan.summary().skipped_unspecified_velocity, 1);
        transform_click(&mut app, &ctx, "Commit transform");
        assert_eq!(app.tracker.editor.cell().note, Note::Note(61));
        assert_eq!(app.tracker.editor.cell().effects.glide.unwrap().target, 60);
        assert_eq!(app.tracker.editor.cell().velocity, None);
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
        transform_click(&mut app, &ctx, "Transform…");
        transform_click(&mut app, &ctx, "Notes");
        transform_click(&mut app, &ctx, "Glide targets");
        transform_number(&mut app, &ctx, "Transpose", "12");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        transform_click(&mut app, &ctx, "Commit transform");
        assert_eq!(app.tracker.editor.cell().note, Note::Note(49));
        assert_eq!(app.tracker.editor.cell().effects.glide.unwrap().target, 72);
    }
    #[test]
    fn transform_commit_refuses_read_mode_modal_close_and_changed_settings() {
        for mode in 0..4 {
            let (mut app, ctx) = transform_app();
            transform_click(&mut app, &ctx, "Transform…");
            transform_number(&mut app, &ctx, "Transpose", "12");
            transform_click(&mut app, &ctx, "Preview transform");
            wait_transform(&mut app, &ctx);
            let before = app.tracker.editor.song.clone();
            match mode {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.close_ready = true,
                2 => {
                    app.tracker.editor.dirty = true;
                    app.session
                        .request(&mut app.tracker.editor, crate::session::Action::Close)
                        .unwrap();
                }
                _ => transform_number(&mut app, &ctx, "Transpose", "7"),
            }
            let point = transform_control(&mut app, &ctx, "Commit transform");
            pad_click(&mut app, &ctx, point);
            wait_transform(&mut app, &ctx);
            assert_eq!(app.tracker.editor.song, before, "mode {mode}");
            if mode == 3 {
                assert!(app
                    .tracker
                    .transforms
                    .capture_preview(&app.tracker.editor)
                    .is_none());
            }
        }
    }
    #[test]
    fn transform_context_navigation_and_revision_aba_discard_ready_preview() {
        let (mut app, ctx) = transform_app();
        transform_click(&mut app, &ctx, "Transform…");
        transform_number(&mut app, &ctx, "Transpose", "12");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        let before = app.tracker.editor.song.clone();
        app.tracker
            .editor
            .enter_note(crate::song::Note::Note(50))
            .unwrap();
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
        render(&mut app, &ctx, vec![]);
        assert!(app
            .tracker
            .transforms
            .capture_preview(&app.tracker.editor)
            .is_none());
        assert!(app.tracker.transforms.is_open());
        app.tracker.editor.row = 1;
        render(&mut app, &ctx, vec![]);
        assert!(!app.tracker.transforms.is_open());
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn transform_alias_kit_and_velocity_overflow_fail_without_document_mutation() {
        for mode in 0..3 {
            let (mut app, ctx, _fixture) = alias_app(mode == 0);
            if mode != 0 {
                app.tracker
                    .editor
                    .select(1, crate::song::TrackId(2))
                    .unwrap();
            }
            let before = app.tracker.editor.song.clone();
            transform_click(&mut app, &ctx, "Transform…");
            if mode == 2 {
                transform_click(&mut app, &ctx, "Notes");
                transform_click(&mut app, &ctx, "Stored velocity");
                transform_number(&mut app, &ctx, "Scale ×", "4");
            } else {
                transform_number(&mut app, &ctx, "Transpose", "12");
            }
            transform_click(&mut app, &ctx, "Preview transform");
            let output = wait_transform(&mut app, &ctx);
            assert!(app
                .tracker
                .transforms
                .capture_preview(&app.tracker.editor)
                .is_none());
            assert_eq!(app.tracker.editor.song, before);
            let labels = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(t) => Some(t.galley.job.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                labels.contains(match mode {
                    0 => "read-only",
                    1 => "drum selectors",
                    _ => "velocity",
                }),
                "{labels}"
            );
        }
    }
    #[test]
    fn transform_scale_validates_decimal_before_f32_narrowing_and_identity_is_noop() {
        for value in ["4.0000000001", "-1e-50", "NaN", "inf", "1"] {
            let (mut app, ctx) = transform_app();
            let before = app.tracker.editor.song.clone();
            let revision = app.tracker.editor.revision();
            transform_click(&mut app, &ctx, "Transform…");
            transform_click(&mut app, &ctx, "Notes");
            transform_click(&mut app, &ctx, "Stored velocity");
            transform_number(&mut app, &ctx, "Scale ×", value);
            transform_click(&mut app, &ctx, "Preview transform");
            wait_transform(&mut app, &ctx);
            if value == "1" {
                let preview = app
                    .tracker
                    .transforms
                    .capture_preview(&app.tracker.editor)
                    .unwrap();
                assert_eq!(preview.summary().changed_cells, 0);
                assert_eq!(preview.candidate(), &before);
            } else {
                assert!(
                    app.tracker
                        .transforms
                        .capture_preview(&app.tracker.editor)
                        .is_none(),
                    "{value}"
                );
            }
            transform_click(&mut app, &ctx, "Commit transform");
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(app.tracker.editor.revision(), revision);
        }
    }
    #[test]
    fn transform_text_focus_and_phrase_author_context_cannot_edit_song_or_projection() {
        use crate::song::*;
        let (mut app, ctx) = transform_app();
        app.tracker
            .editor
            .upsert_phrase(
                PhraseId(1),
                Phrase {
                    name: "Scoped phrase".into(),
                    instrument: InstrumentId(1),
                    rows: 8,
                    lines_per_beat: 4,
                    note_columns: 1,
                    root_note: 60,
                    cells: Default::default(),
                },
            )
            .unwrap();
        transform_click(&mut app, &ctx, "Phrases");
        transform_click(&mut app, &ctx, "Transform…");
        let before = app.tracker.editor.song.clone();
        ctx.memory_mut(|memory| memory.request_focus(crate::tracker_ui::grid_id()));
        render(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::Z),
                key(egui::Key::Delete),
                key(egui::Key::Space),
            ],
        );
        assert_eq!(app.tracker.editor.song, before);
        assert!(app.tracker.editor.edit_mode);
        let edit = matrix_control(&mut app, &ctx, "Edit phrase");
        pad_click(&mut app, &ctx, edit);
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(!app.tracker.transforms.is_open());
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(t) if t.galley.job.text=="Transform…")));
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn actual_transform_swing_nudge_preserves_cells_and_matches_exact_shared_config() {
        use crate::song::*;
        let (mut app, ctx) = transform_app();
        let mut song = app.tracker.editor.song.clone();
        let cells = song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap();
        cells.get_mut(&0).unwrap().get_mut(&0).unwrap().effects = StepEffects {
            retrigger: Some(Retrigger {
                repeats: 2,
                spacing: 0.125,
            }),
            release: Some(0.8),
            ..Default::default()
        };
        cells.insert(
            1,
            std::collections::BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(54),
                    delay: Some(0.125),
                    ..Default::default()
                },
            )]),
        );
        app.tracker.editor.apply_document_edit(song).unwrap();
        let before = app.tracker.editor.song.clone();
        transform_click(&mut app, &ctx, "Transform…");
        transform_number(&mut app, &ctx, "End exclusive", "8");
        transform_click(&mut app, &ctx, "Notes");
        transform_click(&mut app, &ctx, "Whole-event timing");
        transform_number(&mut app, &ctx, "Nudge", "3");
        transform_click(&mut app, &ctx, "Swing odd bins");
        transform_number(&mut app, &ctx, "Swing odd bins", "25");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        let preview = app
            .tracker
            .transforms
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(preview.config().nudge_frames, 3);
        assert_eq!(preview.config().swing.unwrap().amount_percent, 25);
        assert_eq!(preview.summary().swing_frames, 1500);
        assert_eq!(preview.summary().timing_changes, 2);
        assert_eq!(preview.summary().note_changes, 0);
        let expected = preview.candidate().clone();
        transform_click(&mut app, &ctx, "Commit transform");
        assert_eq!(app.tracker.editor.song, expected);
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn actual_transform_explicit_other_pattern_track_scope_preserves_editor_context() {
        use crate::song::*;
        let (mut app, ctx) = transform_app();
        let before = app.tracker.editor.song.clone();
        transform_click(&mut app, &ctx, "Transform…");
        transform_click(&mut app, &ctx, "P1");
        transform_click(&mut app, &ctx, "P2 · B · 64 rows");
        transform_click(&mut app, &ctx, "T1");
        transform_click(&mut app, &ctx, "T2 · Track 2 · 2 columns");
        transform_number(&mut app, &ctx, "Transpose", "12");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        let preview = app
            .tracker
            .transforms
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(
            (preview.config().scope.pattern, preview.config().scope.track),
            (PatternId(2), TrackId(2))
        );
        assert_eq!(preview.summary().affected_occurrences, vec![1]);
        transform_click(&mut app, &ctx, "Commit transform");
        assert_eq!(
            (
                app.tracker.editor.pattern_id(),
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column
            ),
            (PatternId(1), TrackId(1), 0, 0)
        );
        assert_eq!(
            app.tracker.editor.song.patterns[&PatternId(1)],
            before.patterns[&PatternId(1)]
        );
        assert_eq!(
            app.tracker.editor.song.patterns[&PatternId(2)].tracks[&TrackId(2)][&0][&0].note,
            Note::Note(62)
        );
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn transform_numeric_text_rejects_out_of_range_without_clamping_or_rounding() {
        for (label, value) in [
            ("Transpose", "49"),
            ("Transpose", "-49"),
            ("Transpose", "0.5"),
            ("Nudge", "48001"),
            ("Nudge", "-48001"),
            ("Swing odd bins", "51"),
            ("Row start", "-1"),
            ("End exclusive", "-1"),
            ("End exclusive", "0"),
            ("Row start", "2"),
            ("End exclusive", "65537"),
        ] {
            let (mut app, ctx) = transform_app();
            let before = app.tracker.editor.song.clone();
            let revision = app.tracker.editor.revision();
            transform_click(&mut app, &ctx, "Transform…");
            if label == "Nudge" || label == "Swing odd bins" {
                transform_click(&mut app, &ctx, "Whole-event timing");
            }
            if label == "Swing odd bins" {
                transform_click(&mut app, &ctx, "Swing odd bins");
            }
            transform_number(&mut app, &ctx, label, value);
            transform_click(&mut app, &ctx, "Preview transform");
            let output = wait_transform(&mut app, &ctx);
            assert!(
                app.tracker
                    .transforms
                    .capture_preview(&app.tracker.editor)
                    .is_none(),
                "{label} {value}"
            );
            let labels = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(labels.contains("must"), "{label} {value}: {labels}");
            transform_click(&mut app, &ctx, "Commit transform");
            assert_eq!(app.tracker.editor.song, before, "{label} {value}");
            assert_eq!(app.tracker.editor.revision(), revision);
        }
    }
    #[test]
    fn actual_zero_timing_transform_preserves_tiny_delay_and_unset_fields_exactly() {
        let (mut app, ctx) = transform_app();
        let mut song = app.tracker.editor.song.clone();
        song.patterns
            .get_mut(&crate::song::PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&crate::song::TrackId(1))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .delay = Some(1e-18);
        app.tracker.editor.apply_document_edit(song).unwrap();
        let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        let revision = app.tracker.editor.revision();
        transform_click(&mut app, &ctx, "Transform…");
        transform_click(&mut app, &ctx, "Whole-event timing");
        transform_number(&mut app, &ctx, "Nudge", "0");
        transform_click(&mut app, &ctx, "Preview transform");
        wait_transform(&mut app, &ctx);
        assert_eq!(
            app.tracker
                .transforms
                .capture_preview(&app.tracker.editor)
                .unwrap()
                .summary()
                .changed_cells,
            0
        );
        transform_click(&mut app, &ctx, "Commit transform");
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        assert_eq!(app.tracker.editor.revision(), revision);
    }
    fn assert_generated_column_visible(
        output: &egui::FullOutput,
        track: crate::song::TrackId,
        name: &str,
    ) {
        let header = format!("{:02}  {name}", track.0);
        let (left, clip) = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == header => {
                    Some((text.pos.x - 8.0, shape.clip_rect))
                }
                _ => None,
            })
            .expect("Generated track header must be painted");
        assert!(clip.left()<=left+0.1&&clip.right()>=left+222.0-0.1,"Complete first generated column must be revealed, not only selected NOTE: {left}, {clip:?}");
        for label in ["N1", "INS", "VEL", "PAN", "DLY"] {
            assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text==label && text.pos.x>=left && text.pos.x+text.galley.size().x<=left+222.0 && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos,text.galley.size())))),"Generated header {label} must be wholly visible");
        }
    }
    fn generation_app() -> (Leviathan, egui::Context) {
        let (mut app, ctx) = transform_app();
        // This test exercises generation choices, not the expanded native
        // startup layout; keep its instrument picker fixture intentionally small.
        app.tracker
            .editor
            .replace_document(crate::song::example_song())
            .unwrap();
        app.tracker.editor.edit_mode = true;
        let mut song = app.tracker.editor.song.clone();
        // Pure-preview fixture: assignment only, no claim that this test decodes audio.
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some("generation-unit-fixture.wav".into());
        }
        app.tracker.editor.apply_document_edit(song).unwrap();
        (app, ctx)
    }
    fn generation_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        let label_point = transform_control(app, ctx, label);
        let output = render_output(app, ctx, vec![]);
        let label_right = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos.x + text.galley.size().x)
                }
                _ => None,
            })
            .unwrap();
        let point = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    let point = text.pos + text.galley.size() * 0.5;
                    (shape.clip_rect.contains(point)
                        && (point.y - label_point.y).abs() < 3.0
                        && text.pos.x > label_right + 2.0
                        && text.galley.job.text.parse::<f64>().is_ok())
                    .then_some(point)
                }
                _ => None,
            })
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap_or_else(|| panic!("Numeric field not found: {label}"));
        pad_click(app, ctx, point);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
        );
        render(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
                egui::Event::Text(value.into()),
                key(egui::Key::Enter),
            ],
        );
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let output = render_output(app, ctx, vec![]);
        let current_label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    Some(egui::pos2(
                        text.pos.x + text.galley.size().x,
                        text.pos.y + text.galley.size().y * 0.5,
                    ))
                }
                _ => None,
            })
            .unwrap();
        let observed = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    let center = text.pos + text.galley.size() * 0.5;
                    (shape.clip_rect.contains(center)
                        && text.galley.job.text.parse::<f64>().is_ok()
                        && text.pos.x > current_label.x + 2.0
                        && (center.y - current_label.y).abs() < 3.0)
                        .then_some((center.x, text.galley.job.text.as_str()))
                }
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, text)| text);
        assert_eq!(
            observed,
            Some(value),
            "Requested {label}={value} must equal actual painted field, not only intended input"
        );
    }
    fn wait_generation(app: &mut Leviathan, ctx: &egui::Context) -> egui::FullOutput {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let output = render_output(app, ctx, vec![]);
            if !app.tracker.generation.busy() {
                return output;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Generation worker timeout"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn actual_generation_preview_commit_show_undo_matches_shared_candidate() {
        let (mut app, ctx) = generation_app();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
            app.tracker.editor.field,
        );
        transform_click(&mut app, &ctx, "Generate…");
        generation_number(&mut app, &ctx, "Seed", "42");
        generation_number(&mut app, &ctx, "Bars", "2");
        generation_number(&mut app, &ctx, "Steps / bar", "7");
        generation_number(&mut app, &ctx, "Pulses", "3");
        generation_number(&mut app, &ctx, "Rotation", "1");
        generation_number(&mut app, &ctx, "MIDI 01", "72");
        generation_number(&mut app, &ctx, "Velocity 01", "0.45");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        let preview = app
            .tracker
            .generation
            .capture_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        let config = preview.config();
        assert_eq!((config.seed, config.bars), (42, 2));
        assert_eq!(
            config.sequence[0],
            crate::generation::GenerationNote {
                note: 72,
                velocity: 0.45
            }
        );
        assert_eq!(
            config.preset,
            crate::generation::GenerationPreset::Euclidean {
                steps: 7,
                pulses: 3,
                rotation: 1,
                probability: 1.0
            }
        );
        let reference = crate::generation::preview(&before, config.clone()).unwrap();
        assert_eq!(preview.candidate(), reference.candidate());
        assert_eq!(preview.cells(), reference.cells());
        assert_eq!(preview.provenance(), reference.provenance());
        transform_click(&mut app, &ctx, "Commit & show");
        assert_eq!(&app.tracker.editor.song, preview.candidate());
        assert_eq!(
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column
            ),
            (preview.summary().occurrence, preview.summary().track, 0, 0)
        );
        render(&mut app, &ctx, vec![]);
        let output = render_output(&mut app, &ctx, vec![]);
        assert_generated_column_visible(&output, preview.summary().track, &preview.config().name);
        assert!(!app.tracker.generation.is_open());
        assert_eq!(
            (
                app.tracker.editor.song.bpm,
                app.tracker.editor.song.lines_per_beat
            ),
            (before.bpm, before.lines_per_beat)
        );
        for (id, pattern) in &before.patterns {
            assert_eq!(&app.tracker.editor.song.patterns[id], pattern);
        }
        transform_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column,
                app.tracker.editor.field
            ),
            cursor
        );
        assert_selected_field_is_visible(&render_output(&mut app, &ctx, vec![]), true);
        transform_click(&mut app, &ctx, "Redo");
        assert_eq!(&app.tracker.editor.song, preview.candidate());
        assert_selected_field_is_visible(&render_output(&mut app, &ctx, vec![]), true);
    }
    #[test]
    fn generation_numeric_inputs_reject_invalid_text_without_clamping_or_narrowing() {
        for (label, value) in [
            ("Seed", "18446744073709551616"),
            ("Seed", "-1"),
            ("Bars", "17"),
            ("Bars", "0"),
            ("Steps / bar", "129"),
            ("Pulses", "0"),
            ("Pulses", "17"),
            ("Rotation", "16"),
            ("Keep probability", "1.0000000001"),
            ("Keep probability", "-1e-50"),
            ("Keep probability", "NaN"),
            ("MIDI 01", "11"),
            ("MIDI 01", "128"),
            ("MIDI 01", "60.5"),
            ("Velocity 01", "1.0000000001"),
            ("Velocity 01", "-1e-50"),
        ] {
            let (mut app, ctx) = generation_app();
            let before = app.tracker.editor.song.clone();
            let revision = app.tracker.editor.revision();
            transform_click(&mut app, &ctx, "Generate…");
            generation_number(&mut app, &ctx, label, value);
            transform_click(&mut app, &ctx, "Preview generation");
            wait_generation(&mut app, &ctx);
            assert!(
                app.tracker
                    .generation
                    .capture_preview(&app.tracker.editor)
                    .is_none(),
                "{label}={value}"
            );
            transform_click(&mut app, &ctx, "Commit & show");
            assert_eq!(app.tracker.editor.song, before, "{label}={value}");
            assert_eq!(app.tracker.editor.revision(), revision);
        }
    }
    #[test]
    fn generation_preview_commit_refuses_read_modal_close_and_changed_settings() {
        for mode in 0..4 {
            let (mut app, ctx) = generation_app();
            transform_click(&mut app, &ctx, "Generate…");
            transform_click(&mut app, &ctx, "Preview generation");
            wait_generation(&mut app, &ctx);
            assert!(app
                .tracker
                .generation
                .capture_preview(&app.tracker.editor)
                .is_some());
            let before = app.tracker.editor.song.clone();
            match mode {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.close_ready = true,
                2 => {
                    app.tracker.editor.dirty = true;
                    app.session
                        .request(&mut app.tracker.editor, crate::session::Action::Close)
                        .unwrap();
                }
                _ => generation_number(&mut app, &ctx, "Seed", "7"),
            }
            transform_click(&mut app, &ctx, "Commit & show");
            wait_generation(&mut app, &ctx);
            assert_eq!(app.tracker.editor.song, before, "mode {mode}");
            if mode == 3 {
                assert!(app
                    .tracker
                    .generation
                    .capture_preview(&app.tracker.editor)
                    .is_none());
            }
        }
    }
    #[test]
    fn generation_revision_aba_context_and_workspace_changes_invalidate_ready_plan() {
        let (mut app, ctx) = generation_app();
        transform_click(&mut app, &ctx, "Generate…");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        let before = app.tracker.editor.song.clone();
        app.tracker.editor.set_timing(121.0, 4).unwrap();
        app.tracker.editor.undo();
        assert_eq!(app.tracker.editor.song, before);
        render(&mut app, &ctx, vec![]);
        assert!(app
            .tracker
            .generation
            .capture_preview(&app.tracker.editor)
            .is_none());
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        app.tracker.editor.move_rows(1);
        render(&mut app, &ctx, vec![]);
        assert!(!app.tracker.generation.is_open());
        transform_click(&mut app, &ctx, "Generate…");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        app.sample_view = true;
        render(&mut app, &ctx, vec![]);
        assert!(!app.tracker.generation.is_open());
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn generation_all_closed_and_unassigned_instrument_errors_leave_document_unchanged() {
        for mode in 0..2 {
            let (mut app, ctx) = if mode == 0 {
                generation_app()
            } else {
                transform_app()
            };
            let before = app.tracker.editor.song.clone();
            transform_click(&mut app, &ctx, "Generate…");
            if mode == 0 {
                generation_number(&mut app, &ctx, "Keep probability", "0");
            }
            transform_click(&mut app, &ctx, "Preview generation");
            let output = wait_generation(&mut app, &ctx);
            assert!(app
                .tracker
                .generation
                .capture_preview(&app.tracker.editor)
                .is_none());
            let expected = if mode == 0 { "no hits" } else { "no assigned" };
            assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text.contains(expected))));
            assert_eq!(app.tracker.editor.song, before);
        }
    }
    #[test]
    fn generation_empty_order_access_and_sequence_controls_use_same_ordinary_authority() {
        let (mut app, ctx) = generation_app();
        let mut song = app.tracker.editor.song.clone();
        song.order.clear();
        song.patterns.clear();
        song.tracks.clear();
        song.track_order.clear();
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        assert!(!app.tracker.editor.has_grid());
        transform_click(&mut app, &ctx, "Generate…");
        transform_click(&mut app, &ctx, "+ Note");
        generation_number(&mut app, &ctx, "MIDI 05", "80");
        transform_click(&mut app, &ctx, "Remove last");
        generation_number(&mut app, &ctx, "Seed", "18446744073709551615");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        let preview = app
            .tracker
            .generation
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(preview.config().sequence.len(), 4);
        assert_eq!(preview.config().seed, u64::MAX);
        transform_click(&mut app, &ctx, "Commit & show");
        assert!(app.tracker.editor.has_grid());
        assert_eq!(app.tracker.editor.song.order.len(), 1);
        app.tracker.editor.undo();
        assert!(!app.tracker.editor.has_grid());
    }
    #[test]
    fn generation_text_focus_and_phrase_projection_cannot_mutate_song() {
        use crate::song::*;
        let (mut app, ctx) = generation_app();
        app.tracker
            .editor
            .upsert_phrase(
                PhraseId(1),
                Phrase {
                    name: "Generation isolation".into(),
                    instrument: InstrumentId(1),
                    rows: 8,
                    lines_per_beat: 4,
                    note_columns: 1,
                    root_note: 60,
                    cells: Default::default(),
                },
            )
            .unwrap();
        transform_click(&mut app, &ctx, "Phrases");
        transform_click(&mut app, &ctx, "Generate…");
        let before = app.tracker.editor.song.clone();
        ctx.memory_mut(|memory| memory.request_focus(crate::tracker_ui::grid_id()));
        render(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::Z),
                key(egui::Key::Delete),
                key(egui::Key::Space),
            ],
        );
        assert_eq!(app.tracker.editor.song, before);
        assert!(app.tracker.editor.edit_mode);
        let point = matrix_control(&mut app, &ctx, "Edit phrase");
        pad_click(&mut app, &ctx, point);
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(!app.tracker.generation.is_open());
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="Generate…")));
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn generation_kit_sequence_uses_explicit_selectors_and_shared_pad_policies() {
        use crate::song::*;
        let (mut app, ctx) = generation_app();
        let mut song = app.tracker.editor.song.clone();
        song.register_sample(
            SampleId(1),
            SampleAsset {
                path: "generation-unit-source.wav".into(),
                sha256: "0".repeat(64),
                sample_rate: 48000,
                channels: 1,
                frames: 48000,
            },
        )
        .unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                name: "Unit region".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 48000,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        let (instrument, kit) = song
            .create_drum_kit_instrument("Generated kit".into())
            .unwrap();
        for (slot, note) in [36, 38, 42, 46].into_iter().enumerate() {
            song.upsert_drum_pad(
                kit,
                PadId(slot as u64 + 1),
                DrumPad {
                    slot: slot as u8,
                    name: format!("Pad {}", slot + 1),
                    note,
                    region: Some(RegionId(1)),
                    velocity_zones: vec![],
                    root_note: 60,
                    tune_semitones: slot as i8,
                    gain: 0.8,
                    pan: 0.0,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: None,
                },
            )
            .unwrap();
        }
        app.tracker.editor.apply_document_edit(song).unwrap();
        let before = app.tracker.editor.song.clone();
        transform_click(&mut app, &ctx, "Generate…");
        let selected = format!(
            "I1 · {} · pitched",
            before.instruments[&InstrumentId(1)].name
        );
        transform_click(&mut app, &ctx, &selected);
        transform_click(
            &mut app,
            &ctx,
            &format!("I{} · Generated kit · kit selectors", instrument.0),
        );
        for (index, note) in [36, 38, 42, 46].into_iter().enumerate() {
            generation_number(
                &mut app,
                &ctx,
                &format!("MIDI {:02}", index + 1),
                &note.to_string(),
            );
        }
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        let preview = app
            .tracker
            .generation
            .capture_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        assert_eq!(preview.config().instrument, instrument);
        assert_eq!(preview.cells()[0].cell.note, Note::Note(36));
        assert_eq!(preview.cells()[0].cell.instrument, Some(instrument));
        transform_click(&mut app, &ctx, "Commit & show");
        assert_eq!(&app.tracker.editor.song, preview.candidate());
        assert_eq!(app.tracker.editor.song.drum_kits, before.drum_kits);
        assert_eq!(
            crate::song_render::evaluate_model(&app.tracker.editor.song).unwrap(),
            crate::song_render::evaluate_model(preview.candidate()).unwrap()
        );
    }
    #[test]
    fn generation_sixteen_entry_sequence_last_fields_and_footer_remain_reachable() {
        let (mut app, ctx) = generation_app();
        transform_click(&mut app, &ctx, "Generate…");
        for _ in 4..16 {
            transform_click(&mut app, &ctx, "+ Note");
        }
        generation_number(&mut app, &ctx, "MIDI 16", "84");
        generation_number(&mut app, &ctx, "Velocity 16", "0.25");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        let preview = app
            .tracker
            .generation
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(preview.config().sequence.len(), 16);
        assert_eq!(
            preview.config().sequence[15],
            crate::generation::GenerationNote {
                note: 84,
                velocity: 0.25
            }
        );
        transform_click(&mut app, &ctx, "+ Note");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        assert_eq!(
            app.tracker
                .generation
                .capture_preview(&app.tracker.editor)
                .unwrap()
                .config()
                .sequence
                .len(),
            16
        );
        transform_click(&mut app, &ctx, "Cancel / close");
        assert!(!app.tracker.generation.is_open());
    }
    #[test]
    fn generation_historical_fingerprint_updates_after_ordinary_edit_undo_and_clock_change() {
        let (mut app, ctx) = generation_app();
        transform_click(&mut app, &ctx, "Generate…");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        transform_click(&mut app, &ctx, "Commit & show");
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="GEN cells OK")));
        let provenance = app.tracker.editor.pattern().generation.clone();
        let point = transform_control(&mut app, &ctx, "value / OFF");
        pad_click(&mut app, &ctx, point);
        render(&mut app, &ctx, vec![egui::Event::Text("74".into())]);
        transform_click(&mut app, &ctx, "Apply");
        let output = render_output(&mut app, &ctx, vec![]);
        assert_eq!(app.tracker.editor.cell().note, crate::song::Note::Note(74));
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="GEN edited")));
        assert_eq!(app.tracker.editor.pattern().generation, provenance);
        transform_click(&mut app, &ctx, "Undo");
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="GEN cells OK")));
        app.tracker.editor.set_timing(170.0, 4).unwrap();
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text=="GEN cells OK")));
        assert_eq!(app.tracker.editor.pattern().generation, provenance);
    }
    fn assert_selected_field_is_visible(output: &egui::FullOutput, writable: bool) {
        let fill = if writable {
            egui::Color32::from_rgb(38, 90, 91)
        } else {
            egui::Color32::from_rgb(40, 56, 65)
        };
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.fill == fill && shape.clip_rect.contains_rect(rect.rect))), "Restored selected field must be fully inside actual painted tracker clip");
    }
    #[test]
    fn toolbar_history_reveals_exact_nonzero_cursor_repeatedly_and_honors_blocked_state() {
        let (mut app, ctx) = transform_app();
        let mut song = app.tracker.editor.song.clone();
        for pattern in song.patterns.values_mut() {
            pattern.rows = 128;
        }
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.edit_step = 0;
        app.tracker
            .editor
            .select(0, crate::song::TrackId(1))
            .unwrap();
        app.tracker.editor.row = 90;
        app.tracker.editor.column = 1;
        app.tracker.editor.field = crate::editor::Field::Delay;
        let before = app.tracker.editor.song.clone();
        app.tracker.editor.enter_value("0.125").unwrap();
        let after = app.tracker.editor.song.clone();
        app.tracker
            .editor
            .select(1, crate::song::TrackId(4))
            .unwrap();
        app.tracker.editor.row = 3;
        app.tracker.editor.column = 1;
        app.tracker.editor.field = crate::editor::Field::Instrument;
        app.tracker.reveal_current_cursor();
        render(&mut app, &ctx, vec![]);
        let revision = app.tracker.editor.revision();
        for _ in 0..2 {
            transform_click(&mut app, &ctx, "Undo");
            let output = render_output(&mut app, &ctx, vec![]);
            assert_eq!(
                (
                    app.tracker.editor.occurrence,
                    app.tracker.editor.track,
                    app.tracker.editor.row,
                    app.tracker.editor.column,
                    app.tracker.editor.field
                ),
                (
                    0,
                    crate::song::TrackId(1),
                    90,
                    1,
                    crate::editor::Field::Delay
                )
            );
            assert_selected_field_is_visible(&output, true);
            assert_eq!(app.tracker.editor.song, before);
            transform_click(&mut app, &ctx, "Redo");
            let output = render_output(&mut app, &ctx, vec![]);
            assert_eq!(
                (
                    app.tracker.editor.occurrence,
                    app.tracker.editor.track,
                    app.tracker.editor.row,
                    app.tracker.editor.column,
                    app.tracker.editor.field
                ),
                (
                    1,
                    crate::song::TrackId(4),
                    3,
                    1,
                    crate::editor::Field::Instrument
                )
            );
            assert_selected_field_is_visible(&output, true);
            assert_eq!(app.tracker.editor.song, after);
        }
        assert_eq!(
            app.tracker.editor.revision(),
            revision + 4,
            "Viewport reveal adds no history revision"
        );
        app.session.close_ready = true;
        transform_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, after);
        assert_eq!(app.tracker.editor.revision(), revision + 4);
        app.session.close_ready = false;
        app.session
            .request(&mut app.tracker.editor, crate::session::Action::Close)
            .unwrap();
        transform_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, after);
        assert_eq!(app.tracker.editor.revision(), revision + 4);
        app.session.cancel();
        app.tracker.editor.edit_mode = false;
        transform_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
        assert_selected_field_is_visible(&render_output(&mut app, &ctx, vec![]), false);
        transform_click(&mut app, &ctx, "Undo");
        assert_eq!(
            app.tracker.editor.song, before,
            "Disabled exhausted Undo is harmless"
        );
    }
    #[test]
    fn generation_compact_caveats_keep_exact_details_keyboard_reachable_without_staling_preview() {
        let (mut app, ctx) = generation_app();
        transform_click(&mut app, &ctx, "Generate…");
        transform_click(&mut app, &ctx, "Preview generation");
        wait_generation(&mut app, &ctx);
        let preview = app
            .tracker
            .generation
            .capture_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        transform_control(&mut app, &ctx, "Append only; Song clock stays fixed. A later SongEnd can reveal old phrase notes/tails; kit choke can affect held voices.");
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.job.text.starts_with("Append only; Song clock"))));
        assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if preview.summary().warnings.contains(&t.galley.job.text))));
        let label = format!(
            "Full preview caveats ({} details)",
            preview.summary().warnings.len()
        );
        transform_click(&mut app, &ctx, &label);
        for _ in 0..15 {
            render(&mut app, &ctx, vec![]);
        }
        assert!(
            ctx.memory(|m| m.focused().is_some()),
            "Native details header is keyboard focusable"
        );
        render(&mut app, &ctx, vec![key(egui::Key::Space)]);
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..15 {
            render(&mut app, &ctx, vec![]);
        }
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if preview.summary().warnings.contains(&t.galley.job.text))), "Space closes the focused details header");
        render(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..15 {
            render(&mut app, &ctx, vec![]);
        }
        let output = render_output(&mut app, &ctx, vec![]);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if preview.summary().warnings.contains(&t.galley.job.text))), "Enter opens the focused details header");
        for warning in &preview.summary().warnings {
            transform_control(&mut app, &ctx, warning);
            let output = render_output(&mut app, &ctx, vec![]);
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if &t.galley.job.text == warning && s.clip_rect.contains(t.pos+t.galley.size()*0.5))), "Full shared warning must be reachable verbatim inside clip");
        }
        assert_eq!(
            app.tracker
                .generation
                .capture_preview(&app.tracker.editor)
                .unwrap()
                .id(),
            preview.id()
        );
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
    }
    #[test]
    fn wide_history_reveal_survives_short_long_pattern_scrollbar_transitions() {
        fn frame(
            app: &mut Leviathan,
            ctx: &egui::Context,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        }
        fn click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
            let output = frame(app, ctx, vec![]);
            let point = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(t) if t.galley.job.text == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap();
            for pressed in [true, false] {
                frame(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let mut song = app.tracker.editor.song.clone();
        song.track_order = vec![crate::song::TrackId(1)];
        song.tracks.retain(|id, _| *id == crate::song::TrackId(1));
        song.tracks
            .get_mut(&crate::song::TrackId(1))
            .unwrap()
            .note_columns = 1;
        for pattern in song.patterns.values_mut() {
            pattern.rows = 16;
            pattern
                .tracks
                .retain(|id, _| *id == crate::song::TrackId(1));
            for rows in pattern.tracks.values_mut() {
                rows.retain(|row, _| *row < 16);
                for columns in rows.values_mut() {
                    columns.retain(|column, _| *column == 0);
                }
            }
        }
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some("history-reveal-test.wav".into());
        }
        app.tracker.editor.replace_document(song.clone()).unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.row = 9;
        app.tracker.editor.column = 0;
        app.tracker.editor.field = crate::editor::Field::Delay;
        let config = crate::generation::GenerationConfig {
            version: 1,
            name: "Wide history".into(),
            bars: 4,
            instrument: crate::song::InstrumentId(1),
            seed: 42,
            preset: crate::generation::GenerationPreset::Euclidean {
                steps: 128,
                pulses: 128,
                rotation: 0,
                probability: 1.0,
            },
            sequence: vec![crate::generation::GenerationNote {
                note: 60,
                velocity: 0.5,
            }],
        };
        let preview = crate::generation::preview(&song, config).unwrap();
        app.tracker
            .editor
            .commit_generation_preview(app.tracker.editor.revision(), &preview)
            .unwrap();
        app.tracker
            .open_song_position(preview.summary().occurrence, preview.summary().track)
            .unwrap();
        app.tracker.editor.row = preview.summary().rows - 1;
        app.tracker.editor.column = preview.summary().columns - 1;
        app.tracker.editor.field = crate::editor::Field::Delay;
        app.tracker.reveal_current_cursor();
        for _ in 0..30 {
            frame(&mut app, &ctx, vec![]);
        }
        for _ in 0..3 {
            click(&mut app, &ctx, "Undo");
            for _ in 0..20 {
                frame(&mut app, &ctx, vec![]);
            }
            assert_eq!(app.tracker.editor.song, song);
            assert_eq!(
                (
                    app.tracker.editor.row,
                    app.tracker.editor.column,
                    app.tracker.editor.field
                ),
                (9, 0, crate::editor::Field::Delay)
            );
            assert_selected_field_is_visible(&frame(&mut app, &ctx, vec![]), true);
            click(&mut app, &ctx, "Redo");
            // A native repaint is requested while egui can still clamp against
            // its old viewport; after settling the last row/last DLY fully fits.
            let transition = frame(&mut app, &ctx, vec![]);
            assert_eq!(
                transition.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                Duration::ZERO
            );
            for _ in 0..30 {
                frame(&mut app, &ctx, vec![]);
            }
            assert_selected_field_is_visible(&frame(&mut app, &ctx, vec![]), true);
            assert_eq!(
                (
                    app.tracker.editor.row,
                    app.tracker.editor.column,
                    app.tracker.editor.field
                ),
                (
                    preview.summary().rows - 1,
                    preview.summary().columns - 1,
                    crate::editor::Field::Delay
                )
            );
            assert_eq!(&app.tracker.editor.song, preview.candidate());
        }
        click(&mut app, &ctx, "Undo");
        for _ in 0..30 {
            frame(&mut app, &ctx, vec![]);
        }
        click(&mut app, &ctx, "Redo");
        let revision = app.tracker.editor.revision();
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(1000.0, 500.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(600.0, 800.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..35 {
            frame(&mut app, &ctx, vec![]);
        }
        let output = frame(&mut app, &ctx, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.fill == egui::Color32::from_rgb(38,90,91) && shape.clip_rect.contains_rect(rect.rect))), "Manual scrolling during settling cancels reveal instead of being pulled back to the last cell");
        assert_eq!(&app.tracker.editor.song, preview.candidate());
        assert_eq!(app.tracker.editor.revision(), revision);
    }

    fn groove_body_rect(output: &egui::FullOutput) -> egui::Rect {
        output
            .shapes
            .iter()
            .map(|s| s.clip_rect)
            .filter(|r| {
                r.left() > 200.0
                    && r.top() > 180.0
                    && r.width() > 500.0
                    && r.height() > 100.0
                    && r.height() < 420.0
            })
            .max_by(|a, b| a.height().total_cmp(&b.height()))
            .expect("Actual groove scroller clip must be present")
    }
    fn groove_control(app: &mut Leviathan, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        let mut previous = None;
        for attempt in 0..70 {
            let output = render_output(app, ctx, vec![]);
            let target = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(t) if t.galley.job.text == label => {
                        let r = egui::Rect::from_min_size(t.pos, t.galley.size());
                        shape.clip_rect.contains_rect(r).then_some(r.center())
                    }
                    _ => None,
                });
            if let Some(point) = target {
                if previous.is_some_and(|old: egui::Pos2| old.distance(point) < 0.05) {
                    return point;
                }
                previous = Some(point);
            } else {
                previous = None;
                let body = groove_body_rect(&output);
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(body.center()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if attempt < 2 { 2000.0 } else { -55.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                for _ in 0..8 {
                    render(app, ctx, vec![]);
                }
            }
        }
        panic!("Groove control not reachable: {label}");
    }
    fn groove_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let p = groove_control(app, ctx, label);
        pad_click(app, ctx, p);
    }
    fn groove_app() -> (Leviathan, egui::Context) {
        use crate::song::*;
        let (mut app, ctx) = transform_app();
        let mut song = app.tracker.editor.song.clone();
        song.title = "Groove authoring fixture".into();
        song.bpm = 120.0;
        song.lines_per_beat = 4;
        song.order.truncate(1);
        for p in song.patterns.values_mut() {
            p.rows = 8;
            p.tracks.clear();
            p.track_aliases.clear();
        }
        for i in song.instruments.values_mut() {
            i.sample = Some(std::path::PathBuf::from("/tmp/groove-ui-unused.wav"));
        }
        song.register_sample(
            SampleId(1),
            SampleAsset {
                path: "/tmp/groove-ui-unused.wav".into(),
                sha256: "a".repeat(64),
                sample_rate: 48000,
                channels: 1,
                frames: 24000,
            },
        )
        .unwrap();
        song.upsert_marker_set(
            MarkerSetId(1),
            MarkerSet {
                name: "Reviewed timing".into(),
                sample: SampleId(1),
                source_bpm: Some(120.0),
                markers: vec![0, 6600, 12000, 18600, 24000],
                locked_frames: vec![],
            },
        )
        .unwrap();
        for row in 0..4 {
            put_step_test_cell(
                &mut song,
                PatternId(1),
                TrackId(1),
                row,
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(0.5),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.edit_step = 0;
        render(&mut app, &ctx, vec![]);
        (app, ctx)
    }
    fn groove_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        let _ = groove_control(app, ctx, label);
        let id = egui::Id::new(("groove-number", label));
        let mut point = None;
        let mut previous = None;
        for _ in 0..80 {
            render(app, ctx, vec![]);
            let response = ctx
                .read_response(id)
                .expect("Numeric groove field registered");
            if response
                .interact_rect
                .expand(0.01)
                .contains_rect(response.rect)
                && ctx.layer_id_at(response.rect.center()) == Some(response.layer_id)
            {
                if previous
                    .is_some_and(|old: egui::Pos2| old.distance(response.rect.center()) < 0.01)
                {
                    point = Some(response.rect.center());
                    break;
                }
                previous = Some(response.rect.center());
            } else {
                previous = None;
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(820.0, 280.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(
                                0.0,
                                if response.rect.center().y < response.interact_rect.center().y {
                                    50.0
                                } else {
                                    -50.0
                                },
                            ),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        pad_click(
            app,
            ctx,
            point.expect("Exact field must be fully visible before typing"),
        );
        assert!(
            ctx.memory(|memory| memory.has_focus(id)),
            "Exact {label} input must receive focus"
        );
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            }],
        );
        render(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text(value.into()),
            ],
        );
        render(app, ctx, vec![key(egui::Key::Enter)]);
        let _ = render_output(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        // Clearing the old preview changes scroll content height. Read the edited
        // widget only after actual visible layout has settled, never tracker text behind it.
        let _ = groove_control(app, ctx, label);
        let output = render_output(app, ctx, vec![]);
        let rect = ctx.read_response(id).unwrap().rect;
        let painted = output
            .shapes
            .iter()
            .rev()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if rect.intersects(
                        egui::Rect::from_min_size(text.pos, text.galley.size())
                            .intersect(shape.clip_rect),
                    ) =>
                {
                    Some(text.galley.job.text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            painted.contains(&value),
            "Requested {label}={value} must equal actual painted input; {painted:?}"
        );
    }
    fn wait_groove(app: &mut Leviathan, ctx: &egui::Context) {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            render(app, ctx, vec![]);
            if !app.tracker.grooves.busy() {
                break;
            }
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn preview_groove_capture(app: &mut Leviathan, ctx: &egui::Context) {
        groove_click(app, ctx, "Groove…");
        groove_click(app, ctx, "Choose accepted marker set");
        groove_click(app, ctx, "M1 · Reviewed timing · 5 boundaries");
        groove_number(app, ctx, "Cycle steps", "4");
        groove_click(app, ctx, "Preview capture");
        wait_groove(app, ctx);
    }
    #[test]
    fn groove_actual_capture_then_apply_is_shared_nonmutating_preview_and_one_undo_each() {
        let (mut app, ctx) = groove_app();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
            app.tracker.editor.field,
        );
        preview_groove_capture(&mut app, &ctx);
        let capture = app
            .tracker
            .grooves
            .capture_capture_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        assert!(!capture.config().include_start_anchor);
        assert_eq!(capture.template().anchors.len(), 3);
        groove_click(&mut app, &ctx, "Save template");
        assert_eq!(&app.tracker.editor.song, capture.candidate());
        groove_click(&mut app, &ctx, "Apply timing");
        groove_click(&mut app, &ctx, "Whole pattern");
        groove_click(&mut app, &ctx, "Preview timing");
        wait_groove(&mut app, &ctx);
        let preview = app
            .tracker
            .grooves
            .capture_apply_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        assert_eq!(preview.summary().changed_cells, 2);
        assert_eq!(&app.tracker.editor.song, capture.candidate());
        groove_click(&mut app, &ctx, "Commit timing");
        assert_eq!(&app.tracker.editor.song, preview.candidate());
        assert_eq!(
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column,
                app.tracker.editor.field
            ),
            cursor
        );
        groove_click(&mut app, &ctx, "Cancel / close");
        groove_click(&mut app, &ctx, "Undo");
        assert_eq!(&app.tracker.editor.song, capture.candidate());
        groove_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn groove_actual_zero_identity_invalid_numeric_stale_read_modal_and_closing_guards() {
        for guard in 0..3 {
            let (mut app, ctx) = groove_app();
            preview_groove_capture(&mut app, &ctx);
            let before = app.tracker.editor.song.clone();
            match guard {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.close_ready = true,
                _ => app.session.pending = Some(crate::session::Action::Close),
            };
            groove_click(&mut app, &ctx, "Save template");
            assert_eq!(app.tracker.editor.song, before);
        }
        let (mut app, ctx) = groove_app();
        preview_groove_capture(&mut app, &ctx);
        groove_click(&mut app, &ctx, "Save template");
        groove_click(&mut app, &ctx, "Apply timing");
        groove_click(&mut app, &ctx, "Whole pattern");
        groove_click(&mut app, &ctx, "0% identity");
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        groove_click(&mut app, &ctx, "Preview timing");
        wait_groove(&mut app, &ctx);
        let p = app
            .tracker
            .grooves
            .capture_apply_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(p.summary().changed_cells, 0);
        assert_eq!(p.candidate(), &before);
        groove_click(&mut app, &ctx, "Commit timing");
        assert_eq!(app.tracker.editor.revision(), revision);
        for value in ["-1", "101", "0.1", "NaN", "18446744073709551616"] {
            groove_number(&mut app, &ctx, "Strength %", value);
            assert!(app
                .tracker
                .grooves
                .capture_apply_preview(&app.tracker.editor)
                .is_none());
            groove_click(&mut app, &ctx, "Preview timing");
            wait_groove(&mut app, &ctx);
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(app.tracker.editor.revision(), revision);
        }
        groove_number(&mut app, &ctx, "Strength %", "100");
        groove_click(&mut app, &ctx, "Preview timing");
        wait_groove(&mut app, &ctx);
        assert!(app
            .tracker
            .grooves
            .capture_apply_preview(&app.tracker.editor)
            .is_some());
        groove_number(&mut app, &ctx, "Strength %", "50");
        assert!(app
            .tracker
            .grooves
            .capture_apply_preview(&app.tracker.editor)
            .is_none());
        groove_click(&mut app, &ctx, "Commit timing");
        assert_eq!(app.tracker.editor.song, before);
    }
    fn keyzone_cursor(
        e: &crate::editor::Editor,
    ) -> (usize, crate::song::TrackId, u32, u16, crate::editor::Field) {
        (e.occurrence, e.track, e.row, e.column, e.field)
    }
    fn keyzone_body(output: &egui::FullOutput) -> egui::Rect {
        output
            .shapes
            .iter()
            .map(|shape| shape.clip_rect)
            .filter(|rect| {
                rect.left() > 175.0
                    && rect.left() < 220.0
                    && rect.top() > 165.0
                    && rect.width() > 500.0
                    && rect.height() > 150.0
                    && rect.height() < 500.0
            })
            .max_by(|a, b| a.height().total_cmp(&b.height()))
            .expect("Actual keyzone body clip")
    }
    fn keyzone_control(app: &mut Leviathan, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        let mut previous = None;
        for attempt in 0..100 {
            let output = render_output(app, ctx, vec![]);
            let point = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                        let rect = text.visual_bounding_rect();
                        shape.clip_rect.contains_rect(rect).then_some(rect.center())
                    }
                    _ => None,
                });
            if let Some(point) = point {
                if previous.is_some_and(|old: egui::Pos2| old.distance(point) < 0.02) {
                    return point;
                }
                previous = Some(point);
            } else {
                previous = None;
                let body = keyzone_body(&output);
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(body.center().x, body.top() + 12.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if attempt < 2 { 2000.0 } else { -50.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                for _ in 0..5 {
                    render(app, ctx, vec![]);
                }
            }
        }
        let output = render_output(app, ctx, vec![]);
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    Some((text.galley.job.text.clone(), text.pos, shape.clip_rect))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        panic!(
            "Keyzone control not reachable: {label}; body={:?}; painted={labels:?}",
            keyzone_body(&output)
        )
    }
    fn keyzone_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let point = keyzone_control(app, ctx, label);
        pad_click(app, ctx, point);
    }
    fn keyzone_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        let _ = keyzone_control(app, ctx, label);
        let id = egui::Id::new(("keyzone-number", label));
        let mut previous = None;
        let mut point = None;
        for _ in 0..80 {
            let output = render_output(app, ctx, vec![]);
            let response = ctx.read_response(id).expect("Keyzone number registered");
            if response
                .interact_rect
                .expand(0.01)
                .contains_rect(response.rect)
                && ctx.layer_id_at(response.rect.center()) == Some(response.layer_id)
            {
                if previous
                    .is_some_and(|old: egui::Pos2| old.distance(response.rect.center()) < 0.02)
                {
                    point = Some(response.rect.center());
                    break;
                }
                previous = Some(response.rect.center());
            } else {
                previous = None;
                let body = keyzone_body(&output);
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(body.center().x, body.top() + 12.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(
                                0.0,
                                if response.rect.center().y < body.center().y {
                                    55.0
                                } else {
                                    -55.0
                                },
                            ),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        pad_click(app, ctx, point.expect("Keyzone exact input fully visible"));
        assert!(ctx.memory(|memory| memory.has_focus(id)));
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
        );
        render(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
                egui::Event::Text(value.into()),
            ],
        );
        render(app, ctx, vec![key(egui::Key::Enter)]);
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let _ = keyzone_control(app, ctx, label);
        let output = render_output(app, ctx, vec![]);
        let rect = ctx.read_response(id).unwrap().rect;
        assert!(
            output.shapes.iter().any(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) =>
                    text.galley.job.text == value
                        && rect.intersects(text.visual_bounding_rect().intersect(shape.clip_rect)),
                _ => false,
            }),
            "Requested {label}={value} must be observed exactly"
        );
    }
    fn keyzone_app() -> (Leviathan, egui::Context, PadFixture) {
        let (mut app, ctx, fixture) = pad_app();
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Pads"));
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(
            &mut app,
            &ctx,
            control(&output, "Song audio · instruments & offline export"),
        );
        for _ in 0..5 {
            render(&mut app, &ctx, vec![]);
        }
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Melodic map…"));
        for _ in 0..5 {
            render(&mut app, &ctx, vec![]);
        }
        assert!(app.multisample_editor.open);
        (app, ctx, fixture)
    }
    fn keyzone_create_region_zone(app: &mut Leviathan, ctx: &egui::Context) {
        keyzone_click(app, ctx, "New map + instrument");
        keyzone_click(app, ctx, "Add zone");
        keyzone_click(app, ctx, "Choose region");
        keyzone_click(app, ctx, "R1 · Owned hit · source[0, 4800)");
    }
    #[test]
    fn melodic_mapping_actual_new_apply_copy_assign_undo_and_numeric_draft_are_shared() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = keyzone_app();
        let before = app.tracker.editor.song.clone();
        let cursor = keyzone_cursor(&app.tracker.editor);
        keyzone_create_region_zone(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song.patterns, before.patterns);
        assert_eq!(
            app.tracker.editor.song.instruments.len(),
            before.instruments.len() + 1
        );
        keyzone_number(&mut app, &ctx, "Note min", "12");
        keyzone_number(&mut app, &ctx, "Note max", "100");
        keyzone_number(&mut app, &ctx, "Native root MIDI", "65");
        assert!(
            app.tracker.editor.song.multisamples[&MultisampleId(1)]
                .zones
                .is_empty(),
            "Draft is not a document edit"
        );
        keyzone_click(&mut app, &ctx, "Apply map");
        let mut expected = before.clone();
        let (instrument, map) = expected
            .create_multisample_instrument("New melodic map".into())
            .unwrap();
        expected
            .set_keyzones(
                map,
                std::collections::BTreeMap::from([(
                    KeyZoneId(1),
                    KeyZone {
                        name: "Zone 1".into(),
                        note_min: 12,
                        note_max: 100,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(1),
                        root_note: 65,
                    },
                )]),
            )
            .unwrap();
        assert_eq!(app.tracker.editor.song, expected);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        keyzone_click(&mut app, &ctx, "Remove map");
        assert_eq!(
            app.tracker.editor.song, expected,
            "Assigned map removal must refuse atomically"
        );
        keyzone_click(&mut app, &ctx, "Fresh copy");
        let copy = app.tracker.editor.song.clone();
        keyzone_click(&mut app, &ctx, "Remove map");
        assert_eq!(
            app.tracker.editor.song, expected,
            "Remove only the unassigned copy, not shared regions"
        );
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, copy);
        keyzone_click(&mut app, &ctx, "Choose map");
        keyzone_click(&mut app, &ctx, "M2 · New melodic map");
        assert_eq!(app.tracker.editor.song.multisamples.len(), 2);
        assert_eq!(app.tracker.editor.song.instruments, expected.instruments);
        keyzone_click(&mut app, &ctx, "Assign map");
        assert!(matches!(
            app.tracker.editor.song.instruments[&instrument].kind,
            InstrumentKind::Multisample {
                map: MultisampleId(2)
            }
        ));
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song.instruments, expected.instruments);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, expected);
    }
    #[test]
    fn melodic_mapping_actual_invalid_text_and_modal_close_read_guards_do_not_leak_notes() {
        let (mut app, ctx, _fixture) = keyzone_app();
        keyzone_create_region_zone(&mut app, &ctx);
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        for invalid in ["-1", "128", "1.5", "NaN"] {
            keyzone_number(&mut app, &ctx, "Note min", invalid);
            keyzone_click(&mut app, &ctx, "Apply map");
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(app.tracker.editor.revision(), revision);
        }
        keyzone_number(&mut app, &ctx, "Note min", "0");
        for guard in 0..3 {
            let point = keyzone_control(&mut app, &ctx, "Apply map");
            match guard {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.pending = Some(crate::session::Action::New),
                _ => app.session.close_ready = true,
            }
            pad_click(&mut app, &ctx, point);
            ctx.memory_mut(|m| m.request_focus(crate::tracker_ui::grid_id()));
            render(
                &mut app,
                &ctx,
                vec![key(egui::Key::Z), egui::Event::Text("9".into())],
            );
            assert_eq!(app.tracker.editor.song, before);
            assert_eq!(app.tracker.editor.revision(), revision);
            app.tracker.editor.edit_mode = true;
            app.session.pending = None;
            app.session.close_ready = false;
            render(&mut app, &ctx, vec![]);
        }
        keyzone_click(&mut app, &ctx, "Apply map");
        assert_ne!(app.tracker.editor.song, before);
    }
    #[test]
    fn melodic_mapping_region_inspection_returns_without_replacing_mapping_or_cursor() {
        let (mut app, ctx, _fixture) = keyzone_app();
        keyzone_create_region_zone(&mut app, &ctx);
        keyzone_click(&mut app, &ctx, "Apply map");
        let before = app.tracker.editor.song.clone();
        let cursor = keyzone_cursor(&app.tracker.editor);
        let revision = app.tracker.editor.revision();
        keyzone_click(&mut app, &ctx, "Inspect selected region");
        assert!(app.sample_view);
        assert!(!app.multisample_editor.open);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.sample_editor.busy() {
            render(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Song"));
        render(&mut app, &ctx, vec![]);
        assert!(app.multisample_editor.open);
        assert!(!app.sample_view);
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
    }
    #[cfg(unix)]
    fn resample_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        let _ = keyzone_control(app, ctx, label);
        let id = egui::Id::new(("resample-number", label));
        let mut previous = None;
        let mut point = None;
        for _ in 0..80 {
            let output = render_output(app, ctx, vec![]);
            let response = ctx.read_response(id).expect("Resample number registered");
            if response
                .interact_rect
                .expand(0.01)
                .contains_rect(response.rect)
                && ctx.layer_id_at(response.rect.center()) == Some(response.layer_id)
            {
                if previous
                    .is_some_and(|old: egui::Pos2| old.distance(response.rect.center()) < 0.02)
                {
                    point = Some(response.rect.center());
                    break;
                }
                previous = Some(response.rect.center());
            } else {
                previous = None;
                let body = keyzone_body(&output);
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(body.center().x, body.top() + 12.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(
                                0.0,
                                if response.rect.center().y < body.center().y {
                                    55.0
                                } else {
                                    -55.0
                                },
                            ),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        pad_click(app, ctx, point.expect("Resample exact input fully visible"));
        assert!(ctx.memory(|memory| memory.has_focus(id)));
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
        );
        render(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
                egui::Event::Text(value.into()),
            ],
        );
        render(app, ctx, vec![key(egui::Key::Enter)]);
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let _ = keyzone_control(app, ctx, label);
        let output = render_output(app, ctx, vec![]);
        let rect = ctx.read_response(id).unwrap().rect;
        assert!(
            output.shapes.iter().any(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) =>
                    text.galley.job.text == value
                        && rect.intersects(text.visual_bounding_rect().intersect(shape.clip_rect)),
                _ => false,
            }),
            "Requested {label}={value} must be observed exactly"
        );
    }
    #[cfg(unix)]
    fn resample_app() -> (Leviathan, egui::Context, PadFixture) {
        use crate::song::*;
        let (mut app, ctx, fixture) = pad_app();
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Pads"));
        let mut song = app.tracker.editor.song.clone();
        for pattern in song.patterns.values_mut() {
            pattern.rows = 4;
            pattern.tracks.clear();
        }
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
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.song_output = fixture.0.join("song.wav").display().to_string();
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(
            &mut app,
            &ctx,
            control(&output, "Song audio · instruments & offline export"),
        );
        keyzone_click(&mut app, &ctx, "Resample…");
        assert!(app.resample_editor.open);
        (app, ctx, fixture)
    }
    #[cfg(unix)]
    fn wait_resample(app: &mut Leviathan, ctx: &egui::Context) {
        let end = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            render(app, ctx, vec![]);
            if !app.resample_editor.busy() {
                break;
            }
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[cfg(unix)]
    fn print_resample(app: &mut Leviathan, ctx: &egui::Context) {
        keyzone_click(app, ctx, "Prepare preview");
        wait_resample(app, ctx);
        assert!(app.resample_editor.capture_state(&app.tracker.editor)["prepared"].is_object());
        keyzone_click(app, ctx, "Print new WAV");
        wait_resample(app, ctx);
        assert!(app.resample_editor.capture_state(&app.tracker.editor)["printed"].is_object());
    }
    #[test]
    #[cfg(unix)]
    fn resample_actual_three_phase_print_registration_and_undo_preserve_external_file() {
        let (mut app, ctx, fixture) = resample_app();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let cursor = keyzone_cursor(&app.tracker.editor);
        resample_number(&mut app, &ctx, "Native root MIDI", "65");
        keyzone_click(&mut app, &ctx, "Musical frames only");
        keyzone_click(&mut app, &ctx, "Prepare preview");
        wait_resample(&mut app, &ctx);
        let state = app.resample_editor.capture_state(&app.tracker.editor);
        assert_eq!(state["prepared"]["config"]["root_note"], 65);
        assert_eq!(state["prepared"]["summary"]["tail_frames"], 0);
        let output = fixture.0.join("resampled-occurrence.wav");
        assert!(!output.exists());
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        keyzone_click(&mut app, &ctx, "Print new WAV");
        wait_resample(&mut app, &ctx);
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(app.tracker.editor.song, before);
        let state = app.resample_editor.capture_state(&app.tracker.editor);
        let candidate: crate::song::Song =
            serde_json::from_value(state["printed"]["candidate"].clone()).unwrap();
        keyzone_click(&mut app, &ctx, "Register + new instrument");
        wait_resample(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, candidate);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        keyzone_click(&mut app, &ctx, "Close");
        keyzone_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        keyzone_click(&mut app, &ctx, "Redo");
        assert_eq!(app.tracker.editor.song, candidate);
        keyzone_click(&mut app, &ctx, "Resample…");
        assert!(app.resample_editor.capture_state(&app.tracker.editor)["retained"].is_object());
        keyzone_click(&mut app, &ctx, "Forget receipt (file stays)");
        assert!(app.resample_editor.capture_state(&app.tracker.editor)["retained"].is_null());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
    }
    #[test]
    #[cfg(unix)]
    fn resample_actual_invalid_inputs_and_modal_closing_read_registration_guards() {
        let (mut app, ctx, _fixture) = resample_app();
        let before = app.tracker.editor.song.clone();
        for value in ["-1", "128", "NaN", "1.5"] {
            resample_number(&mut app, &ctx, "Native root MIDI", value);
            keyzone_click(&mut app, &ctx, "Prepare preview");
            assert!(app.resample_editor.capture_state(&app.tracker.editor)["error"].is_string());
            assert_eq!(app.tracker.editor.song, before);
        }
        for guard in 0..3 {
            let (mut app, ctx, fixture) = resample_app();
            print_resample(&mut app, &ctx);
            let before = app.tracker.editor.song.clone();
            let button = keyzone_control(&mut app, &ctx, "Register + new instrument");
            match guard {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.pending = Some(crate::session::Action::Close),
                _ => app.session.close_ready = true,
            };
            pad_click(&mut app, &ctx, button);
            render(&mut app, &ctx, vec![key(egui::Key::Z)]);
            assert_eq!(app.tracker.editor.song, before);
            assert!(!app.resample_editor.busy());
            assert!(fixture.0.join("resampled-occurrence.wav").exists());
        }
    }
    #[test]
    #[cfg(unix)]
    fn resample_actual_close_new_and_no_clobber_keep_published_receipt_reachable() {
        let (mut app, ctx, fixture) = resample_app();
        print_resample(&mut app, &ctx);
        let output = fixture.0.join("resampled-occurrence.wav");
        let bytes = std::fs::read(&output).unwrap();
        keyzone_click(&mut app, &ctx, "Close");
        assert!(!app.resample_editor.open);
        keyzone_click(&mut app, &ctx, "New");
        assert!(!app.resample_editor.open);
        assert!(app.resample_editor.capture_state(&app.tracker.editor)["retained"].is_object());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        keyzone_click(&mut app, &ctx, "Resample…");
        let state = app.resample_editor.capture_state(&app.tracker.editor);
        assert!(state["retained"].is_object());
        assert!(state["printed"].is_null());
        keyzone_click(&mut app, &ctx, "Forget receipt (file stays)");
        keyzone_click(&mut app, &ctx, "Prepare preview");
        wait_resample(&mut app, &ctx);
        keyzone_click(&mut app, &ctx, "Print new WAV");
        wait_resample(&mut app, &ctx);
        assert!(app.resample_editor.capture_state(&app.tracker.editor)["error"].is_string());
        assert_eq!(std::fs::read(output).unwrap(), bytes);
    }
    fn tempo_app() -> (Leviathan, egui::Context, PadFixture) {
        use crate::song::*;
        let (mut app, ctx, fixture) = pad_app();
        let path = fixture.0.join("tempo.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for frame in 0..192000 {
            let local = frame % 24000;
            writer
                .write_sample(if local < 600 {
                    ((frame % 61) as i16 - 30) * 500
                } else {
                    0
                })
                .unwrap();
        }
        writer.finalize().unwrap();
        let imported = crate::sample_asset::import(&path).unwrap();
        let mut song = app.tracker.editor.song.clone();
        song.register_sample(SampleId(2), imported.asset).unwrap();
        song.upsert_region(
            RegionId(2),
            SampleRegion {
                name: "Tempo source".into(),
                sample: SampleId(2),
                start_frame: 0,
                end_frame: 192000,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        song.upsert_marker_set(
            MarkerSetId(1),
            MarkerSet {
                name: "Reviewed timing".into(),
                sample: SampleId(2),
                source_bpm: None,
                markers: (0..=8).map(|i| i * 24000).collect(),
                locked_frames: vec![48000],
            },
        )
        .unwrap();
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.sample_editor
            .inspect_region(&app.tracker.editor, RegionId(2), InstrumentId(1), None)
            .unwrap();
        let out = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&out, "Samples"));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.sample_editor.busy() {
            render(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        tempo_click(&mut app, &ctx, "Tempo / pulse", false);
        tempo_click(&mut app, &ctx, "Choose accepted markers", true);
        tempo_click(&mut app, &ctx, "1 · Reviewed timing", true);
        (app, ctx, fixture)
    }
    fn tempo_control(
        app: &mut Leviathan,
        ctx: &egui::Context,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..160 {
            let output = render_output(app, ctx, vec![]);
            if let Some(pos) = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text == label
                        || (label == "candidate120"
                            && text.galley.job.text.trim_start().starts_with("120.0   ")) =>
                {
                    let rect = text.visual_bounding_rect();
                    (shape.clip_rect.contains_rect(rect) && !text.galley.elided)
                        .then_some(rect.center())
                }
                _ => None,
            }) {
                if previous.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.02) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                previous = Some(pos);
                if stable >= 3 {
                    return pos;
                }
                continue;
            }
            previous = None;
            stable = 0;
            let clip = output
                .shapes
                .iter()
                .map(|s| s.clip_rect)
                .filter(|r| {
                    r.left() < 20.0 && r.right() > 870.0 && r.top() > 80.0 && r.height() > 250.0
                })
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .unwrap_or(egui::Rect::from_min_max(
                    egui::pos2(8.0, 90.0),
                    egui::pos2(892.0, 590.0),
                ));
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(clip.center().x, clip.top() + 8.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if down { -75.0 } else { 75.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        panic!("Visible native tempo control missing: {label}");
    }
    fn tempo_click(app: &mut Leviathan, ctx: &egui::Context, label: &str, down: bool) {
        let pos = tempo_control(app, ctx, label, down);
        pad_click(app, ctx, pos);
    }
    fn tempo_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        tempo_control(app, ctx, label, false);
        let id = egui::Id::new(("tempo-number", label));
        let rect = ctx.read_response(id).unwrap().rect;
        assert!(
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900.0, 620.0))
                .contains_rect(rect)
        );
        pad_click(app, ctx, rect.center());
        assert!(ctx.memory(|m| m.has_focus(id)));
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                }],
            );
        }
        render(app, ctx, vec![egui::Event::Text(value.into())]);
        render(app, ctx, vec![]);
        let state = app.sample_editor.capture_tempo_state(&app.tracker.editor);
        let field = match label {
            "BPM minimum" => "min_bpm",
            "BPM maximum" => "max_bpm",
            _ => panic!("Unknown tempo text"),
        };
        assert_eq!(state["draft"][field], value);
    }
    fn tempo_analyze(app: &mut Leviathan, ctx: &egui::Context) {
        tempo_click(app, ctx, "Analyze tempo / pulse", true);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.sample_editor.busy() {
            render(app, ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        render(app, ctx, vec![]);
    }
    #[test]
    fn actual_app_tempo_review_apply_undo_preserves_song_clock_markers_and_cursor() {
        let (mut app, ctx, _fixture) = tempo_app();
        let base = app.tracker.editor.song.clone();
        let cursor = keyzone_cursor(&app.tracker.editor);
        let revision = app.tracker.editor.revision();
        tempo_number(&mut app, &ctx, "BPM minimum", "119.9");
        tempo_number(&mut app, &ctx, "BPM maximum", "120.1");
        tempo_analyze(&mut app, &ctx);
        let state = app.sample_editor.capture_tempo_state(&app.tracker.editor);
        assert_eq!(state["current"], true);
        assert!(state["selected"].is_null());
        assert_eq!(app.tracker.editor.song, base);
        tempo_click(&mut app, &ctx, "candidate120", true);
        render(&mut app, &ctx, vec![]);
        let state = app.sample_editor.capture_tempo_state(&app.tracker.editor);
        let expected: crate::song::Song =
            serde_json::from_value(state["use_plan"]["candidate"].clone()).unwrap();
        assert_eq!(app.tracker.editor.song, base);
        tempo_click(&mut app, &ctx, "Use selected BPM only", true);
        assert_eq!(app.tracker.editor.song, expected);
        assert_eq!(app.tracker.editor.revision(), revision + 1);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        let mut only = base.clone();
        only.marker_sets
            .get_mut(&crate::song::MarkerSetId(1))
            .unwrap()
            .source_bpm = Some(120.0);
        assert_eq!(app.tracker.editor.song, only);
        tempo_click(&mut app, &ctx, "Undo", false);
        assert_eq!(app.tracker.editor.song, base);
        assert_eq!(
            app.sample_editor.capture_tempo_state(&app.tracker.editor)["current"],
            false
        );
        tempo_click(&mut app, &ctx, "Redo", false);
        assert_eq!(app.tracker.editor.song, expected);
    }
    #[test]
    fn actual_app_tempo_invalid_and_read_modal_closing_controls_cannot_mutate_notes() {
        let (mut app, ctx, _fixture) = tempo_app();
        let base = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        for value in ["120.05", "NaN", "400.1"] {
            tempo_number(&mut app, &ctx, "BPM minimum", value);
            tempo_analyze(&mut app, &ctx);
            assert!(
                app.sample_editor.capture_tempo_state(&app.tracker.editor)["error"].is_string()
            );
            assert_eq!(app.tracker.editor.song, base);
            assert_eq!(app.tracker.editor.revision(), revision);
        }
        tempo_number(&mut app, &ctx, "BPM minimum", "119.9");
        tempo_number(&mut app, &ctx, "BPM maximum", "120.1");
        tempo_analyze(&mut app, &ctx);
        tempo_click(&mut app, &ctx, "candidate120", true);
        let pos = tempo_control(&mut app, &ctx, "Use selected BPM only", true);
        app.tracker.editor.edit_mode = false;
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, base);
        app.tracker.editor.edit_mode = true;
        app.session.close_ready = true;
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, base);
        app.session.close_ready = false;
        app.session.pending = Some(crate::session::Action::Close);
        pad_click(&mut app, &ctx, pos);
        assert_eq!(app.tracker.editor.song, base);
        app.session.cancel();
        ctx.memory_mut(|m| m.request_focus(crate::tracker_ui::grid_id()));
        render(
            &mut app,
            &ctx,
            vec![key(egui::Key::Z), key(egui::Key::ArrowDown)],
        );
        assert_eq!(app.tracker.editor.song, base);
        assert_eq!(app.tracker.editor.row, 0);
    }
    fn catalog_control(
        app: &mut Leviathan,
        ctx: &egui::Context,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..200 {
            let output = render_output(app, ctx, vec![]);
            if let Some(pos) = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    let rect = text.visual_bounding_rect();
                    (shape.clip_rect.contains_rect(rect) && !text.galley.elided)
                        .then_some(rect.center())
                }
                _ => None,
            }) {
                if previous.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.02) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                previous = Some(pos);
                if stable >= 3 {
                    return pos;
                }
                continue;
            }
            previous = None;
            stable = 0;
            let clip = output
                .shapes
                .iter()
                .map(|s| s.clip_rect)
                .filter(|r| {
                    r.left() < 30.0
                        && r.width() > 600.0
                        && r.top() > 150.0
                        && (145.0..450.0).contains(&r.height())
                })
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .unwrap_or(egui::Rect::from_min_max(
                    egui::pos2(8.0, 90.0),
                    egui::pos2(892.0, 590.0),
                ));
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(clip.right() - 24.0, clip.center().y)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if down { -70.0 } else { 70.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let output = render_output(app, ctx, vec![]);
        let texts = output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::epaint::Shape::Text(t) = &s.shape {
                    Some((&t.galley.job.text, s.clip_rect))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        panic!("Catalog control missing {label}: {texts:?}");
    }
    fn catalog_click(app: &mut Leviathan, ctx: &egui::Context, label: &str, down: bool) {
        let point = catalog_control(app, ctx, label, down);
        pad_click(app, ctx, point);
    }
    fn catalog_settle(app: &mut Leviathan, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while app.sample_editor.busy() {
            render(app, ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..4 {
            render(app, ctx, vec![]);
        }
    }
    fn catalog_app() -> (Leviathan, egui::Context, PadFixture) {
        let (mut app, ctx, fixture) = tempo_app();
        use crate::song::{Cell, InstrumentId, Note, RegionId};
        let pattern = app.tracker.editor.pattern_id();
        let track = app.tracker.editor.track;
        app.tracker
            .editor
            .song
            .assign_region(InstrumentId(1), Some(RegionId(2)))
            .unwrap();
        app.tracker
            .editor
            .song
            .patterns
            .get_mut(&pattern)
            .unwrap()
            .tracks
            .entry(track)
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
        app.tracker.editor.song.validate().unwrap();
        tempo_click(&mut app, &ctx, "Slice catalog", false);
        (app, ctx, fixture)
    }
    fn catalog_note(app: &mut Leviathan, ctx: &egui::Context, value: &str) {
        catalog_control(app, ctx, "Review note", true);
        let id = egui::Id::new("slice-review-note");
        let rect = ctx.read_response(id).unwrap().rect;
        pad_click(app, ctx, rect.center());
        assert!(ctx.memory(|m| m.has_focus(id)));
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                }],
            );
        }
        render(app, ctx, vec![egui::Event::Text(value.into())]);
        render(app, ctx, vec![]);
        assert_eq!(
            app.sample_editor.capture_catalog_state(&app.tracker.editor)["role_draft"]["label"]
                ["note"],
            value
        );
    }
    #[test]
    fn actual_catalog_analysis_store_manual_ghost_and_history_preserve_playback() {
        use crate::{
            slice_features::{Articulation, PercussionRole},
            song::RegionId,
        };
        let (mut app, ctx, _fixture) = catalog_app();
        let base = app.tracker.editor.song.clone();
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.track,
            app.tracker.editor.row,
            app.tracker.editor.column,
            app.tracker.editor.field,
        );
        let manifest = crate::song_render::evaluate(&base, None).unwrap();
        catalog_click(&mut app, &ctx, "Select all regions", false);
        catalog_click(&mut app, &ctx, "Analyze selected regions", false);
        catalog_settle(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, base);
        let state = app.sample_editor.capture_catalog_state(&app.tracker.editor);
        assert_eq!(state["measured"]["records"].as_array().unwrap().len(), 1);
        assert!(state["current"].as_bool().unwrap());
        catalog_click(&mut app, &ctx, "Store reviewed catalog", true);
        catalog_settle(&mut app, &ctx);
        let stored = app.tracker.editor.song.clone();
        assert!(stored.region_catalog[&RegionId(2)].reviewed_role.is_none());
        assert_eq!(
            crate::song_render::evaluate(&stored, None).unwrap(),
            manifest
        );
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, base);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Redo"));
        assert_eq!(app.tracker.editor.song, stored);
        catalog_click(&mut app, &ctx, "R2", false);
        catalog_click(&mut app, &ctx, "Unknown", true);
        catalog_click(&mut app, &ctx, "Mixed", true);
        catalog_click(&mut app, &ctx, "Unspecified", true);
        catalog_click(&mut app, &ctx, "Ghost · manual", true);
        catalog_note(&mut app, &ctx, "Contextual manual ghost, not detected");
        assert_eq!(app.tracker.editor.song, stored);
        catalog_click(&mut app, &ctx, "Save manual role only", true);
        let review = app.tracker.editor.song.region_catalog[&RegionId(2)]
            .reviewed_role
            .as_ref()
            .unwrap();
        assert_eq!(review.label.role, PercussionRole::Mixed);
        assert_eq!(review.label.articulation, Articulation::Ghost);
        assert!(review.based_on_feature_sha256.is_none());
        assert_eq!(
            crate::song_render::evaluate(&app.tracker.editor.song, None).unwrap(),
            manifest
        );
        assert_eq!(
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.track,
                app.tracker.editor.row,
                app.tracker.editor.column,
                app.tracker.editor.field
            ),
            cursor
        );
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, stored);
    }
    #[test]
    fn actual_catalog_explicit_feature_review_invalid_text_and_read_mode_are_atomic() {
        use crate::{
            slice_features::{Articulation, PercussionRole},
            song::RegionId,
        };
        let (mut app, ctx, _fixture) = catalog_app();
        let base = app.tracker.editor.song.clone();
        catalog_click(&mut app, &ctx, "Select all regions", false);
        catalog_click(&mut app, &ctx, "Analyze selected regions", false);
        catalog_settle(&mut app, &ctx);
        catalog_click(&mut app, &ctx, "R2", false);
        catalog_click(&mut app, &ctx, "Unknown", true);
        catalog_click(&mut app, &ctx, "Mixed", true);
        catalog_click(&mut app, &ctx, "Unspecified", true);
        catalog_click(&mut app, &ctx, "Ghost · manual", true);
        catalog_note(&mut app, &ctx, &"x".repeat(257));
        catalog_click(&mut app, &ctx, "Save manual role only", true);
        assert_eq!(app.tracker.editor.song, base);
        assert!(app.sample_editor.capture_catalog_state(&app.tracker.editor)["error"].is_string());
        catalog_note(&mut app, &ctx, "Explicit review of measured region");
        catalog_click(
            &mut app,
            &ctx,
            "Include this explicit role in catalog Store",
            true,
        );
        let state = app.sample_editor.capture_catalog_state(&app.tracker.editor);
        assert_eq!(state["choices"]["2"]["articulation"], "ghost");
        app.tracker.editor.edit_mode = false;
        catalog_click(&mut app, &ctx, "Store reviewed catalog", true);
        catalog_settle(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, base);
        app.tracker.editor.edit_mode = true;
        catalog_click(&mut app, &ctx, "Store reviewed catalog", true);
        catalog_settle(&mut app, &ctx);
        let role = app.tracker.editor.song.region_catalog[&RegionId(2)]
            .reviewed_role
            .as_ref()
            .unwrap();
        assert_eq!(role.label.role, PercussionRole::Mixed);
        assert_eq!(role.label.articulation, Articulation::Ghost);
        assert!(role.based_on_feature_sha256.is_some());
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, base);
    }
    #[test]
    fn actual_catalog_stored_metadata_without_audio_clear_and_bounds_staleness() {
        use crate::{
            slice_features::{self, Articulation, PercussionRole, RoleLabel},
            song::{RegionId, SampleId},
        };
        let (mut app, ctx, _fixture) = catalog_app();
        catalog_click(&mut app, &ctx, "Select all regions", false);
        catalog_click(&mut app, &ctx, "Analyze selected regions", false);
        catalog_settle(&mut app, &ctx);
        catalog_click(&mut app, &ctx, "Store reviewed catalog", true);
        catalog_settle(&mut app, &ctx);
        app.tracker
            .editor
            .set_region_role(
                RegionId(2),
                Some(RoleLabel {
                    role: PercussionRole::Unknown,
                    articulation: Articulation::Unspecified,
                    note: "Deliberately reviewed Unknown".into(),
                }),
            )
            .unwrap();
        let path = app.tracker.editor.song.samples[&SampleId(2)].path.clone();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        app.sample_editor.reset_document();
        tempo_click(&mut app, &ctx, "Registered sources", false);
        tempo_click(&mut app, &ctx, "2 · tempo.wav", false);
        tempo_click(&mut app, &ctx, "Inspect stored catalog", true);
        catalog_click(&mut app, &ctx, "R2", false);
        let state = app.sample_editor.capture_catalog_state(&app.tracker.editor);
        assert_eq!(state["inspection"]["features_current_match"], true);
        assert!(state["measured"].is_null());
        assert!(!app.sample_editor.busy());
        let before = app.tracker.editor.song.clone();
        catalog_click(&mut app, &ctx, "Select all regions", false);
        catalog_click(&mut app, &ctx, "Analyze selected regions", false);
        catalog_settle(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, before);
        assert!(app.sample_editor.capture_catalog_state(&app.tracker.editor)["error"].is_string());
        catalog_click(&mut app, &ctx, "Clear descriptors only", true);
        let entry = &app.tracker.editor.song.region_catalog[&RegionId(2)];
        assert!(entry.features.is_none());
        assert!(entry.reviewed_role.is_some());
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, before);
        let mut region = app.tracker.editor.song.regions[&RegionId(2)].clone();
        region.start_frame += 1;
        app.tracker
            .editor
            .upsert_region(RegionId(2), region)
            .unwrap();
        render(&mut app, &ctx, vec![]);
        let inspection =
            slice_features::inspect_catalog(&app.tracker.editor.song, RegionId(2)).unwrap();
        assert_eq!(inspection.features_current_match, Some(false));
        assert_eq!(inspection.reviewed_role_current_match, Some(false));
        assert_eq!(
            app.sample_editor.capture_catalog_state(&app.tracker.editor)["inspection"],
            serde_json::to_value(inspection).unwrap()
        );
        std::fs::write(path, bytes).unwrap();
    }
    fn thin_control(app: &mut Leviathan, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        let mut previous = None;
        let mut at_top = false;
        for _attempt in 0..100 {
            let output = render_output(app, ctx, vec![]);
            let point = output.shapes.iter().rev().find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if t.galley.job.text == label => {
                    let r = egui::Rect::from_min_size(t.pos, t.galley.size());
                    s.clip_rect.contains_rect(r).then_some(r.center())
                }
                _ => None,
            });
            if let Some(p) = point {
                if previous.is_some_and(|old: egui::Pos2| old.distance(p) < 0.02) {
                    return p;
                }
                previous = Some(p);
            } else {
                previous = None;
                let state = app.tracker.thin.capture_state(&app.tracker.editor);
                if state["body_layout"]["offset_y"]
                    .as_f64()
                    .is_some_and(|y| y < 0.1)
                {
                    at_top = true;
                }
                let r = state["body_layout"]["rect"]
                    .as_array()
                    .expect("Measured THIN ScrollArea viewport");
                let body = egui::Rect::from_min_max(
                    egui::pos2(r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32),
                    egui::pos2(r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32),
                );
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(body.right() - 3.0, body.center().y)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if at_top { -45.0 } else { 2000.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                for _ in 0..8 {
                    render(app, ctx, vec![]);
                }
            }
        }
        panic!(
            "THIN control unreachable: {label}; layout {:?}",
            app.tracker.thin.capture_state(&app.tracker.editor)["body_layout"]
        );
    }
    fn thin_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let p = thin_control(app, ctx, label);
        pad_click(app, ctx, p);
    }
    fn thin_app() -> (Leviathan, egui::Context) {
        let (mut app, ctx) = transform_app();
        app.tracker
            .editor
            .replace_document(crate::thin_ui::test_song())
            .unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.editor.edit_step = 0;
        render(&mut app, &ctx, vec![]);
        (app, ctx)
    }
    fn open_thin(app: &mut Leviathan, ctx: &egui::Context) {
        let output = render_output(app, ctx, vec![]);
        pad_click(
            app,
            ctx,
            control(&output, "Song audio · instruments & offline export"),
        );
        let output = render_output(app, ctx, vec![]);
        pad_click(app, ctx, control(&output, "Thin…"));
        render(app, ctx, vec![]);
        assert!(app.tracker.thin.is_open());
    }
    fn wait_thin(app: &mut Leviathan, ctx: &egui::Context) {
        let end = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            render(app, ctx, vec![]);
            if !app.tracker.thin.busy() {
                break;
            }
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn thin_number(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        let _ = thin_control(app, ctx, label);
        let id = egui::Id::new(("thin-number", label));
        let mut point = None;
        let mut previous = None;
        for _ in 0..80 {
            render(app, ctx, vec![]);
            let response = ctx
                .read_response(id)
                .expect("Numeric THIN field registered");
            if response
                .interact_rect
                .expand(0.01)
                .contains_rect(response.rect)
                && ctx.layer_id_at(response.rect.center()) == Some(response.layer_id)
            {
                if previous
                    .is_some_and(|old: egui::Pos2| old.distance(response.rect.center()) < 0.01)
                {
                    point = Some(response.rect.center());
                    break;
                }
                previous = Some(response.rect.center());
            } else {
                previous = None;
                render(
                    app,
                    ctx,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(822.0, 280.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(
                                0.0,
                                if response.rect.center().y < response.interact_rect.center().y {
                                    50.0
                                } else {
                                    -50.0
                                },
                            ),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        pad_click(
            app,
            ctx,
            point.expect("Exact field must be fully visible before typing"),
        );
        assert!(
            ctx.memory(|memory| memory.has_focus(id)),
            "Exact {label} input must receive focus"
        );
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            }],
        );
        render(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text(value.into()),
            ],
        );
        render(app, ctx, vec![key(egui::Key::Enter)]);
        let _ = render_output(
            app,
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        // Clearing the old preview changes scroll content height. Read the edited
        // widget only after actual visible layout has settled, never tracker text behind it.
        let _ = thin_control(app, ctx, label);
        let output = render_output(app, ctx, vec![]);
        let rect = ctx.read_response(id).unwrap().rect;
        let painted = output
            .shapes
            .iter()
            .rev()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if rect.intersects(
                        egui::Rect::from_min_size(text.pos, text.galley.size())
                            .intersect(shape.clip_rect),
                    ) =>
                {
                    Some(text.galley.job.text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            painted.contains(&value),
            "Requested {label}={value} must equal actual painted input; {painted:?}"
        );
    }

    fn preview_thin(app: &mut Leviathan, ctx: &egui::Context) {
        open_thin(app, ctx);
        thin_click(app, ctx, "Whole pattern");
        thin_number(app, ctx, "Removal %", "100");
        thin_click(app, ctx, "Preview THIN");
        wait_thin(app, ctx);
    }
    #[test]
    fn thin_actual_scoped_preview_commit_undo_redo_matches_shared_ordinary_cells() {
        let (mut app, ctx) = thin_app();
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let cursor = keyzone_cursor(&app.tracker.editor);
        preview_thin(&mut app, &ctx);
        let p = app
            .tracker
            .thin
            .capture_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        assert_eq!(p.summary().removed_cells, 3);
        assert!(p.uses().iter().any(|u| u.row == 12 && u.velocity == 0.0));
        assert_eq!(
            p.decisions().iter().find(|d| d.row == 12).unwrap().reason,
            crate::mutations::ThinReason::Removed
        );
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        let shared = crate::mutations::preview(&before, p.config().clone()).unwrap();
        assert_eq!(p.id(), shared.id());
        assert_eq!(p.decisions(), shared.decisions());
        assert_eq!(p.uses(), shared.uses());
        assert!(p
            .decisions()
            .iter()
            .any(|d| d.reason == crate::mutations::ThinReason::StaleReview));
        thin_click(&mut app, &ctx, "Removed cells");
        thin_click(&mut app, &ctx, "Resolved uses");
        thin_click(&mut app, &ctx, "Commit THIN");
        assert_eq!(&app.tracker.editor.song, p.candidate());
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        assert!(!app.tracker.thin.is_open());
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Undo"));
        assert_eq!(app.tracker.editor.song, before);
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Redo"));
        assert_eq!(&app.tracker.editor.song, p.candidate());
    }
    #[test]
    fn thin_actual_failed_replacement_retains_readonly_review_and_exact_invalid_inputs() {
        let (mut app, ctx) = thin_app();
        let before = app.tracker.editor.song.clone();
        preview_thin(&mut app, &ctx);
        let id = app
            .tracker
            .thin
            .capture_preview(&app.tracker.editor)
            .unwrap()
            .id()
            .to_owned();
        thin_number(&mut app, &ctx, "Removal cap", "0");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        let state = app.tracker.thin.capture_state(&app.tracker.editor);
        assert_eq!(state["preview"]["id"], id);
        assert_eq!(state["current"], false);
        assert!(state["error"].as_str().unwrap().contains("max_removed"));
        thin_click(&mut app, &ctx, "Commit THIN");
        assert_eq!(app.tracker.editor.song, before);
        for (label, value) in [
            ("Removal %", "101"),
            ("Removal %", "-1"),
            ("Removal %", "0.1"),
            ("Removal %", "NaN"),
            ("Seed", "18446744073709551616"),
            ("Removal cap", "4097"),
            ("Row start", "-1"),
        ] {
            thin_number(&mut app, &ctx, label, value);
            thin_click(&mut app, &ctx, "Preview THIN");
            wait_thin(&mut app, &ctx);
            let state = app.tracker.thin.capture_state(&app.tracker.editor);
            assert_eq!(state["current"], false);
            assert_eq!(state["preview"]["id"], id);
            assert!(state["error"].is_string());
            assert_eq!(app.tracker.editor.song, before);
        }
    }
    #[test]
    fn thin_actual_zero_and_read_modal_close_navigation_guards_preserve_document() {
        for guard in 0..3 {
            let (mut app, ctx) = thin_app();
            preview_thin(&mut app, &ctx);
            let before = app.tracker.editor.song.clone();
            match guard {
                0 => app.tracker.editor.edit_mode = false,
                1 => app.session.pending = Some(crate::session::Action::Close),
                _ => app.session.close_ready = true,
            };
            thin_click(&mut app, &ctx, "Commit THIN");
            assert_eq!(app.tracker.editor.song, before);
        }
        let (mut app, ctx) = thin_app();
        open_thin(&mut app, &ctx);
        thin_click(&mut app, &ctx, "Whole pattern");
        thin_number(&mut app, &ctx, "Removal %", "0");
        thin_number(&mut app, &ctx, "Seed", "18446744073709551615");
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        assert_eq!(
            app.tracker
                .thin
                .capture_preview(&app.tracker.editor)
                .unwrap()
                .candidate(),
            &before
        );
        thin_click(&mut app, &ctx, "Commit THIN");
        assert_eq!(app.tracker.editor.revision(), revision);
        app.tracker.editor.row = 1;
        render(&mut app, &ctx, vec![]);
        assert!(!app.tracker.thin.is_open());
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn thin_actual_protection_toggles_remain_explicit_and_whole_preview_cap_is_not_truncation() {
        let (mut app, ctx) = thin_app();
        preview_thin(&mut app, &ctx);
        thin_click(&mut app, &ctx, "Kick");
        thin_click(&mut app, &ctx, "Snare");
        thin_number(&mut app, &ctx, "Removal cap", "4");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        assert!(app.tracker.thin.capture_state(&app.tracker.editor)["error"].is_string());
        thin_number(&mut app, &ctx, "Removal cap", "4096");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        let p = app
            .tracker
            .thin
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert!(p.config().protected_roles.is_empty());
        assert_eq!(p.summary().removed_cells, 9);
        assert!(p
            .decisions()
            .iter()
            .filter(|d| d.reason == crate::mutations::ThinReason::Removed)
            .all(|d| [0, 1, 3].contains(&(d.row % 6))));
    }
    #[test]
    fn thin_actual_seeded_probability_and_max_seed_are_exact_shared_gate_inputs() {
        let (mut app, ctx) = thin_app();
        open_thin(&mut app, &ctx);
        thin_click(&mut app, &ctx, "Whole pattern");
        thin_number(&mut app, &ctx, "Removal %", "37");
        thin_number(&mut app, &ctx, "Seed", "42");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        let p = app
            .tracker
            .thin
            .capture_preview(&app.tracker.editor)
            .unwrap()
            .clone();
        assert_eq!(p.summary().random_draws, 3);
        assert_eq!(p.summary().effective_probability, 0.37_f32);
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        assert_eq!(
            app.tracker
                .thin
                .capture_preview(&app.tracker.editor)
                .unwrap()
                .id(),
            p.id()
        );
        thin_number(&mut app, &ctx, "Seed", "18446744073709551615");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        let q = app
            .tracker
            .thin
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(q.config().seed, u64::MAX);
        let reference =
            crate::mutations::preview(&app.tracker.editor.song, q.config().clone()).unwrap();
        assert_eq!(q.decisions(), reference.decisions());
        assert_eq!(q.record(), reference.record());
    }
    #[test]
    fn thin_actual_phrase_author_and_keyboard_focus_never_mutate_synthetic_projection() {
        use crate::song::*;
        let (mut app, ctx) = thin_app();
        app.tracker
            .editor
            .upsert_phrase(
                PhraseId(1),
                Phrase {
                    name: "THIN isolated phrase".into(),
                    instrument: InstrumentId(1),
                    rows: 8,
                    lines_per_beat: 4,
                    note_columns: 1,
                    root_note: 60,
                    cells: Default::default(),
                },
            )
            .unwrap();
        let p = matrix_control(&mut app, &ctx, "Phrases");
        pad_click(&mut app, &ctx, p);
        open_thin(&mut app, &ctx);
        let before = app.tracker.editor.song.clone();
        ctx.memory_mut(|m| m.request_focus(crate::tracker_ui::grid_id()));
        render(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::Z),
                key(egui::Key::Delete),
                key(egui::Key::Space),
            ],
        );
        assert_eq!(app.tracker.editor.song, before);
        assert!(app.tracker.editor.edit_mode);
        thin_click(&mut app, &ctx, "Cancel / close");
        let p = matrix_control(&mut app, &ctx, "Edit phrase");
        pad_click(&mut app, &ctx, p);
        render(&mut app, &ctx, vec![]);
        assert!(app.tracker.is_phrase_authoring());
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Thin…"));
        render(&mut app, &ctx, vec![]);
        assert!(!app.tracker.thin.is_open());
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn thin_actual_zero_still_refuses_unassigned_source_without_inventing_silence() {
        let (mut app, ctx) = transform_app();
        let before = app.tracker.editor.song.clone();
        open_thin(&mut app, &ctx);
        thin_number(&mut app, &ctx, "Removal %", "0");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        let state = app.tracker.thin.capture_state(&app.tracker.editor);
        assert!(state["error"].as_str().unwrap().contains("assigned"));
        assert!(state["preview"].is_null());
        assert_eq!(app.tracker.editor.song, before);
    }
    #[test]
    fn thin_actual_pattern_track_columns_and_half_open_rows_are_explicit_not_cursor_navigation() {
        use crate::song::*;
        let (mut app, ctx) = thin_app();
        let mut song = app.tracker.editor.song.clone();
        let mut pattern = song.patterns[&PatternId(1)].clone();
        pattern.name = "Other scope".into();
        pattern.tracks.clear();
        song.patterns.insert(PatternId(2), pattern);
        song.tracks.get_mut(&TrackId(2)).unwrap().name = "Other track".into();
        song.tracks.get_mut(&TrackId(2)).unwrap().note_columns = 2;
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        let before = app.tracker.editor.song.clone();
        let cursor = keyzone_cursor(&app.tracker.editor);
        open_thin(&mut app, &ctx);
        thin_click(&mut app, &ctx, "P1");
        thin_click(&mut app, &ctx, "P2 · Other scope · 16 rows");
        thin_click(&mut app, &ctx, "T1");
        thin_click(&mut app, &ctx, "T2 · Other track · 2 columns");
        thin_number(&mut app, &ctx, "Row start", "1");
        thin_number(&mut app, &ctx, "End exclusive", "8");
        thin_click(&mut app, &ctx, "N2");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        let p = app
            .tracker
            .thin
            .capture_preview(&app.tracker.editor)
            .unwrap();
        assert_eq!(
            p.config().scope,
            crate::transforms::TransformScope {
                pattern: PatternId(2),
                track: TrackId(2),
                row_start: 1,
                row_end: 8,
                columns: vec![0, 1]
            }
        );
        assert_eq!(p.candidate(), &before);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        thin_click(&mut app, &ctx, "No columns");
        thin_click(&mut app, &ctx, "Preview THIN");
        wait_thin(&mut app, &ctx);
        assert!(app.tracker.thin.capture_state(&app.tracker.editor)["error"].is_string());
        assert_eq!(app.tracker.editor.song, before);
    }
    fn audition_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let pos = pad_control(app, ctx, label);
        pad_click(app, ctx, pos);
    }
    fn audition_top(app: &mut Leviathan, ctx: &egui::Context) {
        for x in [90.0, 400.0] {
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(x, 380.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 10000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for _ in 0..8 {
            render(app, ctx, vec![]);
        }
    }
    fn audition_app(region: bool, held: bool) -> (Leviathan, egui::Context, PadFixture) {
        let (mut app, ctx, fixture) = pad_app();
        audition_click(&mut app, &ctx, "01\nC2");
        if region {
            audition_click(&mut app, &ctx, "Inspect region");
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while app.sample_editor.busy() {
                render(&mut app, &ctx, vec![]);
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            render(&mut app, &ctx, vec![]);
        }
        app.diagnostic_audio.enabled = true;
        app.diagnostic_audio.held = held;
        (app, ctx, fixture)
    }
    fn audition_wait(app: &mut Leviathan, ctx: &egui::Context, complete: bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while if complete {
            app.transport_busy()
        } else {
            !app.diagnostic_audio.prepared && app.transport_busy()
        } {
            render(app, ctx, vec![]);
            assert!(
                std::time::Instant::now() < deadline,
                "Audition worker timeout: {}",
                app.capture_audition_state()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        render(app, ctx, vec![]);
    }
    fn audition_document(app: &Leviathan) -> serde_json::Value {
        let e = &app.tracker.editor;
        serde_json::json!({"song":e.song,"cursor":format!("{:?}",keyzone_cursor(e)),"revision":e.revision(),"dirty":e.dirty,
            "undo":e.can_undo(),"redo":e.can_redo(),"playhead":app.tracker.playhead})
    }
    fn audition_pcm(song: &crate::song::Song) -> Vec<f32> {
        let mut engine = crate::song_render::Engine::new(song, None).unwrap();
        let mut pcm = vec![];
        let mut block = [0.0; 1024];
        loop {
            let n = engine
                .block(&mut block, crate::audition::TOTAL_FRAMES)
                .unwrap();
            if n == 0 {
                break;
            }
            pcm.extend_from_slice(&block[..n * 2]);
        }
        pcm
    }
    #[test]
    fn audition_actual_region_and_pad_read_only_clicks_match_engine_and_never_edit() {
        for region in [false, true] {
            let (mut app, ctx, _fixture) = audition_app(region, false);
            app.tracker.editor.edit_mode = false;
            app.loop_song = true; // Audition must never inherit Song loop policy.
            let before = audition_document(&app);
            let request = app.audition_request().unwrap();
            let expected = request.prepare(&app.tracker.editor.song).unwrap();
            let reference = audition_pcm(expected.song());
            app.diagnostic_audio.auto_advance = true;
            audition_click(
                &mut app,
                &ctx,
                if region {
                    "Audition draft"
                } else {
                    "Audition selected"
                },
            );
            audition_wait(&mut app, &ctx, true);
            assert_eq!(app.audition.starts, 1, "{}", app.capture_audition_state());
            assert!(app.audition.error.is_none(), "{:?}", app.audition.error);
            assert_eq!(app.audition.last.as_ref().unwrap().id(), expected.id());
            assert_eq!(app.diagnostic_audio.pcm, reference);
            assert_eq!(app.diagnostic_audio.pcm.len(), 240000);
            assert_eq!(app.diagnostic_audio.after_stop_silent, Some(true));
            assert_eq!(audition_document(&app), before);
        }
    }
    #[test]
    fn audition_actual_repeated_start_stop_and_song_exclusion_preserve_cursor() {
        let (mut app, ctx, _fixture) = audition_app(false, true);
        let before = audition_document(&app);
        audition_click(&mut app, &ctx, "Audition selected");
        assert!(app.transport_busy());
        audition_click(&mut app, &ctx, "Audition selected");
        assert_eq!(app.audition.starts, 1);
        assert!(app
            .start_transport(app.tracker.editor.song.clone(), None, false)
            .is_err());
        audition_click(&mut app, &ctx, "Stop audition");
        app.diagnostic_audio.release();
        audition_wait(&mut app, &ctx, true);
        assert!(app.diagnostic_audio.pcm.is_empty());
        assert_eq!(app.diagnostic_audio.after_stop_silent, Some(true));
        assert_eq!(audition_document(&app), before);
        // Song-owned work, including its stopped-but-unretired worker, owns the same driver.
        app.audition.song_started();
        app.diagnostic_audio.held = true;
        app.start_transport(app.tracker.editor.song.clone(), None, false)
            .unwrap();
        // Realtime Follow deliberately replaces Pads with the read-only playback snapshot.
        // Turn it off through the visible control before checking that Song ownership blocks
        // a new audition from the Pads view.
        let output = render_output(&mut app, &ctx, vec![]);
        pad_click(&mut app, &ctx, control(&output, "Follow"));
        assert!(!app.follow);
        audition_click(&mut app, &ctx, "Audition selected");
        assert_eq!(app.audition.starts, 1);
        app.stop_transport();
        assert!(app
            .start_transport(app.tracker.editor.song.clone(), None, false)
            .is_err());
        audition_wait(&mut app, &ctx, true);
        assert_eq!(audition_document(&app), before);
    }
    #[test]
    fn audition_actual_stop_prefix_and_disabled_song_controls_are_not_full_render_claims() {
        let (mut app, ctx, _fixture) = audition_app(false, false);
        let before = audition_document(&app);
        audition_click(&mut app, &ctx, "Audition selected");
        audition_wait(&mut app, &ctx, false);
        let reference = audition_pcm(app.audition.last.as_ref().unwrap().song());
        for _ in 0..4 {
            app.diagnostic_audio.advance();
            let target = app.diagnostic_audio.pcm.len() + 1024;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while app.diagnostic_audio.pcm.len() < target {
                render(&mut app, &ctx, vec![]);
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let follow = app.follow;
        let loop_song = app.loop_song;
        audition_top(&mut app, &ctx);
        for label in ["Follow", "Loop", "Seek to edit row", "▶ Play"] {
            let output = render_output(&mut app, &ctx, vec![]);
            pad_click(&mut app, &ctx, control(&output, label));
        }
        assert_eq!(app.follow, follow);
        assert_eq!(app.loop_song, loop_song);
        assert_eq!(app.audition.starts, 1);
        audition_click(&mut app, &ctx, "Stop audition");
        audition_wait(&mut app, &ctx, true);
        assert_eq!(app.diagnostic_audio.pcm, reference[..4096]);
        assert_eq!(app.diagnostic_audio.after_stop_silent, Some(true));
        assert_eq!(audition_document(&app), before);
    }
    #[test]
    fn audition_actual_late_preparation_is_revoked_by_selection_focus_view_and_document() {
        for mode in 0..8 {
            let (mut app, ctx, fixture) = audition_app(false, true);
            let before = audition_document(&app);
            audition_click(&mut app, &ctx, "Audition selected");
            let expected_epoch = app.audition.epoch;
            // The explicit Cells toolbar can add a wrapped line at narrow widths.
            // Return via actual wheel input before choosing controls above Audition.
            if mode == 0 || mode == 2 {
                audition_top(&mut app, &ctx);
            }
            match mode {
                0 => audition_click(&mut app, &ctx, "02\nC#2"),
                1 => {
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(900.0, 620.0),
                            )),
                            focused: false,
                            events: vec![egui::Event::WindowFocused(false)],
                            ..Default::default()
                        },
                        |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
                    );
                }
                2 => audition_click(&mut app, &ctx, "Edit mapping"),
                3 => {
                    app.sample_view = true;
                    render(&mut app, &ctx, vec![]);
                }
                4 => {
                    app.request_action(crate::session::Action::New);
                }
                5 => {
                    let path = fixture.0.join("same.leviathan.json");
                    crate::project::save(
                        &app.tracker.editor.song,
                        &path,
                        crate::project::SaveMode::CreateNew,
                    )
                    .unwrap();
                    app.request_action(crate::session::Action::Open(path));
                }
                6 => {
                    app.session.pending = Some(crate::session::Action::Close);
                    render(&mut app, &ctx, vec![]);
                }
                _ => {
                    app.session.close_ready = true;
                    render(&mut app, &ctx, vec![]);
                }
            }
            assert!(
                app.audition.canceled,
                "mode {mode}, state {}",
                app.capture_audition_state()
            );
            app.diagnostic_audio.release();
            audition_wait(&mut app, &ctx, true);
            assert!(app.audition.canceled, "mode {mode}");
            assert!(app.audition.epoch > expected_epoch);
            assert!(app.diagnostic_audio.pcm.is_empty(), "mode {mode}");
            assert_eq!(app.diagnostic_audio.after_stop_silent, Some(true));
            assert_eq!(app.diagnostic_audio.starts, 1);
            if mode != 4 && mode != 5 {
                assert_eq!(audition_document(&app), before, "mode {mode}");
            }
        }
    }
    #[test]
    fn audition_actual_unsaved_region_draft_change_revokes_held_run_without_apply() {
        let (mut app, ctx, _fixture) = audition_app(true, true);
        let before = audition_document(&app);
        audition_click(&mut app, &ctx, "Audition draft");
        audition_top(&mut app, &ctx);
        pad_type(&mut app, &ctx, "Owned hit", "Unapplied draft");
        app.diagnostic_audio.release();
        audition_wait(&mut app, &ctx, true);
        assert!(app.audition.canceled);
        assert!(app.diagnostic_audio.pcm.is_empty());
        assert_eq!(audition_document(&app), before);
        let crate::audition_ui::Request::Region { draft, .. } =
            app.sample_editor.audition_request().unwrap()
        else {
            panic!()
        };
        assert_eq!(draft.name, "Unapplied draft");
    }
    #[test]
    fn audition_actual_source_tamper_failure_is_visible_and_explicit_retry_succeeds() {
        let (mut app, ctx, fixture) = audition_app(false, false);
        let before = audition_document(&app);
        let path = fixture.0.join("owned.wav");
        let original = std::fs::read(&path).unwrap();
        let mut wrong = original.clone();
        *wrong.last_mut().unwrap() ^= 1;
        std::fs::write(&path, wrong).unwrap();
        audition_click(&mut app, &ctx, "Audition selected");
        audition_wait(&mut app, &ctx, true);
        assert!(
            app.audition.error.is_some(),
            "{}",
            app.capture_audition_state()
        );
        assert!(app.diagnostic_audio.pcm.is_empty());
        std::fs::write(&path, original).unwrap();
        app.diagnostic_audio.auto_advance = true;
        audition_click(&mut app, &ctx, "Audition selected");
        audition_wait(&mut app, &ctx, true);
        assert_eq!(app.audition.starts, 2);
        assert!(app.audition.error.is_none());
        assert_eq!(app.diagnostic_audio.pcm.len(), 240000);
        assert_eq!(audition_document(&app), before);
    }
    #[cfg(not(feature = "realtime"))]
    #[test]
    fn audition_actual_normal_non_realtime_build_is_unavailable_without_opt_in() {
        let (mut app, ctx, _fixture) = pad_app();
        audition_click(&mut app, &ctx, "01\nC2");
        let before = audition_document(&app);
        audition_click(&mut app, &ctx, "Audition selected");
        assert!(!app.audio_available());
        assert_eq!(app.audition.starts, 0);
        assert!(!app.transport_busy());
        assert_eq!(audition_document(&app), before);
    }
    #[test]
    fn audition_actual_invalid_region_text_cancels_preparation_and_cannot_start() {
        let (mut app, ctx, _fixture) = audition_app(true, true);
        let before = audition_document(&app);
        audition_click(&mut app, &ctx, "Sampler playback · off / legacy behavior");
        audition_click(&mut app, &ctx, "Amplitude envelope");
        audition_click(&mut app, &ctx, "Audition draft");
        assert_eq!(app.audition.starts, 1);
        audition_top(&mut app, &ctx);
        pad_control(&mut app, &ctx, "Attack ms");
        let id = egui::Id::new(("region-playback", "Attack ms"));
        let rect = ctx.read_response(id).unwrap().rect;
        assert!(
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900.0, 620.0))
                .contains_rect(rect)
        );
        pad_click(&mut app, &ctx, rect.center());
        assert!(ctx.memory(|m| m.has_focus(id)));
        for pressed in [true, false] {
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
            );
        }
        render(&mut app, &ctx, vec![egui::Event::Text("10001".into())]);
        assert!(app
            .sample_editor
            .audition_request()
            .unwrap_err()
            .contains("Attack"));
        assert!(app.audition.canceled);
        app.diagnostic_audio.release();
        audition_wait(&mut app, &ctx, true);
        audition_click(&mut app, &ctx, "Audition draft");
        assert!(app.audition.error.as_ref().unwrap().contains("Attack"));
        assert_eq!(app.audition.starts, 1);
        assert!(app.diagnostic_audio.pcm.is_empty());
        assert_eq!(audition_document(&app), before);
    }
    #[test]
    fn audition_actual_zoned_pad_uses_committed_bin127_first_rr_and_bounds_loop() {
        use crate::song::*;
        let (mut app, ctx, _fixture) = audition_app(false, false);
        let mut song = app.tracker.editor.song.clone();
        let mut region = song.regions[&RegionId(1)].clone();
        region.start_frame = 100;
        region.end_frame = 400;
        region.root_note = 72;
        region.forward_loop = Some(ForwardLoop {
            start_frame: 110,
            end_frame: 210,
        });
        song.upsert_region(RegionId(2), region).unwrap();
        let pad = song
            .drum_kits
            .get_mut(&DrumKitId(1))
            .unwrap()
            .pads
            .get_mut(&PadId(1))
            .unwrap();
        pad.region = None;
        pad.velocity_zones = vec![
            VelocityZone {
                min_velocity: 0,
                max_velocity: 126,
                alternatives: vec![PadLayer {
                    id: PadLayerId(11),
                    region: RegionId(1),
                }],
            },
            VelocityZone {
                min_velocity: 127,
                max_velocity: 127,
                alternatives: vec![
                    PadLayer {
                        id: PadLayerId(21),
                        region: RegionId(2),
                    },
                    PadLayer {
                        id: PadLayerId(22),
                        region: RegionId(1),
                    },
                ],
            },
        ];
        pad.tune_semitones = 12;
        pad.gain = 0.25;
        pad.pan = 0.3;
        app.tracker.editor.replace_document(song).unwrap();
        let before = audition_document(&app);
        app.diagnostic_audio.auto_advance = true;
        for run in 1..=2 {
            audition_click(&mut app, &ctx, "Audition selected");
            audition_wait(&mut app, &ctx, true);
            let preview = app.audition.last.as_ref().unwrap();
            let summary = preview.summary();
            let layer = summary.layer_selection.as_ref().unwrap();
            assert_eq!(
                (
                    layer.velocity_bin,
                    layer.round_robin_index,
                    layer.round_robin_ordinal
                ),
                (127, 0, 0)
            );
            assert_eq!(layer.layer, PadLayerId(21));
            assert_eq!(summary.region.region, RegionId(2));
            assert_eq!(summary.on_gain, 0.25);
            assert_eq!(summary.pad.as_ref().unwrap().tune_semitones, 12);
            assert_eq!(app.diagnostic_audio.pcm, audition_pcm(preview.song()));
            assert_eq!(app.diagnostic_audio.pcm.len(), 240000);
            assert_eq!(app.audition.starts, run);
        }
        assert_eq!(audition_document(&app), before);
    }
    #[test]
    fn audition_exact_selected_input_binding_catches_unchecked_float_and_source_changes() {
        use crate::song::*;
        for change in 0..4 {
            let (mut app, ctx, _fixture) = audition_app(false, true);
            audition_click(&mut app, &ctx, "Audition selected");
            let revision = app.tracker.editor.revision();
            match change {
                0 => {
                    app.tracker
                        .editor
                        .song
                        .drum_kits
                        .get_mut(&DrumKitId(1))
                        .unwrap()
                        .pads
                        .get_mut(&PadId(1))
                        .unwrap()
                        .pan = -0.0
                }
                1 => {
                    app.tracker
                        .editor
                        .song
                        .regions
                        .get_mut(&RegionId(1))
                        .unwrap()
                        .root_note = 61
                }
                2 => {
                    app.tracker
                        .editor
                        .song
                        .samples
                        .get_mut(&SampleId(1))
                        .unwrap()
                        .sha256 = "b".repeat(64)
                }
                _ => app
                    .tracker
                    .editor
                    .song
                    .title
                    .push_str(" unrelated unchecked metadata"),
            }
            render(&mut app, &ctx, vec![]);
            assert_eq!(app.tracker.editor.revision(), revision);
            assert_eq!(
                app.audition.canceled,
                change != 3,
                "Exact input binding, not unrelated full Song equality"
            );
            app.stop_transport();
            app.diagnostic_audio.release();
            audition_wait(&mut app, &ctx, true);
            assert!(app.diagnostic_audio.pcm.is_empty());
        }
    }
    #[test]
    fn audition_actual_focus_loss_and_regain_in_one_frame_revokes_held_and_active_runs() {
        for held in [true, false] {
            let (mut app, ctx, _fixture) = audition_app(false, held);
            let before = audition_document(&app);
            audition_click(&mut app, &ctx, "Audition selected");
            if !held {
                audition_wait(&mut app, &ctx, false);
                for _ in 0..4 {
                    let target = app.diagnostic_audio.pcm.len() + 1024;
                    app.diagnostic_audio.advance();
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    while app.diagnostic_audio.pcm.len() < target {
                        render(&mut app, &ctx, vec![]);
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
            let prefix = app.diagnostic_audio.pcm.clone();
            render(
                &mut app,
                &ctx,
                vec![
                    egui::Event::WindowFocused(false),
                    egui::Event::WindowFocused(true),
                ],
            );
            assert!(
                ctx.input(|i| i.focused),
                "Raw final focus is true despite an observed loss event"
            );
            assert!(ctx.input(|i| i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false)))));
            assert!(
                app.audition.canceled,
                "Loss event must revoke, including same-frame regain"
            );
            app.diagnostic_audio.release();
            audition_wait(&mut app, &ctx, true);
            assert_eq!(app.diagnostic_audio.pcm, prefix);
            assert_eq!(app.diagnostic_audio.after_stop_silent, Some(true));
            for _ in 0..20 {
                render(&mut app, &ctx, vec![]);
            }
            assert_eq!(app.audition.starts, 1, "Regaining focus must not restart");
            assert!(!app.transport_busy());
            assert_eq!(audition_document(&app), before);
            app.diagnostic_audio.held = false;
            audition_click(&mut app, &ctx, "Audition selected");
            audition_wait(&mut app, &ctx, false);
            assert_eq!(
                app.audition.starts, 2,
                "A fresh explicit click is allowed after retirement"
            );
            audition_click(&mut app, &ctx, "Stop audition");
            audition_wait(&mut app, &ctx, true);
            assert_eq!(audition_document(&app), before);
        }
    }
    fn marker_bank_app() -> (Leviathan, egui::Context, PadFixture) {
        let (mut app, ctx, fixture) = tempo_app();
        tempo_click(&mut app, &ctx, "Break workflow", false);
        tempo_click(&mut app, &ctx, "Markers", true);
        tempo_click(&mut app, &ctx, "New marker draft", true);
        tempo_click(&mut app, &ctx, "M1 · Reviewed timing", true);
        tempo_click(&mut app, &ctx, "Build pad bank…", false);
        for _ in 0..6 {
            render(&mut app, &ctx, vec![]);
        }
        assert_eq!(app.capture_marker_bank_state()["open"], true);
        (app, ctx, fixture)
    }
    fn marker_bank_control(
        app: &mut Leviathan,
        ctx: &egui::Context,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        let mut prior = None;
        let mut stable = 0;
        for _ in 0..180 {
            let out = render_output(app, ctx, vec![]);
            if let Some(pos) = out.shapes.iter().find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if t.galley.job.text == label => {
                    let r = t.visual_bounding_rect();
                    (s.clip_rect.contains_rect(r)
                        && !t.galley.elided
                        && egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900., 620.))
                            .contains_rect(r))
                    .then_some(r.center())
                }
                _ => None,
            }) {
                if prior.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.1) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                prior = Some(pos);
                if stable >= 3 {
                    return pos;
                }
                continue;
            }
            let state = app.capture_marker_bank_state();
            let r = state["body_layout"]["rect"]
                .as_array()
                .expect("Bank body measured");
            let pos = egui::pos2(
                (r[0].as_f64().unwrap() + r[2].as_f64().unwrap()) as f32 / 2.,
                (r[1].as_f64().unwrap() + r[3].as_f64().unwrap()) as f32 / 2.,
            );
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., if down { -70. } else { 70. }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        panic!(
            "Bank control not reachable: {label}; {}",
            app.capture_marker_bank_state()
        );
    }
    fn marker_bank_click(app: &mut Leviathan, ctx: &egui::Context, label: &str, down: bool) {
        let p = marker_bank_control(app, ctx, label, down);
        pad_click(app, ctx, p);
    }
    fn marker_bank_text(
        app: &mut Leviathan,
        ctx: &egui::Context,
        label: &str,
        id: &str,
        value: &str,
    ) {
        marker_bank_control(app, ctx, label, false);
        let id = egui::Id::new(id);
        let r = ctx.read_response(id).unwrap().rect;
        assert!(
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900., 620.)).contains_rect(r)
        );
        pad_click(app, ctx, r.center());
        assert!(ctx.memory(|m| m.has_focus(id)));
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
            );
        }
        let out = render_output(app, ctx, vec![egui::Event::Text(value.into())]);
        assert!(
            out.shapes.iter().any(
                |s| matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.job.text==value)
            ),
            "Typed exact bank input was not painted"
        );
    }
    fn marker_bank_wait(app: &mut Leviathan, ctx: &egui::Context) {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while app.sample_editor.busy() {
            render(app, ctx, vec![]);
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..8 {
            render(app, ctx, vec![]);
        }
    }
    fn marker_bank_preview(app: &mut Leviathan, ctx: &egui::Context) {
        marker_bank_text(
            app,
            ctx,
            "Interval indices",
            "marker-bank-intervals",
            "0, 2, 7",
        );
        marker_bank_click(app, ctx, "Preview bank", true);
        marker_bank_wait(app, ctx);
        assert!(
            app.capture_marker_bank_state()["preview"].is_object(),
            "{}",
            app.capture_marker_bank_state()
        );
    }
    #[test]
    fn marker_bank_actual_native_preview_create_explicit_open_and_undo_are_separate() {
        let (mut app, ctx, _fixture) = marker_bank_app();
        let before = app.tracker.editor.song.clone();
        let cursor = keyzone_cursor(&app.tracker.editor);
        let revision = app.tracker.editor.revision();
        assert!(before.marker_sets[&crate::song::MarkerSetId(1)]
            .source_bpm
            .is_none());
        marker_bank_preview(&mut app, &ctx);
        let state = app.capture_marker_bank_state();
        let candidate: crate::song::Song =
            serde_json::from_value(state["preview"]["candidate"].clone()).unwrap();
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!(state["preview"]["summary"]["pads"][2]["interval_index"], 7);
        marker_bank_click(&mut app, &ctx, "Create bank", true);
        marker_bank_wait(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, candidate);
        assert_eq!(app.tracker.editor.revision(), revision + 1);
        assert!(app.sample_view);
        assert_eq!(app.audition.starts, 0);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        let after = app.tracker.editor.revision();
        marker_bank_click(&mut app, &ctx, "Create bank", true);
        assert_eq!(app.tracker.editor.revision(), after);
        marker_bank_click(&mut app, &ctx, "Open new kit", true);
        assert!(!app.sample_view);
        assert!(app.tracker.pads.open);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        assert_eq!(app.audition.starts, 0);
        assert_eq!(
            app.tracker.pads.audition_request().unwrap(),
            crate::audition_ui::Request::Pad {
                instrument: crate::song::InstrumentId(25),
                pad: crate::song::PadId(1)
            }
        );
        keyzone_click(&mut app, &ctx, "Undo");
        assert_eq!(app.tracker.editor.song, before);
        keyzone_click(&mut app, &ctx, "Redo");
        assert_eq!(app.tracker.editor.song, candidate);
    }
    #[test]
    fn marker_bank_actual_tamper_refuses_atomic_create_and_restored_source_can_retry() {
        let (mut app, ctx, fixture) = marker_bank_app();
        marker_bank_preview(&mut app, &ctx);
        let before = app.tracker.editor.song.clone();
        let revision = app.tracker.editor.revision();
        let path = fixture.0.join("tempo.wav");
        let original = std::fs::read(&path).unwrap();
        let mut altered = original.clone();
        *altered.last_mut().unwrap() ^= 1;
        std::fs::write(&path, altered).unwrap();
        marker_bank_click(&mut app, &ctx, "Create bank", true);
        marker_bank_wait(&mut app, &ctx);
        assert!(app.capture_marker_bank_state()["error"]
            .as_str()
            .unwrap()
            .contains("SHA-256"));
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.tracker.editor.revision(), revision);
        std::fs::write(path, original).unwrap();
        marker_bank_click(&mut app, &ctx, "Create bank", true);
        marker_bank_wait(&mut app, &ctx);
        assert_eq!(app.capture_marker_bank_state()["committed"], true);
        assert_eq!(app.tracker.editor.revision(), revision + 1);
    }
    #[test]
    fn marker_bank_actual_invalid_and_read_mode_preview_do_not_mutate_or_accept_markers() {
        let (mut app, ctx, _fixture) = marker_bank_app();
        let before = app.tracker.editor.song.clone();
        for value in [
            "0, 0",
            "7, 0",
            "0.1",
            "65536",
            "0, 8",
            "0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16",
        ] {
            marker_bank_text(
                &mut app,
                &ctx,
                "Interval indices",
                "marker-bank-intervals",
                value,
            );
            marker_bank_click(&mut app, &ctx, "Preview bank", true);
            marker_bank_wait(&mut app, &ctx);
            assert!(
                app.capture_marker_bank_state()["error"].is_string(),
                "{value}"
            );
            assert_eq!(app.tracker.editor.song, before);
        }
        app.tracker.editor.edit_mode = false;
        marker_bank_preview(&mut app, &ctx);
        marker_bank_click(&mut app, &ctx, "Create bank", true);
        marker_bank_wait(&mut app, &ctx);
        assert_eq!(app.tracker.editor.song, before);
        assert_eq!(app.capture_marker_bank_state()["committed"], false);
        marker_bank_click(&mut app, &ctx, "Cancel / close", true);
        assert_eq!(app.capture_marker_bank_state()["open"], false);
    }
    #[test]
    fn marker_bank_actual_queued_verified_completion_is_revoked_by_same_frame_actions() {
        for guard in 0..4 {
            let (mut app, ctx, _fixture) = marker_bank_app();
            marker_bank_preview(&mut app, &ctx);
            let before = app.tracker.editor.song.clone();
            let target = if guard == 0 {
                marker_bank_control(&mut app, &ctx, "Cancel / close", true)
            } else if guard == 1 {
                control(&render_output(&mut app, &ctx, vec![]), "New")
            } else {
                egui::Pos2::ZERO
            };
            let create = marker_bank_control(&mut app, &ctx, "Create bank", true);
            render(&mut app, &ctx, vec![egui::Event::PointerMoved(create)]);
            for pressed in [true, false] {
                render(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerButton {
                        pos: create,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            // Drain only the real worker result, deliberately before another App
            // frame can commit it. This pins the late-completion input boundary.
            let end = std::time::Instant::now() + Duration::from_secs(10);
            while app.sample_editor.busy() {
                app.sample_editor.poll();
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(app.capture_marker_bank_state()["verified_waiting"], true);
            assert_eq!(app.tracker.editor.song, before);
            let events = match guard {
                0 | 1 => vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                2 => vec![
                    egui::Event::WindowFocused(false),
                    egui::Event::WindowFocused(true),
                ],
                _ => {
                    app.session.pending = Some(crate::session::Action::Close);
                    vec![]
                }
            };
            render(&mut app, &ctx, events);
            assert_eq!(app.capture_marker_bank_state()["committed"], false);
            assert_eq!(app.capture_marker_bank_state()["authorized"], false);
            if guard != 1 {
                assert_eq!(app.tracker.editor.song, before);
            } else {
                assert!(app.tracker.editor.song.samples.is_empty());
                assert!(app.tracker.editor.song.drum_kits.is_empty());
            }
            for _ in 0..5 {
                render(&mut app, &ctx, vec![]);
            }
            assert_eq!(app.audition.starts, 0);
            assert_eq!(app.capture_marker_bank_state()["committed"], false);
        }
    }

    fn cells_app() -> (Leviathan, egui::Context) {
        use crate::song::*;
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        let e = &mut app.tracker.editor;
        let id = e.pattern_id();
        let rows = e
            .song
            .patterns
            .get_mut(&id)
            .unwrap()
            .tracks
            .entry(e.track)
            .or_default();
        rows.clear();
        rows.entry(0).or_default().insert(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                velocity: Some(0.3),
                delay: Some(-0.0),
                ..Default::default()
            },
        );
        rows.entry(0).or_default().insert(1, Cell::default());
        rows.entry(1).or_default().insert(
            1,
            Cell {
                note: Note::Off,
                ..Default::default()
            },
        );
        rows.entry(5).or_default().insert(
            0,
            Cell {
                note: Note::Note(66),
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        );
        for _ in 0..5 {
            render(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }
    fn cells_control(
        app: &mut Leviathan,
        ctx: &egui::Context,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        let viewport = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900.0, 620.0));
        let mut prior = None;
        let mut stable = 0;
        for _ in 0..180 {
            let out = render_output(app, ctx, vec![]);
            if let Some(pos) = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    let r = t.visual_bounding_rect();
                    (s.clip_rect.contains_rect(r) && viewport.contains_rect(r) && !t.galley.elided)
                        .then_some(r.center())
                }
                _ => None,
            }) {
                if prior.is_some_and(|p: egui::Pos2| p.distance(pos) < 0.1) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                prior = Some(pos);
                if stable >= 3 {
                    return pos;
                }
                continue;
            }
            let state = app.capture_cell_blocks_state();
            let body = &state["body"];
            let p = egui::pos2(
                (body["min"][0].as_f64().unwrap() + body["max"][0].as_f64().unwrap()) as f32 * 0.5,
                (body["min"][1].as_f64().unwrap() + body["max"][1].as_f64().unwrap()) as f32 * 0.5,
            );
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if down { -65.0 } else { 65.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        panic!(
            "Cell rectangle control not reachable: {label}; {}",
            app.capture_cell_blocks_state()
        );
    }
    fn cells_click(app: &mut Leviathan, ctx: &egui::Context, label: &str, down: bool) {
        let p = cells_control(app, ctx, label, down);
        pad_click(app, ctx, p);
    }
    fn cells_text(app: &mut Leviathan, ctx: &egui::Context, label: &str, value: &str) {
        cells_control(app, ctx, label, false);
        let id = egui::Id::new(("cell-rectangle", label));
        let r = ctx.read_response(id).unwrap().rect;
        assert!(
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900.0, 620.0)).contains_rect(r)
        );
        pad_click(app, ctx, r.center());
        assert!(ctx.memory(|m| m.has_focus(id)));
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
            );
        }
        let out = render_output(app, ctx, vec![egui::Event::Text(value.into())]);
        assert!(
            out.shapes
                .iter()
                .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()==value)),
            "Exact typed input not painted: {value}"
        );
    }
    fn cells_wait(app: &mut Leviathan, ctx: &egui::Context) {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while app.tracker.cell_blocks.busy() {
            render(app, ctx, vec![]);
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..6 {
            render(app, ctx, vec![]);
        }
    }
    fn cells_copy(app: &mut Leviathan, ctx: &egui::Context) {
        cells_click(app, ctx, "Cells…", false);
        cells_text(app, ctx, "Row end", "2");
        cells_text(app, ctx, "Column end", "2");
        cells_click(app, ctx, "Copy", true);
        cells_wait(app, ctx);
        assert_eq!(
            app.capture_cell_blocks_state()["clipboard"]["summary"]["positions"],
            4
        );
    }
    fn cells_command(app: &mut Leviathan, ctx: &egui::Context, k: egui::Key) {
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![egui::Event::Key {
                    key: k,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
            );
        }
    }
    #[test]
    fn cell_rectangle_actual_copy_paste_preview_commit_undo_redo_preserves_exact_cells_and_cursor()
    {
        let (mut app, ctx) = cells_app();
        let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        let revision = app.tracker.editor.revision();
        cells_copy(&mut app, &ctx);
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        cells_click(&mut app, &ctx, "Close", true);
        cells_click(&mut app, &ctx, "C4", false);
        assert!(ctx.memory(|m| m.has_focus(crate::tracker_ui::grid_id())));
        let out = render_output(&mut app, &ctx, vec![egui::Event::Copy]);
        assert!(
            !out.platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(_))),
            "Grid Copy does not publish the private block to the OS clipboard"
        );
        cells_wait(&mut app, &ctx);
        cells_click(&mut app, &ctx, "Close", true);
        cells_click(&mut app, &ctx, "C4", false);
        assert!(ctx.memory(|m| m.has_focus(crate::tracker_ui::grid_id())));
        for _ in 0..4 {
            render(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::ArrowDown,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.tracker.editor.row, 4);
        cells_command(&mut app, &ctx, egui::Key::V);
        cells_wait(&mut app, &ctx);
        let state = app.capture_cell_blocks_state();
        assert_eq!(state["current"], true);
        assert_eq!(state["preview"]["request"]["destination"]["row_start"], 4);
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before,
            "Ctrl/V previews, never commits"
        );
        let candidate = state["preview"]["candidate"].clone();
        let candidate_json = state["preview"]["candidate_json"]
            .as_str()
            .unwrap()
            .to_owned();
        let cursor = (
            app.tracker.editor.occurrence,
            app.tracker.editor.row,
            app.tracker.editor.column,
            app.tracker.editor.field,
        );
        cells_click(&mut app, &ctx, "Commit", true);
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            candidate_json
        );
        assert_eq!(
            app.tracker.editor.pattern().tracks[&app.tracker.editor.track][&4][&0]
                .delay
                .unwrap()
                .to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(
            serde_json::to_value(&app.tracker.editor.song).unwrap(),
            candidate
        );
        assert_eq!(
            (
                app.tracker.editor.occurrence,
                app.tracker.editor.row,
                app.tracker.editor.column,
                app.tracker.editor.field
            ),
            cursor
        );
        cells_click(&mut app, &ctx, "Close", true);
        cells_click(&mut app, &ctx, "Undo", false);
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        cells_click(&mut app, &ctx, "Redo", false);
        assert_eq!(
            serde_json::to_value(&app.tracker.editor.song).unwrap(),
            candidate
        );
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            candidate_json
        );
        assert_eq!(
            app.tracker.editor.pattern().tracks[&app.tracker.editor.track][&4][&0]
                .delay
                .unwrap()
                .to_bits(),
            (-0.0f64).to_bits()
        );
        assert!(app.capture_cell_blocks_state()["clipboard"].is_object());
        assert_eq!(app.audition.starts, 0);
    }
    #[test]
    fn cell_rectangle_actual_invalid_text_and_read_mode_do_not_mutate_or_replace_copy() {
        let (mut app, ctx) = cells_app();
        cells_copy(&mut app, &ctx);
        let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        let clipboard = app.capture_cell_blocks_state()["clipboard"].clone();
        for value in ["-1", "2.5", "65537", "0"] {
            cells_text(&mut app, &ctx, "Row end", value);
            cells_click(&mut app, &ctx, "Copy", true);
            cells_wait(&mut app, &ctx);
            let state = app.capture_cell_blocks_state();
            assert!(state["error"].is_string());
            assert_eq!(state["draft"]["row_end"], value);
            assert_eq!(state["clipboard"], clipboard);
            assert_eq!(
                serde_json::to_vec(&app.tracker.editor.song).unwrap(),
                before
            );
        }
        cells_text(&mut app, &ctx, "Row end", "2");
        app.tracker.editor.edit_mode = false;
        cells_click(&mut app, &ctx, "Preview Clear", true);
        cells_wait(&mut app, &ctx);
        assert_eq!(app.capture_cell_blocks_state()["current"], true);
        cells_click(&mut app, &ctx, "Commit", true);
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        cells_click(&mut app, &ctx, "Copy", true);
        cells_wait(&mut app, &ctx);
        assert!(app.capture_cell_blocks_state()["preview"].is_null());
    }
    #[test]
    fn cell_rectangle_actual_text_focus_and_identical_document_replacement_revoke_authority() {
        let (mut app, ctx) = cells_app();
        cells_copy(&mut app, &ctx);
        let clipboard = app.capture_cell_blocks_state()["clipboard"].clone();
        cells_text(&mut app, &ctx, "Row end", "2");
        cells_command(&mut app, &ctx, egui::Key::A);
        let out = render_output(&mut app, &ctx, vec![egui::Event::Copy]);
        assert!(
            out.platform_output
                .commands
                .iter()
                .any(|command| matches!(command,egui::OutputCommand::CopyText(text) if text=="2")),
            "TextEdit retains its native text clipboard, not the block clipboard"
        );
        render(&mut app, &ctx, vec![egui::Event::Paste("7".into())]);
        assert_eq!(app.capture_cell_blocks_state()["draft"]["row_end"], "7");
        assert_eq!(app.capture_cell_blocks_state()["clipboard"], clipboard);
        assert!(app.capture_cell_blocks_state()["preview"].is_null());
        cells_text(&mut app, &ctx, "Row end", "2");
        cells_click(&mut app, &ctx, "Preview Clear", true);
        cells_wait(&mut app, &ctx);
        let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
            ],
        );
        assert_eq!(app.capture_cell_blocks_state()["current"], false);
        cells_click(&mut app, &ctx, "Commit", true);
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        let same = app.tracker.editor.song.clone();
        app.tracker.editor.replace_document(same).unwrap();
        render(&mut app, &ctx, vec![]);
        assert!(app.capture_cell_blocks_state()["clipboard"].is_null());
        assert!(app.capture_cell_blocks_state()["preview"].is_null());
        assert_eq!(app.audition.starts, 0);
    }

    #[test]
    fn cell_rectangle_actual_ready_worker_is_revoked_before_same_frame_close_new_modal_and_focus_pair(
    ) {
        for action in 0..4 {
            let (mut app, ctx) = cells_app();
            cells_click(&mut app, &ctx, "Cells…", false);
            let close = cells_control(&mut app, &ctx, "Close", true);
            let new = cells_control(&mut app, &ctx, "New", false);
            let copy = cells_control(&mut app, &ctx, "Copy", true);
            let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
            pad_click(&mut app, &ctx, copy);
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while app.capture_cell_blocks_state()["completion_waiting"] != true {
                app.tracker.cell_blocks.poll();
                assert!(
                    std::time::Instant::now() < deadline,
                    "Actual worker did not complete"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(app.capture_cell_blocks_state()["clipboard"].is_null());
            let click = |p| {
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let events = match action {
                0 => click(close),
                1 => click(new),
                2 => {
                    app.tracker.editor.dirty = true;
                    app.session
                        .request(&mut app.tracker.editor, crate::session::Action::Close)
                        .unwrap();
                    vec![]
                }
                _ => vec![
                    egui::Event::WindowFocused(false),
                    egui::Event::WindowFocused(true),
                ],
            };
            render(&mut app, &ctx, events);
            for _ in 0..6 {
                render(&mut app, &ctx, vec![]);
            }
            assert!(
                app.capture_cell_blocks_state()["clipboard"].is_null(),
                "Late Copy installed for action {action}"
            );
            assert!(app.capture_cell_blocks_state()["preview"].is_null());
            assert_eq!(app.audition.starts, 0);
            if action != 1 {
                assert_eq!(
                    serde_json::to_vec(&app.tracker.editor.song).unwrap(),
                    before
                );
            }
        }
    }

    fn mix_control(app: &mut Leviathan, ctx: &egui::Context, label: &str) -> egui::Pos2 {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 620.0));
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..12 {
            let out = render_output(app, ctx, vec![]);
            if let Some(pos) = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    let r = t.visual_bounding_rect();
                    let mix_local =
                        matches!(label, "Preview" | "Apply" | "Close" | "Reset to neutral");
                    let footer_top = app.mix_editor.capture_state(&app.tracker.editor)["body"]
                        ["max"][1]
                        .as_f64()
                        .unwrap_or(0.0) as f32;
                    ((!mix_local || r.top() >= footer_top)
                        && s.clip_rect.contains_rect(r)
                        && viewport.contains_rect(r)
                        && !t.galley.elided)
                        .then_some(r.center())
                }
                _ => None,
            }) {
                if previous.is_some_and(|p: egui::Pos2| p.distance(pos) < 0.1) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                previous = Some(pos);
                if stable >= 3 {
                    return pos;
                }
            }
        }
        panic!(
            "Mix control not fully visible: {label}; {}",
            app.mix_editor.capture_state(&app.tracker.editor)
        );
    }
    fn mix_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let p = mix_control(app, ctx, label);
        pad_click(app, ctx, p);
    }
    fn mix_type(app: &mut Leviathan, ctx: &egui::Context, track: u64, label: &str, value: &str) {
        let state = app.mix_editor.capture_state(&app.tracker.editor);
        let body = &state["body"];
        if let (Some(x), Some(y)) = (body["min"][0].as_f64(), body["min"][1].as_f64()) {
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(x as f32 + 40.0, y as f32 + 20.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 2000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for _ in 0..8 {
            render(app, ctx, vec![]);
        }
        let id = egui::Id::new(("track-mix-value", track, label));
        let rect = ctx.read_response(id).unwrap().rect;
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 620.0))
                .contains_rect(rect)
        );
        pad_click(app, ctx, rect.center());
        assert!(ctx.memory(|m| m.has_focus(id)));
        for pressed in [true, false] {
            render(
                app,
                ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
            );
        }
        let out = render_output(app, ctx, vec![egui::Event::Text(value.into())]);
        assert!(
            out.shapes
                .iter()
                .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()==value)),
            "Typed mix text not observed: {value}"
        );
    }
    fn mix_wait(app: &mut Leviathan, ctx: &egui::Context) {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while app.mix_editor.busy() {
            render(app, ctx, vec![]);
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..6 {
            render(app, ctx, vec![]);
        }
    }
    #[test]
    fn mix_actual_draft_slider_exact_text_preview_apply_and_toolbar_history() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.tracker.editor.edit_mode = true;
        let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        let cursor = keyzone_cursor(&app.tracker.editor);
        mix_click(&mut app, &ctx, "Mix…");
        let slider = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("track-mix-slider", 1u64, "Gain"))))
            .unwrap();
        pad_click(
            &mut app,
            &ctx,
            egui::pos2(slider.left() + slider.width() * 0.35, slider.center().y),
        );
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        assert_eq!(app.audition.starts, 0);
        mix_type(&mut app, &ctx, 1, "Gain", "0.5");
        mix_type(&mut app, &ctx, 1, "Stereo balance", "-0");
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        let state = app.mix_editor.capture_state(&app.tracker.editor);
        assert_eq!(state["current"], true);
        let expected = state["preview"]["candidate_json"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        mix_click(&mut app, &ctx, "Apply");
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            expected
        );
        assert_eq!(
            app.tracker.editor.song.track_mix[&crate::song::TrackId(1)]
                .balance
                .to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        assert!(app.mix_delivery.message.contains("NOT LIVE-APPLIED"));
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        mix_click(&mut app, &ctx, "Close");
        mix_click(&mut app, &ctx, "Undo");
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        mix_click(&mut app, &ctx, "Redo");
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            expected
        );
        assert_eq!(app.audition.starts, 0);
    }
    #[test]
    fn mix_actual_invalid_read_mode_and_held_completion_focus_pair_do_not_commit() {
        let ctx = egui::Context::default();
        let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let before = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        mix_click(&mut app, &ctx, "Mix…");
        mix_type(&mut app, &ctx, 1, "Gain", "1.0000000000000002");
        mix_click(&mut app, &ctx, "Preview");
        assert!(app.mix_editor.capture_state(&app.tracker.editor)["error"].is_string());
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        mix_type(&mut app, &ctx, 1, "Gain", "0.25");
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        assert_eq!(
            app.mix_editor.capture_state(&app.tracker.editor)["current"],
            true
        );
        mix_click(&mut app, &ctx, "Apply");
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        app.tracker.editor.edit_mode = true;
        mix_type(&mut app, &ctx, 1, "Gain", "0.75");
        let p = mix_control(&mut app, &ctx, "Preview");
        for pressed in [true, false] {
            render(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while app.mix_editor.capture_state(&app.tracker.editor)["completion_waiting"] != true {
            app.mix_editor.poll();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
            ],
        );
        for _ in 0..8 {
            render(&mut app, &ctx, vec![]);
        }
        assert_eq!(
            app.mix_editor.capture_state(&app.tracker.editor)["current"],
            false
        );
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            before
        );
        assert_eq!(app.mix_delivery.accepted_revision, 0);
    }
    fn mix_audio_app(held: bool) -> (Leviathan, egui::Context, PadFixture) {
        use crate::song::*;
        let (mut app, ctx, fixture) = pad_app();
        let mut song = app.tracker.editor.song.clone();
        song.patterns.get_mut(&PatternId(1)).unwrap().rows = 16;
        song.regions.get_mut(&RegionId(1)).unwrap().forward_loop = Some(ForwardLoop {
            start_frame: 0,
            end_frame: 4800,
        });
        song.assign_region(InstrumentId(1), Some(RegionId(1)))
            .unwrap();
        song.patterns.get_mut(&PatternId(1)).unwrap().tracks.insert(
            TrackId(1),
            std::collections::BTreeMap::from([(
                0,
                std::collections::BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        velocity: Some(0.4),
                        ..Default::default()
                    },
                )]),
            )]),
        );
        app.tracker.editor.replace_document(song).unwrap();
        app.tracker.editor.edit_mode = true;
        app.tracker.set_pad_view(false);
        app.diagnostic_audio.enabled = true;
        app.diagnostic_audio.held = held;
        app.diagnostic_audio.auto_advance = false;
        render(&mut app, &ctx, vec![]);
        (app, ctx, fixture)
    }
    #[test]
    fn mix_actual_song_request_enqueued_pcm_and_mix_only_history_are_not_heard_claims() {
        let (mut app, ctx, _fixture) = mix_audio_app(false);
        let before = app.tracker.editor.song.clone();
        mix_click(&mut app, &ctx, "▶ Play");
        audition_wait(&mut app, &ctx, false);
        assert!(app.diagnostic_audio.prepared);
        assert!(app.diagnostic_audio.pcm.is_empty());
        mix_click(&mut app, &ctx, "Mix…");
        mix_type(&mut app, &ctx, 1, "Gain", "0.25");
        mix_type(&mut app, &ctx, 1, "Stereo balance", "0.5");
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        mix_click(&mut app, &ctx, "Apply");
        assert_eq!(
            app.mix_delivery.accepted_revision,
            1,
            "{}",
            app.capture_track_mix_state()
        );
        assert_eq!(app.mix_delivery.enqueued_revision, 0);
        assert!(app.diagnostic_audio.pcm.is_empty());
        assert_eq!(app.audition.starts, 0);
        let mut reference = crate::song_render::Engine::new(&before, None).unwrap();
        let target = reference
            .mix_layout()
            .unwrap()
            .snapshot(&app.tracker.editor.song.track_mix)
            .unwrap();
        reference.apply_mix_snapshot(&target).unwrap();
        let mut expected = [0.0f32; 1024];
        assert_eq!(
            reference
                .block(&mut expected, crate::audition::TOTAL_FRAMES)
                .unwrap(),
            512
        );
        app.diagnostic_audio.advance();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while app.diagnostic_audio.pcm.len() < 1024 {
            render(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(&app.diagnostic_audio.pcm[..1024], &expected);
        assert_eq!(app.mix_delivery.enqueued_revision, 1);
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
            ],
        );
        assert_eq!(
            app.mix_delivery.accepted_revision, 1,
            "Focus cancels draft authority, not accepted Song mix"
        );
        assert!(app.transport_shared().unwrap().is_running());
        mix_click(&mut app, &ctx, "Close");
        assert_eq!(
            app.mix_delivery.accepted_revision, 1,
            "Closing Mix does not reset the accepted target"
        );
        mix_click(&mut app, &ctx, "Undo");
        assert_eq!(app.mix_delivery.accepted_revision, 2);
        mix_click(&mut app, &ctx, "Redo");
        assert_eq!(app.mix_delivery.accepted_revision, 3);
        app.tracker.editor.mark_saved();
        render(&mut app, &ctx, vec![]);
        assert_eq!(
            app.mix_delivery.accepted_revision, 3,
            "Save is not a live submission"
        );
        let same = app.tracker.editor.song.clone();
        app.tracker.editor.replace_document(same).unwrap();
        render(&mut app, &ctx, vec![]);
        assert!(!app.mix_delivery.available);
        assert!(app.mix_delivery.message.contains("Document replaced"));
        assert!(app
            .transport_shared()
            .unwrap()
            .submit_live_mix(
                &app.tracker.editor.document_owner(),
                &app.tracker.editor.song
            )
            .is_err());
        app.stop_transport();
        audition_wait(&mut app, &ctx, true);
    }
    #[test]
    fn mix_actual_loading_noop_retry_stop_and_audition_refusals_keep_document_commit() {
        let (mut app, ctx, _fixture) = mix_audio_app(true);
        mix_click(&mut app, &ctx, "▶ Play");
        mix_click(&mut app, &ctx, "Mix…");
        mix_type(&mut app, &ctx, 1, "Gain", "0.5");
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        mix_click(&mut app, &ctx, "Apply");
        let committed = serde_json::to_vec(&app.tracker.editor.song).unwrap();
        let revision = app.tracker.editor.revision();
        assert!(
            app.mix_delivery.message.contains("still loading"),
            "{}",
            app.capture_track_mix_state()
        );
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        assert!(app.diagnostic_audio.pcm.is_empty());
        app.diagnostic_audio.release();
        audition_wait(&mut app, &ctx, false);
        assert_eq!(
            app.mix_delivery.accepted_revision, 0,
            "Readiness must not retry automatically"
        );
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        mix_click(&mut app, &ctx, "Apply");
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!(
            serde_json::to_vec(&app.tracker.editor.song).unwrap(),
            committed
        );
        assert_eq!(app.mix_delivery.accepted_revision, 1);
        mix_type(&mut app, &ctx, 1, "Gain", "0.75");
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        mix_click(&mut app, &ctx, "■ Stop");
        mix_click(&mut app, &ctx, "Apply");
        assert_eq!(
            app.tracker.editor.song.track_mix[&crate::song::TrackId(1)].gain,
            0.75
        );
        assert!(app.mix_delivery.message.contains("NOT LIVE-APPLIED"));
        assert_eq!(app.mix_delivery.accepted_revision, 1);
        audition_wait(&mut app, &ctx, true);

        let (mut app, ctx, _fixture) = audition_app(false, true);
        audition_click(&mut app, &ctx, "Audition selected");
        assert!(app.transport_busy());
        mix_click(&mut app, &ctx, "Mix…");
        mix_type(&mut app, &ctx, 1, "Gain", "0.5");
        assert!(
            !app.audition.canceled,
            "Opening/drafting Mix is not a new sound action"
        );
        assert_eq!(app.audition.starts, 1);
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        mix_click(&mut app, &ctx, "Apply");
        assert!(app
            .mix_delivery
            .message
            .contains("Audition does not accept"));
        assert!(
            app.audition.canceled,
            "Existing revision guard stops the old audition"
        );
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        assert_eq!(app.audition.starts, 1);
        audition_wait(&mut app, &ctx, true);
        assert!(app.diagnostic_audio.pcm.is_empty());
    }
    #[test]
    fn mix_actual_queued_preview_close_new_modal_and_focus_pair_never_authorize_apply() {
        for guard in 0..4 {
            let ctx = egui::Context::default();
            let mut app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
            app.tracker.editor.edit_mode = true;
            mix_click(&mut app, &ctx, "Mix…");
            mix_type(&mut app, &ctx, 1, "Gain", "0.5");
            let preview = mix_control(&mut app, &ctx, "Preview");
            let close = mix_control(&mut app, &ctx, "Close");
            let new = mix_control(&mut app, &ctx, "New");
            let event_click = |p: egui::Pos2| {
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            render(&mut app, &ctx, event_click(preview));
            let until = std::time::Instant::now() + Duration::from_secs(10);
            while app.mix_editor.capture_state(&app.tracker.editor)["completion_waiting"] != true {
                app.mix_editor.poll();
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            let events = match guard {
                0 => event_click(close),
                1 => event_click(new),
                2 => {
                    app.tracker.editor.dirty = true;
                    app.session.pending = Some(crate::session::Action::Close);
                    vec![]
                }
                _ => vec![
                    egui::Event::WindowFocused(false),
                    egui::Event::WindowFocused(true),
                ],
            };
            render(&mut app, &ctx, events);
            for _ in 0..6 {
                render(&mut app, &ctx, vec![]);
            }
            assert_eq!(
                app.mix_editor.capture_state(&app.tracker.editor)["current"],
                false,
                "guard{guard}: {}",
                app.capture_track_mix_state()
            );
            assert!(app.tracker.editor.song.track_mix.is_empty());
            assert_eq!(app.mix_delivery.accepted_revision, 0);
            assert_eq!(app.audition.starts, 0);
        }
    }

    fn meter_block(app: &mut Leviathan, ctx: &egui::Context) {
        let before = app.diagnostic_audio.pcm.len();
        app.diagnostic_audio.advance();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.diagnostic_audio.pcm.len() == before {
            render(app, ctx, vec![]);
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                app.capture_audition_state()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        render(app, ctx, vec![]);
    }
    fn meter_wait_report(app: &mut Leviathan, ctx: &egui::Context) -> serde_json::Value {
        for _ in 0..12 {
            let state = app.capture_track_mix_state()["ui"]["meters"].clone();
            if state["report"].is_object() {
                return state;
            }
            meter_block(app, ctx);
        }
        panic!(
            "No complete actual worker meter report: {}",
            app.capture_track_mix_state()
        );
    }
    #[test]
    fn meter_actual_report_matches_pcm_and_preserves_focused_numeric_draft_and_history() {
        let (mut app, ctx, _fixture) = mix_audio_app(false);
        let before = serde_json::to_string(&app.tracker.editor.song).unwrap();
        let revision = app.tracker.editor.revision();
        let cursor = keyzone_cursor(&app.tracker.editor);
        let history = (
            app.tracker.editor.can_undo(),
            app.tracker.editor.can_redo(),
            app.tracker.editor.dirty,
        );
        mix_click(&mut app, &ctx, "▶ Play");
        audition_wait(&mut app, &ctx, false);
        mix_click(&mut app, &ctx, "Mix…");
        assert!(app.capture_track_mix_state()["ui"]["meters"]["report"].is_null());
        mix_type(&mut app, &ctx, 1, "Gain", "0.123456789");
        let field = egui::Id::new(("track-mix-value", 1u64, "Gain"));
        let state = meter_wait_report(&mut app, &ctx);
        assert!(
            ctx.memory(|m| m.has_focus(field)),
            "Report arrival stole numeric focus"
        );
        assert_eq!(
            app.capture_track_mix_state()["ui"]["draft"]["rows"][0]["gain_text"],
            "0.123456789"
        );
        let report = &state["report"];
        assert_eq!(report["emitted_start"], 0);
        assert_eq!(report["emitted_end_exclusive"], 2400);
        assert_eq!(report["partial"], false);
        let mut engine = crate::song_render::Engine::new(&app.tracker.editor.song, None).unwrap();
        let mut expected = Vec::new();
        let mut block = [0.0f32; 1024];
        while expected.len() < 4800.max(app.diagnostic_audio.pcm.len()) {
            let n = engine
                .block(&mut block, crate::audition::TOTAL_FRAMES)
                .unwrap();
            assert!(n > 0);
            expected.extend_from_slice(&block[..n * 2]);
        }
        assert_eq!(
            app.diagnostic_audio
                .pcm
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            expected[..app.diagnostic_audio.pcm.len()]
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>()
        );
        let mut peak = [0.0f32; 2];
        for frame in expected[..4800].as_chunks::<2>().0 {
            for (peak, value) in peak.iter_mut().zip(frame) {
                *peak = peak.max(value.abs());
            }
        }
        for (channel, peak) in peak.into_iter().enumerate() {
            assert_eq!(
                (report["master_peak"][channel].as_f64().unwrap() as f32).to_bits(),
                peak.to_bits()
            );
            assert_eq!(
                (report["track_peaks"][0][channel].as_f64().unwrap() as f32).to_bits(),
                peak.to_bits()
            );
        }
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            before
        );
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!(keyzone_cursor(&app.tracker.editor), cursor);
        assert_eq!(
            (
                app.tracker.editor.can_undo(),
                app.tracker.editor.can_redo(),
                app.tracker.editor.dirty
            ),
            history
        );
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        assert_eq!(app.audition.starts, 0);
    }
    #[test]
    fn meter_actual_zero_and_shared_legacy_are_distinct_from_missing_windows() {
        use crate::song::*;
        for legacy in [false, true] {
            let (mut app, ctx, _fixture) = mix_audio_app(false);
            let mut song = app.tracker.editor.song.clone();
            if legacy {
                let path = song.samples[&SampleId(1)].path.clone();
                let instrument = song.instruments.get_mut(&InstrumentId(1)).unwrap();
                instrument.region = None;
                instrument.sample = Some(path);
                instrument.kind = InstrumentKind::Pitched;
                let cells = song.patterns[&PatternId(1)].tracks[&TrackId(1)].clone();
                song.patterns
                    .get_mut(&PatternId(1))
                    .unwrap()
                    .tracks
                    .insert(TrackId(2), cells);
            } else {
                song.patterns
                    .get_mut(&PatternId(1))
                    .unwrap()
                    .tracks
                    .get_mut(&TrackId(1))
                    .unwrap()
                    .get_mut(&0)
                    .unwrap()
                    .get_mut(&0)
                    .unwrap()
                    .velocity = Some(0.0);
            }
            app.tracker.editor.replace_document(song).unwrap();
            mix_click(&mut app, &ctx, "▶ Play");
            audition_wait(&mut app, &ctx, false);
            mix_click(&mut app, &ctx, "Mix…");
            let state = meter_wait_report(&mut app, &ctx);
            assert!(state["report"].is_object());
            if legacy {
                assert_eq!(state["layout"]["tracks"].as_array().unwrap().len(), 2);
                for slot in 0..2 {
                    assert_eq!(
                        state["layout"]["tracks"][slot]["availability"]["kind"],
                        "shared_legacy"
                    );
                    assert!(state["report"]["track_peaks"][slot].is_null());
                }
                assert!(state["report"]["master_peak"][0].as_f64().unwrap() > 0.0);
            } else {
                assert_eq!(
                    state["layout"]["tracks"][0]["availability"]["kind"],
                    "available"
                );
                assert_eq!(
                    state["report"]["track_peaks"][0],
                    serde_json::json!([0.0, 0.0])
                );
                assert_eq!(
                    state["report"]["master_peak"],
                    serde_json::json!([0.0, 0.0])
                );
            }
        }
    }
    #[test]
    fn meter_actual_generation_stop_equal_document_replacement_and_audition_clear() {
        let (mut app, ctx, _fixture) = mix_audio_app(false);
        mix_click(&mut app, &ctx, "▶ Play");
        audition_wait(&mut app, &ctx, false);
        mix_click(&mut app, &ctx, "Mix…");
        let first = meter_wait_report(&mut app, &ctx);
        let generation = first["generation"].as_u64().unwrap();
        app.transport_shared().unwrap().seek(12000);
        render(&mut app, &ctx, vec![]);
        let state = app.capture_track_mix_state()["ui"]["meters"].clone();
        assert!(state["report"].is_null());
        assert!(state["generation"].as_u64().unwrap() > generation);
        let next = meter_wait_report(&mut app, &ctx);
        assert_eq!(next["report"]["first_song_frame"], 12000);
        let same = app.tracker.editor.song.clone();
        app.tracker.editor.replace_document(same).unwrap();
        render(&mut app, &ctx, vec![]);
        assert!(app.capture_track_mix_state()["ui"]["meters"]["report"].is_null());
        assert!(!app.capture_track_mix_state()["owner_current"]
            .as_bool()
            .unwrap());
        app.stop_transport();
        render(&mut app, &ctx, vec![]);
        assert!(app.capture_track_mix_state()["ui"]["meters"]["report"].is_null());
        let (mut audition, context, _other) = audition_app(false, true);
        audition_click(&mut audition, &context, "Audition selected");
        mix_click(&mut audition, &context, "Mix…");
        let meters = audition.capture_track_mix_state()["ui"]["meters"].clone();
        assert!(meters["report"].is_null());
        assert!(meters["reason"].as_str().unwrap().contains("Audition"));
    }
    #[test]
    fn meter_first_report_arriving_during_held_close_preserves_stable_actions() {
        let (mut app, ctx, _fixture) = mix_audio_app(false);
        mix_click(&mut app, &ctx, "▶ Play");
        audition_wait(&mut app, &ctx, false);
        mix_click(&mut app, &ctx, "Mix…");
        mix_type(&mut app, &ctx, 1, "Gain", "0.5");
        mix_click(&mut app, &ctx, "Preview");
        mix_wait(&mut app, &ctx);
        let before = serde_json::to_string(&app.tracker.editor.song).unwrap();
        let revision = app.tracker.editor.revision();
        let close = mix_control(&mut app, &ctx, "Close");
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(close),
                egui::Event::PointerButton {
                    pos: close,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let report = meter_wait_report(&mut app, &ctx);
        assert!(report["report"].is_object());
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(close),
                egui::Event::PointerButton {
                    pos: close,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(!app.mix_editor.is_open());
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            before
        );
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        assert!(app.transport_shared().unwrap().is_running());
    }

    fn meter_detail_click(app: &mut Leviathan, ctx: &egui::Context, label: &str) {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 620.0));
        let mut direction = true;
        let mut edges = 0;
        let mut stalled = 0;
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..180 {
            let out = render_output(app, ctx, vec![]);
            let state = app.capture_track_mix_state();
            let b = &state["ui"]["body"];
            let body = egui::Rect::from_min_max(
                egui::pos2(
                    b["min"][0].as_f64().unwrap() as f32,
                    b["min"][1].as_f64().unwrap() as f32,
                ),
                egui::pos2(
                    b["max"][0].as_f64().unwrap() as f32,
                    b["max"][1].as_f64().unwrap() as f32,
                ),
            );
            let visible = out
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        let r = t.visual_bounding_rect();
                        (shape.clip_rect.contains_rect(r)
                            && viewport.contains_rect(r)
                            && !t.galley.elided)
                            .then_some(r.center())
                    }
                    _ => None,
                });
            if let Some(p) = visible {
                if previous.is_some_and(|old: egui::Pos2| old.distance(p) < 0.1) {
                    stable += 1
                } else {
                    stable = 0;
                }
                previous = Some(p);
                if stable >= 3 {
                    pad_click(app, ctx, p);
                    return;
                }
                continue;
            }
            if let Some(y) = out.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.visual_bounding_rect().center().y)
                }
                _ => None,
            }) {
                direction = y > body.center().y;
            }
            let old = b["offset"].as_f64().unwrap();
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(body.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if direction { -140.0 } else { 140.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            for _ in 0..5 {
                render(app, ctx, vec![]);
            }
            let new = app.capture_track_mix_state()["ui"]["body"]["offset"]
                .as_f64()
                .unwrap();
            if (new - old).abs() < 0.01 {
                stalled += 1
            } else {
                stalled = 0;
            }
            if stalled >= 2 {
                direction = !direction;
                edges += 1;
                stalled = 0;
                if edges > 2 {
                    break;
                }
            }
        }
        panic!(
            "Meter details control unreachable: {label}; {}",
            app.capture_track_mix_state()
        );
    }
    fn meter_raw_focus(app: &mut Leviathan, ctx: &egui::Context) {
        let id = egui::Id::new("track-mix-meter-raw");
        for _ in 0..80 {
            render(app, ctx, vec![]);
            let state = app.capture_track_mix_state();
            let b = &state["ui"]["body"];
            let body = egui::Rect::from_min_max(
                egui::pos2(
                    b["min"][0].as_f64().unwrap() as f32,
                    b["min"][1].as_f64().unwrap() as f32,
                ),
                egui::pos2(
                    b["max"][0].as_f64().unwrap() as f32,
                    b["max"][1].as_f64().unwrap() as f32,
                ),
            );
            if let Some(r) = ctx.read_response(id).map(|r| r.rect) {
                let visible = r.intersect(body);
                if visible.height() > 16.0 {
                    pad_click(app, ctx, visible.left_top() + egui::vec2(12.0, 8.0));
                    assert!(ctx.memory(|m| m.has_focus(id)));
                    return;
                }
            }
            render(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(body.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -100.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        panic!("Raw meter TextEdit not reachable");
    }
    #[test]
    fn meter_actual_snapshot_copy_readonly_home_end_and_follow_survive_new_reports() {
        let (mut app, ctx, _fixture) = mix_audio_app(false);
        mix_click(&mut app, &ctx, "▶ Play");
        audition_wait(&mut app, &ctx, false);
        mix_click(&mut app, &ctx, "Mix…");
        meter_wait_report(&mut app, &ctx);
        meter_detail_click(&mut app, &ctx, "Exact settings, identity and full caveats");
        meter_detail_click(&mut app, &ctx, "Raw meter window and attribution");
        let before = serde_json::to_string(&app.tracker.editor.song).unwrap();
        let revision = app.tracker.editor.revision();
        let pcm_before = app.diagnostic_audio.pcm.clone();
        meter_detail_click(&mut app, &ctx, "Snapshot latest report");
        assert_eq!(
            app.diagnostic_audio.pcm, pcm_before,
            "Inspection must not advance audio"
        );
        let snapshot = app.capture_track_mix_state()["ui"]["meters"]["inspection_snapshot"].clone();
        let exact = snapshot["text"].as_str().unwrap().to_string();
        for _ in 0..6 {
            meter_block(&mut app, &ctx);
        }
        let current = app.capture_track_mix_state()["ui"]["meters"].clone();
        assert_eq!(current["inspection_snapshot"], snapshot);
        assert_ne!(current["raw_text"], exact);
        meter_raw_focus(&mut app, &ctx);
        for key in [egui::Key::Home, egui::Key::End] {
            for pressed in [true, false] {
                render(
                    &mut app,
                    &ctx,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                    }],
                );
            }
            let mut visible = false;
            let mut measured = Vec::new();
            for _ in 0..60 {
                let out = render_output(&mut app, &ctx, vec![]);
                measured.clear();
                for shape in &out.shapes {
                    if let egui::Shape::Text(t) = &shape.shape {
                        if t.galley.text() == exact {
                            let row = if key == egui::Key::Home {
                                t.galley.rows.first()
                            } else {
                                t.galley.rows.last()
                            }
                            .unwrap();
                            let rect = row.rect().translate(t.pos.to_vec2());
                            measured.push((rect, shape.clip_rect));
                            visible |= shape.clip_rect.contains_rect(rect)
                                && egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(900.0, 620.0),
                                )
                                .contains_rect(rect);
                        }
                    }
                }
                if visible {
                    break;
                }
            }
            assert!(visible,"Actual {key:?} must reveal exact raw first/final glyph row; measured {measured:?}; focus {:?}; caret {:?}; body {}",ctx.memory(|m|m.focused()),egui::text_edit::TextEditState::load(&ctx,egui::Id::new("track-mix-meter-raw")).map(|s|s.cursor.char_range()),app.capture_track_mix_state()["ui"]["body"]);
        }
        for pressed in [true, false] {
            render(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
            );
        }
        let copied = render_output(&mut app, &ctx, vec![egui::Event::Copy]);
        assert!(copied
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command,egui::OutputCommand::CopyText(text) if text==&exact)));
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Text("must not mutate raw report".into())],
        );
        assert_eq!(
            app.capture_track_mix_state()["ui"]["meters"]["inspection_snapshot"],
            snapshot
        );
        let pcm_before_follow = app.diagnostic_audio.pcm.clone();
        meter_detail_click(&mut app, &ctx, "Follow latest");
        assert!(app.capture_track_mix_state()["ui"]["meters"]["inspection_snapshot"].is_null());
        assert_eq!(app.diagnostic_audio.pcm, pcm_before_follow);
        assert_eq!(
            serde_json::to_string(&app.tracker.editor.song).unwrap(),
            before
        );
        assert_eq!(app.tracker.editor.revision(), revision);
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        assert!(app.transport_shared().unwrap().is_running());
    }

    #[test]
    fn meter_actual_track_peaks_above_unity_preserve_raw_values_under_master_cancellation() {
        use crate::song::*;
        let (mut app, ctx, fixture) = mix_audio_app(false);
        let mut song = app.tracker.editor.song.clone();
        let mut reader = hound::WavReader::open(&song.samples[&SampleId(1)].path).unwrap();
        let path = fixture.0.join("opposite.wav");
        let mut writer = hound::WavWriter::create(&path, reader.spec()).unwrap();
        for sample in reader.samples::<i16>() {
            writer.write_sample(-sample.unwrap()).unwrap();
        }
        writer.finalize().unwrap();
        song.register_sample(
            SampleId(2),
            crate::sample_asset::import(&path).unwrap().asset,
        )
        .unwrap();
        let mut region = song.regions[&RegionId(1)].clone();
        region.sample = SampleId(2);
        region.name = "Opposite polarity".into();
        song.upsert_region(RegionId(2), region).unwrap();
        let mut second = song.instruments[&InstrumentId(1)].clone();
        second.name = "Opposite track".into();
        second.region = Some(RegionId(2));
        song.instruments.insert(InstrumentId(2), second);
        for track in [TrackId(1), TrackId(2)] {
            song.tracks.get_mut(&track).unwrap().note_columns = 64;
            let columns = (0..64)
                .map(|column| {
                    (
                        column,
                        Cell {
                            note: Note::Note(60),
                            instrument: Some(InstrumentId(track.0)),
                            velocity: Some(1.0),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            song.patterns
                .get_mut(&PatternId(1))
                .unwrap()
                .tracks
                .insert(track, std::collections::BTreeMap::from([(0, columns)]));
        }
        song.validate().unwrap();
        app.tracker.editor.replace_document(song).unwrap();
        mix_click(&mut app, &ctx, "▶ Play");
        audition_wait(&mut app, &ctx, false);
        mix_click(&mut app, &ctx, "Mix…");
        let state = meter_wait_report(&mut app, &ctx);
        let report = &state["report"];
        for slot in 0..2 {
            for channel in 0..2 {
                assert!(
                    report["track_peaks"][slot][channel].as_f64().unwrap() > 1.0,
                    "{}",
                    report
                );
            }
        }
        for channel in 0..2 {
            assert!(
                report["master_peak"][channel].as_f64().unwrap() < 0.001,
                "{}",
                report
            );
        }
        let raw: serde_json::Value =
            serde_json::from_str(state["raw_text"].as_str().unwrap()).unwrap();
        assert_eq!(raw["latest_report"], *report);
        for slot in 0..2 {
            assert!(
                state["track_display_db"][format!("{}", slot + 1)][0]
                    .as_f64()
                    .unwrap()
                    > 0.0
            );
        }
        assert_eq!(app.mix_delivery.accepted_revision, 0);
        assert_eq!(app.audition.starts, 0);
    }
}
