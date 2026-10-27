//! Explicit read-only audition controls and request binding. The production driver
//! remains the existing host Transport; the opt-in capture driver uses its exact
//! Producer/Shared path without a device, and never represents physical output.
use crate::{
    audition::{self, AuditionPreview, AuditionSummary},
    editor::Editor,
    song::{
        InstrumentId, InstrumentKind, PadId, RegionId, SampleAsset, SampleId, SampleRegion, Song,
    },
};
use eframe::egui::{self, RichText};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Request {
    Region {
        asset: SampleAsset,
        draft: SampleRegion,
        source: Option<SampleId>,
        source_choice: Option<SampleId>,
        selected: Option<RegionId>,
        epoch: u64,
    },
    Pad {
        instrument: InstrumentId,
        pad: PadId,
    },
}
impl Request {
    pub(crate) fn prepare(&self, song: &Song) -> Result<AuditionPreview, String> {
        match self {
            Self::Region { asset, draft, .. } => audition::region_preview(asset, draft),
            Self::Pad { instrument, pad } => audition::pad_preview(song, *instrument, *pad),
        }
    }
    // Exact float/string binding only for bounded audition input, not a new
    // selector/evaluator. Document revision and explicit owner epoch revoke resets.
    fn binding(&self, song: &Song) -> Result<Vec<u8>, String> {
        let value = match self {
            Self::Region { asset, draft, .. } => {
                if asset.path.as_os_str().len() > audition::MAX_PATH_BYTES
                    || asset.sha256.len() > 64
                    || draft.name.len() > 256
                {
                    return Err("Audition input exceeds bounded metadata".into());
                }
                serde_json::to_vec(self)
            }
            Self::Pad { instrument, pad } => {
                let instrument = song
                    .instruments
                    .get(instrument)
                    .ok_or("Selected audition instrument was removed")?;
                let InstrumentKind::DrumKit { kit } = instrument.kind else {
                    return Err("Selected instrument is no longer a drum kit".into());
                };
                let kit = song
                    .drum_kits
                    .get(&kit)
                    .ok_or("Selected audition kit was removed")?;
                let pad = kit
                    .pads
                    .get(pad)
                    .ok_or("Selected audition pad was removed")?;
                if pad.velocity_zones.len() > 8
                    || pad.velocity_zones.iter().any(|z| z.alternatives.len() > 8)
                {
                    return Err("Audition mapping exceeds its bounded source list".into());
                }
                if instrument.name.len() > 256
                    || kit.name.len() > 256
                    || pad.name.len() > 128
                    || instrument
                        .sample
                        .as_ref()
                        .is_some_and(|p| p.as_os_str().len() > audition::MAX_PATH_BYTES)
                {
                    return Err("Audition input names exceed bounded metadata".into());
                }
                let ids = pad.source_regions().collect::<BTreeSet<_>>();
                if ids.len() > 32 {
                    return Err("Audition mapping exceeds 32 sources".into());
                }
                let regions = ids
                    .iter()
                    .map(|id| (*id, song.regions.get(id)))
                    .collect::<Vec<_>>();
                let samples = regions
                    .iter()
                    .filter_map(|(_, r)| r.map(|r| (r.sample, song.samples.get(&r.sample))))
                    .collect::<Vec<_>>();
                if regions
                    .iter()
                    .any(|(_, r)| r.is_some_and(|r| r.name.len() > 256))
                    || samples.iter().any(|(_, a)| {
                        a.is_some_and(|a| {
                            a.path.as_os_str().len() > audition::MAX_PATH_BYTES
                                || a.sha256.len() > 64
                        })
                    })
                {
                    return Err("Audition source metadata exceeds bounded input".into());
                }
                serde_json::to_vec(&(self, instrument, &kit.name, pad, regions, samples))
            }
        }
        .map_err(|e| e.to_string())?;
        if value.len() > audition::MAX_METADATA_BYTES {
            return Err("Audition input binding exceeds 256 KiB".into());
        }
        Ok(value)
    }
}
pub(crate) enum Action {
    Start(Request),
    Invalid(String),
    Stop,
}
struct Binding {
    revision: u64,
    epoch: u64,
    request: Request,
    exact: Vec<u8>,
}
pub(crate) struct Prepared {
    pub preview: AuditionPreview,
    binding: Binding,
}
#[derive(Default)]
pub(crate) struct AuditionUi {
    pub owned: bool,
    pub epoch: u64,
    binding: Option<Binding>,
    pub last: Option<AuditionPreview>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub canceled: bool,
    pub engine_ready: bool,
    pub starts: u64,
}
impl AuditionUi {
    pub fn prepare(editor: &Editor, request: Request) -> Result<Prepared, String> {
        let preview = request.prepare(&editor.song)?;
        let exact = request.binding(&editor.song)?;
        Ok(Prepared {
            preview,
            binding: Binding {
                revision: editor.revision(),
                epoch: 0,
                request,
                exact,
            },
        })
    }
    pub fn install(&mut self, mut prepared: Prepared) {
        self.epoch = self.epoch.wrapping_add(1);
        prepared.binding.epoch = self.epoch;
        self.owned = true;
        self.binding = Some(prepared.binding);
        self.last = Some(prepared.preview);
        self.error = None;
        self.notice = None;
        self.canceled = false;
        self.engine_ready = false;
        self.starts = self.starts.saturating_add(1);
    }
    pub fn current(&self, editor: &Editor, current: Option<&Request>) -> bool {
        self.binding.as_ref().is_some_and(|b| {
            b.revision == editor.revision()
                && b.epoch == self.epoch
                && current == Some(&b.request)
                && b.request
                    .binding(&editor.song)
                    .is_ok_and(|bytes| bytes == b.exact)
        })
    }
    pub fn cancel(&mut self, reason: &str) {
        if self.owned {
            self.epoch = self.epoch.wrapping_add(1);
            self.canceled = true;
            self.binding = None;
            self.notice = Some(reason.into());
        }
    }
    pub fn finish(&mut self, error: Option<String>) {
        self.binding = None;
        if !self.canceled {
            self.error = error;
        }
    }
    pub fn song_started(&mut self) {
        self.owned = false;
        self.binding = None;
        self.canceled = false;
    }
}
#[derive(Clone, Default)]
pub(crate) struct PanelState {
    pub available: bool,
    pub blocked: bool,
    pub busy: bool,
    pub owned: bool,
    pub ready: bool,
    pub diagnostic: bool,
    pub ended: bool,
    pub stopping: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub summary: Option<AuditionSummary>,
}
impl PanelState {
    pub fn controls(&self, ui: &mut egui::Ui, label: &str, has_target: bool) -> (bool, bool) {
        let mut start = false;
        let mut stop = false;
        ui.horizontal_wrapped(|ui|{
            start=ui.add_enabled(self.available&&!self.blocked&&!self.busy&&has_target,egui::Button::new(label))
                .on_hover_text("Explicit read-only snapshot. Fixed input velocity 1 / bin 127, 2s musical gate + 0.5s tail, at most 2.5s. No insertion, Apply, selection change or automatic retrigger.").clicked();
            stop=ui.add_enabled(self.owned&&self.busy,egui::Button::new("Stop audition"))
                .on_hover_text("Immediate hard Stop may click. Already submitted hardware buffers cannot be retracted.").clicked();
        });
        if !self.available {
            ui.label(
                RichText::new("Audition unavailable · this build has no realtime backend")
                    .small()
                    .weak(),
            );
        } else if self.busy && !self.owned {
            ui.label(
                RichText::new("Song transport busy · stop Song first; no takeover")
                    .small()
                    .weak(),
            );
        } else if self.owned {
            let text = if self.stopping {
                "AUDITION STOPPING"
            } else if self.busy && self.ready {
                "AUDITION SNAPSHOT"
            } else if self.busy {
                "AUDITION PREPARING"
            } else if self.ended {
                "AUDITION ENDED"
            } else {
                "AUDITION STOPPED"
            };
            ui.label(RichText::new(text).small().color(crate::tracker_ui::AQUA));
        }
        if self.diagnostic {
            ui.label(
                RichText::new("DEVICE-FREE DIAGNOSTIC · no device/audio-output claim")
                    .small()
                    .color(egui::Color32::from_rgb(229, 185, 112)),
            );
        }
        ui.label(
            RichText::new("Fixed 2s + 0.5s tail · unity / bin 127 · cold context/RR reset")
                .small()
                .weak(),
        );
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("Audition: {error}"));
        }
        if let Some(notice) = &self.notice {
            ui.label(RichText::new(notice).small().weak());
        }
        if let Some(summary) = &self.summary {
            egui::CollapsingHeader::new("Audition details").show(ui,|ui|{
                ui.label(if self.owned && self.busy && !self.stopping { "Captured audition snapshot (not a live draft)" } else { "Previous audition snapshot (not the current draft)" });
                ui.label(format!("{:?}",summary.target));
                if matches!(summary.target, audition::AuditionTarget::RegionDraft) { ui.label("Region/sample IDs below are audition-local IDs, not source/region registrations."); }
                ui.monospace(format!("ON note {} · gain {:?} · pan {:?}\nMaster {:?} · {} output frames",summary.sounding_note,summary.on_gain,summary.pan,summary.master_gain,summary.total_frames));
                ui.label(format!("Source region {:?}",summary.region));
                if let Some(pad)=&summary.pad {ui.label(format!("Committed selector {} / tune {:+} st / {:?} / {:?} / choke {:?}",pad.note,pad.tune_semitones,pad.mode,pad.new_note,pad.choke_group));}
                if let Some(layer)=&summary.layer_selection {ui.label(format!("Chosen velocity/RR source: {layer:?}"));}
                let path=summary.source_path.to_string_lossy();
                let shown=path.chars().take(512).collect::<String>();
                ui.label(format!("Source path{}: {}",if shown.len()<path.len(){" (display elided; Copy is exact)"}else{""},shown));
                if ui.small_button("Copy audition source path").clicked(){ui.ctx().copy_text(path.into_owned());}
                ui.label("The pure plan does not verify source bytes; the shared Engine verifies the chosen source while preparing this run. Not a permanent verification or a listening receipt.");
                for warning in &summary.warnings{ui.label(RichText::new(warning).small().weak());}
            });
        }
        (start, stop)
    }
}

/// Explicit test/capture-only adapter: no CPAL/device/lease. Every emitted block
/// still goes through the unchanged Producer and Shared::fill. Work and source
/// decoding stay on its one tracked worker, never the egui paint thread.
#[cfg(any(test, feature = "ui-capture"))]
pub(crate) mod diagnostic {
    use crate::{
        song::Song,
        transport::{Producer, Shared},
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver, SyncSender, TryRecvError},
            Arc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };
    enum Message {
        Prepared,
        Block(Vec<f32>),
        Finished(Result<(), String>),
    }
    #[derive(Default)]
    pub struct Driver {
        pub enabled: bool,
        pub auto_advance: bool,
        pub shared: Option<Arc<Shared>>,
        pub snapshot: Option<Song>,
        worker: Option<JoinHandle<()>>,
        receiver: Option<Receiver<Message>>,
        tick: Option<SyncSender<()>>,
        pub error: Option<String>,
        pub pcm: Vec<f32>,
        pub starts: u64,
        pub prepared: bool,
        pub held: bool,
        gate: Option<Arc<AtomicBool>>,
        pending: bool,
        pub after_stop_silent: Option<bool>,
    }
    impl Driver {
        pub fn busy(&self) -> bool {
            self.worker.is_some()
        }
        pub fn start(&mut self, song: Song) -> Result<(), String> {
            if self.busy() {
                return Err("Transport is active or still stopping".into());
            }
            let shared = Arc::new(Shared::default());
            let state = shared.clone();
            let snapshot = song.clone();
            let gate = Arc::new(AtomicBool::new(!self.held));
            let worker_gate = gate.clone();
            let (tx, rx) = mpsc::sync_channel(2);
            let (tick, ticks) = mpsc::sync_channel(1);
            let worker = thread::Builder::new()
                .name("audition-device-free-diagnostic".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || -> Result<(), String> {
                            while !worker_gate.load(Ordering::SeqCst) {
                                if state.quit.load(Ordering::SeqCst) {
                                    return Ok(());
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                            let mut producer = Producer::new(&song, None, state.clone())?;
                            if producer.engine.manifest.musical_frames
                                + producer.engine.manifest.tail_frames
                                > crate::audition::TOTAL_FRAMES
                            {
                                return Err(
                                    "Device-free UI diagnostic is limited to 120000 frames".into(),
                                );
                            }
                            tx.send(Message::Prepared)
                                .map_err(|_| "Diagnostic receiver closed")?;
                            while state.is_running() && !state.quit.load(Ordering::SeqCst) {
                                match ticks.recv_timeout(Duration::from_millis(5)) {
                                    Ok(()) => (),
                                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                                    Err(_) => break,
                                }
                                if state.quit.load(Ordering::SeqCst) {
                                    break;
                                }
                                producer.pump_until_buffered()?;
                                let mut pcm = vec![0.0; 1024];
                                state.fill(&mut pcm);
                                tx.send(Message::Block(pcm))
                                    .map_err(|_| "Diagnostic receiver closed")?;
                            }
                            Ok(())
                        },
                    ))
                    .unwrap_or_else(|_| Err("Device-free diagnostic worker panicked".into()));
                    if result.is_err() {
                        state.stop();
                    }
                    let _ = tx.send(Message::Finished(result));
                })
                .map_err(|e| e.to_string())?;
            self.shared = Some(shared);
            self.snapshot = Some(snapshot);
            self.worker = Some(worker);
            self.receiver = Some(rx);
            self.tick = Some(tick);
            self.gate = Some(gate);
            self.error = None;
            self.pcm.clear();
            self.prepared = false;
            self.pending = false;
            self.after_stop_silent = None;
            self.starts += 1;
            Ok(())
        }
        pub fn stop(&self) {
            if let Some(s) = &self.shared {
                s.stop();
            }
        }
        pub fn release(&self) {
            if let Some(gate) = &self.gate {
                gate.store(true, Ordering::SeqCst);
            }
        }
        pub fn advance(&mut self) {
            if self.prepared && !self.pending && self.busy() {
                if let Some(tx) = &self.tick {
                    if tx.try_send(()).is_ok() {
                        self.pending = true;
                    }
                }
            }
        }
        fn drain(&mut self) {
            if let Some(rx) = &self.receiver {
                loop {
                    match rx.try_recv() {
                        Ok(Message::Prepared) => self.prepared = true,
                        Ok(Message::Block(pcm)) => {
                            self.pending = false;
                            let remaining = (crate::audition::TOTAL_FRAMES as usize * 2)
                                .saturating_sub(self.pcm.len());
                            self.pcm.extend_from_slice(&pcm[..remaining.min(pcm.len())]);
                        }
                        Ok(Message::Finished(result)) => {
                            self.error = result.err();
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
            }
        }
        pub fn poll(&mut self) {
            self.drain();
            if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
                let joined = self.worker.take().unwrap().join();
                // A terminal message may arrive after the first drain but before
                // is_finished. Re-read after join, never discard a queued error.
                self.drain();
                if joined.is_err() {
                    self.error = Some("Device-free diagnostic worker disconnected".into());
                    if let Some(shared) = &self.shared {
                        shared.stop();
                    }
                }
                self.receiver = None;
                self.tick = None;
                self.pending = false;
                if let Some(shared) = &self.shared {
                    let mut callback = [1.0; 1024];
                    shared.fill(&mut callback);
                    self.after_stop_silent = Some(callback.iter().all(|x| *x == 0.0));
                }
            }
            if self.auto_advance {
                self.advance();
            }
        }
    }
    impl Drop for Driver {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request::Region {
            asset: SampleAsset {
                path: std::env::temp_dir().join("explicit-owned.wav"),
                sha256: "a".repeat(64),
                sample_rate: 48000,
                channels: 1,
                frames: 4800,
            },
            draft: SampleRegion {
                name: "Draft".into(),
                sample: SampleId(9),
                start_frame: 0,
                end_frame: 4800,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
            source: Some(SampleId(9)),
            source_choice: Some(SampleId(9)),
            selected: Some(RegionId(7)),
            epoch: 2,
        }
    }
    #[test]
    fn audition_binding_is_bounded_input_not_unrelated_full_document_clone() {
        let mut editor = Editor::default();
        editor.song.title = "Unrelated".repeat(1_000_000);
        let request = request();
        let mut ui = AuditionUi::default();
        ui.install(AuditionUi::prepare(&editor, request.clone()).unwrap());
        assert!(ui.current(&editor, Some(&request)));
        editor.song.title.clear();
        assert!(
            ui.current(&editor, Some(&request)),
            "Unchecked unrelated data is not audition input authority"
        );
        let mut different = request.clone();
        if let Request::Region { draft, .. } = &mut different {
            draft.start_frame = 1;
        }
        assert!(!ui.current(&editor, Some(&different)));
        if let Request::Region { epoch, .. } = &mut different {
            *epoch = 3;
        }
        assert!(!ui.current(&editor, Some(&different)));
        ui.cancel("explicit reset");
        assert!(!ui.current(&editor, Some(&request)));
        ui.finish(Some("late completion error".into()));
        assert!(ui.error.is_none());
    }
    #[test]
    fn audition_binding_rejects_oversize_before_serialization_and_tracks_revision() {
        let mut editor = Editor::default();
        let mut request = request();
        if let Request::Region { draft, .. } = &mut request {
            draft.name = "x".repeat(257);
        }
        assert!(request.binding(&editor.song).is_err());
        request = self::request();
        let mut ui = AuditionUi::default();
        ui.install(AuditionUi::prepare(&editor, request.clone()).unwrap());
        editor.replace_document(editor.song.clone()).unwrap();
        assert!(
            !ui.current(&editor, Some(&request)),
            "Identical document replacement revokes revision"
        );
    }
}
