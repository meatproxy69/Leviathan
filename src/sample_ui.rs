//! Native, non-destructive source/region inspector. Decoding, hashing and analysis
//! only run on the bounded control worker; worker completion never edits a Song.
use crate::{
    break_analysis::{self, AnalysisConfig, ChannelExtrema, TransientAnalysis, WaveformBin},
    break_compiler::{
        self, CompileConfig, CompilePreview, CompileRoll, CompileSummary, CompileTiming,
        MarkerCleanup, Subdivision, TailPolicy,
    },
    editor::Editor,
    marker_bank_ui::{self, MarkerBankUi},
    onset_service::{self, CachedSource, SourceAnalysis},
    onset_v2::{Config as OnsetConfig, Preset},
    sample_asset,
    slice_features_ui::{self, SliceFeaturesUi},
    song::{
        AmpEnvelope, ForwardLoop, InstrumentId, InstrumentKind, MarkerSet, MarkerSetId, RegionId,
        SampleAsset, SampleId, SampleRegion, Song, TrackId,
    },
    tempo_ui::{self, TempoUi},
    tracker_ui::{self, AQUA},
};
use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, Stroke};
use phonic::AudioFileBuffer;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
};

const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const PROPOSAL: Color32 = Color32::from_rgb(229, 185, 112);

struct Preview {
    source: Option<SampleId>,
    asset: SampleAsset,
    buffer: Arc<AudioFileBuffer>,
    waveform: Vec<WaveformBin>,
}
#[derive(Clone)]
struct PreparedCompile {
    revision: u64,
    plan: Arc<CompilePreview>,
    source_path: PathBuf,
}
struct ExperimentalRequest {
    asset: SampleAsset,
    source_path: PathBuf,
    revision: u64,
    config: OnsetConfig,
    cache: Option<Arc<CachedSource>>,
}
struct ExperimentalResult {
    asset: SampleAsset,
    revision: u64,
    cache: Arc<CachedSource>,
    result: SourceAnalysis,
    reused: bool,
}
struct ExperimentalUi {
    enabled: bool,
    config: OnsetConfig,
    cache: Option<Arc<CachedSource>>,
    result: Option<Arc<ExperimentalResult>>,
    selected: usize,
    show_peaks: bool,
    show_attacks: bool,
    show_curves: bool,
    curve_metric: u8,
    curve_points: usize,
}
impl Default for ExperimentalUi {
    fn default() -> Self {
        Self {
            enabled: false,
            config: onset_service::experimental_config(Preset::Balanced),
            cache: None,
            result: None,
            selected: 0,
            show_peaks: true,
            show_attacks: true,
            show_curves: false,
            curve_metric: 0,
            curve_points: 256,
        }
    }
}
impl ExperimentalUi {
    fn clear_source(&mut self) {
        self.cache = None;
        self.result = None;
        self.selected = 0;
    }
}
enum Request {
    MarkerBank(Box<marker_bank_ui::WorkRequest>),
    Catalog(Box<slice_features_ui::WorkRequest>),
    Tempo(Box<tempo_ui::WorkRequest>),
    Experimental(Box<ExperimentalRequest>),
    Load {
        path: PathBuf,
        expected: Option<(SampleId, SampleAsset)>,
    },
    Analyze {
        asset: SampleAsset,
        buffer: Arc<AudioFileBuffer>,
        config: AnalysisConfig,
    },
    Compile {
        song: Box<Song>,
        revision: u64,
        config: CompileConfig,
        base: Option<PathBuf>,
    },
    Verify(PreparedCompile),
}
enum Completed {
    MarkerBank(Box<marker_bank_ui::WorkResult>),
    Catalog(Box<slice_features_ui::WorkResult>),
    Tempo(Box<tempo_ui::WorkResult>),
    Experimental(Box<ExperimentalResult>),
    Loaded(Preview),
    Analyzed(TransientAnalysis),
    Compiled(PreparedCompile),
    Verified(PreparedCompile),
}
#[derive(Clone, Copy)]
enum WorkerOwner {
    General,
    Tempo,
    Catalog,
    MarkerBank,
}
struct Pending {
    owner: WorkerOwner,
    epoch: u64,
    receiver: Receiver<Result<Completed, String>>,
}
/// Text is intentional: invalid or large source-frame values are never clamped,
/// and merely opening the inspector cannot rewrite a persisted integer.
#[derive(Clone, Debug, PartialEq)]
struct PlaybackDraft {
    envelope: bool,
    attack: String,
    decay: String,
    sustain: String,
    release: String,
    looping: bool,
    loop_start: String,
    loop_end: String,
}
impl Default for PlaybackDraft {
    fn default() -> Self {
        Self {
            envelope: false,
            attack: "0".into(),
            decay: "0".into(),
            sustain: "100".into(),
            release: "50".into(),
            looping: false,
            loop_start: String::new(),
            loop_end: String::new(),
        }
    }
}
impl PlaybackDraft {
    fn from_region(region: &SampleRegion) -> Self {
        let envelope = region.amp_envelope.unwrap_or_default();
        let looping = region.forward_loop.unwrap_or(ForwardLoop {
            start_frame: region.start_frame,
            end_frame: region.end_frame,
        });
        Self {
            envelope: region.amp_envelope.is_some(),
            attack: envelope.attack_ms.to_string(),
            decay: envelope.decay_ms.to_string(),
            sustain: envelope.sustain_percent.to_string(),
            release: envelope.release_ms.to_string(),
            looping: region.forward_loop.is_some(),
            loop_start: looping.start_frame.to_string(),
            loop_end: looping.end_frame.to_string(),
        }
    }
    fn parse(&self) -> Result<(Option<AmpEnvelope>, Option<ForwardLoop>), String> {
        let envelope = if self.envelope {
            Some(AmpEnvelope {
                attack_ms: playback_integer(&self.attack, "Attack", 10_000)? as u16,
                decay_ms: playback_integer(&self.decay, "Decay", 10_000)? as u16,
                sustain_percent: playback_integer(&self.sustain, "Sustain", 100)? as u8,
                release_ms: playback_integer(&self.release, "Release", 500)? as u16,
            })
        } else {
            None
        };
        let looping = if self.looping {
            Some(ForwardLoop {
                start_frame: playback_integer(&self.loop_start, "Loop start", u64::MAX)?,
                end_frame: playback_integer(&self.loop_end, "Loop end", u64::MAX)?,
            })
        } else {
            None
        };
        Ok((envelope, looping))
    }
}
fn playback_integer(text: &str, label: &str, max: u64) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "{label}: enter a whole number from 0 to {max}; no rounding"
        ));
    }
    text.parse::<u64>()
        .ok()
        .filter(|value| *value <= max)
        .ok_or_else(|| format!("{label}: must be between 0 and {max}; value was not clamped"))
}
fn playback_text(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .id(egui::Id::new(("region-playback", label)))
                .desired_width(width),
        )
        .on_hover_text(format!("Exact text: {value}\nWhole integer; invalid text is retained and Apply rejects it without clamping."));
    });
}
#[derive(Clone, Debug, PartialEq)]
struct RegionDraft {
    name: String,
    start: u64,
    end: u64,
    root: u8,
    playback: PlaybackDraft,
}
impl Default for RegionDraft {
    fn default() -> Self {
        Self {
            name: "Region 1".into(),
            start: 0,
            end: 1,
            root: 60,
            playback: PlaybackDraft::default(),
        }
    }
}
#[derive(Default)]
pub struct SampleUi {
    pub(crate) audition: crate::audition_ui::PanelState,
    audition_action: Option<crate::audition_ui::Action>,
    path: String,
    picker: Option<Receiver<Option<PathBuf>>>,
    source_choice: Option<SampleId>,
    preview: Option<Preview>,
    selected: Option<(RegionId, SampleRegion)>,
    draft: RegionDraft,
    draft_active: bool,
    instrument: u64,
    map_on_apply: bool,
    workflow: bool,
    sequence_view: bool,
    compile_config: Option<CompileConfig>,
    compiled: Option<PreparedCompile>,
    verified: Option<PreparedCompile>,
    commit_authorized: bool,
    committed: Option<CompileSummary>,
    cleanup: Option<(Vec<u64>, MarkerCleanup)>,
    generated_position: Option<(usize, TrackId)>,
    marker_selected: Option<(MarkerSetId, MarkerSet)>,
    marker_draft: Option<MarkerSet>,
    marker_index: usize,
    marker_frame: u64,
    rejected_suggestions: BTreeSet<u64>,
    pending: Option<Pending>,
    inspect_after_load: Option<(RegionId, SampleRegion, InstrumentId)>,
    epoch: u64,
    analysis: Option<TransientAnalysis>,
    experimental: ExperimentalUi,
    tempo: TempoUi,
    tempo_view: bool,
    features: SliceFeaturesUi,
    features_view: bool,
    marker_bank: MarkerBankUi,
    config: AnalysisConfig,
    show_suggestions: bool,
    error: Option<String>,
    notice: String,
}
impl SampleUi {
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_marker_bank_state(&self) -> serde_json::Value {
        let mut state = self.marker_bank.capture_state();
        state["source_busy"] = self.busy().into();
        state["markers_saved"] = self.markers_saved().into();
        state
    }
    pub(crate) fn take_marker_bank_kit(&mut self) -> Option<(InstrumentId, crate::song::PadId)> {
        self.marker_bank.take_open_kit()
    }
    #[cfg(feature = "ui-capture")]
    pub(crate) fn capture_experimental_analysis(&self, editor: &Editor) -> Option<&SourceAnalysis> {
        self.experimental
            .result
            .as_ref()
            .filter(|prepared| self.experimental_current(editor, prepared))
            .map(|prepared| &prepared.result)
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_tempo_state(&self, editor: &Editor) -> serde_json::Value {
        let mut state = self.tempo.capture_state(
            editor,
            self.preview.as_ref().and_then(|p| p.source),
            self.preview.as_ref().map(|p| &p.asset),
            &self.experimental.config,
        );
        state["source"] =
            serde_json::to_value(self.preview.as_ref().and_then(|p| p.source)).unwrap();
        state["asset"] = serde_json::to_value(self.preview.as_ref().map(|p| &p.asset)).unwrap();
        state["cache_binding"] = serde_json::to_value(
            self.experimental
                .cache
                .as_ref()
                .map(|cache| cache.binding()),
        )
        .unwrap();
        state["busy"] = self.busy().into();
        state["view"] = self.tempo_view.into();
        state["onset_config"] = serde_json::to_value(&self.experimental.config).unwrap();
        state
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_catalog_state(&self, editor: &Editor) -> serde_json::Value {
        let source = if self.features_view {
            self.source_choice
                .or_else(|| self.preview.as_ref().and_then(|p| p.source))
        } else {
            self.preview
                .as_ref()
                .and_then(|p| p.source)
                .or(self.source_choice)
        };
        let asset = source.and_then(|id| editor.song.samples.get(&id));
        let mut state = self.features.capture_state(editor, source, asset);
        state["source"] = serde_json::to_value(source).unwrap();
        state["asset"] = serde_json::to_value(asset).unwrap();
        state["busy"] = self.busy().into();
        state["view"] = self.features_view.into();
        state["region_draft"] = serde_json::json!({"selected":self.selected.as_ref().map(|(id,_)|id),"name":self.draft.name,"start":self.draft.start,"end":self.draft.end,"root":self.draft.root,"map_on_apply":self.map_on_apply});
        state
    }
    pub(crate) fn audition_request(&self) -> Result<crate::audition_ui::Request, String> {
        if !self.draft_active
            || self.busy()
            || self.workflow
            || self.tempo_view
            || self.features_view
        {
            return Err("Select an available region draft in Regions before auditioning".into());
        }
        let preview = self
            .preview
            .as_ref()
            .ok_or("Load a verified source preview first")?;
        if preview.asset.path.as_os_str().len() > crate::audition::MAX_PATH_BYTES
            || preview.asset.sha256.len() > 64
            || self.draft.name.len() > 256
        {
            return Err("Audition input exceeds bounded source/draft metadata".into());
        }
        let (amp_envelope, forward_loop) = self.draft.playback.parse()?;
        Ok(crate::audition_ui::Request::Region {
            asset: preview.asset.clone(),
            source: preview.source,
            source_choice: self.source_choice,
            selected: self.selected.as_ref().map(|(id, _)| *id),
            epoch: self.epoch,
            draft: SampleRegion {
                name: self.draft.name.clone(),
                sample: preview.source.unwrap_or(SampleId(1)),
                start_frame: self.draft.start,
                end_frame: self.draft.end,
                root_note: self.draft.root,
                amp_envelope,
                forward_loop,
            },
        })
    }
    pub(crate) fn take_audition_action(&mut self) -> Option<crate::audition_ui::Action> {
        self.audition_action.take()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// Reuse the verified source worker and region inspector without changing a
    /// mapping or the tracker cursor. Editing a kit's region must not silently
    /// replace its instrument kind with a pitched-region assignment.
    pub(crate) fn inspect_region(
        &mut self,
        editor: &Editor,
        id: RegionId,
        instrument: InstrumentId,
        base: Option<&Path>,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("A source task is already running".into());
        }
        let region = editor
            .song
            .regions
            .get(&id)
            .ok_or("Region no longer exists")?
            .clone();
        let asset = editor
            .song
            .samples
            .get(&region.sample)
            .ok_or("Region source is missing")?
            .clone();
        if !editor.song.instruments.contains_key(&instrument) {
            return Err("Instrument no longer exists".into());
        }
        self.workflow = false;
        self.tempo_view = false;
        self.features_view = false;
        if self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.source == Some(region.sample) && preview.asset == asset)
        {
            self.select_inspected_region(id, region, instrument);
        } else {
            let path = if asset.path.is_relative() {
                base.map_or(asset.path.clone(), |base| base.join(&asset.path))
            } else {
                asset.path.clone()
            };
            self.start(Request::Load {
                path,
                expected: Some((region.sample, asset)),
            })?;
            self.inspect_after_load = Some((id, region, instrument));
        }
        Ok(())
    }
    fn select_inspected_region(
        &mut self,
        id: RegionId,
        region: SampleRegion,
        instrument: InstrumentId,
    ) {
        self.draft = draft_from(&region);
        self.selected = Some((id, region));
        self.instrument = instrument.0;
        self.draft_active = true;
        self.map_on_apply = false;
        self.notice = "Inspecting the shared region. Map on Apply is OFF; editing bounds does not replace the instrument assignment.".into();
    }
    /// New/Open invalidates old results without allowing a late completion to
    /// populate or mutate the replacement document. The worker remains tracked.
    pub fn reset_document(&mut self) {
        self.audition_action = None;
        self.epoch = self.epoch.wrapping_add(1);
        self.preview = None;
        self.inspect_after_load = None;
        self.selected = None;
        self.source_choice = None;
        self.analysis = None;
        self.experimental = ExperimentalUi::default();
        self.tempo.reset();
        self.tempo_view = false;
        self.features.reset();
        self.features_view = false;
        self.marker_bank.reset();
        self.marker_selected = None;
        self.marker_draft = None;
        self.compile_config = None;
        self.compiled = None;
        self.verified = None;
        self.commit_authorized = false;
        self.committed = None;
        self.cleanup = None;
        self.rejected_suggestions.clear();
        self.path.clear();
        self.picker = None;
        self.error = None;
        self.notice.clear();
    }
    fn start(&mut self, request: Request) -> Result<(), String> {
        if self.busy() {
            return Err("A source task is already running".into());
        }
        if matches!(&request, Request::Load { .. }) {
            self.inspect_after_load = None;
            self.marker_bank.close();
        }
        let owner = match &request {
            Request::Tempo(_) => WorkerOwner::Tempo,
            Request::Catalog(_) => WorkerOwner::Catalog,
            Request::MarkerBank(_) => WorkerOwner::MarkerBank,
            _ => WorkerOwner::General,
        };
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("leviathan-source-worker".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(request)))
                        .unwrap_or_else(|_| {
                            Err("Source worker panicked; no document changes were applied".into())
                        });
                let _ = sender.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.pending = Some(Pending {
            owner,
            epoch: self.epoch,
            receiver,
        });
        self.error = None;
        Ok(())
    }
    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Source worker disconnected".into()),
        };
        let owner = pending.owner;
        let current = pending.epoch == self.epoch;
        self.pending = None;
        if !current {
            return;
        }
        match result {
            Ok(Completed::MarkerBank(result)) => self.marker_bank.finish(*result),
            Ok(Completed::Catalog(result)) => self.features.finish(*result),
            Ok(Completed::Tempo(result)) => self.tempo.finish(*result),
            Ok(Completed::Loaded(preview)) => {
                self.marker_bank.reset();
                self.tempo.reset();
                self.features.reset();
                self.draft = RegionDraft {
                    name: "Region 1".into(),
                    start: 0,
                    end: preview.asset.frames,
                    root: 60,
                    playback: PlaybackDraft::default(),
                };
                self.selected = None;
                self.draft_active = true;
                self.source_choice = preview.source;
                self.marker_selected = None;
                self.marker_draft = Some(MarkerSet {
                    name: "Markers 1".into(),
                    sample: preview.source.unwrap_or(SampleId(0)),
                    source_bpm: None,
                    locked_frames: vec![],
                    markers: vec![0, preview.asset.frames],
                });
                self.marker_frame = preview.asset.frames / 2;
                self.marker_index = 0;
                self.rejected_suggestions.clear();
                self.compile_config = None;
                self.compiled = None;
                self.verified = None;
                self.commit_authorized = false;
                self.committed = None;
                self.cleanup = None;
                self.preview = Some(preview);
                self.analysis = None;
                self.experimental.clear_source();
                self.map_on_apply = true;
                self.notice = if self
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.source.is_some())
                {
                    "Registered source verified; select a committed region or create a new draft."
                } else {
                    "Preview only. Apply a region to register it in the document."
                }
                .into();
                if let Some((id, region, instrument)) = self.inspect_after_load.take() {
                    if self.preview.as_ref().and_then(|preview| preview.source)
                        == Some(region.sample)
                    {
                        self.select_inspected_region(id, region, instrument);
                    }
                }
            }
            Ok(Completed::Experimental(prepared)) => {
                self.experimental.cache = Some(Arc::clone(&prepared.cache));
                self.experimental.selected = 0;
                self.notice = if prepared.reused { "Experimental cached repick completed: no FFT or decoding. Manual markers are unchanged." } else { "Experimental analysis ready. Inspect coarse peaks and boundary estimates; no marker edits were applied." }.into();
                self.experimental.result = Some(Arc::from(prepared));
                self.show_suggestions = true;
            }
            Ok(Completed::Analyzed(analysis)) => {
                self.analysis = Some(analysis);
                self.show_suggestions = true;
                self.notice =
                    "Suggestions updated; committed regions and manual draft were not changed."
                        .into();
            }
            Ok(Completed::Compiled(prepared)) => {
                self.compiled = Some(prepared);
                self.notice =
                    "Preview ready. No document changes. Review the plan before Commit.".into();
            }
            Ok(Completed::Verified(prepared)) => {
                if self.commit_authorized {
                    self.verified = Some(prepared);
                }
            }
            Err(error) => {
                match owner {
                    WorkerOwner::Tempo => self.tempo.fail(error.clone()),
                    WorkerOwner::Catalog => self.features.fail(error.clone()),
                    WorkerOwner::MarkerBank => self.marker_bank.fail(error.clone()),
                    WorkerOwner::General => (),
                }
                self.commit_authorized = false;
                self.error = Some(error);
            }
        }
    }
    pub fn cancel_commit(&mut self) {
        self.marker_bank.cancel_commit();
        self.tempo.cancel_commit();
        self.features.cancel_commit();
        self.commit_authorized = false;
        self.verified = None;
    }
    pub fn take_generated_position(&mut self) -> Option<(usize, TrackId)> {
        self.generated_position.take()
    }
    fn result(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }
    fn sync(&mut self, editor: &Editor) {
        if !editor
            .song
            .instruments
            .contains_key(&InstrumentId(self.instrument))
        {
            self.instrument = editor.song.instruments.keys().next().map_or(0, |id| id.0);
        }
        // Never recreate a deleted source/region from a stale UI draft after Undo/Open.
        if self.preview.as_ref().is_some_and(|preview| {
            preview
                .source
                .is_some_and(|id| editor.song.samples.get(&id) != Some(&preview.asset))
        }) {
            self.preview = None;
            self.selected = None;
            self.analysis = None;
            self.experimental.clear_source();
            self.notice = "Selected source changed or was removed. Load it again.".into();
        }
        if let Some((id, source)) = &self.marker_selected {
            match editor.song.marker_sets.get(id) {
                Some(set)
                    if self.preview.as_ref().and_then(|preview| preview.source)
                        != Some(set.sample) =>
                {
                    self.marker_selected = None;
                    self.marker_draft = None;
                    self.notice =
                        "Marker set uses another source; reload it before editing.".into();
                }
                Some(set) if set != source => {
                    self.marker_selected = Some((*id, set.clone()));
                    self.marker_draft = Some(set.clone());
                    self.marker_index = 0;
                }
                None => {
                    self.marker_selected = None;
                    self.marker_draft = None;
                    self.notice =
                        "Marker set removed. Choose New markers to start a new draft.".into();
                }
                _ => (),
            }
        }
        if self
            .source_choice
            .is_some_and(|id| !editor.song.samples.contains_key(&id))
        {
            self.source_choice = None;
        }
        if let Some((id, source)) = &self.selected {
            match editor.song.regions.get(id) {
                Some(region)
                    if self.preview.as_ref().and_then(|preview| preview.source)
                        != Some(region.sample) =>
                {
                    self.selected = None;
                    self.draft_active = false;
                    self.notice =
                        "Region now uses another source. Reload that source before editing.".into();
                }
                Some(region) if region != source => {
                    self.draft = draft_from(region);
                    self.selected = Some((*id, region.clone()));
                }
                None => {
                    self.selected = None;
                    self.draft_active = false;
                    self.draft = RegionDraft::default();
                    self.notice =
                        "Region removed. Choose New region to start a fresh draft.".into();
                }
                _ => (),
            }
        }
    }
    fn apply(&mut self, editor: &mut Editor) -> Result<(), String> {
        self.notice.clear();
        if self.busy() {
            return Err("Wait for the source task to finish".into());
        }
        if !self.draft_active {
            return Err("Choose New region or select an existing region before applying".into());
        }
        let (amp_envelope, forward_loop) = self.draft.playback.parse()?;
        let preview = self.preview.as_ref().ok_or("Load a source first")?;
        let mut candidate = editor.song.clone();
        let source_id = if let Some(id) = preview.source {
            if candidate.samples.get(&id) != Some(&preview.asset) {
                return Err("Source changed; reload before applying".into());
            }
            id
        } else if let Some((id, _)) = candidate
            .samples
            .iter()
            .find(|(_, asset)| *asset == &preview.asset)
        {
            *id
        } else {
            let id = SampleId(next_id(candidate.samples.keys().map(|id| id.0))?);
            candidate.register_sample(id, preview.asset.clone())?;
            id
        };
        let region_id = if let Some((id, source)) = &self.selected {
            if candidate.regions.get(id) != Some(source) {
                return Err("Region changed; reselect before applying".into());
            }
            *id
        } else {
            RegionId(next_id(candidate.regions.keys().map(|id| id.0))?)
        };
        let region = SampleRegion {
            name: self.draft.name.clone(),
            sample: source_id,
            start_frame: self.draft.start,
            end_frame: self.draft.end,
            root_note: self.draft.root,
            amp_envelope,
            forward_loop,
        };
        candidate.upsert_region(region_id, region.clone())?;
        if self.map_on_apply {
            candidate.assign_region(InstrumentId(self.instrument), Some(region_id))?;
        }
        editor.apply_document_edit(candidate)?;
        self.preview.as_mut().unwrap().source = Some(source_id);
        self.source_choice = Some(source_id);
        self.selected = Some((region_id, region));
        self.notice = "Region applied (one undo). Original source bytes were not modified.".into();
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor, base: Option<&Path>) -> bool {
        if let Some(picker) = &self.picker {
            match picker.try_recv() {
                Ok(Some(path)) => {
                    self.path = path.display().to_string();
                    self.picker = None;
                    let result = self.start(Request::Load {
                        path,
                        expected: None,
                    });
                    self.result(result);
                }
                Ok(None) => self.picker = None,
                Err(TryRecvError::Disconnected) => {
                    self.picker = None;
                    self.error = Some("Native file picker disconnected".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        self.sync(editor);
        self.finish_verified_commit(editor, ui.is_enabled());
        // This inspector is never a musical keyboard target, even when returning
        // from a focused tracker. Its text/numeric fields keep all note keys.
        ui.memory_mut(|memory| memory.surrender_focus(tracker_ui::grid_id()));
        let mut back = false;
        ui.horizontal_wrapped(|ui| {
            back = ui.button("< Song").clicked();
            ui.label(RichText::new("SOURCES / REGIONS").strong().color(AQUA));
            ui.toggle_value(&mut editor.edit_mode, "Edit mode");
            ui.label(
                RichText::new("Immutable original · non-destructive source regions")
                    .small()
                    .color(MUTED),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .desired_width(300.0)
                    .hint_text("WAV / FLAC source path"),
            );
            if ui
                .add_enabled(!self.busy() && self.picker.is_none(), egui::Button::new("Browse…"))
                .on_hover_text("Choose a WAV or FLAC source with the native file picker")
                .clicked()
            {
                let (sender, receiver) = mpsc::channel();
                let ctx = ui.ctx().clone();
                match thread::Builder::new()
                    .name("leviathan-file-picker".into())
                    .spawn(move || {
                        let path = rfd::FileDialog::new()
                            .add_filter("Audio source", &["wav", "flac"])
                            .pick_file();
                        let _ = sender.send(path);
                        ctx.request_repaint();
                    })
                {
                    Ok(_) => self.picker = Some(receiver),
                    Err(error) => self.error = Some(format!("Could not start file picker: {error}")),
                }
            }
            if ui
                .add_enabled(
                    !self.busy() && self.picker.is_none() && !self.path.trim().is_empty(),
                    egui::Button::new("Load preview"),
                )
                .clicked()
            {
                let input = PathBuf::from(self.path.trim());
                let path = if input.is_relative() {
                    base.map_or(input.clone(), |base| base.join(&input))
                } else {
                    input
                };
                let result = self.start(Request::Load {
                    path,
                    expected: None,
                });
                self.result(result);
            }
            egui::ComboBox::from_id_salt("registered-source")
                .width(160.0)
                .selected_text(
                    self.source_choice
                        .map_or("Registered sources".into(), |id| format!("Source {}", id.0)),
                )
                .show_ui(ui, |ui| {
                    for (id, asset) in &editor.song.samples {
                        ui.selectable_value(
                            &mut self.source_choice,
                            Some(*id),
                            format!(
                                "{} · {}",
                                id.0,
                                asset.path.file_name().unwrap_or_default().to_string_lossy()
                            ),
                        );
                    }
                });
            if ui
                .add_enabled(
                    !self.busy() && self.source_choice.is_some(),
                    egui::Button::new("Reload verified"),
                )
                .clicked()
            {
                if let Some((id, asset)) = self.source_choice.and_then(|id| {
                    editor
                        .song
                        .samples
                        .get(&id)
                        .map(|asset| (id, asset.clone()))
                }) {
                    let path = if asset.path.is_relative() {
                        base.map_or(asset.path.clone(), |base| base.join(&asset.path))
                    } else {
                        asset.path.clone()
                    };
                    let result = self.start(Request::Load {
                        path,
                        expected: Some((id, asset)),
                    });
                    self.result(result);
                }
            }
            if ui.add_enabled(editor.edit_mode && !self.busy() && self.source_choice.is_some(), egui::Button::new("Remove source")).on_hover_text("Remove unused document reference only, never the file. Refuses while any region references this source.").clicked() {
                if let Some(id) = self.source_choice { let result = editor.remove_sample(id); self.result(result); self.sync(editor); }
            }
        });
        if self.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Source worker active · explicit commit guards remain in force");
            });
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if self.preview.as_ref().and_then(|preview| preview.source) != self.source_choice
            || self.preview.is_none()
        {
            if let Some((source, asset)) = self.source_choice.and_then(|id| {
                editor
                    .song
                    .samples
                    .get(&id)
                    .cloned()
                    .map(|asset| (id, asset))
            }) {
                if !self.features_view && ui.button("Inspect stored catalog").on_hover_text("Read historical metadata and edit manual roles without loading audio. Analyze and Store still verify the source on the worker.").clicked(){self.features_view=true;self.tempo_view=false;}
                if self.features_view {
                    if ui.button("< Source preview").clicked() {
                        self.features_view = false;
                        self.features.cancel_commit();
                    }
                    ui.label(RichText::new(format!("Stored metadata view · S{} / {} Hz / {} channels · no file read for inspection",source.0,asset.sample_rate,asset.channels)).small().color(MUTED));
                    let busy = self.busy();
                    let request = self.features.show(
                        ui,
                        editor,
                        slice_features_ui::Context {
                            source,
                            asset: &asset,
                            path: resolve_source(&asset, base),
                            cache: None,
                            busy,
                        },
                    );
                    if let Some(request) = request {
                        let result = self.start(Request::Catalog(Box::new(request)));
                        self.result(result);
                    }
                    return back;
                }
            }
        }
        let Some(preview) = &self.preview else {
            ui.separator();
            ui.label("Load a WAV or FLAC to inspect its waveform, define source-frame boundaries, and map a region to an instrument.");
            ui.label(RichText::new("Load is a preview, not a document edit. Apply is explicit and undoable. No source rewriting or time-stretching. Explicit region-draft audition requires an available realtime backend. Break workflow offers explicitly reviewed sequence compilation.").small().color(MUTED));
            return back;
        };
        let asset = preview.asset.clone();
        ui.label(
            RichText::new(format!(
                "{}  ·  {} Hz / {} ch  ·  {} real source frames / {:.3} s",
                asset.path.file_name().unwrap_or_default().to_string_lossy(),
                asset.sample_rate,
                asset.channels,
                asset.frames,
                asset.frames as f64 / f64::from(asset.sample_rate)
            ))
            .monospace(),
        );
        ui.label(
            RichText::new(format!(
                "Snapshot SHA-256 {}… · rechecked on render / reload",
                &asset.sha256[..16.min(asset.sha256.len())]
            ))
            .small()
            .color(MUTED),
        )
        .on_hover_text(&asset.sha256);
        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    !self.tempo_view && !self.features_view && !self.workflow,
                    "Regions",
                )
                .clicked()
            {
                self.workflow = false;
                self.tempo_view = false;
                self.features_view = false;
            }
            if ui
                .selectable_label(
                    !self.tempo_view && !self.features_view && self.workflow,
                    "Break workflow",
                )
                .clicked()
            {
                self.workflow = true;
                self.tempo_view = false;
                self.features_view = false;
            }
            if ui
                .selectable_label(self.tempo_view, "Tempo / pulse")
                .clicked()
            {
                self.tempo_view = true;
                self.features_view = false;
            }
            if ui
                .selectable_label(self.features_view, "Slice catalog")
                .clicked()
            {
                self.features_view = true;
                self.tempo_view = false;
            }
        });
        if !self.tempo_view {
            self.tempo.cancel_commit();
        }
        if !self.features_view {
            self.features.cancel_commit();
        }
        if back || !self.workflow || self.tempo_view || self.features_view || self.sequence_view {
            self.marker_bank.close();
        }
        if self.features_view {
            if let Some(source) = self.preview.as_ref().and_then(|preview| preview.source) {
                let busy = self.busy();
                let request = self.features.show(
                    ui,
                    editor,
                    slice_features_ui::Context {
                        source,
                        asset: &asset,
                        path: resolve_source(&asset, base),
                        cache: self.experimental.cache.as_ref(),
                        busy,
                    },
                );
                if let Some(request) = request {
                    let result = self.start(Request::Catalog(Box::new(request)));
                    self.result(result);
                }
            } else {
                ui.label("Register this source and explicit regions before catalog analysis. No source or region is created automatically.");
            }
            return back;
        }
        if self.tempo_view {
            if let Some(source) = self.preview.as_ref().and_then(|preview| preview.source) {
                let busy = self.busy();
                let request = self.tempo.show(
                    ui,
                    editor,
                    tempo_ui::Context {
                        source,
                        asset: &asset,
                        cache: self.experimental.cache.as_ref(),
                        onset: &self.experimental.config,
                        path: resolve_source(&asset, base),
                        busy,
                    },
                );
                if let Some(request) = request {
                    let result = self.start(Request::Tempo(Box::new(request)));
                    self.result(result);
                }
            } else {
                ui.label("Register this source with an explicit region Apply before tempo review. No BPM is inferred or stored automatically.");
            }
            return back;
        }
        self.waveform(ui, editor);
        if self.workflow {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.sequence_view, false, "Markers");
                ui.selectable_value(&mut self.sequence_view, true, "Sequence");
            });
            if self.sequence_view {
                self.marker_bank.close();
                self.sequence_controls(ui, editor, &asset, base);
            } else {
                self.marker_controls(ui, editor, &asset, base);
                let marker_set = self.marker_selected.as_ref().map(|(id, _)| *id);
                let source = self
                    .preview
                    .as_ref()
                    .and_then(|preview| preview.source)
                    .filter(|source| self.source_choice == Some(*source))
                    .filter(|source| editor.song.samples.contains_key(source));
                let source_path = source
                    .and_then(|id| editor.song.samples.get(&id))
                    .map(|current| resolve_source(current, base))
                    .unwrap_or_default();
                let context = marker_bank_ui::Context {
                    marker_set,
                    source,
                    markers_saved: self.markers_saved() && !back,
                    path: source_path,
                    busy: self.busy(),
                };
                if let Some(request) = self.marker_bank.show(ui, editor, context) {
                    let result = self.start(Request::MarkerBank(Box::new(request)));
                    if let Err(error) = result {
                        self.marker_bank.fail(error);
                    }
                }
            }
            return back || self.generated_position.is_some();
        }
        ui.label(RichText::new("Aqua: draft · gray: committed · amber: suggestions. Click/drag = start; Shift-click/drag = end. Apply required.").small().color(MUTED));
        ui.separator();
        let writable = editor.edit_mode && ui.is_enabled() && !self.busy();
        let mut select = None;
        ui.horizontal_top(|ui| {
            // The separator must not stretch this compact inspector row to the
            // entire remaining viewport and push analysis below a blank area.
            ui.set_max_height(195.0);
            ui.allocate_ui_with_layout(egui::vec2(190.0, 180.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_width(175.0);
                ui.set_max_width(190.0);
                ui.label(RichText::new("COMMITTED REGIONS").small().strong().color(MUTED));
                if ui.add_enabled(!self.busy(), egui::Button::new("New region")).clicked() {
                    self.selected = None;
                    self.draft_active = true;
                    self.draft = RegionDraft { name: format!("Region {}", next_id(editor.song.regions.keys().map(|id| id.0)).unwrap_or(1)), start: 0, end: asset.frames, root: 60, playback: PlaybackDraft::default() };
                }
                egui::ScrollArea::vertical().id_salt("region-list").max_height(130.0).show(ui, |ui| {
                    for (id, region) in &editor.song.regions {
                        if self.preview.as_ref().unwrap().source != Some(region.sample) { continue; }
                        if ui.selectable_label(self.selected.as_ref().is_some_and(|(selected, _)| selected == id), format!("{:02}  {}", id.0, region.name)).on_hover_text(format!("[{}, {}) source frames", region.start_frame, region.end_frame)).clicked() { select = Some((*id, region.clone())); }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.label(RichText::new(if self.selected.is_some() { "REGION EDIT DRAFT" } else { "NEW REGION DRAFT" }).strong().color(AQUA));
                ui.add_enabled_ui(writable, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.draft.name).desired_width(165.0).hint_text("Region name"));
                        ui.label("Root MIDI"); ui.add(egui::DragValue::new(&mut self.draft.root).range(0..=127));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Start"); ui.add(egui::DragValue::new(&mut self.draft.start).range(0..=asset.frames.saturating_sub(1)).speed(1.0));
                        ui.label("End (exclusive)"); ui.add(egui::DragValue::new(&mut self.draft.end).range(1..=asset.frames).speed(1.0));
                        ui.label("SOURCE frames");
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.map_on_apply, "Map on Apply");
                        egui::ComboBox::from_id_salt("region-instrument").selected_text(format!("Instrument {}", self.instrument)).show_ui(ui, |ui| {
                            for (id, instrument) in &editor.song.instruments { ui.selectable_value(&mut self.instrument, id.0, format!("{} · {}", id.0, instrument.name)); }
                        });
                    });
                });
                let mapping = editor.song.instruments.get(&InstrumentId(self.instrument)).map_or("missing instrument".into(), |instrument| {
                    if let InstrumentKind::DrumKit { kit } = &instrument.kind { format!("drum kit {} (Map on Apply replaces it)", kit.0) }
                    else if let InstrumentKind::Multisample { map } = &instrument.kind { format!("melodic map M{} (Map on Apply replaces it)", map.0) }
                    else if let Some(id) = instrument.region { format!("region {}", id.0) }
                    else if instrument.sample.is_some() { "legacy sample path".into() } else { "unassigned".into() }
                });
                ui.label(RichText::new(format!("Instrument {} currently: {mapping}", self.instrument)).small().color(MUTED));
                self.playback_controls(ui, writable && self.draft_active);
                let (start,stop)=self.audition.controls(ui,"Audition draft",self.draft_active&&!self.busy());
                if start {self.audition_action=Some(match self.audition_request(){Ok(request)=>crate::audition_ui::Action::Start(request),Err(error)=>crate::audition_ui::Action::Invalid(error)});}
                if stop {self.audition_action=Some(crate::audition_ui::Action::Stop);}
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(writable && self.draft_active, egui::Button::new("Apply region")).clicked() { let result = self.apply(editor); self.result(result); }
                    if ui.add_enabled(writable && self.selected.is_some(), egui::Button::new("Remove region")).on_hover_text("Refuses while assigned; unmap first").clicked() {
                        let result = editor.remove_region(self.selected.as_ref().unwrap().0); self.result(result);
                    }
                    if ui.add_enabled(writable, egui::Button::new("Unmap instrument")).clicked() { let result = editor.assign_region(InstrumentId(self.instrument), None); self.result(result); }
                });
                if let Some(error) = &self.error { ui.label(RichText::new(format!("Apply: {error}")).small().color(Color32::LIGHT_RED)); }
                ui.label(RichText::new("[start, end): original source frames, not output frames. Root preserves native pitch; BPM schedules notes only. Linear resampling; optional forward loop/ADSR. No embedded loops or time-stretch.").small().color(MUTED));
            });
        });
        if let Some((id, region)) = select {
            self.draft_active = true;
            self.draft = draft_from(&region);
            self.selected = Some((id, region));
            self.notice = format!(
                "Committed region {} selected. Editing is draft-only until Apply region.",
                id.0
            );
        }
        self.suggestions(ui, editor, &asset, base, writable);
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).small().color(MUTED));
        }
        back
    }
    fn playback_controls(&mut self, ui: &mut egui::Ui, writable: bool) {
        let draft = &mut self.draft.playback;
        let status = match (draft.envelope, draft.looping) {
            (false, false) => "off / legacy behavior",
            (true, false) => "ADSR / one-shot",
            (false, true) => "legacy release / forward loop",
            (true, true) => "ADSR / forward loop",
        };
        let response = egui::CollapsingHeader::new(format!("Sampler playback · {status}"))
            .id_salt("region-playback")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(RichText::new("Shared region: Apply changes every instrument/pad using it. Audition draft is separate, read-only and never applies these settings.").small().color(PROPOSAL));
                ui.add_enabled_ui(writable, |ui| {
                    ui.checkbox(&mut draft.envelope, "Amplitude envelope");
                    ui.add_enabled_ui(draft.envelope, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            playback_text(ui, "Attack ms", &mut draft.attack, 48.0);
                            playback_text(ui, "Decay ms", &mut draft.decay, 48.0);
                            playback_text(ui, "Sustain %", &mut draft.sustain, 42.0);
                            playback_text(ui, "Release ms", &mut draft.release, 48.0);
                        });
                    });
                    ui.label(RichText::new("A/D: 0–10000 ms · S: 0–100% · R: 0–500 ms. Output time, independent of pitch/BPM; R=0 is abrupt and may click.").small().color(MUTED));
                    if ui.checkbox(&mut draft.looping, "Forward loop").changed()
                        && draft.looping && draft.loop_start.is_empty() && draft.loop_end.is_empty()
                    {
                        draft.loop_start = self.draft.start.to_string();
                        draft.loop_end = self.draft.end.to_string();
                    }
                    ui.add_enabled_ui(draft.looping, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            playback_text(ui, "Loop start", &mut draft.loop_start, 105.0);
                            playback_text(ui, "Loop end", &mut draft.loop_end, 105.0);
                        });
                    });
                    ui.label(RichText::new(format!("SOURCE frames [start, end), end exclusive; at least 2 frames inside region [{}, {}).", self.draft.start, self.draft.end)).small().color(MUTED));
                });
                if draft.looping {
                    ui.label(RichText::new("Prefix once, then loop through release; no post-loop tail/crossfade. Endpoints may click. Loop voices do not support offset/reverse; playback/export checks existing events.").small().color(PROPOSAL));
                }
                let details = egui::CollapsingHeader::new("Playback semantics")
                    .id_salt("region-playback-semantics")
                    .show(ui, |ui| {
                        ui.label(RichText::new("Envelope off preserves legacy voice gain and exponential de-click. Enabled ADSR is linear amplitude at the fixed 48 kHz output clock; sustain is a level, not a duration. Ordinary OFF respects pad OneShot/Gated policy; timed release always initiates the region release (or legacy de-click), even for OneShot. Choke, pad replacement and track mute use a fixed 50 ms safety release with ADSR enabled; hard cut remains immediate. A loop does not sustain beyond the song's stop policy. Apply validates model bounds; it does not decode audio or prove the arrangement is renderable.").small().color(MUTED));
                    });
                if details.header_response.clicked() {
                    details.header_response.request_focus();
                }
                if let Err(error) = draft.parse() {
                    ui.label(RichText::new(error).small().color(PROPOSAL));
                }
            });
        if response.header_response.clicked() {
            response.header_response.request_focus();
        }
    }
    fn markers_saved(&self) -> bool {
        self.marker_selected
            .as_ref()
            .is_some_and(|(_, source)| self.marker_draft.as_ref() == Some(source))
    }
    fn prepared_current(&self, editor: &Editor, prepared: &PreparedCompile) -> bool {
        self.markers_saved()
            && self.compile_config.as_ref() == Some(prepared.plan.config())
            && editor.revision() == prepared.revision
            && &editor.song == prepared.plan.base()
    }
    fn finish_verified_commit(&mut self, editor: &mut Editor, enabled: bool) {
        let Some(prepared) = self.verified.take() else {
            return;
        };
        let allowed = self.commit_authorized
            && enabled
            && editor.edit_mode
            && self.workflow
            && self.sequence_view;
        self.commit_authorized = false;
        if !allowed {
            self.notice = "Commit canceled because the editor was blocked or the sequence view was left. Nothing was applied.".into();
            return;
        }
        if !self.prepared_current(editor, &prepared) {
            self.error = Some(
                "Preview is stale: document, markers or options changed. Preview again.".into(),
            );
            return;
        }
        match editor.commit_preview(prepared.revision, &prepared.plan) {
            Ok(()) => {
                self.committed = Some(prepared.plan.summary().clone());
                self.compiled = None;
                self.notice = "Sequence committed as ordinary editable cells (one undo). Open generated pattern to inspect, edit, save or export.".into();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn sequence_controls(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        asset: &SampleAsset,
        base: Option<&Path>,
    ) {
        let Some((id, source)) = self.marker_selected.clone() else {
            ui.label("Save an accepted marker set in Markers first. A preview never silently accepts suggestions or registers source metadata.");
            return;
        };
        if self
            .compile_config
            .as_ref()
            .is_none_or(|config| config.marker_set != id)
        {
            self.compile_config = Some(CompileConfig {
                marker_set: id,
                target_bpm: editor.song.bpm,
                timing: CompileTiming::SourceRhythm,
                cycles: 1,
                roll: None,
                tail: TailPolicy::Overlap,
                minimum_slice_frames: u64::from(asset.sample_rate).div_ceil(200).min(asset.frames),
                root_note: 60,
                pad_tails: true,
                name: "Break sequence".into(),
            });
            self.compiled = None;
            self.committed = None;
            self.cleanup = None;
        }
        let saved = self.markers_saved();
        let writable = ui.is_enabled() && editor.edit_mode && !self.busy();
        ui.label(
            RichText::new(format!(
                "SEQUENCE FROM M{} / {} · {} slices",
                id.0,
                source.name,
                source.markers.len().saturating_sub(1)
            ))
            .strong()
            .color(AQUA),
        );
        ui.label(RichText::new(match source.source_bpm { Some(bpm) => format!("Source BPM {bpm:.3} supplied manually · source beat 0 = frame 0 · native pitch, no time-stretch"), None => "Source BPM UNKNOWN. Declare it explicitly in Markers; nothing is inferred.".into() }).small().color(if source.source_bpm.is_some() { MUTED } else { PROPOSAL }));
        if !saved {
            ui.colored_label(
                PROPOSAL,
                "Marker draft differs from the saved set. Save markers before Preview/Commit.",
            );
        }
        let busy = self.busy();
        let config = self.compile_config.as_mut().unwrap();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut config.name)
                        .desired_width(170.0)
                        .hint_text("Generated pattern name"),
                );
                ui.label("Target SONG BPM");
                ui.add(
                    egui::DragValue::new(&mut config.target_bpm)
                        .range(20.0..=400.0)
                        .speed(0.25)
                        .max_decimals(3),
                );
                ui.label(format!("LPB {} unchanged", editor.song.lines_per_beat));
                ui.label("Cycles");
                ui.add(egui::DragValue::new(&mut config.cycles).range(1..=16));
            });
            ui.horizontal_wrapped(|ui| {
                let source_rhythm = matches!(config.timing, CompileTiming::SourceRhythm);
                if ui
                    .selectable_label(source_rhythm, "Source rhythm")
                    .clicked()
                {
                    config.timing = CompileTiming::SourceRhythm;
                }
                if ui.selectable_label(!source_rhythm, "Even grid").clicked() && source_rhythm {
                    config.timing = CompileTiming::Grid {
                        division: Subdivision::Sixteenth,
                    };
                }
                if let CompileTiming::Grid { division } = &mut config.timing {
                    division_control(ui, "compile-grid", division);
                }
                ui.label("Root MIDI");
                ui.add(egui::DragValue::new(&mut config.root_note).range(0..=127));
                let mut roll = config.roll.is_some();
                if ui.checkbox(&mut roll, "Roll each slice").changed() {
                    config.roll = roll.then_some(CompileRoll {
                        division: Subdivision::SixtyFourth,
                        hits: 2,
                    });
                }
                if let Some(roll) = &mut config.roll {
                    division_control(ui, "compile-roll", &mut roll.division);
                    ui.add(egui::DragValue::new(&mut roll.hits).range(2..=16));
                    ui.label("total hits");
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Tail policy");
                ui.selectable_value(&mut config.tail, TailPolicy::Overlap, "Overlap");
                ui.selectable_value(
                    &mut config.tail,
                    TailPolicy::ReleasePrevious,
                    "Release previous",
                );
                ui.selectable_value(&mut config.tail, TailPolicy::CutPrevious, "Cut previous");
                ui.checkbox(&mut config.pad_tails, "Pad final native tails");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Minimum slice");
                ui.add(
                    egui::DragValue::new(&mut config.minimum_slice_frames)
                        .range(1..=asset.frames)
                        .speed(1.0),
                );
                ui.label("source frames");
                if ui.button("Review short boundaries").clicked() {
                    let markers = self.marker_draft.as_ref().unwrap().markers.clone();
                    match break_compiler::suggest_marker_cleanup(
                        asset,
                        &markers,
                        config.minimum_slice_frames,
                    ) {
                        Ok(cleanup) => self.cleanup = Some((markers, cleanup)),
                        Err(error) => self.error = Some(error),
                    }
                }
            });
        });
        ui.label(RichText::new("Preview allocates one NEW track and pattern, plus a region and instrument per slice, then appends one order entry. Commit changes GLOBAL Song BPM: existing patterns are retimed too. LPB is not changed.").small().color(PROPOSAL));
        ui.label(RichText::new("Roll spacing is exact musical subdivision; hits must fit before the next slice. Cut may click; release uses the existing fade. With padding OFF, pattern/song-end release can shorten final native tails.").small().color(MUTED));
        if let Some((original, cleanup)) = &self.cleanup {
            let current = self
                .marker_draft
                .as_ref()
                .is_some_and(|set| &set.markers == original)
                && cleanup.minimum_slice_frames
                    == self.compile_config.as_ref().unwrap().minimum_slice_frames;
            let locks_preserved = self.marker_draft.as_ref().is_some_and(|set| {
                set.locked_frames
                    .iter()
                    .all(|frame| cleanup.markers.contains(frame))
            });
            if !locks_preserved {
                ui.colored_label(PROPOSAL, "Cleanup would remove a LOCKED boundary. Unlock explicitly, retain it manually, or lower the minimum slice length.");
            }
            ui.horizontal(|ui| {
                ui.label(format!("Cleanup proposal: remove {} boundaries, keep {}", cleanup.removed.len(), cleanup.markers.len()));
                if ui.add_enabled(writable && current && locks_preserved, egui::Button::new("Use cleanup draft")).clicked() {
                    self.marker_draft.as_mut().unwrap().markers = cleanup.markers.clone();
                    self.marker_index = 0; self.sequence_view = false;
                    self.notice = "Cleanup copied to marker DRAFT only. Review and Save markers before previewing again.".into();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("cleanup-review")
                .max_height(50.0)
                .show(ui, |ui| {
                    for removed in &cleanup.removed {
                        ui.monospace(format!(
                            "{} source frames · {:?}",
                            removed.frame, removed.reason
                        ));
                    }
                });
            if !current {
                ui.label(
                    RichText::new("Cleanup proposal is stale; review again.")
                        .small()
                        .color(PROPOSAL),
                );
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.busy() && saved && source.source_bpm.is_some(),
                    egui::Button::new("Preview sequence"),
                )
                .clicked()
            {
                let result = self.start(Request::Compile {
                    song: Box::new(editor.song.clone()),
                    revision: editor.revision(),
                    config: self.compile_config.as_ref().unwrap().clone(),
                    base: base.map(Path::to_path_buf),
                });
                self.compiled = None;
                self.committed = None;
                self.result(result);
            }
            if let Some(prepared) = &self.compiled {
                let current = self.prepared_current(editor, prepared);
                if ui
                    .add_enabled(writable && current, egui::Button::new("Commit sequence"))
                    .clicked()
                {
                    let prepared = prepared.clone();
                    let result = self.start(Request::Verify(prepared));
                    self.commit_authorized = result.is_ok();
                    self.result(result);
                }
                if !current {
                    ui.colored_label(PROPOSAL, "STALE PREVIEW — prepare again");
                }
            }
        });
        if let Some(prepared) = &self.compiled {
            let summary = prepared.plan.summary();
            ui.label(
                RichText::new("PREVIEW READY · no document changes")
                    .strong()
                    .color(AQUA),
            );
            ui.monospace(format!(
                "{} slices -> {} regions / {} instruments · {} starts / {} columns",
                summary.slice_count,
                summary.regions.len(),
                summary.instruments.len(),
                summary.note_starts,
                summary.note_columns
            ));
            ui.monospace(format!(
                "Pattern {} · track {} · append POS {} · {} rows ({} sequence + {} tail)",
                summary.pattern.0,
                summary.track.0,
                summary.occurrence,
                summary.pattern_rows,
                summary.sequence_rows,
                summary.tail_padding_rows
            ));
            ui.label(format!(
                "Song {:.3} -> {:.3} BPM; source {:.3} BPM",
                summary.previous_bpm, summary.target_bpm, summary.source_bpm
            ));
            for warning in &summary.warnings {
                ui.label(RichText::new(warning).small().color(PROPOSAL));
            }
        }
        if let Some(summary) = &self.committed {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "COMMITTED · P{} / T{} · {} editable starts",
                        summary.pattern.0, summary.track.0, summary.note_starts
                    ))
                    .strong()
                    .color(AQUA),
                );
                let exists = editor
                    .song
                    .order
                    .get(summary.occurrence)
                    .is_some_and(|order| order.pattern == summary.pattern)
                    && editor.song.tracks.contains_key(&summary.track);
                if ui
                    .add_enabled(exists, egui::Button::new("Open generated pattern"))
                    .clicked()
                {
                    self.generated_position = Some((summary.occurrence, summary.track));
                }
            });
        }
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).small().color(MUTED));
        }
    }
    fn new_markers(&mut self, editor: &Editor) {
        if let Some(preview) = &self.preview {
            let next = next_id(editor.song.marker_sets.keys().map(|id| id.0)).unwrap_or(1);
            self.marker_selected = None;
            self.marker_draft = Some(MarkerSet {
                name: format!("Markers {next}"),
                sample: preview.source.unwrap_or(SampleId(0)),
                source_bpm: None,
                locked_frames: vec![],
                markers: vec![0, preview.asset.frames],
            });
            self.marker_index = 0;
            self.marker_frame = preview.asset.frames / 2;
        }
    }
    fn save_markers(&mut self, editor: &mut Editor) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the source task to finish".into());
        }
        let preview = self.preview.as_ref().ok_or("Load a source first")?;
        let mut set = self
            .marker_draft
            .clone()
            .ok_or("Choose New markers first")?;
        let mut candidate = editor.song.clone();
        let source = if let Some(id) = preview.source {
            if candidate.samples.get(&id) != Some(&preview.asset) {
                return Err("Source changed; reload before saving markers".into());
            }
            id
        } else if let Some((id, _)) = candidate
            .samples
            .iter()
            .find(|(_, asset)| *asset == &preview.asset)
        {
            *id
        } else {
            let id = SampleId(next_id(candidate.samples.keys().map(|id| id.0))?);
            candidate.register_sample(id, preview.asset.clone())?;
            id
        };
        let id = if let Some((id, original)) = &self.marker_selected {
            if candidate.marker_sets.get(id) != Some(original) {
                return Err("Marker set changed; reselect before saving".into());
            }
            *id
        } else {
            MarkerSetId(next_id(candidate.marker_sets.keys().map(|id| id.0))?)
        };
        set.sample = source;
        candidate.upsert_marker_set(id, set.clone())?;
        editor.apply_document_edit(candidate)?;
        self.preview.as_mut().unwrap().source = Some(source);
        self.source_choice = Some(source);
        if self
            .cleanup
            .as_ref()
            .is_some_and(|(_, cleanup)| cleanup.markers == set.markers)
        {
            self.cleanup = None;
        }
        self.marker_selected = Some((id, set.clone()));
        self.marker_draft = Some(set);
        self.notice =
            "Markers and source metadata saved (one undo). Original PCM is unchanged.".into();
        Ok(())
    }
    fn insert_marker(&mut self, frame: u64) -> Result<(), String> {
        let set = self
            .marker_draft
            .as_mut()
            .ok_or("Choose New markers first")?;
        let end = *set.markers.last().ok_or("Marker endpoints missing")?;
        if frame > end {
            return Err("Marker must be inside the source".into());
        }
        if let Ok(index) = set.markers.binary_search(&frame) {
            self.marker_index = index;
            return Ok(());
        }
        if set.markers.len() >= 257 {
            return Err("At most 257 boundaries / 256 slices".into());
        }
        set.markers.push(frame);
        set.markers.sort_unstable();
        self.marker_index = set.markers.binary_search(&frame).unwrap();
        self.rejected_suggestions.remove(&frame);
        Ok(())
    }
    fn move_marker(&mut self) -> Result<(), String> {
        let set = self.marker_draft.as_mut().ok_or("Choose a marker set")?;
        if self.marker_index == 0 || self.marker_index + 1 >= set.markers.len() {
            return Err("Source endpoints are fixed".into());
        }
        if set.locked_frames.contains(&set.markers[self.marker_index]) {
            return Err("Unlock this saved boundary explicitly before moving it".into());
        }
        let frame = self.marker_frame;
        if frame == 0 || frame >= *set.markers.last().unwrap() {
            return Err("Interior marker must be between the fixed endpoints".into());
        }
        if set.markers[self.marker_index] == frame {
            return Ok(());
        }
        if set.markers.contains(&frame) {
            return Err("A boundary already exists at this frame".into());
        }
        set.markers[self.marker_index] = frame;
        set.markers.sort_unstable();
        self.marker_index = set.markers.binary_search(&frame).unwrap();
        Ok(())
    }
    fn remove_marker_draft(&mut self) -> Result<(), String> {
        let set = self.marker_draft.as_mut().ok_or("Choose a marker set")?;
        if self.marker_index == 0 || self.marker_index + 1 >= set.markers.len() {
            return Err("Source endpoints are fixed".into());
        }
        if set.locked_frames.contains(&set.markers[self.marker_index]) {
            return Err("Unlock this saved boundary explicitly before removing it".into());
        }
        set.markers.remove(self.marker_index);
        self.marker_index = 0;
        Ok(())
    }
    fn toggle_marker_lock(&mut self, editor: &mut Editor) -> Result<(), String> {
        if !self.markers_saved() {
            return Err("Save pending marker edits before changing a persistent lock".into());
        }
        let (id, original) = self
            .marker_selected
            .as_ref()
            .ok_or("Save the marker set first")?;
        if editor.song.marker_sets.get(id) != Some(original) {
            return Err("Marker set changed; reselect before locking".into());
        }
        let frame = *original
            .markers
            .get(self.marker_index)
            .ok_or("Select a boundary")?;
        let id = *id;
        editor.set_marker_lock(id, frame, !original.locked_frames.contains(&frame))?;
        let saved = editor.song.marker_sets[&id].clone();
        self.marker_selected = Some((id, saved.clone()));
        self.marker_draft = Some(saved);
        self.notice = "Persistent boundary lock saved (one undo). Detector proposals cannot change accepted boundaries.".into();
        Ok(())
    }
    fn marker_controls(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        asset: &SampleAsset,
        base: Option<&Path>,
    ) {
        let writable = ui.is_enabled() && editor.edit_mode && !self.busy();
        let bank_available = self.markers_saved() && !self.busy();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("PERSISTED SOURCE MARKERS")
                    .strong()
                    .color(AQUA),
            );
            let label = self
                .marker_selected
                .as_ref()
                .map_or("New marker draft".into(), |(id, set)| {
                    format!("M{} · {}", id.0, set.name)
                });
            egui::ComboBox::from_id_salt("marker-set-choice")
                .width(190.0)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (id, set) in &editor.song.marker_sets {
                        if Some(set.sample)
                            != self.preview.as_ref().and_then(|preview| preview.source)
                        {
                            continue;
                        }
                        if ui
                            .selectable_label(
                                self.marker_selected
                                    .as_ref()
                                    .is_some_and(|(selected, _)| selected == id),
                                format!("M{} · {}", id.0, set.name),
                            )
                            .clicked()
                        {
                            self.marker_selected = Some((*id, set.clone()));
                            self.marker_draft = Some(set.clone());
                            self.marker_index = 0;
                        }
                    }
                });
            if ui
                .add_enabled(!self.busy(), egui::Button::new("New markers"))
                .clicked()
            {
                self.new_markers(editor);
            }
            if ui.add_enabled(bank_available, egui::Button::new("Build pad bank…"))
                .on_hover_text("Review 1–16 saved adjacent marker intervals as a new kit; no source BPM required, no pattern edits, no automatic audition.")
                .clicked()
            {
                if let Some((id, set)) = &self.marker_selected {
                    self.marker_bank.open(*id, set.sample);
                }
            }
            if ui
                .add_enabled(
                    writable && self.marker_draft.is_some(),
                    egui::Button::new("Save markers"),
                )
                .clicked()
            {
                let result = self.save_markers(editor);
                self.result(result);
            }
            if ui
                .add_enabled(
                    writable
                        && self
                            .marker_selected
                            .as_ref()
                            .is_some_and(|(_, set)| set.locked_frames.is_empty()),
                    egui::Button::new("Remove set"),
                )
                .clicked()
            {
                let result = editor.remove_marker_set(self.marker_selected.as_ref().unwrap().0);
                self.result(result);
                self.sync(editor);
            }
        });
        let Some(set) = &mut self.marker_draft else {
            ui.label("Choose New markers or an existing set. Deleted drafts are never recreated automatically.");
            return;
        };
        ui.add_enabled_ui(writable, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut set.name)
                        .desired_width(170.0)
                        .hint_text("Marker set name"),
                );
                let mut known = set.source_bpm.is_some();
                if ui.checkbox(&mut known, "Source BPM supplied").changed() {
                    set.source_bpm = known.then_some(70.0);
                }
                if let Some(bpm) = &mut set.source_bpm {
                    ui.add(
                        egui::DragValue::new(bpm)
                            .range(20.0..=400.0)
                            .speed(0.25)
                            .max_decimals(3),
                    );
                } else {
                    ui.label(RichText::new("UNKNOWN").color(PROPOSAL));
                }
                ui.label("Source beat 0 = frame 0");
            });
        });
        ui.label(RichText::new("Source BPM is a manual declaration, never inferred from duration/onsets. Endpoints 0 and source end are fixed. Edits/Accept are pending until Save markers; Undo then restores the whole set.").small().color(MUTED));
        let mut marker_choice = None;
        let markers = set.markers.clone();
        let locked_frames = set.locked_frames.clone();
        let selected_locked = markers
            .get(self.marker_index)
            .is_some_and(|frame| locked_frames.contains(frame));
        let changed = self
            .marker_selected
            .as_ref()
            .is_none_or(|(_, original)| original != set);
        ui.label(
            RichText::new(format!(
                "{} boundaries / {} slices · {}",
                markers.len(),
                markers.len().saturating_sub(1),
                if changed {
                    "UNSAVED MARKER DRAFT"
                } else {
                    "saved marker set"
                }
            ))
            .small()
            .color(if changed { PROPOSAL } else { MUTED }),
        );
        ui.horizontal_top(|ui| {
            ui.set_max_height(115.0);
            egui::ScrollArea::vertical().id_salt("accepted-marker-list").max_height(105.0).max_width(210.0).show(ui, |ui| {
                ui.vertical(|ui| {
                    for (index, frame) in markers.iter().enumerate() {
                        if ui.selectable_label(self.marker_index == index, RichText::new(format!("{:03}  {:>10}{}", index, frame, if index == 0 || index + 1 == markers.len() { " fixed" } else if locked_frames.contains(frame) { " LOCK" } else { "" })).monospace()).clicked() { marker_choice = Some((index, *frame)); }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.add_enabled_ui(writable, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("SOURCE frame"); ui.add(egui::DragValue::new(&mut self.marker_frame).range(0..=asset.frames).speed(1.0));
                        if ui.button("Add boundary").clicked() { let result = self.insert_marker(self.marker_frame); self.result(result); }
                        let interior = self.marker_index > 0 && self.marker_index + 1 < markers.len();
                        if ui.add_enabled(interior && !selected_locked, egui::Button::new("Move selected")).clicked() { let result = self.move_marker(); self.result(result); }
                        if ui.add_enabled(interior && !selected_locked, egui::Button::new("Remove selected")).clicked() { let result = self.remove_marker_draft(); self.result(result); }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    let interior = self.marker_index > 0 && self.marker_index + 1 < markers.len();
                    if ui.add_enabled(writable && interior && !changed, egui::Button::new(if selected_locked { "Unlock selected" } else { "Lock selected" })).clicked() { let result = self.toggle_marker_lock(editor); self.result(result); }
                    ui.label(RichText::new("Locks persist immediately / one undo. Save pending edits first; endpoints stay fixed.").small().color(MUTED));
                });
                ui.label(RichText::new("Click waveform to choose a frame, then Add/Move. Aqua = accepted/manual draft; amber = analysis proposals; white = current insertion position. Short prefixes/slices need review, not blind acceptance.").small().color(MUTED));
            });
        });
        if let Some((index, frame)) = marker_choice {
            self.marker_index = index;
            self.marker_frame = frame;
        }
        ui.separator();
        self.marker_suggestions(ui, editor, asset, base, writable);
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).small().color(MUTED));
        }
    }
    fn analysis_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ONSET ENGINE").small().color(MUTED));
            ui.selectable_value(&mut self.experimental.enabled, false, "v1 · default");
            ui.selectable_value(&mut self.experimental.enabled, true, "v2 · experimental");
        });
    }
    fn experimental_current(&self, editor: &Editor, prepared: &ExperimentalResult) -> bool {
        self.preview
            .as_ref()
            .is_some_and(|preview| preview.asset == prepared.asset)
            && editor.revision() == prepared.revision
            && self.experimental.config == prepared.result.analysis.detection.provenance.config
    }
    fn start_experimental(
        &mut self,
        editor: &Editor,
        base: Option<&Path>,
        reuse: bool,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the current source task".into());
        }
        let preview = self.preview.as_ref().ok_or("Load a source first")?;
        let asset = preview.asset.clone();
        let source_path = resolve_source(&asset, base);
        let cache = if reuse {
            Some(
                self.experimental
                    .cache
                    .as_ref()
                    .ok_or("Prepare experimental cache first")?
                    .clone(),
            )
        } else {
            None
        };
        self.tempo.invalidate_cached();
        self.experimental.result = None;
        if !reuse {
            self.experimental.cache = None;
        }
        self.start(Request::Experimental(Box::new(ExperimentalRequest {
            asset,
            source_path,
            revision: editor.revision(),
            config: self.experimental.config.clone(),
            cache,
        })))
    }
    fn experimental_suggestions(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        asset: &SampleAsset,
        base: Option<&Path>,
        writable: bool,
    ) {
        ui.label(RichText::new("EXPERIMENTAL · not a default upgrade. Vibrato, chirps and beating tones can produce false positives; weak dense hits can still be missed. GhostHunter means sensitivity, not drum classification.").small().color(PROPOSAL));
        let busy = self.busy();
        let supported = (crate::break_analysis::MIN_SOURCE_RATE
            ..=crate::break_analysis::MAX_SOURCE_RATE)
            .contains(&asset.sample_rate)
            && asset.frames
                <= u64::from(asset.sample_rate) * crate::break_analysis::MAX_DURATION_SECONDS;
        if !supported {
            ui.colored_label(PROPOSAL, "Experimental analysis requires mono/stereo 8–384 kHz and at most 300 seconds. Manual markers remain available.");
        }
        ui.add_enabled_ui(!busy && supported, |ui| {
            ui.horizontal_wrapped(|ui| {
                let old = self.experimental.config.preset;
                let mut preset = old;
                egui::ComboBox::from_id_salt("experimental-onset-preset").width(115.0).selected_text(format!("{preset:?}")).show_ui(ui, |ui| {
                    for choice in [Preset::Conservative, Preset::Balanced, Preset::GhostHunter] { ui.selectable_value(&mut preset, choice, format!("{choice:?}")); }
                });
                if preset != old { self.experimental.config = onset_service::experimental_config(preset); }
                ui.label("Threshold");
                ui.add(egui::DragValue::new(&mut self.experimental.config.threshold).range(0.1..=0.95).speed(0.01)).on_hover_text("Lower = more sensitive. Cached repick uses the same raw evidence; no new FFT.");
                ui.label("MAD x");
                ui.add(egui::DragValue::new(&mut self.experimental.config.mad_multiplier).range(0.5..=12.0).speed(0.05));
                ui.checkbox(&mut self.experimental.config.enable_phase, "Phase support").on_hover_text("OFF in application presets. Optional supporting evidence, never a standalone trigger; may add false positives.");
                ui.checkbox(&mut self.experimental.config.refinement.enabled, "Refine boundaries");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Soft gap ms");
                let hard = self.experimental.config.dense_gap_ms;
                ui.add(egui::DragValue::new(&mut self.experimental.config.min_gap_ms).range(hard..=1000.0).speed(1.0));
                ui.label("Hard gap ms");
                let soft = self.experimental.config.min_gap_ms;
                ui.add(egui::DragValue::new(&mut self.experimental.config.dense_gap_ms).range(1.0..=soft).speed(0.5));
                let cached = self.experimental.cache.is_some();
                if ui.button(if cached { "Repick cached" } else { "Analyze v2" }).clicked() { let result = self.start_experimental(editor, base, cached); self.result(result); }
                if cached && ui.button("Rebuild cache").clicked() { let result = self.start_experimental(editor, base, false); self.result(result); }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_suggestions, "Overlay markers");
            ui.checkbox(
                &mut self.experimental.show_peaks,
                RichText::new("Coarse peaks").color(Color32::from_rgb(137, 156, 220)),
            );
            ui.checkbox(
                &mut self.experimental.show_attacks,
                RichText::new("Boundary estimates").color(PROPOSAL),
            );
            ui.label(
                RichText::new(format!(
                    "Phase {} · base policy {}",
                    if self.experimental.config.enable_phase {
                        "ON (optional)"
                    } else {
                        "OFF"
                    },
                    onset_service::POLICY
                ))
                .small()
                .color(MUTED),
            );
        });
        if let Some(cache) = &self.experimental.cache {
            ui.label(RichText::new(format!("One SHA-bound cache · {:.2} MiB / 96 MiB maximum · cached repick does no FFT/decoding; identity verification stays on worker", cache.retained_bytes() as f64 / (1024.0*1024.0))).small().color(MUTED));
        }
        let Some(prepared) = self.experimental.result.clone() else {
            return;
        };
        let current = self.experimental_current(editor, &prepared);
        let available = writable && current && !self.busy();
        if !current {
            ui.colored_label(PROPOSAL, "STALE ANALYSIS: document, source or settings changed. Repick before accepting proposals.");
        }
        let analysis = &prepared.result.analysis;
        ui.label(
            RichText::new(format!(
                "{} · {} proposals · {} refinement collisions · {}",
                prepared.result.binding.algorithm,
                analysis.suggestions.len(),
                analysis.refinement_collisions,
                if prepared.reused {
                    "CACHED REPICK / no FFT"
                } else {
                    "NEW EXTRACTION"
                }
            ))
            .small()
            .color(AQUA),
        );
        ui.label(RichText::new("Confidence is a heuristic score, NOT probability. Peak and boundary are different estimates. Accept/Use copies an estimate to a draft; Save/Apply is still explicit. Detector runs never replace accepted markers. Reject is a session proposal decision, not deletion of a saved boundary.").small().color(MUTED));
        if self.workflow {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(available, egui::Button::new("Accept unrejected boundaries")).clicked() {
                    let mut frames = self.marker_draft.as_ref().unwrap().markers.clone();
                    frames.extend(analysis.suggestions.iter().filter(|onset| !self.rejected_suggestions.contains(&onset.attack_frame)).map(|onset| onset.attack_frame));
                    frames.sort_unstable(); frames.dedup();
                    if frames.len() > 257 { self.error = Some("At most 257 boundaries; no proposals were silently truncated. Accept individually or repick with a larger gap.".into()); }
                    else { self.marker_draft.as_mut().unwrap().markers = frames; }
                }
                if ui.button("Clear rejections").clicked() { self.rejected_suggestions.clear(); }
            });
        }
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical().id_salt("experimental-onset-list").max_height(110.0).show_rows(ui, row_height, analysis.suggestions.len(), |ui, rows| {
            for index in rows {
                let onset = &analysis.suggestions[index];
                let frame = onset.attack_frame;
                let present = self.marker_draft.as_ref().is_some_and(|set| set.markers.contains(&frame));
                let rejected = self.rejected_suggestions.contains(&frame);
                ui.horizontal(|ui| {
                    if ui.add_enabled(available && (!self.workflow || !present), egui::Button::new(if self.workflow { "Accept" } else { "Use" })).clicked() {
                        if self.workflow { let result = self.insert_marker(frame); self.result(result); }
                        else {
                            self.selected = None; self.draft_active = true;
                            self.draft = RegionDraft { name: format!("V2 boundary {}", index+1), start: frame, end: analysis.suggestions.get(index+1).map_or(asset.frames, |next| next.attack_frame), root: self.draft.root, playback: PlaybackDraft::default() };
                            self.notice = "Experimental boundary copied into a NEW region draft, not committed. Review before Apply.".into();
                        }
                    }
                    if ui.add_enabled(available && !rejected, egui::Button::new("Reject")).clicked() { self.rejected_suggestions.insert(frame); }
                    if ui.selectable_label(self.experimental.selected == index, RichText::new(format!("peak {:>8}  boundary {:>8}  score {:.2}  {}", onset.candidate.peak_frame, frame, onset.candidate.confidence, if present { "accepted" } else if rejected { "rejected" } else { "proposal" })).monospace()).clicked() { self.experimental.selected = index; }
                });
            }
        });
        let Some(onset) = analysis.suggestions.get(self.experimental.selected) else {
            return;
        };
        let candidate = &onset.candidate;
        ui.monospace(format!(
            "SOURCE frames: peak {} -> boundary {} · shift {:+} ({:+.3} ms) · search [{}, {})",
            candidate.peak_frame,
            onset.attack_frame,
            onset.refinement_shift,
            onset.refinement_shift as f64 * 1000.0 / f64::from(asset.sample_rate),
            onset.search_start,
            onset.search_end
        ));
        ui.label(RichText::new(format!("Votes flux/energy/phase/fast: {:?} · heuristic confidence {:.4} · raw strength {:.3e}", candidate.detector_votes, candidate.confidence, candidate.strength)).small().color(MUTED));
        let hop = prepared.cache.provenance().fusion_hop_frames as u64;
        let raw = prepared
            .cache
            .raw_curves()
            .get((candidate.peak_frame / hop) as usize);
        ui.horizontal_wrapped(|ui| {
            ui.toggle_value(&mut self.experimental.show_curves, "Novelty curves");
            if self.experimental.show_curves {
                egui::ComboBox::from_id_salt("novelty-curve-metric").width(80.0).selected_text(["Raw flux", "Raw energy", "Raw phase"][self.experimental.curve_metric as usize]).show_ui(ui, |ui| {
                    for (index, name) in ["Raw flux", "Raw energy", "Raw phase"].iter().enumerate() { ui.selectable_value(&mut self.experimental.curve_metric, index as u8, *name); }
                });
                egui::ComboBox::from_id_salt("novelty-curve-span").width(90.0).selected_text(format!("{} hops", self.experimental.curve_points)).show_ui(ui, |ui| {
                    for points in [16,64,256,1024] { ui.selectable_value(&mut self.experimental.curve_points, points, format!("{points} hops")); }
                });
                ui.label(RichText::new("Separate band scales; raw evidence, not confidence. Spectrogram not implemented.").small().color(MUTED));
            }
        });
        if current && self.experimental.show_curves {
            novelty_curves(
                ui,
                &prepared.cache,
                onset,
                self.experimental.curve_metric,
                self.experimental.curve_points,
                self.show_suggestions && self.experimental.show_peaks,
                self.show_suggestions && self.experimental.show_attacks,
            );
        }
        ui.label(RichText::new("BAND NOVELTY AT COARSE PEAK · raw flux/energy vs normalized evidence; neither is a probability").small().color(MUTED));
        for (index, band) in analysis
            .detection
            .provenance
            .effective_bands
            .iter()
            .enumerate()
        {
            let evidence = candidate.evidence.bands[index];
            let raw = raw.map(|frame| frame.bands[index]).unwrap_or_default();
            ui.monospace(format!(
                "{:>5.0}–{:>5.0} Hz  raw F {:.2e} / E {:.2e}   normalized F {:.3} E {:.3} P {:.3}",
                band.low_hz,
                band.high_hz,
                raw.flux,
                raw.energy,
                evidence.flux,
                evidence.energy,
                evidence.phase
            ));
        }
        ui.label(
            RichText::new(format!(
                "Source SHA {}… · config {}… · source beat/BPM are NOT detected",
                &prepared.result.binding.source_sha256[..16],
                &prepared.result.config_sha256[..16]
            ))
            .small()
            .color(MUTED),
        );
    }
    fn marker_suggestions(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        asset: &SampleAsset,
        base: Option<&Path>,
        writable: bool,
    ) {
        self.analysis_selector(ui);
        if self.experimental.enabled {
            self.experimental_suggestions(ui, editor, asset, base, writable);
            return;
        }
        let supported = asset.sample_rate >= break_analysis::MIN_SOURCE_RATE
            && asset.frames <= u64::from(asset.sample_rate) * break_analysis::MAX_DURATION_SECONDS;
        ui.horizontal_wrapped(|ui| {
            ui.label("Onset sensitivity");
            ui.add(
                egui::DragValue::new(&mut self.config.sensitivity)
                    .range(0.0..=1.0)
                    .speed(0.01),
            );
            ui.label("Spacing ms");
            ui.add(egui::DragValue::new(&mut self.config.min_spacing_ms).range(1.0..=1000.0));
            if ui
                .add_enabled(
                    !self.busy() && supported,
                    egui::Button::new("Analyze markers"),
                )
                .clicked()
            {
                let preview = self.preview.as_ref().unwrap();
                let mut config = self.config.clone();
                config.waveform_bins = 0;
                let result = self.start(Request::Analyze {
                    asset: preview.asset.clone(),
                    buffer: Arc::clone(&preview.buffer),
                    config,
                });
                self.result(result);
            }
            ui.checkbox(&mut self.show_suggestions, "Show proposals");
        });
        ui.label(RichText::new("Energy / first-difference suggestions, not spectral flux or BPM detection. Reject only dismisses a proposal; it never removes manual/accepted boundaries. Reanalysis never overwrites your marker draft.").small().color(MUTED));
        let Some(analysis) = &self.analysis else {
            return;
        };
        let proposals: Vec<_> = analysis
            .suggestions
            .iter()
            .map(|onset| (onset.frame, onset.confidence))
            .collect();
        ui.horizontal(|ui| {
            if ui.add_enabled(writable, egui::Button::new("Accept unrejected")).clicked() {
                let mut frames = self.marker_draft.as_ref().unwrap().markers.clone();
                frames.extend(proposals.iter().filter(|(frame,_)| !self.rejected_suggestions.contains(frame)).map(|(frame,_)| *frame)); frames.sort_unstable(); frames.dedup();
                if frames.len() > 257 { self.error = Some("At most 257 boundaries. Accept individual proposals or increase analysis spacing.".into()); }
                else { self.marker_draft.as_mut().unwrap().markers = frames; }
            }
            if ui.button("Clear rejections").clicked() { self.rejected_suggestions.clear(); }
            ui.label(format!("{} proposals", proposals.len()));
        });
        egui::ScrollArea::vertical()
            .id_salt("marker-proposals")
            .max_height(100.0)
            .show(ui, |ui| {
                for (frame, confidence) in proposals {
                    let present = self.marker_draft.as_ref().unwrap().markers.contains(&frame);
                    let rejected = self.rejected_suggestions.contains(&frame);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(writable && !present, egui::Button::new("Accept"))
                            .clicked()
                        {
                            let result = self.insert_marker(frame);
                            self.result(result);
                        }
                        if ui
                            .add_enabled(writable && !rejected, egui::Button::new("Reject"))
                            .clicked()
                        {
                            self.rejected_suggestions.insert(frame);
                        }
                        ui.monospace(format!(
                            "{:>10} frames · margin {:.2} · {}",
                            frame,
                            confidence,
                            if present {
                                "boundary exists"
                            } else if rejected {
                                "rejected"
                            } else {
                                "proposal"
                            }
                        ));
                    });
                }
            });
    }
    fn waveform(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        let preview = self.preview.as_ref().unwrap();
        let height = if self.workflow {
            100.0
        } else if ui.available_height() < 450.0 {
            110.0
        } else {
            160.0
        };
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(15, 23, 28));
        let frames = preview.asset.frames;
        let at = |frame: u64| rect.left() + rect.width() * (frame as f64 / frames as f64) as f32;
        let pending = Rect::from_min_max(
            egui::pos2(at(self.draft.start.min(frames)), rect.top()),
            egui::pos2(at(self.draft.end.min(frames)), rect.bottom()),
        );
        if !self.workflow && self.draft.start < self.draft.end {
            painter.rect_filled(pending, 0.0, Color32::from_rgb(27, 48, 51));
        }
        for channel in 0..preview.asset.channels as usize {
            let lane = rect.height() / f32::from(preview.asset.channels);
            let center = rect.top() + lane * (channel as f32 + 0.5);
            painter.hline(
                rect.x_range(),
                center,
                Stroke::new(1.0_f32, Color32::from_rgb(39, 56, 65)),
            );
            for bin in &preview.waveform {
                let x = at((bin.start_frame + bin.end_frame) / 2);
                let range = bin.channels[channel];
                painter.vline(
                    x,
                    (center - range.max.clamp(-1.0, 1.0) * lane * 0.42)
                        ..=(center - range.min.clamp(-1.0, 1.0) * lane * 0.42),
                    Stroke::new(1.0_f32, AQUA),
                );
            }
        }
        for region in editor
            .song
            .regions
            .values()
            .filter(|region| Some(region.sample) == preview.source)
        {
            for frame in [region.start_frame, region.end_frame] {
                painter.vline(at(frame), rect.y_range(), Stroke::new(1.0_f32, MUTED));
            }
        }
        if self.show_suggestions && self.experimental.enabled {
            if let Some(prepared) = self
                .experimental
                .result
                .as_ref()
                .filter(|prepared| self.experimental_current(editor, prepared))
            {
                for onset in &prepared.result.analysis.suggestions {
                    if self.experimental.show_peaks {
                        painter.vline(
                            at(onset.candidate.peak_frame),
                            rect.y_range(),
                            Stroke::new(1.0_f32, Color32::from_rgb(137, 156, 220)),
                        );
                    }
                    if self.experimental.show_attacks {
                        painter.vline(
                            at(onset.attack_frame),
                            rect.y_range(),
                            Stroke::new(1.0_f32, PROPOSAL),
                        );
                    }
                }
            }
        }
        if self.show_suggestions && !self.experimental.enabled {
            if let Some(analysis) = &self.analysis {
                for onset in &analysis.suggestions {
                    painter.vline(
                        at(onset.frame),
                        rect.y_range(),
                        Stroke::new(1.0_f32, PROPOSAL),
                    );
                }
            }
        }
        if self.workflow {
            if let Some(set) = &self.marker_draft {
                for (index, frame) in set.markers.iter().enumerate() {
                    painter.vline(
                        at(*frame),
                        rect.y_range(),
                        Stroke::new(
                            if index == self.marker_index {
                                2.0_f32
                            } else {
                                1.0_f32
                            },
                            AQUA,
                        ),
                    );
                }
            }
            painter.vline(
                at(self.marker_frame.min(frames)),
                rect.y_range(),
                Stroke::new(1.0_f32, Color32::WHITE),
            );
        }
        for (frame, label) in [(self.draft.start, "START"), (self.draft.end, "END")] {
            if self.workflow {
                break;
            }
            painter.vline(
                at(frame.min(frames)),
                rect.y_range(),
                Stroke::new(2.0_f32, AQUA),
            );
            let x = if rect.width() >= 48.0 {
                at(frame.min(frames)).clamp(rect.left() + 24.0, rect.right() - 24.0)
            } else {
                rect.center().x
            };
            painter.text(
                egui::pos2(x, rect.top() + 4.0),
                Align2::CENTER_TOP,
                label,
                FontId::monospace(10.0),
                AQUA,
            );
        }
        if ui.is_enabled()
            && editor.edit_mode
            && !self.busy()
            && (response.clicked() || response.dragged())
        {
            if let Some(position) = response.interact_pointer_pos() {
                let frame = (((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64
                    * frames as f64)
                    .round() as u64;
                if self.workflow {
                    self.marker_frame = frame;
                    if let Some(set) = &self.marker_draft {
                        if let Some(index) = set
                            .markers
                            .iter()
                            .position(|existing| (at(*existing) - position.x).abs() < 5.0)
                        {
                            self.marker_index = index;
                            self.marker_frame = set.markers[index];
                        }
                    }
                } else if ui.input(|input| input.modifiers.shift) {
                    self.draft.end = frame.max(self.draft.start.saturating_add(1)).min(frames);
                } else {
                    self.draft.start = frame.min(self.draft.end.saturating_sub(1));
                }
            }
        }
        response.on_hover_text(if self.experimental.enabled { "Violet = coarse spectral/energy peak; amber = estimated boundary. They are distinct source-frame estimates, not tempo detection. Aqua accepted/manual boundaries are authoritative; stale experimental overlays are hidden. Use exact numeric fields and review before Save/Apply." } else if self.workflow { "Click/drag selects a source-frame position. Add/Move changes the marker draft; Save markers persists it. Source frame0 is fixed beat0. Aqua boundaries are accepted/manual; amber are uncommitted suggestions." } else { "Click/drag: draft start. Shift-click/drag: draft end. Numeric fields give exact source frames. Aqua = draft, gray = committed boundaries, amber = suggestions. Apply is required." });
    }
    fn suggestions(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        asset: &SampleAsset,
        base: Option<&Path>,
        writable: bool,
    ) {
        let supported = asset.sample_rate >= break_analysis::MIN_SOURCE_RATE
            && asset.frames <= u64::from(asset.sample_rate) * break_analysis::MAX_DURATION_SECONDS;
        egui::CollapsingHeader::new("Transient suggestions · never automatic edits").default_open(false).show(ui, |ui| {
            self.analysis_selector(ui);
            if self.experimental.enabled { self.experimental_suggestions(ui, editor, asset, base, writable); return; }
            ui.horizontal_wrapped(|ui| {
                ui.label("Sensitivity"); ui.add(egui::DragValue::new(&mut self.config.sensitivity).range(0.0..=1.0).speed(0.01));
                ui.label("Min spacing"); ui.add(egui::DragValue::new(&mut self.config.min_spacing_ms).range(1.0..=1000.0).speed(1.0)); ui.label("ms");
                if ui.add_enabled(!self.busy() && supported, egui::Button::new("Suggest onsets")).clicked() {
                    let preview = self.preview.as_ref().unwrap();
                    let mut config = self.config.clone(); config.waveform_bins = 0;
                    let result = self.start(Request::Analyze { asset: preview.asset.clone(), buffer: Arc::clone(&preview.buffer), config }); self.result(result);
                }
                ui.checkbox(&mut self.show_suggestions, "Show amber markers");
            });
            ui.label(RichText::new("Adaptive energy / first-difference detector, not spectral flux, BPM, instrument classification, or calibrated probability. Use copies one proposal into a NEW draft; it never replaces a committed region.").small().color(MUTED));
            if !supported { ui.label("Analysis needs rate 8–384 kHz and duration at most 300 s. Manual regions remain available."); }
            if let Some(analysis) = &self.analysis {
                ui.label(format!("{} suggestions · applied sensitivity {:.2} / spacing >= {} source frames", analysis.suggestions.len(), analysis.provenance.config.sensitivity, analysis.provenance.min_spacing_frames));
                let mut proposal = None;
                egui::ScrollArea::vertical().id_salt("onset-suggestions").max_height(85.0).show(ui, |ui| {
                    for (index, onset) in analysis.suggestions.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(writable, egui::Button::new("Use")).clicked() { proposal = Some(index); }
                            ui.monospace(format!("{:>9} frames  / {:>8.3} s  / margin {:.2}", onset.frame, onset.frame as f64 / f64::from(asset.sample_rate), onset.confidence));
                        });
                    }
                });
                if let Some(index) = proposal {
                    self.selected = None;
                    self.draft_active = true;
                    self.draft = RegionDraft { name: format!("Onset {}", index + 1), start: analysis.suggestions[index].frame, end: analysis.suggestions.get(index + 1).map_or(asset.frames, |next| next.frame), root: self.draft.root, playback: PlaybackDraft::default() };
                    self.notice = "Proposal copied to a new uncommitted draft. Review boundaries, then Apply.".into();
                }
            }
        });
    }
}
fn novelty_curves(
    ui: &mut egui::Ui,
    cache: &CachedSource,
    onset: &crate::onset_v2::Suggestion,
    metric: u8,
    span_points: usize,
    show_peak: bool,
    show_boundary: bool,
) {
    let curves = cache.raw_curves();
    if curves.is_empty() {
        return;
    }
    let hop = cache.provenance().fusion_hop_frames as u64;
    let center = (onset.candidate.peak_frame / hop) as usize;
    let span_points = span_points.clamp(16, 1024);
    let start = center.saturating_sub(span_points / 2);
    let end = (start + span_points + 1).min(curves.len());
    let first_frame = start as u64 * hop;
    let end_frame = ((end as u64) * hop).min(cache.binding().source_frames);
    ui.label(RichText::new(format!("CURVE WINDOW [{first_frame}, {end_frame}) SOURCE frames · violet peak / amber boundary · {} cached points per band", end-start)).small().color(MUTED));
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 132.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(15, 23, 28));
    let plot_left = rect.left() + 86.0;
    let width = (rect.right() - plot_left - 4.0).max(1.0);
    let span = (end_frame - first_frame).max(1) as f64;
    let at =
        |frame: u64| plot_left + (frame.saturating_sub(first_frame) as f64 / span) as f32 * width;
    for band in 0..3 {
        let top = rect.top() + band as f32 * 44.0;
        let baseline = top + 40.0;
        let value = |frame: &crate::onset_v2::EnvelopeFrame| match metric {
            1 => frame.bands[band].energy,
            2 => frame.bands[band].phase,
            _ => frame.bands[band].flux,
        };
        let maximum = curves[start..end].iter().map(value).fold(0.0_f64, f64::max);
        let scale = maximum.max(f64::MIN_POSITIVE);
        let points: Vec<_> = curves[start..end]
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                egui::pos2(
                    at((start + index) as u64 * hop),
                    baseline - (value(frame) / scale) as f32 * 30.0,
                )
            })
            .collect();
        painter.hline(
            plot_left..=rect.right(),
            baseline,
            Stroke::new(1.0_f32, Color32::from_rgb(39, 56, 65)),
        );
        painter.add(egui::Shape::line(points, Stroke::new(1.0_f32, AQUA)));
        let effective = &cache.provenance().effective_bands[band];
        painter.text(
            egui::pos2(rect.left() + 4.0, top + 7.0),
            Align2::LEFT_TOP,
            format!(
                "{}–{} Hz",
                effective.low_hz as u32, effective.high_hz as u32
            ),
            FontId::monospace(10.0),
            MUTED,
        );
        painter.text(
            egui::pos2(rect.left() + 4.0, top + 22.0),
            Align2::LEFT_TOP,
            format!("max {maximum:.1e}"),
            FontId::monospace(10.0),
            MUTED,
        );
    }
    for (frame, color, visible) in [
        (
            onset.candidate.peak_frame,
            Color32::from_rgb(137, 156, 220),
            show_peak,
        ),
        (onset.attack_frame, PROPOSAL, show_boundary),
    ] {
        if visible && (first_frame..end_frame).contains(&frame) {
            painter.vline(at(frame), rect.y_range(), Stroke::new(1.0_f32, color));
        }
    }
}

fn division_control(ui: &mut egui::Ui, id: &'static str, value: &mut Subdivision) {
    egui::ComboBox::from_id_salt(id)
        .width(66.0)
        .selected_text(format!("1/{}", value.denominator()))
        .show_ui(ui, |ui| {
            for division in [
                Subdivision::Sixteenth,
                Subdivision::ThirtySecond,
                Subdivision::SixtyFourth,
                Subdivision::OneTwentyEighth,
            ] {
                ui.selectable_value(value, division, format!("1/{}", division.denominator()));
            }
        });
}
fn draft_from(region: &SampleRegion) -> RegionDraft {
    RegionDraft {
        name: region.name.clone(),
        start: region.start_frame,
        end: region.end_frame,
        root: region.root_note,
        playback: PlaybackDraft::from_region(region),
    }
}
fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, String> {
    ids.max()
        .map_or(Some(1), |id| id.checked_add(1))
        .ok_or_else(|| "ID space exhausted".into())
}
fn execute(request: Request) -> Result<Completed, String> {
    match request {
        Request::MarkerBank(request) => {
            marker_bank_ui::execute(*request).map(|result| Completed::MarkerBank(Box::new(result)))
        }
        Request::Catalog(request) => {
            slice_features_ui::execute(*request).map(|result| Completed::Catalog(Box::new(result)))
        }
        Request::Tempo(request) => {
            tempo_ui::execute(*request).map(|result| Completed::Tempo(Box::new(result)))
        }
        Request::Experimental(request) => {
            let ExperimentalRequest {
                asset,
                source_path,
                revision,
                config,
                cache,
            } = *request;
            let reused = cache.is_some();
            let (cache, result) = if let Some(cache) = cache {
                sample_asset::verify_identity(&asset, &source_path)?;
                let result = onset_service::repick(&cache, &asset, &config)?;
                (cache, result)
            } else {
                let prepared = onset_service::prepare_verified(&asset, &source_path, &config)?;
                (prepared.cache, prepared.result)
            };
            Ok(Completed::Experimental(Box::new(ExperimentalResult {
                asset,
                revision,
                cache,
                result,
                reused,
            })))
        }
        Request::Compile {
            song,
            revision,
            config,
            base,
        } => {
            let plan = break_compiler::preview(&song, &config)?;
            let source_path = resolve_source(plan.source(), base.as_deref());
            sample_asset::verify_identity(plan.source(), &source_path)?;
            Ok(Completed::Compiled(PreparedCompile {
                revision,
                plan: Arc::new(plan),
                source_path,
            }))
        }
        Request::Verify(prepared) => {
            sample_asset::verify_identity(prepared.plan.source(), &prepared.source_path)?;
            Ok(Completed::Verified(prepared))
        }
        Request::Load { path, expected } => {
            let (asset, buffer, source) = if let Some((id, asset)) = expected {
                let buffer =
                    sample_asset::load_verified(&asset, &path, sample_asset::MAX_SAMPLE_VALUES)?;
                (asset, buffer, Some(id))
            } else {
                let path = crate::project::absolute(&path)?;
                let imported = sample_asset::import(&path)?;
                (imported.asset, imported.buffer, None)
            };
            let real = asset.frames as usize * asset.channels as usize;
            let waveform = overview(&buffer.buffer()[..real], asset.channels as usize, 1024);
            Ok(Completed::Loaded(Preview {
                source,
                asset,
                buffer,
                waveform,
            }))
        }
        Request::Analyze {
            asset,
            buffer,
            config,
        } => {
            let real = asset.frames as usize * asset.channels as usize;
            break_analysis::analyze_transients(
                &buffer.buffer()[..real],
                asset.channels as usize,
                asset.sample_rate,
                &config,
            )
            .map(Completed::Analyzed)
            .map_err(|error| error.to_string())
        }
    }
}
fn resolve_source(asset: &SampleAsset, base: Option<&Path>) -> PathBuf {
    if asset.path.is_relative() {
        base.map_or(asset.path.clone(), |base| base.join(&asset.path))
    } else {
        asset.path.clone()
    }
}
fn overview(pcm: &[f32], channels: usize, bins: usize) -> Vec<WaveformBin> {
    let frames = pcm.len() / channels;
    let count = bins.min(frames);
    (0..count)
        .map(|index| {
            let start = index * frames / count;
            let end = (index + 1) * frames / count;
            let channels = (0..channels)
                .map(|channel| {
                    let mut min = f32::INFINITY;
                    let mut max = f32::NEG_INFINITY;
                    for frame in start..end {
                        let value = pcm[frame * channels + channel];
                        min = min.min(value);
                        max = max.max(value);
                    }
                    ChannelExtrema { min, max }
                })
                .collect();
            WaveformBin {
                start_frame: start as u64,
                end_frame: end as u64,
                channels,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Note;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    fn fixture() -> Fixture {
        let path = std::env::temp_dir().join(format!(
            "leviathan-source-ui-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let mut writer = hound::WavWriter::new(
            file,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..4800 {
            let local = frame % 1200;
            let sample = if (240..440).contains(&local) {
                ((local - 240) as f32 * 0.25).sin() * 0.8
            } else {
                0.0
            };
            writer.write_sample(sample).unwrap();
            writer.write_sample(-sample).unwrap();
        }
        writer.finalize().unwrap();
        Fixture(path)
    }
    fn experimental_fixture() -> Fixture {
        let fixture = fixture();
        let mut writer = hound::WavWriter::create(
            &fixture.0,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        let mut seed = 0x12345678_u32;
        for frame in 0..48_000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let local = frame % 12_000;
            let value = if (1000..3000).contains(&local) {
                (seed as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
                    * 0.8
                    * (1.0 - (local - 1000) as f32 / 2000.0)
            } else {
                0.0
            };
            writer.write_sample(value).unwrap();
            writer.write_sample(-value).unwrap();
        }
        writer.finalize().unwrap();
        fixture
    }
    fn loaded(path: &Path) -> SampleUi {
        let mut sample = SampleUi::default();
        sample
            .start(Request::Load {
                path: path.to_owned(),
                expected: None,
            })
            .unwrap();
        settle(&mut sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        assert!(sample.preview.is_some());
        sample
    }
    fn settle(sample: &mut SampleUi) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while sample.busy() && Instant::now() < deadline {
            sample.poll();
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!sample.busy(), "Source worker timed out");
    }
    fn frame(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                modifiers: events
                    .iter()
                    .rev()
                    .find_map(|event| match event {
                        egui::Event::PointerButton { modifiers, .. }
                        | egui::Event::Key { modifiers, .. } => Some(*modifiers),
                        _ => None,
                    })
                    .unwrap_or_default(),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("test-inspector")
                            .show(ui, |ui| {
                                sample.show(ui, editor, None);
                            });
                    });
                });
            },
        )
    }
    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }
    fn scroll_to_control(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        label: &str,
    ) -> egui::Pos2 {
        for _ in 0..80 {
            let output = frame(ctx, sample, editor, vec![], true);
            if let Some(pos) = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    let rect = text.visual_bounding_rect();
                    shape.clip_rect.contains_rect(rect).then_some(rect.center())
                }
                _ => None,
            }) {
                return pos;
            }
            frame(
                ctx,
                sample,
                editor,
                vec![
                    egui::Event::PointerMoved(egui::pos2(850.0, 400.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -40.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        panic!("Control not reachable after actual scrolling: {label}")
    }
    fn text(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    let pos = text.pos + text.galley.size() * 0.5;
                    shape.clip_rect.contains(pos).then_some(pos)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Visible control missing: {label}"))
    }
    fn click(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        pos: egui::Pos2,
        modifiers: egui::Modifiers,
        enabled: bool,
    ) {
        for pressed in [true, false] {
            frame(
                ctx,
                sample,
                editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
                enabled,
            );
        }
    }
    #[test]
    fn playback_draft_roundtrips_exact_optional_values_and_rejects_invalid_integer_text() {
        let mut region = SampleRegion {
            name: "Exact playback".into(),
            sample: SampleId(1),
            start_frame: 0,
            end_frame: u64::MAX,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        };
        assert_eq!(
            PlaybackDraft::from_region(&region).parse().unwrap(),
            (None, None)
        );
        region.amp_envelope = Some(AmpEnvelope {
            attack_ms: 10_000,
            decay_ms: 0,
            sustain_percent: 37,
            release_ms: 500,
        });
        region.forward_loop = Some(ForwardLoop {
            start_frame: u64::MAX - 2,
            end_frame: u64::MAX,
        });
        let draft = PlaybackDraft::from_region(&region);
        assert_eq!(
            draft.parse().unwrap(),
            (region.amp_envelope, region.forward_loop)
        );
        for bad in ["-1", "NaN", "1.5", "1e2", "18446744073709551616", ""] {
            let mut invalid = draft.clone();
            invalid.loop_start = bad.into();
            assert!(invalid.parse().is_err(), "{bad}");
            assert_eq!(invalid.loop_start, bad, "No clamping or normalization");
        }
        for (field, max) in [(0, 10000), (1, 10000), (2, 100), (3, 500)] {
            let mut invalid = draft.clone();
            let target = match field {
                0 => &mut invalid.attack,
                1 => &mut invalid.decay,
                2 => &mut invalid.sustain,
                _ => &mut invalid.release,
            };
            *target = (max + 1).to_string();
            assert!(invalid.parse().is_err());
        }
        let mut disabled = draft;
        disabled.envelope = false;
        disabled.looping = false;
        disabled.attack = "invalid unused draft".into();
        assert_eq!(disabled.parse().unwrap(), (None, None));
    }
    #[test]
    fn combined_region_playback_apply_is_one_undo_preserves_options_and_rejects_invalid_shrink() {
        let fixture = fixture();
        let bytes = fs::read(&fixture.0).unwrap();
        let mut sample = loaded(&fixture.0);
        sample.instrument = 1;
        let mut editor = Editor {
            edit_mode: true,
            row: 7,
            ..Default::default()
        };
        let before = editor.song.clone();
        sample.draft.playback = PlaybackDraft {
            envelope: true,
            attack: "7".into(),
            decay: "11".into(),
            sustain: "63".into(),
            release: "0".into(),
            looping: true,
            loop_start: "1200".into(),
            loop_end: "3600".into(),
        };
        sample.apply(&mut editor).unwrap();
        let committed = editor.song.clone();
        let region = &committed.regions[&RegionId(1)];
        assert_eq!(region.amp_envelope.unwrap().release_ms, 0);
        assert_eq!(region.forward_loop.unwrap().end_frame, 3600);
        assert_eq!(sample.draft, draft_from(region));
        editor.undo();
        assert_eq!(editor.song, before);
        assert_eq!(editor.row, 7);
        editor.redo();
        assert_eq!(editor.song, committed);
        sample.sync(&editor);
        sample.draft.end = 3000;
        let revision = editor.revision();
        assert!(sample.apply(&mut editor).is_err());
        assert_eq!(editor.song, committed);
        assert_eq!(editor.revision(), revision);
        // Both boundary and playback changes can be valid together, without an
        // invalid intermediate upsert or a second history record.
        sample.draft.playback.loop_end = "2900".into();
        sample.apply(&mut editor).unwrap();
        assert_eq!(editor.song.regions[&RegionId(1)].end_frame, 3000);
        editor.undo();
        assert_eq!(editor.song, committed);
        sample.sync(&editor);
        assert_eq!(sample.draft, draft_from(&committed.regions[&RegionId(1)]));
        sample.draft.name = "Renamed only".into();
        sample.apply(&mut editor).unwrap();
        assert_eq!(
            editor.song.regions[&RegionId(1)].amp_envelope,
            region.amp_envelope
        );
        assert_eq!(
            editor.song.regions[&RegionId(1)].forward_loop,
            region.forward_loop
        );
        assert_eq!(fs::read(&fixture.0).unwrap(), bytes);
    }
    fn playback_test_frame(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                modifiers: events
                    .iter()
                    .rev()
                    .find_map(|e| {
                        if let egui::Event::Key { modifiers, .. } = e {
                            Some(*modifiers)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default(),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        sample.playback_controls(ui, editor.edit_mode && ui.is_enabled());
                        if ui
                            .add_enabled(editor.edit_mode, egui::Button::new("Apply region"))
                            .clicked()
                        {
                            let result = sample.apply(editor);
                            sample.result(result);
                        }
                    });
                });
            },
        )
    }
    fn playback_test_click(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        pos: egui::Pos2,
        enabled: bool,
    ) {
        for pressed in [true, false] {
            playback_test_frame(
                ctx,
                sample,
                editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                enabled,
            );
        }
        for _ in 0..16 {
            playback_test_frame(ctx, sample, editor, vec![], enabled);
        }
    }
    fn playback_test_number(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        label: &str,
        value: &str,
    ) {
        let rect = ctx
            .read_response(egui::Id::new(("region-playback", label)))
            .unwrap()
            .rect;
        assert!(Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900.0, 620.0)).contains_rect(rect));
        playback_test_click(ctx, sample, editor, rect.center(), true);
        for pressed in [true, false] {
            playback_test_frame(
                ctx,
                sample,
                editor,
                vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                }],
                true,
            );
        }
        playback_test_frame(
            ctx,
            sample,
            editor,
            vec![egui::Event::Text(value.into())],
            true,
        );
        let output = playback_test_frame(ctx, sample, editor, vec![], true);
        assert!(
            texts(&output).iter().any(|text| text == value),
            "Requested integer must actually be painted: {value}"
        );
    }
    #[test]
    fn actual_playback_controls_enter_exact_values_apply_noop_and_invalid_is_atomic() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        sample.instrument = 1;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        let before = editor.song.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Sampler playback · off / legacy behavior"),
            true,
        );
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Amplitude envelope"),
            true,
        );
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Forward loop"),
            true,
        );
        for (label, value) in [
            ("Attack ms", "17"),
            ("Decay ms", "29"),
            ("Sustain %", "43"),
            ("Release ms", "0"),
            ("Loop start", "240"),
            ("Loop end", "440"),
        ] {
            playback_test_number(&ctx, &mut sample, &mut editor, label, value);
        }
        assert_eq!(
            editor.song, before,
            "Typing is draft only, not musical note entry"
        );
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Apply region"),
            true,
        );
        assert!(sample.error.is_none(), "{:?}", sample.error);
        let committed = editor.song.clone();
        assert_eq!(
            committed.regions[&RegionId(1)].amp_envelope,
            Some(AmpEnvelope {
                attack_ms: 17,
                decay_ms: 29,
                sustain_percent: 43,
                release_ms: 0
            })
        );
        assert_eq!(
            committed.regions[&RegionId(1)].forward_loop,
            Some(ForwardLoop {
                start_frame: 240,
                end_frame: 440
            })
        );
        let revision = editor.revision();
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Apply region"),
            true,
        );
        assert_eq!(
            editor.revision(),
            revision,
            "Unchanged Apply is exact no-op"
        );
        for (label, bad) in [
            ("Attack ms", "10001"),
            ("Sustain %", "100.000000001"),
            ("Release ms", "-1"),
            ("Loop start", "18446744073709551616"),
        ] {
            let saved = sample.draft.playback.clone();
            playback_test_number(&ctx, &mut sample, &mut editor, label, bad);
            let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
            playback_test_click(
                &ctx,
                &mut sample,
                &mut editor,
                text(&out, "Apply region"),
                true,
            );
            assert!(sample.error.is_some());
            assert_eq!(editor.song, committed);
            assert_eq!(editor.revision(), revision);
            sample.draft.playback = saved;
        }
        editor.undo();
        assert_eq!(editor.song, before);
    }
    #[test]
    fn playback_controls_read_mode_and_disabled_parent_cannot_change_or_apply_draft() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Sampler playback · off / legacy behavior"),
            true,
        );
        let before = editor.song.clone();
        let draft = sample.draft.clone();
        for (mode, enabled) in [(false, true), (true, false)] {
            editor.edit_mode = mode;
            let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], enabled);
            playback_test_click(
                &ctx,
                &mut sample,
                &mut editor,
                text(&out, "Amplitude envelope"),
                enabled,
            );
            playback_test_click(
                &ctx,
                &mut sample,
                &mut editor,
                text(&out, "Apply region"),
                enabled,
            );
            assert_eq!(sample.draft, draft);
            assert_eq!(editor.song, before);
        }
    }
    #[test]
    fn registered_source_and_region_selection_notice_and_release_help_are_truthful() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        sample.map_on_apply = false;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.apply(&mut editor).unwrap();
        let before = editor.song.clone();
        let revision = editor.revision();
        sample
            .start(Request::Load {
                path: fixture.0.clone(),
                expected: Some((SampleId(1), editor.song.samples[&SampleId(1)].clone())),
            })
            .unwrap();
        settle(&mut sample);
        assert_eq!(
            sample.notice,
            "Registered source verified; select a committed region or create a new draft."
        );
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for _ in 0..16 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "01  Region 1"),
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(
            sample.notice,
            "Committed region 1 selected. Editing is draft-only until Apply region."
        );
        assert!(!sample.notice.contains("register it"));
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Sampler playback · off / legacy behavior"),
            true,
        );
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        playback_test_click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Playback semantics"),
            true,
        );
        let out = playback_test_frame(&ctx, &mut sample, &mut editor, vec![], true);
        assert!(texts(&out).iter().any(|text| text.contains("timed release always initiates the region release (or legacy de-click), even for OneShot")));
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
    }
    #[test]
    fn worker_preview_excludes_guard_preserves_antiphase_channels_and_never_edits_document() {
        let fixture = fixture();
        let before = fs::read(&fixture.0).unwrap();
        let sample = loaded(&fixture.0);
        let preview = sample.preview.as_ref().unwrap();
        assert_eq!(preview.asset.frames, 4800);
        assert_eq!(preview.buffer.frame_count(), 4801);
        assert_eq!(preview.waveform.last().unwrap().end_frame, 4800);
        assert!(preview
            .waveform
            .iter()
            .any(|bin| bin.channels[0].max > 0.7 && bin.channels[1].min < -0.7));
        assert_eq!(fs::read(&fixture.0).unwrap(), before);
        assert!(sample.selected.is_none());
        assert!(preview.source.is_none());
    }
    #[test]
    fn real_waveform_boundary_edits_apply_source_region_mapping_as_one_undo() {
        let fixture = fixture();
        let before_bytes = fs::read(&fixture.0).unwrap();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            row: 7,
            ..Default::default()
        };
        let before = editor.song.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for _ in 0..16 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        let wave = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == Color32::from_rgb(15, 23, 28) => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .unwrap();
        click(
            &ctx,
            &mut sample,
            &mut editor,
            egui::pos2(wave.left() + wave.width() * 0.2, wave.center().y),
            egui::Modifiers::NONE,
            true,
        );
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        let wave = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == Color32::from_rgb(15, 23, 28) => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .unwrap();
        click(
            &ctx,
            &mut sample,
            &mut editor,
            egui::pos2(wave.left() + wave.width() * 0.6, wave.center().y),
            egui::Modifiers::SHIFT,
            true,
        );
        assert_eq!((sample.draft.start, sample.draft.end), (960, 2880));
        assert_eq!(editor.song, before, "Waveform changes are pending only");
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&output, "Apply region"),
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.error.is_none(), "{:?}", sample.error);
        assert_eq!(editor.song.samples.len(), 1);
        assert_eq!(editor.song.regions.len(), 1);
        assert_eq!(
            editor.song.instruments[&InstrumentId(1)].region,
            Some(RegionId(1))
        );
        assert_eq!(editor.song.regions[&RegionId(1)].start_frame, 960);
        assert_eq!(editor.row, 7);
        editor.undo();
        assert_eq!(editor.song, before);
        assert_eq!(editor.row, 7);
        sample.sync(&editor);
        assert!(
            sample.preview.is_none(),
            "Undo must invalidate stale registered preview"
        );
        assert_eq!(fs::read(&fixture.0).unwrap(), before_bytes);
    }
    #[test]
    fn invalid_region_and_mapping_reject_whole_batch_without_orphan_source() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.instrument = 1;
        sample.draft.start = 10;
        sample.draft.end = 5;
        let before = editor.song.clone();
        assert!(sample.apply(&mut editor).is_err());
        assert_eq!(editor.song, before);
        sample.draft.end = 50;
        sample.instrument = u64::MAX;
        assert!(sample.apply(&mut editor).is_err());
        assert_eq!(editor.song, before);
    }
    #[test]
    fn blocked_and_read_mode_inspector_cannot_apply_or_leak_musical_keys() {
        for blocked in [true, false] {
            let fixture = fixture();
            let mut sample = loaded(&fixture.0);
            let mut editor = Editor {
                edit_mode: blocked,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            tracker_ui::install_theme(&ctx);
            frame(&ctx, &mut sample, &mut editor, vec![], true);
            let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
            let before = editor.song.clone();
            ctx.memory_mut(|memory| memory.request_focus(tracker_ui::grid_id()));
            click(
                &ctx,
                &mut sample,
                &mut editor,
                text(&output, "Apply region"),
                egui::Modifiers::NONE,
                !blocked,
            );
            frame(
                &ctx,
                &mut sample,
                &mut editor,
                vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                !blocked,
            );
            assert_eq!(editor.song, before);
            assert!(!ctx.memory(|memory| memory.has_focus(tracker_ui::grid_id())));
        }
    }
    #[test]
    fn late_worker_result_after_new_open_is_discarded_without_new_preview() {
        let fixture = fixture();
        let result = execute(Request::Load {
            path: fixture.0.clone(),
            expected: None,
        });
        let (sender, receiver) = mpsc::channel();
        let mut sample = SampleUi {
            pending: Some(Pending {
                owner: WorkerOwner::General,
                epoch: 0,
                receiver,
            }),
            ..Default::default()
        };
        sample.reset_document();
        sender.send(result).ok().unwrap();
        sample.poll();
        assert!(!sample.busy());
        assert!(sample.preview.is_none());
        assert!(sample.error.is_none());
    }
    #[test]
    fn deleted_region_draft_cannot_silently_recreate_and_analysis_keeps_manual_boundaries() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            ..Default::default()
        };
        sample.map_on_apply = false;
        sample.draft.start = 240;
        sample.draft.end = 800;
        sample.apply(&mut editor).unwrap();
        let before = editor.song.clone();
        let draft = sample.draft.clone();
        let preview = sample.preview.as_ref().unwrap();
        sample
            .start(Request::Analyze {
                asset: preview.asset.clone(),
                buffer: Arc::clone(&preview.buffer),
                config: AnalysisConfig::default(),
            })
            .unwrap();
        settle(&mut sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        assert!(!sample.analysis.as_ref().unwrap().suggestions.is_empty());
        assert_eq!(sample.draft, draft);
        assert_eq!(editor.song, before);
        editor.remove_region(RegionId(1)).unwrap();
        sample.sync(&editor);
        assert!(sample.apply(&mut editor).is_err());
        assert!(editor.song.regions.is_empty());
    }
    #[test]
    fn inspected_name_text_and_suggestions_are_not_song_note_entry() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            ..Default::default()
        };
        editor.enter_note(Note::Note(64)).unwrap();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&output, "Region 1"),
            egui::Modifiers::NONE,
            true,
        );
        let before = editor.song.clone();
        frame(
            &ctx,
            &mut sample,
            &mut editor,
            vec![
                egui::Event::Text("Z".into()),
                egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        assert_eq!(editor.song, before);
        assert!(sample.draft.name.contains('Z'));
    }
    #[test]
    fn actual_remove_source_refuses_references_then_removes_only_unused_metadata() {
        let fixture = fixture();
        let bytes = fs::read(&fixture.0).unwrap();
        let mut sample = loaded(&fixture.0);
        sample.map_on_apply = false;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.apply(&mut editor).unwrap();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        let before = editor.song.clone();
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&output, "Remove source"),
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(editor.song, before);
        assert!(sample.error.is_some());
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&output, "Remove region"),
            egui::Modifiers::NONE,
            true,
        );
        assert!(editor.song.regions.is_empty());
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&output, "Remove source"),
            egui::Modifiers::NONE,
            true,
        );
        assert!(editor.song.samples.is_empty());
        assert!(sample.preview.is_none());
        assert_eq!(fs::read(&fixture.0).unwrap(), bytes);
        editor.undo();
        assert_eq!(editor.song.samples.len(), 1);
        assert!(editor.song.regions.is_empty());
    }
    #[test]
    fn actual_suggestion_use_creates_new_pending_draft_and_preserves_manual_region() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        sample.map_on_apply = false;
        sample.draft.start = 240;
        sample.draft.end = 800;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.apply(&mut editor).unwrap();
        let before = editor.song.clone();
        let manual = editor.song.regions[&RegionId(1)].clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for _ in 0..16 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let point = scroll_to_control(
            &ctx,
            &mut sample,
            &mut editor,
            "Transient suggestions · never automatic edits",
        );
        click(
            &ctx,
            &mut sample,
            &mut editor,
            point,
            egui::Modifiers::NONE,
            true,
        );
        for _ in 0..16 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let point = scroll_to_control(&ctx, &mut sample, &mut editor, "Suggest onsets");
        click(
            &ctx,
            &mut sample,
            &mut editor,
            point,
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.busy());
        settle(&mut sample);
        assert_eq!(editor.song, before);
        let expected = sample.analysis.as_ref().unwrap().suggestions[0].frame;
        frame(
            &ctx,
            &mut sample,
            &mut editor,
            vec![
                egui::Event::PointerMoved(egui::pos2(850.0, 400.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -250.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        for _ in 0..16 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let point = scroll_to_control(&ctx, &mut sample, &mut editor, "Use");
        click(
            &ctx,
            &mut sample,
            &mut editor,
            point,
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(editor.song, before);
        assert!(sample.selected.is_none());
        assert_eq!(sample.draft.start, expected);
        assert_eq!(sample.draft.name, "Onset 1");
        let output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&output, "Apply region"),
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(editor.song.regions.len(), 2);
        assert_eq!(editor.song.regions[&RegionId(1)], manual);
        editor.undo();
        assert_eq!(editor.song, before);
    }
    fn prepared_sample(fixture: &Fixture) -> (SampleUi, Editor) {
        let mut sample = loaded(&fixture.0);
        sample.workflow = true;
        sample.sequence_view = true;
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            row: 6,
            ..Default::default()
        };
        let set = sample.marker_draft.as_mut().unwrap();
        set.source_bpm = Some(70.0);
        set.markers = (0..=8).map(|n| n * 600).collect();
        sample.save_markers(&mut editor).unwrap();
        sample.compile_config = Some(CompileConfig {
            marker_set: sample.marker_selected.as_ref().unwrap().0,
            target_bpm: 170.0,
            timing: CompileTiming::SourceRhythm,
            cycles: 1,
            roll: None,
            tail: TailPolicy::Overlap,
            minimum_slice_frames: 1,
            root_note: 60,
            pad_tails: true,
            name: "Test break".into(),
        });
        (sample, editor)
    }
    fn compile_ready(sample: &mut SampleUi, editor: &Editor) {
        sample
            .start(Request::Compile {
                song: Box::new(editor.song.clone()),
                revision: editor.revision(),
                config: sample.compile_config.as_ref().unwrap().clone(),
                base: None,
            })
            .unwrap();
        settle(sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        assert!(sample.compiled.is_some());
    }
    fn find_control(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        for _ in 0..10 {
            let output = frame(ctx, sample, editor, vec![], true);
            if let Some(pos) = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    let pos = text.pos + text.galley.size() * 0.5;
                    shape.clip_rect.contains(pos).then_some(pos)
                }
                _ => None,
            }) {
                return pos;
            }
            frame(
                ctx,
                sample,
                editor,
                vec![
                    egui::Event::PointerMoved(egui::pos2(888.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if down { -130.0 } else { 130.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
            for _ in 0..4 {
                frame(ctx, sample, editor, vec![], true);
            }
        }
        panic!("Control not reachable by real scrolling: {label}");
    }
    #[test]
    fn marker_save_registers_source_atomically_unknown_bpm_stays_unknown_and_undo_removes_both() {
        let fixture = fixture();
        let bytes = fs::read(&fixture.0).unwrap();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        let before = editor.song.clone();
        assert!(sample.marker_draft.as_ref().unwrap().source_bpm.is_none());
        sample.insert_marker(240).unwrap();
        sample.insert_marker(1200).unwrap();
        assert_eq!(editor.song, before);
        sample.save_markers(&mut editor).unwrap();
        let set = &editor.song.marker_sets[&MarkerSetId(1)];
        assert_eq!(set.markers, vec![0, 240, 1200, 4800]);
        assert!(set.source_bpm.is_none());
        assert_eq!(editor.song.samples.len(), 1);
        assert!(editor.song.regions.is_empty());
        editor.undo();
        assert_eq!(editor.song, before);
        sample.sync(&editor);
        assert!(sample.marker_selected.is_none());
        assert_eq!(fs::read(&fixture.0).unwrap(), bytes);
    }
    #[test]
    fn real_marker_accept_reject_move_save_and_reanalysis_keep_manual_boundaries() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        sample.workflow = true;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for _ in 0..12 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let pos = find_control(&ctx, &mut sample, &mut editor, "Source BPM supplied", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(sample.marker_draft.as_ref().unwrap().source_bpm, Some(70.0));
        assert!(editor.song.samples.is_empty());
        let pos = find_control(&ctx, &mut sample, &mut editor, "Analyze markers", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        let proposals = sample.analysis.as_ref().unwrap().suggestions.clone();
        assert!(proposals.len() >= 2);
        let pos = find_control(&ctx, &mut sample, &mut editor, "Reject", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        let rejected = *sample.rejected_suggestions.first().unwrap();
        let pos = find_control(&ctx, &mut sample, &mut editor, "Accept unrejected", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(!sample
            .marker_draft
            .as_ref()
            .unwrap()
            .markers
            .contains(&rejected));
        let index = sample.marker_draft.as_ref().unwrap().markers.len() - 2;
        let old = sample.marker_draft.as_ref().unwrap().markers[index];
        sample.marker_index = index;
        sample.marker_frame = old + 3;
        let pos = find_control(&ctx, &mut sample, &mut editor, "Move selected", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample
            .marker_draft
            .as_ref()
            .unwrap()
            .markers
            .contains(&(old + 3)));
        let pos = find_control(&ctx, &mut sample, &mut editor, "Save markers", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        let before = editor.song.clone();
        let draft = sample.marker_draft.clone();
        let pos = find_control(&ctx, &mut sample, &mut editor, "Analyze markers", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert_eq!(sample.marker_draft, draft);
        assert_eq!(editor.song, before);
    }
    #[test]
    fn actual_preview_and_commit_allocate_eight_instruments_without_mutating_original_or_preview_base(
    ) {
        let fixture = fixture();
        let bytes = fs::read(&fixture.0).unwrap();
        let (mut sample, mut editor) = prepared_sample(&fixture);
        let before = editor.song.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for _ in 0..12 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let pos = find_control(&ctx, &mut sample, &mut editor, "Preview sequence", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        assert_eq!(editor.song, before);
        assert_eq!(
            sample
                .compiled
                .as_ref()
                .unwrap()
                .plan
                .summary()
                .instruments
                .len(),
            8
        );
        let pos = find_control(&ctx, &mut sample, &mut editor, "Commit sequence", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert_eq!(
            editor.song, before,
            "Worker verification alone never applies model changes"
        );
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        assert_eq!(editor.song.bpm, 170.0);
        assert_eq!(editor.song.lines_per_beat, before.lines_per_beat);
        assert_eq!(editor.song.instruments.len(), before.instruments.len() + 8);
        assert_eq!(editor.song.regions.len(), 8);
        assert_eq!(editor.song.order.len(), before.order.len() + 1);
        assert_eq!(editor.row, 6);
        assert_eq!(
            editor.song.marker_sets[&MarkerSetId(1)].source_bpm,
            Some(70.0)
        );
        editor.undo();
        assert_eq!(editor.song, before);
        assert_eq!(fs::read(&fixture.0).unwrap(), bytes);
    }
    #[test]
    fn verified_commit_refuses_revision_option_marker_mode_and_closing_staleness() {
        for variant in 0..6 {
            let fixture = fixture();
            let (mut sample, mut editor) = prepared_sample(&fixture);
            compile_ready(&mut sample, &editor);
            let prepared = sample.compiled.clone().unwrap();
            match variant {
                0 => {
                    editor.enter_note(Note::Note(66)).unwrap();
                    editor.undo();
                }
                1 => {
                    sample.compile_config.as_mut().unwrap().target_bpm = 171.0;
                }
                2 => {
                    sample.marker_draft.as_mut().unwrap().markers.insert(1, 16);
                }
                3 => {
                    sample.sequence_view = false;
                }
                4 => {
                    editor.edit_mode = false;
                }
                _ => (),
            }
            let before = editor.song.clone();
            sample.commit_authorized = true;
            sample.verified = Some(prepared);
            sample.finish_verified_commit(&mut editor, variant != 5);
            assert_eq!(
                editor.song, before,
                "Stale/blocked variant {variant} committed"
            );
            assert!(sample.verified.is_none());
            sample.finish_verified_commit(&mut editor, true);
            assert_eq!(editor.song, before, "Canceled commit must not re-arm");
        }
    }
    #[test]
    fn commit_verification_worker_rejects_external_source_change() {
        let fixture = fixture();
        let (mut sample, editor) = prepared_sample(&fixture);
        compile_ready(&mut sample, &editor);
        let prepared = sample.compiled.clone().unwrap();
        let mut bytes = fs::read(&fixture.0).unwrap();
        let n = bytes.len();
        bytes[n - 1] ^= 1;
        fs::write(&fixture.0, bytes).unwrap();
        sample.start(Request::Verify(prepared)).unwrap();
        sample.commit_authorized = true;
        settle(&mut sample);
        assert!(sample.error.as_ref().unwrap().contains("SHA-256"));
        assert!(sample.verified.is_none());
        assert!(!sample.commit_authorized);
    }
    #[test]
    fn cleanup_is_explicit_draft_only_and_pending_short_boundary_blocks_preview() {
        let fixture = fixture();
        let (mut sample, mut editor) = prepared_sample(&fixture);
        sample.sequence_view = false;
        sample.insert_marker(16).unwrap();
        sample.save_markers(&mut editor).unwrap();
        sample.sequence_view = true;
        sample.compile_config.as_mut().unwrap().minimum_slice_frames = 240;
        let before = editor.song.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for _ in 0..12 {
            frame(&ctx, &mut sample, &mut editor, vec![], true);
        }
        let pos = find_control(
            &ctx,
            &mut sample,
            &mut editor,
            "Review short boundaries",
            true,
        );
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(editor.song, before);
        assert!(sample.marker_draft.as_ref().unwrap().markers.contains(&16));
        let pos = find_control(&ctx, &mut sample, &mut editor, "Use cleanup draft", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(editor.song, before);
        assert!(!sample.marker_draft.as_ref().unwrap().markers.contains(&16));
        assert!(!sample.markers_saved());
        assert!(!sample.sequence_view);
        let pos = find_control(&ctx, &mut sample, &mut editor, "Save markers", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.markers_saved());
        assert!(
            sample.cleanup.is_none(),
            "Accepted saved cleanup is no longer a stale proposal"
        );
    }
    #[test]
    fn experimental_analysis_is_opt_in_cached_source_bound_and_service_identical() {
        let fixture = experimental_fixture();
        let mut sample = loaded(&fixture.0);
        let editor = Editor::default();
        assert!(!sample.experimental.enabled);
        assert!(!sample.experimental.config.enable_phase);
        let before = editor.song.clone();
        let revision = editor.revision();
        sample.start_experimental(&editor, None, false).unwrap();
        settle(&mut sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        let prepared = sample.experimental.result.as_ref().unwrap().clone();
        assert!(!prepared.result.analysis.suggestions.is_empty());
        assert_eq!(
            prepared.result,
            onset_service::repick(
                &prepared.cache,
                &prepared.asset,
                &sample.experimental.config
            )
            .unwrap()
        );
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
        sample.experimental.config.threshold = 0.4;
        sample.experimental.config.refinement.enabled = false;
        assert!(!sample.experimental_current(&editor, &prepared));
        sample.start_experimental(&editor, None, true).unwrap();
        assert!(sample.experimental.result.is_none());
        settle(&mut sample);
        let repicked = sample.experimental.result.as_ref().unwrap();
        assert!(repicked.reused);
        assert!(
            Arc::ptr_eq(&repicked.cache, &prepared.cache),
            "Repick must keep the original raw cache, not extract again"
        );
        assert_eq!(
            repicked.result,
            onset_service::repick(
                &repicked.cache,
                &repicked.asset,
                &sample.experimental.config
            )
            .unwrap()
        );
        assert!(repicked
            .result
            .analysis
            .suggestions
            .iter()
            .all(|onset| onset.attack_frame == onset.candidate.peak_frame));
        assert_eq!(editor.song, before);
    }
    #[test]
    fn experimental_result_rejects_document_aba_source_tamper_and_late_new_open() {
        let fixture = experimental_fixture();
        let bytes = fs::read(&fixture.0).unwrap();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.start_experimental(&editor, None, false).unwrap();
        settle(&mut sample);
        let prepared = sample.experimental.result.as_ref().unwrap().clone();
        let before = editor.song.clone();
        editor.enter_note(Note::Note(67)).unwrap();
        editor.undo();
        assert_eq!(editor.song, before);
        assert!(
            !sample.experimental_current(&editor, &prepared),
            "Edit/Undo must invalidate the old result"
        );
        let mut changed = bytes.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        fs::write(&fixture.0, changed).unwrap();
        sample.start_experimental(&editor, None, true).unwrap();
        settle(&mut sample);
        assert!(sample.experimental.result.is_none());
        assert!(sample.error.as_ref().unwrap().contains("SHA-256"));
        assert_eq!(editor.song, before);
        fs::write(&fixture.0, bytes).unwrap();
        sample.start_experimental(&editor, None, true).unwrap();
        sample.reset_document();
        settle(&mut sample);
        assert!(sample.experimental.result.is_none());
        assert!(sample.experimental.cache.is_none());
        assert!(!sample.experimental.enabled);
        assert!(sample.preview.is_none());
    }
    #[test]
    fn actual_experimental_controls_keep_manual_boundaries_and_show_cached_curves() {
        let fixture = experimental_fixture();
        let mut sample = loaded(&fixture.0);
        sample.workflow = true;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.insert_marker(1000).unwrap();
        sample.save_markers(&mut editor).unwrap();
        let manual = sample.marker_draft.clone();
        let song = editor.song.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        let pos = find_control(&ctx, &mut sample, &mut editor, "v2 · experimental", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.experimental.enabled);
        assert!(!sample.experimental.config.enable_phase);
        let pos = find_control(&ctx, &mut sample, &mut editor, "Analyze v2", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert_eq!(sample.marker_draft, manual);
        assert_eq!(editor.song, song);
        let first = sample
            .experimental
            .result
            .as_ref()
            .unwrap()
            .result
            .analysis
            .suggestions[0]
            .attack_frame;
        let pos = find_control(&ctx, &mut sample, &mut editor, "Reject", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.rejected_suggestions.contains(&first));
        assert_eq!(sample.marker_draft, manual);
        let pos = find_control(&ctx, &mut sample, &mut editor, "Refine boundaries", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(!sample.experimental.config.refinement.enabled);
        let pos = find_control(&ctx, &mut sample, &mut editor, "Repick cached", false);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert!(sample.experimental.result.as_ref().unwrap().reused);
        assert_eq!(sample.marker_draft, manual);
        assert_eq!(editor.song, song);
        assert!(sample.rejected_suggestions.contains(&first));
        let pos = find_control(&ctx, &mut sample, &mut editor, "Novelty curves", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(
            sample.experimental.show_curves,
            "Actual curve toggle must be enabled"
        );
        let mut output = frame(&ctx, &mut sample, &mut editor, vec![], true);
        for _ in 0..12 {
            if texts(&output)
                .iter()
                .any(|text| text.starts_with("CURVE WINDOW"))
            {
                break;
            }
            output = frame(
                &ctx,
                &mut sample,
                &mut editor,
                vec![
                    egui::Event::PointerMoved(egui::pos2(885.0, 410.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -90.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        assert!(
            texts(&output)
                .iter()
                .any(|text| text.starts_with("CURVE WINDOW")),
            "Actual cached curve plot must be reachable by scrolling"
        );
        assert!(
            sample.experimental.cache.as_ref().unwrap().retained_bytes()
                <= onset_service::MAX_RETAINED_BYTES
        );
        let pos = find_control(
            &ctx,
            &mut sample,
            &mut editor,
            "Accept unrejected boundaries",
            false,
        );
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample
            .marker_draft
            .as_ref()
            .unwrap()
            .markers
            .contains(&1000));
        assert_eq!(
            editor.song, song,
            "Accept is draft-only until explicit Save"
        );
        let draft = sample.marker_draft.clone();
        sample.experimental.config.threshold = 0.42;
        let pos = find_control(
            &ctx,
            &mut sample,
            &mut editor,
            "Accept unrejected boundaries",
            false,
        );
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(sample.marker_draft, draft, "Stale result must not accept");
    }
    #[test]
    fn persistent_lock_click_is_one_undo_and_blocks_move_remove_and_cleanup() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        sample.workflow = true;
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.insert_marker(16).unwrap();
        sample.save_markers(&mut editor).unwrap();
        let before = editor.song.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        let pos = find_control(&ctx, &mut sample, &mut editor, "Lock selected", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        let id = sample.marker_selected.as_ref().unwrap().0;
        assert_eq!(editor.song.marker_sets[&id].locked_frames, vec![16]);
        assert!(sample.markers_saved());
        sample.marker_frame = 20;
        assert!(sample.move_marker().is_err());
        assert!(sample.remove_marker_draft().is_err());
        sample.sequence_view = true;
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        let pos = find_control(
            &ctx,
            &mut sample,
            &mut editor,
            "Review short boundaries",
            true,
        );
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        let draft = sample.marker_draft.clone();
        let pos = find_control(&ctx, &mut sample, &mut editor, "Use cleanup draft", true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            pos,
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(sample.marker_draft, draft);
        editor.undo();
        sample.sync(&editor);
        assert_eq!(editor.song, before);
        assert!(sample
            .marker_draft
            .as_ref()
            .unwrap()
            .locked_frames
            .is_empty());
    }
    fn tempo_source_fixture() -> (Fixture, SampleUi, Editor) {
        let fixture = experimental_fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.instrument = 1;
        sample.apply(&mut editor).unwrap();
        let source = sample.preview.as_ref().unwrap().source.unwrap();
        editor
            .upsert_marker_set(
                MarkerSetId(1),
                MarkerSet {
                    name: "Tempo markers".into(),
                    sample: source,
                    source_bpm: None,
                    markers: vec![0, 1000, 13000, 25000, 37000, 48000],
                    locked_frames: vec![13000],
                },
            )
            .unwrap();
        (fixture, sample, editor)
    }
    #[test]
    fn actual_tempo_tab_keeps_region_draft_and_marker_analysis_is_nonmutating() {
        let (_fixture, mut sample, mut editor) = tempo_source_fixture();
        let base = editor.song.clone();
        let revision = editor.revision();
        let draft = sample.draft.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Tempo / pulse"),
            egui::Modifiers::NONE,
            true,
        );
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Choose accepted markers"),
            egui::Modifiers::NONE,
            true,
        );
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "1 · Tempo markers"),
            egui::Modifiers::NONE,
            true,
        );
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Analyze tempo / pulse"),
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.busy());
        settle(&mut sample);
        let snapshot = sample.capture_tempo_state(&editor);
        assert_eq!(snapshot["current"], true);
        assert!(snapshot["selected"].is_null());
        assert_eq!(snapshot["preview"]["analysis"]["evidence_count"], 4);
        assert_eq!(editor.song, base);
        assert_eq!(editor.revision(), revision);
        assert_eq!(sample.draft, draft);
        let marker_id = sample.capture_tempo_state(&editor)["preview"]["id"].clone();
        sample.start_experimental(&editor, None, false).unwrap();
        settle(&mut sample);
        assert_eq!(sample.capture_tempo_state(&editor)["current"], true);
        assert_eq!(
            sample.capture_tempo_state(&editor)["preview"]["id"],
            marker_id
        );
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Regions"),
            egui::Modifiers::NONE,
            true,
        );
        assert!(!sample.tempo_view);
        assert_eq!(sample.draft, draft);
        assert_eq!(editor.song, base);
    }
    #[test]
    fn cached_tempo_reuses_exact_arc_picker_and_new_or_tamper_cannot_apply_stale_evidence() {
        let (fixture, mut sample, mut editor) = tempo_source_fixture();
        sample.start_experimental(&editor, None, false).unwrap();
        settle(&mut sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        let cache = Arc::clone(sample.experimental.cache.as_ref().unwrap());
        let raw = serde_json::to_value(cache.curve_page(0, 8).unwrap()).unwrap();
        let base = editor.song.clone();
        let revision = editor.revision();
        let source = sample.preview.as_ref().unwrap().source.unwrap();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        sample.tempo_view = true;
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Cached v2 points"),
            egui::Modifiers::NONE,
            true,
        );
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Analyze tempo / pulse"),
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert!(sample.error.is_none(), "{:?}", sample.error);
        let state = sample.capture_tempo_state(&editor);
        let expected = crate::tempo::analyze_cached(
            &editor.song,
            source,
            &cache,
            &sample.experimental.config,
            crate::tempo::CachedRequest {
                start_frame: 0,
                end_frame: 48000,
                basis: crate::tempo::PointBasis::RefinedAttack,
            },
            crate::tempo::TempoConfig::default(),
        )
        .unwrap();
        assert_eq!(
            state["preview"]["analysis"],
            serde_json::to_value(expected.analysis()).unwrap()
        );
        assert_eq!(
            state["preview"]["points"],
            serde_json::to_value(expected.points()).unwrap()
        );
        assert_eq!(state["current"], true);
        assert!(Arc::ptr_eq(
            &cache,
            sample.experimental.cache.as_ref().unwrap()
        ));
        assert_eq!(
            serde_json::to_value(cache.curve_page(0, 8).unwrap()).unwrap(),
            raw
        );
        sample.experimental.config.threshold += 0.01;
        assert_eq!(sample.capture_tempo_state(&editor)["current"], false);
        sample.experimental.config.threshold -= 0.01;
        let original = fs::read(&fixture.0).unwrap();
        let mut changed = original.clone();
        *changed.last_mut().unwrap() ^= 1;
        fs::write(&fixture.0, &changed).unwrap();
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Analyze tempo / pulse"),
            egui::Modifiers::NONE,
            true,
        );
        settle(&mut sample);
        assert!(sample.capture_tempo_state(&editor)["error"].is_string());
        assert_eq!(editor.song, base);
        assert_eq!(editor.revision(), revision);
        fs::write(&fixture.0, original).unwrap();
        let out = frame(&ctx, &mut sample, &mut editor, vec![], true);
        click(
            &ctx,
            &mut sample,
            &mut editor,
            text(&out, "Analyze tempo / pulse"),
            egui::Modifiers::NONE,
            true,
        );
        assert!(sample.busy());
        sample.reset_document();
        settle(&mut sample);
        assert!(sample.capture_tempo_state(&editor)["preview"].is_null());
        assert!(sample.experimental.cache.is_none());
        assert_eq!(editor.song, base);
    }
    fn sample_tempo_control(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..160 {
            let out = frame(ctx, sample, editor, vec![], true);
            if let Some(pos) = out.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text == label
                        || (label.starts_with("candidate:")
                            && text
                                .galley
                                .job
                                .text
                                .trim_start()
                                .starts_with(&format!("{}   ", &label[10..]))) =>
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
            frame(
                ctx,
                sample,
                editor,
                vec![
                    egui::Event::PointerMoved(egui::pos2(880.0, 20.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if down { -70.0 } else { 70.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        panic!("Visible source tempo control missing: {label}");
    }
    fn sample_tempo_click(
        ctx: &egui::Context,
        sample: &mut SampleUi,
        editor: &mut Editor,
        label: &str,
        down: bool,
    ) {
        let pos = sample_tempo_control(ctx, sample, editor, label, down);
        frame(
            ctx,
            sample,
            editor,
            vec![egui::Event::PointerMoved(pos)],
            true,
        );
        click(ctx, sample, editor, pos, egui::Modifiers::NONE, true);
    }
    #[test]
    fn actual_cached_use_rechecks_after_review_and_cancel_read_modal_then_retry_are_atomic() {
        let (fixture, mut sample, mut editor) = tempo_source_fixture();
        sample.start_experimental(&editor, None, false).unwrap();
        settle(&mut sample);
        sample.tempo_view = true;
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        sample_tempo_click(&ctx, &mut sample, &mut editor, "Cached v2 points", false);
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            "Analyze tempo / pulse",
            true,
        );
        settle(&mut sample);
        let state = sample.capture_tempo_state(&editor);
        let bpm = state["preview"]["analysis"]["candidates"][0]["bpm"]
            .as_f64()
            .expect("Authored periodic noise yields timing evidence");
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            &format!("candidate:{bpm:.1}"),
            true,
        );
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            "Choose BPM target markers",
            true,
        );
        sample_tempo_click(&ctx, &mut sample, &mut editor, "1 · Tempo markers", true);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        let state = sample.capture_tempo_state(&editor);
        let expected: Song =
            serde_json::from_value(state["use_plan"]["candidate"].clone()).unwrap();
        let before = editor.song.clone();
        let revision = editor.revision();
        let original = fs::read(&fixture.0).unwrap();
        let mut tampered = original.clone();
        *tampered.last_mut().unwrap() ^= 1;
        fs::write(&fixture.0, tampered).unwrap();
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            "Use selected BPM only",
            true,
        );
        assert!(sample.busy());
        settle(&mut sample);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        assert!(sample.capture_tempo_state(&editor)["error"].is_string());
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
        fs::write(&fixture.0, &original).unwrap();
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            "Use selected BPM only",
            true,
        );
        assert!(sample.busy());
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            "Cancel BPM application",
            true,
        );
        settle(&mut sample);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        assert_eq!(editor.song, before);
        for disabled_parent in [false, true] {
            sample_tempo_click(
                &ctx,
                &mut sample,
                &mut editor,
                "Use selected BPM only",
                true,
            );
            settle(&mut sample);
            assert_eq!(
                sample.capture_tempo_state(&editor)["verified_waiting"],
                true
            );
            assert_eq!(editor.song, before);
            if !disabled_parent {
                editor.edit_mode = false;
            }
            frame(&ctx, &mut sample, &mut editor, vec![], !disabled_parent);
            editor.edit_mode = true;
            frame(&ctx, &mut sample, &mut editor, vec![], true);
            assert_eq!(editor.song, before);
            assert_eq!(
                sample.capture_tempo_state(&editor)["commit_authorized"],
                false
            );
        }
        sample_tempo_click(
            &ctx,
            &mut sample,
            &mut editor,
            "Use selected BPM only",
            true,
        );
        settle(&mut sample);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        assert_eq!(editor.song, expected);
        assert_eq!(editor.revision(), revision + 1);
        assert_eq!(fs::read(&fixture.0).unwrap(), original);
        editor.undo();
        assert_eq!(editor.song, before);
    }
    #[test]
    fn catalog_uses_one_source_worker_and_new_document_discards_late_analysis() {
        let fixture = fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.instrument = 1;
        sample.map_on_apply = false;
        sample.apply(&mut editor).unwrap();
        let draft = sample.draft.clone();
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for label in [
            "Slice catalog",
            "Select all regions",
            "Analyze selected regions",
        ] {
            let point = find_control(&ctx, &mut sample, &mut editor, label, true);
            click(
                &ctx,
                &mut sample,
                &mut editor,
                point,
                egui::Modifiers::NONE,
                true,
            );
        }
        assert!(sample.busy());
        assert_eq!(sample.draft, draft);
        assert!(sample
            .start(Request::Load {
                path: fixture.0.clone(),
                expected: None
            })
            .is_err());
        sample.reset_document();
        settle(&mut sample);
        assert!(!sample.busy());
        assert!(sample.capture_catalog_state(&editor)["measured"].is_null());
        assert!(sample.preview.is_none());
        assert!(editor.song.region_catalog.is_empty());
    }
    #[test]
    fn catalog_native_cache_result_survives_unrelated_picker_change_without_marker_edits() {
        let fixture = experimental_fixture();
        let mut sample = loaded(&fixture.0);
        let mut editor = Editor {
            edit_mode: true,
            ..Default::default()
        };
        sample.instrument = 1;
        sample.map_on_apply = false;
        sample.apply(&mut editor).unwrap();
        let before = editor.song.clone();
        let draft = sample.draft.clone();
        sample.start_experimental(&editor, None, false).unwrap();
        settle(&mut sample);
        let ctx = egui::Context::default();
        tracker_ui::install_theme(&ctx);
        for label in [
            "Slice catalog",
            "Select all regions",
            "Analyze selected regions",
        ] {
            let point = find_control(&ctx, &mut sample, &mut editor, label, true);
            click(
                &ctx,
                &mut sample,
                &mut editor,
                point,
                egui::Modifiers::NONE,
                true,
            );
        }
        settle(&mut sample);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        let state = sample.capture_catalog_state(&editor);
        assert_eq!(state["measured"]["reused_cache"], true);
        assert_eq!(state["current"], true);
        let measured = state["measured"].clone();
        sample.experimental.config.threshold = 0.6;
        sample.start_experimental(&editor, None, true).unwrap();
        settle(&mut sample);
        frame(&ctx, &mut sample, &mut editor, vec![], true);
        let state = sample.capture_catalog_state(&editor);
        assert_eq!(state["measured"], measured);
        assert_eq!(state["current"], true);
        assert_eq!(editor.song, before);
        assert_eq!(sample.draft, draft);
    }
}
