//! Advisory tempo/pulse review. The Samples worker owns analysis; only an
//! explicitly selected hypothesis may update an existing marker set's BPM.
use crate::{
    editor::Editor,
    onset_service::CachedSource,
    onset_v2::Config as OnsetConfig,
    sample_asset,
    song::{MarkerSetId, SampleAsset, SampleId, Song},
    tempo::{self, CachedRequest, MarkerRequest, PointBasis, TempoConfig, TempoPreview},
    tracker_ui::AQUA,
};
use eframe::egui::{self, Color32, RichText};
use std::{path::PathBuf, sync::Arc};

const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const CAUTION: Color32 = Color32::from_rgb(229, 185, 112);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceMode {
    Markers,
    Cached,
}
#[derive(Clone, Debug, PartialEq)]
struct Draft {
    mode: EvidenceMode,
    marker: Option<MarkerSetId>,
    start_index: String,
    end_index: String,
    include_start: bool,
    start_frame: String,
    end_frame: String,
    basis: PointBasis,
    min_bpm: String,
    max_bpm: String,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            mode: EvidenceMode::Markers,
            marker: None,
            start_index: "0".into(),
            end_index: "1".into(),
            include_start: false,
            start_frame: "0".into(),
            end_frame: String::new(),
            basis: PointBasis::RefinedAttack,
            min_bpm: "20".into(),
            max_bpm: "400".into(),
        }
    }
}
fn integer<T: std::str::FromStr>(text: &str, label: &str) -> Result<T, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{label} requires an unsigned integer; no clamping"));
    }
    text.parse()
        .map_err(|_| format!("{label} is outside the supported integer range"))
}
fn bpm_tenths(text: &str, label: &str) -> Result<u16, String> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if fraction.len() > 1
        || (text.contains('.') && fraction.is_empty())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!(
            "{label} requires BPM with at most one decimal place; no rounding"
        ));
    }
    let whole: u16 = integer(whole, label)?;
    whole
        .checked_mul(10)
        .and_then(|v| {
            v.checked_add(if fraction.is_empty() {
                0
            } else {
                u16::from(fraction.as_bytes()[0] - b'0')
            })
        })
        .ok_or_else(|| format!("{label} is outside the supported range"))
}
impl Draft {
    fn config(&self) -> Result<TempoConfig, String> {
        let min_bpm_tenths = bpm_tenths(&self.min_bpm, "Minimum BPM")?;
        let max_bpm_tenths = bpm_tenths(&self.max_bpm, "Maximum BPM")?;
        if !(200..=4000).contains(&min_bpm_tenths)
            || !(min_bpm_tenths..=4000).contains(&max_bpm_tenths)
        {
            return Err("BPM range must be ordered within 20..400; no clamping".into());
        }
        Ok(TempoConfig {
            version: tempo::VERSION,
            min_bpm_tenths,
            max_bpm_tenths,
        })
    }
    fn markers(&self) -> Result<MarkerRequest, String> {
        Ok(MarkerRequest {
            marker_set: self.marker.ok_or("Select an accepted marker set")?,
            start_index: integer(&self.start_index, "Start marker index")?,
            end_index: integer(&self.end_index, "End marker index")?,
            include_start: self.include_start,
        })
    }
    fn cached(&self) -> Result<CachedRequest, String> {
        Ok(CachedRequest {
            start_frame: integer(&self.start_frame, "Window start")?,
            end_frame: integer(&self.end_frame, "Window end")?,
            basis: self.basis,
        })
    }
}

enum EvidenceRequest {
    Markers(MarkerRequest),
    Cached {
        cache: Arc<CachedSource>,
        config: Box<OnsetConfig>,
        request: CachedRequest,
        path: PathBuf,
    },
}
pub(crate) struct AnalysisRequest {
    song: Box<Song>,
    revision: u64,
    source: SampleId,
    asset: SampleAsset,
    draft: Draft,
    epoch: u64,
    evidence: EvidenceRequest,
}
pub(crate) struct AnalysisResult {
    revision: u64,
    source: SampleId,
    asset: SampleAsset,
    draft: Draft,
    epoch: u64,
    onset_config: Option<OnsetConfig>,
    preview: Arc<TempoPreview>,
}
fn analyze(request: AnalysisRequest) -> Result<AnalysisResult, String> {
    let AnalysisRequest {
        song,
        revision,
        source,
        asset,
        draft,
        epoch,
        evidence,
    } = request;
    let config = draft.config()?;
    let (preview, onset_config) = match evidence {
        EvidenceRequest::Markers(markers) => {
            (tempo::analyze_markers(&song, markers, config)?, None)
        }
        EvidenceRequest::Cached {
            cache,
            config: onset,
            request,
            path,
        } => {
            sample_asset::verify_identity(&asset, &path)?;
            (
                tempo::analyze_cached(&song, source, &cache, &onset, request, config)?,
                Some(*onset),
            )
        }
    };
    Ok(AnalysisResult {
        revision,
        source,
        asset,
        draft,
        epoch,
        onset_config,
        preview: Arc::new(preview),
    })
}

pub(crate) enum WorkRequest {
    Analyze(Box<AnalysisRequest>),
    Verify(Box<Verification>),
}
pub(crate) enum WorkResult {
    Analyzed(Box<AnalysisResult>),
    Verified(Box<Verification>),
}
pub(crate) struct Verification {
    revision: u64,
    source: SampleId,
    asset: SampleAsset,
    path: PathBuf,
    draft: Draft,
    epoch: u64,
    onset: OnsetConfig,
    tempo_id: String,
    selected: usize,
    target: MarkerSetId,
    plan: Arc<tempo::UseBpmPreview>,
}
pub(crate) fn execute(request: WorkRequest) -> Result<WorkResult, String> {
    match request {
        WorkRequest::Analyze(request) => {
            analyze(*request).map(|result| WorkResult::Analyzed(Box::new(result)))
        }
        WorkRequest::Verify(request) => {
            sample_asset::verify_identity(&request.asset, &request.path)?;
            Ok(WorkResult::Verified(request))
        }
    }
}

#[derive(Default)]
pub(crate) struct TempoUi {
    draft: Draft,
    source: Option<SampleId>,
    epoch: u64,
    ready: Option<AnalysisResult>,
    selected: Option<usize>,
    target: Option<MarkerSetId>,
    use_plan: Option<(usize, MarkerSetId, Arc<tempo::UseBpmPreview>)>,
    plan_key: Option<(usize, MarkerSetId)>,
    verified: Option<Verification>,
    commit_authorized: bool,
    pulse_page: usize,
    error: Option<String>,
    notice: String,
}
impl TempoUi {
    pub(crate) fn reset(&mut self) {
        self.cancel_commit();
        self.epoch = self.epoch.wrapping_add(1);
        self.source = None;
        self.ready = None;
        self.selected = None;
        self.target = None;
        self.use_plan = None;
        self.plan_key = None;
        self.error = None;
        self.notice.clear();
    }
    pub(crate) fn invalidate_cached(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self
            .ready
            .as_ref()
            .is_some_and(|r| r.onset_config.is_some())
        {
            self.cancel_commit();
        }
    }
    pub(crate) fn finish(&mut self, result: WorkResult) {
        match result {
            WorkResult::Analyzed(result) => self.finish_analysis(*result),
            WorkResult::Verified(result) => {
                if self.commit_authorized {
                    self.verified = Some(*result);
                }
            }
        }
    }
    fn finish_analysis(&mut self, result: AnalysisResult) {
        self.cancel_commit();
        self.ready = Some(result);
        self.selected = None;
        self.use_plan = None;
        self.plan_key = None;
        self.pulse_page = 0;
        self.error = None;
        self.notice =
            "Hypotheses ready. Select one to inspect; none was chosen or applied automatically."
                .into();
    }
    pub(crate) fn fail(&mut self, error: String) {
        self.cancel_commit();
        self.error = Some(error);
        self.notice.clear();
    }
    fn sync(&mut self, editor: &Editor, source: SampleId, asset: &SampleAsset) {
        if self.source != Some(source) {
            self.reset();
            self.source = Some(source);
            self.draft = Draft {
                end_frame: asset.frames.to_string(),
                ..Draft::default()
            };
        }
        if self.draft.marker.is_some_and(|id| {
            editor
                .song
                .marker_sets
                .get(&id)
                .is_none_or(|set| set.sample != source)
        }) {
            self.draft.marker = None;
        }
        if self.target.is_some_and(|id| {
            editor
                .song
                .marker_sets
                .get(&id)
                .is_none_or(|set| set.sample != source)
        }) {
            self.target = None;
            self.use_plan = None;
            self.plan_key = None;
        }
    }
    fn current(
        &self,
        editor: &Editor,
        source: SampleId,
        asset: &SampleAsset,
        onset: &OnsetConfig,
    ) -> bool {
        self.ready.as_ref().is_some_and(|r| {
            r.revision == editor.revision()
                && r.source == source
                && r.asset == *asset
                && (r.onset_config.is_none() || r.epoch == self.epoch)
                && r.draft == self.draft
                && r.preview.base() == &editor.song
                && r.onset_config.as_ref().is_none_or(|config| config == onset)
        })
    }
    fn request(
        &mut self,
        editor: &Editor,
        source: SampleId,
        asset: &SampleAsset,
        cache: Option<&Arc<CachedSource>>,
        onset: &OnsetConfig,
        path: PathBuf,
    ) -> Result<AnalysisRequest, String> {
        self.draft.config()?;
        let evidence = match self.draft.mode {
            EvidenceMode::Markers => EvidenceRequest::Markers(self.draft.markers()?),
            EvidenceMode::Cached => EvidenceRequest::Cached {
                cache: Arc::clone(cache.ok_or("Analyze v2 first to prepare a source-bound cache")?),
                config: Box::new(onset.clone()),
                request: self.draft.cached()?,
                path,
            },
        };
        self.cancel_commit();
        self.ready = None;
        self.selected = None;
        self.use_plan = None;
        self.plan_key = None;
        self.error = None;
        self.notice =
            "Analyzing timing evidence on the source worker; markers and BPM remain unchanged."
                .into();
        Ok(AnalysisRequest {
            song: Box::new(editor.song.clone()),
            revision: editor.revision(),
            source,
            asset: asset.clone(),
            draft: self.draft.clone(),
            epoch: self.epoch,
            evidence,
        })
    }
}

pub(crate) struct Context<'a> {
    pub source: SampleId,
    pub asset: &'a SampleAsset,
    pub cache: Option<&'a Arc<CachedSource>>,
    pub onset: &'a OnsetConfig,
    pub path: PathBuf,
    pub busy: bool,
}
fn marker_label(id: MarkerSetId, name: &str) -> String {
    let mut chars = name.chars();
    let prefix: String = chars.by_ref().take(48).collect();
    format!(
        "{} · {}{}",
        id.0,
        prefix,
        if chars.next().is_some() { "…" } else { "" }
    )
}
fn marker_hint(id: MarkerSetId, name: &str) -> String {
    format!("Marker set {}; name {} UTF-8 bytes. Labels show at most48 characters with explicit elision; the stable numeric ID is authoritative.",id.0,name.len())
}
fn input(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .id(egui::Id::new(("tempo-number", label)))
            .desired_width(width),
    );
    response.on_hover_text(value.as_str());
}
impl TempoUi {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        context: Context<'_>,
    ) -> Option<WorkRequest> {
        let Context {
            source,
            asset,
            cache,
            onset,
            path,
            busy,
        } = context;
        self.sync(editor, source, asset);
        self.finish_verified(editor, source, asset, onset, ui.is_enabled());
        ui.label(
            RichText::new("TEMPO / PULSE · ADVISORY TIMING HYPOTHESES")
                .strong()
                .color(AQUA),
        );
        ui.label(RichText::new("Scores are uncalibrated heuristics, not probabilities. Half/double tempo and pulse phase may be ambiguous; no automatic winner, downbeat, meter or loop-length inference.").small().color(CAUTION));
        ui.horizontal_wrapped(|ui| {
            ui.label("Evidence");
            ui.selectable_value(
                &mut self.draft.mode,
                EvidenceMode::Markers,
                "Accepted markers",
            );
            ui.selectable_value(
                &mut self.draft.mode,
                EvidenceMode::Cached,
                "Cached v2 points",
            );
        });
        match self.draft.mode {
            EvidenceMode::Markers => {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("tempo-marker-source")
                        .height(180.0)
                        .selected_text(
                            self.draft
                                .marker
                                .map_or("Choose accepted markers".into(), |id| {
                                    format!("Markers {}", id.0)
                                }),
                        )
                        .show_ui(ui, |ui| {
                            for (id, set) in &editor.song.marker_sets {
                                if set.sample != source {
                                    continue;
                                }
                                if ui
                                    .selectable_value(
                                        &mut self.draft.marker,
                                        Some(*id),
                                        marker_label(*id, &set.name),
                                    )
                                    .on_hover_text(marker_hint(*id, &set.name))
                                    .changed()
                                {
                                    self.draft.start_index = "0".into();
                                    self.draft.end_index =
                                        set.markers.len().saturating_sub(1).to_string();
                                    self.target = Some(*id);
                                }
                            }
                        });
                    input(ui, "Start index", &mut self.draft.start_index, 48.0);
                    input(ui, "End index (excluded)", &mut self.draft.end_index, 48.0);
                    ui.checkbox(&mut self.draft.include_start, "Include start anchor");
                });
                if let Some(set) = self
                    .draft
                    .marker
                    .and_then(|id| editor.song.marker_sets.get(&id))
                {
                    let bounds = integer::<usize>(&self.draft.start_index, "Start")
                        .ok()
                        .and_then(|a| set.markers.get(a).copied())
                        .zip(
                            integer::<usize>(&self.draft.end_index, "End")
                                .ok()
                                .and_then(|b| set.markers.get(b).copied()),
                        );
                    ui.label(RichText::new(bounds.map_or("Window indices are invalid or outside this marker set".into(),|(a,b)|format!("SOURCE frame window [{a}, {b}) · {} accepted boundaries · stored BPM {}",set.markers.len(),set.source_bpm.map_or("UNKNOWN".into(),|b|b.to_string())))).small().color(MUTED));
                }
                ui.label(RichText::new("Only committed boundaries are evidence; unsaved marker drafts and rejected proposals are excluded. End is structural/excluded; start is opt-in. Stored source metadata, no new audio verification.").small().color(MUTED));
            }
            EvidenceMode::Cached => {
                ui.horizontal_wrapped(|ui| {
                    input(ui, "Window start", &mut self.draft.start_frame, 100.0);
                    input(
                        ui,
                        "Window end (excluded)",
                        &mut self.draft.end_frame,
                        100.0,
                    );
                    ui.selectable_value(&mut self.draft.basis, PointBasis::Peak, "Coarse peaks");
                    ui.selectable_value(
                        &mut self.draft.basis,
                        PointBasis::RefinedAttack,
                        "Refined attacks",
                    );
                });
                ui.label(RichText::new(if cache.is_some(){"Current source-bound v2 cache + current picker settings. Worker rechecks source identity and repicks without FFT/decoding; peak/attack choice is explicit."}else{"No source-bound v2 cache. Prepare Analyze v2 in Transient suggestions first; this view does not silently run a new detector."}).small().color(MUTED));
                ui.label(RichText::new("V2 timing proposals can contain missed hits or false positives (including vibrato/chirps); accepting BPM does not accept any onset marker.").small().color(CAUTION));
            }
        }
        ui.horizontal_wrapped(|ui| {
            input(ui, "BPM minimum", &mut self.draft.min_bpm, 65.0);
            input(ui, "BPM maximum", &mut self.draft.max_bpm, 65.0);
            ui.label(
                RichText::new("20..400 BPM · 0.1 grid · no rounding/clamping")
                    .small()
                    .color(MUTED),
            );
        });
        let mut request = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && (self.draft.mode == EvidenceMode::Markers || cache.is_some()),
                    egui::Button::new("Analyze tempo / pulse"),
                )
                .clicked()
            {
                match self.request(editor, source, asset, cache, onset, path.clone()) {
                    Ok(next) => request = Some(WorkRequest::Analyze(Box::new(next))),
                    Err(error) => self.fail(error),
                }
            }
            if busy {
                ui.spinner();
                ui.label(if self.commit_authorized {
                    "Verifying cached source; guarded completion applies only the reviewed BPM"
                } else {
                    "Source task running; no BPM application is currently authorized"
                });
            }
        });
        let current = self.current(editor, source, asset, onset);
        self.show_result(ui, current);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("tempo-bpm-target")
                .height(180.0)
                .selected_text(
                    self.target
                        .map_or("Choose BPM target markers".into(), |id| {
                            format!("BPM target: markers {}", id.0)
                        }),
                )
                .show_ui(ui, |ui| {
                    for (id, set) in &editor.song.marker_sets {
                        if set.sample == source {
                            ui.selectable_value(
                                &mut self.target,
                                Some(*id),
                                marker_label(*id, &set.name),
                            )
                            .on_hover_text(marker_hint(*id, &set.name));
                        }
                    }
                });
        });
        if current {
            if let Some((selected, target)) = self.selected.zip(self.target) {
                if self.plan_key != Some((selected, target)) {
                    self.plan_key = Some((selected, target));
                    match self
                        .ready
                        .as_ref()
                        .unwrap()
                        .preview
                        .use_bpm(target, selected)
                    {
                        Ok(plan) => {
                            self.use_plan = Some((selected, target, Arc::new(plan)));
                            self.error = None;
                        }
                        Err(error) => {
                            self.use_plan = None;
                            self.fail(error);
                        }
                    }
                }
            } else {
                self.use_plan = None;
                self.plan_key = None;
            }
        }
        if let Some((_, _, plan)) = &self.use_plan {
            let summary = plan.summary();
            ui.label(
                RichText::new(format!(
                    "{}: markers {} / S{} · {} -> {:.1} BPM",
                    if current {
                        "BPM-only review"
                    } else {
                        "STALE BPM-only review"
                    },
                    summary.marker_set.0,
                    summary.sample.0,
                    summary
                        .before_bpm
                        .map_or("UNKNOWN".into(), |b| b.to_string()),
                    summary.after_bpm
                ))
                .strong()
                .color(if current { AQUA } else { CAUTION }),
            );
            egui::CollapsingHeader::new("BPM-only change details").show(ui, |ui| {
                for warning in &summary.warnings {
                    ui.label(RichText::new(warning).small().color(CAUTION));
                }
            });
        }
        let enabled = editor.edit_mode && !busy && current && self.use_plan.is_some();
        if ui
            .add_enabled(enabled, egui::Button::new("Use selected BPM only"))
            .clicked()
        {
            let ready = self.ready.as_ref().unwrap();
            if let Some(onset_config) = &ready.onset_config {
                self.commit_authorized = true;
                self.error = None;
                self.notice="Rechecking cached source bytes on the worker before BPM-only commit. Closing/changing context cancels application, not the tracked worker.".into();
                request = Some(WorkRequest::Verify(Box::new(Verification {
                    revision: ready.revision,
                    source,
                    asset: asset.clone(),
                    path,
                    draft: self.draft.clone(),
                    epoch: self.epoch,
                    onset: onset_config.clone(),
                    tempo_id: ready.preview.id().into(),
                    selected: self.selected.unwrap(),
                    target: self.target.unwrap(),
                    plan: Arc::clone(&self.use_plan.as_ref().unwrap().2),
                })));
            } else {
                let before = editor.revision();
                let result =
                    editor.commit_tempo_preview(ready.revision, &self.use_plan.as_ref().unwrap().2);
                self.committed(result, before, editor.revision());
            }
        }
        if self.commit_authorized && ui.button("Cancel BPM application").clicked() {
            self.cancel_commit();
            self.notice =
                "BPM application canceled; source worker remains tracked until completion.".into();
        }
        if self
            .ready
            .as_ref()
            .is_some_and(|r| r.onset_config.is_some())
        {
            ui.label(RichText::new("Cached review is provisional: Use BPM rechecks source bytes off-thread before commit. No permanent file-validity or calibrated-confidence claim.").small().color(MUTED));
        }
        ui.label(RichText::new("Pulse phase is advisory only. Use BPM never moves markers or applies phase; the break compiler still uses source frame 0 as beat 0. No stretching, quantization or audio audition.").small().color(CAUTION));
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).small().color(MUTED));
        }
        request
    }
}

impl TempoUi {
    fn show_result(&mut self, ui: &mut egui::Ui, current: bool) {
        let Some(ready) = &self.ready else {
            return;
        };
        let analysis = ready.preview.analysis();
        let binding = &analysis.binding;
        ui.separator();
        ui.label(
            RichText::new(format!(
                "{} · {} evidence points · {} candidate(s) · revision {}",
                if current {
                    "REVIEW"
                } else {
                    "STALE — reanalyze before Use BPM"
                },
                analysis.evidence_count,
                analysis.candidates.len(),
                ready.revision
            ))
            .strong()
            .color(if current { AQUA } else { CAUTION }),
        );
        ui.label(
            RichText::new(format!(
                "S{} · source [{}, {}) · {} Hz · {}",
                binding.sample.0,
                binding.window_start_frame,
                binding.window_end_frame,
                binding.source_rate,
                analysis.algorithm
            ))
            .small()
            .color(MUTED),
        );
        if let Some(reason) = &analysis.insufficient_reason {
            ui.colored_label(
                CAUTION,
                format!("Insufficient evidence: {reason}. No fallback BPM."),
            );
        }
        ui.label(
            RichText::new("  BPM    score   fit   interval   coverage   matched   evidence")
                .monospace()
                .small()
                .color(MUTED),
        );
        egui::ScrollArea::vertical().id_salt("tempo-candidates").max_height(142.0).show(ui,|ui| {
            for (index,candidate) in analysis.candidates.iter().enumerate() {
                let label=format!("{:>6.1}   {:>5.3}  {:>5.3}   {:>5.3}     {:>5.3}     {:>4}     {:?}",candidate.bpm,candidate.score,candidate.phase_fit,candidate.interval_vote,candidate.pulse_coverage,candidate.matched_points,candidate.strength);
                if ui.selectable_label(self.selected==Some(index),RichText::new(label).monospace()).on_hover_text(format!("Candidate {}: exact BPM {}/10; score {:?}; phase fit {:?}; interval vote {:?}; relative interval vote {:?}; coverage {:?}. These are heuristic components, not calibrated confidence.",index,candidate.bpm_tenths,candidate.score,candidate.phase_fit,candidate.interval_vote,candidate.relative_interval_vote,candidate.pulse_coverage)).clicked() {self.selected=Some(index);self.pulse_page=0;}
            }
        });
        if let Some((index, candidate)) = self.selected.and_then(|index| {
            analysis
                .candidates
                .get(index)
                .map(|candidate| (index, candidate))
        }) {
            ui.label(RichText::new(format!("{} {:.1} BPM · pulse phase {:.6} SOURCE frames from window start (exact {}/{})",if current{"Selected"}else{"STALE selected"},candidate.bpm,candidate.phase_frames,candidate.phase_numerator,candidate.phase_denominator)).strong().color(AQUA));
            ui.label(RichText::new(format!("Fit {:.4} · vote {:.4} (relative {:.4}) · coverage {}/{} · {} extra points/pulse · {} outside-window matches",candidate.phase_fit,candidate.interval_vote,candidate.relative_interval_vote,candidate.occupied_pulses,candidate.pulse_count,candidate.multiple_points_per_pulse,candidate.outside_window_matches)).small().color(MUTED));
            ui.label(RichText::new(format!("Span {:.4} beats · mean error {:.6} beats · {:?} timing evidence (not probability)",candidate.span_beats,candidate.mean_error_beats,candidate.strength)).small().color(MUTED));
            if !candidate.aliases.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Tempo ambiguity:").small().color(CAUTION));
                    for alias in &candidate.aliases {
                        ui.label(
                            RichText::new(format!(
                                "{:.1} BPM ×{}/{}{}",
                                f64::from(alias.bpm_tenths) / 10.0,
                                alias.ratio_numerator,
                                alias.ratio_denominator,
                                if alias.returned {
                                    " (listed)"
                                } else {
                                    " (not listed)"
                                }
                            ))
                            .small()
                            .color(CAUTION),
                        );
                    }
                });
            }
            let first = ready
                .preview
                .pulse_page(index, 0, 128)
                .expect("valid selected tempo candidate");
            let pages = first.total.div_ceil(128).max(1);
            self.pulse_page = self.pulse_page.min(pages - 1);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if current {
                        "PULSE PREVIEW · advisory / no playback"
                    } else {
                        "STALE PULSE PREVIEW · advisory / no playback"
                    })
                    .small()
                    .color(MUTED),
                );
                if ui
                    .add_enabled(self.pulse_page > 0, egui::Button::new("< Pulses"))
                    .clicked()
                {
                    self.pulse_page -= 1;
                }
                ui.label(format!("Page {} / {}", self.pulse_page + 1, pages));
                if ui
                    .add_enabled(self.pulse_page + 1 < pages, egui::Button::new("Pulses >"))
                    .clicked()
                {
                    self.pulse_page += 1;
                }
            });
            let page = ready
                .preview
                .pulse_page(index, self.pulse_page * 128, 128)
                .expect("bounded pulse page");
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width().max(20.0), 64.0),
                egui::Sense::hover(),
            );
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, Color32::from_rgb(14, 24, 29));
            let span = (binding.window_end_frame - binding.window_start_frame).max(1) as f64;
            let x = |frame: u64| {
                rect.left()
                    + ((frame.saturating_sub(binding.window_start_frame) as f64 / span) as f32)
                        * rect.width()
            };
            for &frame in ready.preview.points() {
                painter.line_segment(
                    [
                        egui::pos2(x(frame), rect.top() + 36.0),
                        egui::pos2(x(frame), rect.bottom() - 3.0),
                    ],
                    egui::Stroke::new(1.0_f32, MUTED),
                );
            }
            for &frame in &page.frames {
                painter.line_segment(
                    [
                        egui::pos2(x(frame), rect.top() + 3.0),
                        egui::pos2(x(frame), rect.top() + 31.0),
                    ],
                    egui::Stroke::new(1.0_f32, AQUA),
                );
            }
            ui.label(RichText::new(format!("SOURCE {} .. {} · aqua pulses {}..{} / {}; gray evidence points. Phase is not a downbeat.",binding.window_start_frame,binding.window_end_frame,page.start,page.start+page.frames.len(),page.total)).small().color(MUTED));
            egui::CollapsingHeader::new("Exact pulse source frames / selected components").show(ui,|ui| {
                ui.label(format!("Exact phase numerator {} / denominator {} source frames relative to {}",candidate.phase_numerator,candidate.phase_denominator,binding.window_start_frame));
                ui.label(format!("Exact score {:?}; fit {:?}; interval {:?}; relative {:?}; coverage {:?}; span {:?}; mean error {:?}",candidate.score,candidate.phase_fit,candidate.interval_vote,candidate.relative_interval_vote,candidate.pulse_coverage,candidate.span_beats,candidate.mean_error_beats));
                egui::ScrollArea::vertical().id_salt("tempo-pulse-frames").max_height(110.0).show_rows(ui,16.0,page.frames.len(),|ui,rows| {for row in rows {ui.monospace(format!("Pulse {:04}   SOURCE frame {}",page.start+row,page.frames[row]));}});
            });
        } else {
            ui.label(RichText::new("Select a candidate to inspect phase and enable explicit BPM review. The first row is not automatically selected.").small().color(MUTED));
        }
        egui::CollapsingHeader::new("Evidence binding / full warnings / raw points").show(ui,|ui| {
            ui.label(&binding.identity_basis);ui.monospace(format!("Source SHA-256 {}",binding.source_sha256));ui.monospace(format!("Points SHA-256 {}",binding.points_sha256));ui.monospace(format!("Preview {}",ready.preview.id()));
            match &binding.kind {tempo::EvidenceKind::AcceptedMarkers {request,marker_set_sha256}=>{ui.label(format!("Accepted markers {}: indices {}..{}; include start {}",request.marker_set.0,request.start_index,request.end_index,request.include_start));ui.monospace(format!("Marker snapshot SHA-256 {marker_set_sha256}"));},tempo::EvidenceKind::CachedV2 {request,extraction_key,onset_config_sha256,onset_algorithm}=>{ui.label(format!("Cached {:?} [{}, {}), {}",request.basis,request.start_frame,request.end_frame,onset_algorithm));ui.monospace(format!("Extraction {extraction_key}"));ui.monospace(format!("Picker SHA-256 {onset_config_sha256}"));}}
            ui.label(format!("{} interval pairs; {} work units; {} local maxima; {} separated candidates; shortlist truncated {}",analysis.interval_pairs,analysis.estimated_work,analysis.local_maxima_count,analysis.separated_candidate_count,analysis.shortlist_truncated));
            for warning in &analysis.warnings {ui.label(RichText::new(warning).small().color(CAUTION));}
            egui::ScrollArea::vertical().id_salt("tempo-raw-evidence").max_height(110.0).show_rows(ui,16.0,ready.preview.points().len(),|ui,rows| {for row in rows {ui.monospace(format!("Evidence {:04}   SOURCE frame {}",row,ready.preview.points()[row]));}});
        });
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_state(
        &self,
        editor: &Editor,
        source: Option<SampleId>,
        asset: Option<&SampleAsset>,
        onset: &OnsetConfig,
    ) -> serde_json::Value {
        let current = source
            .zip(asset)
            .is_some_and(|(source, asset)| self.current(editor, source, asset, onset));
        serde_json::json!({"draft":{"mode":format!("{:?}",self.draft.mode),"marker":self.draft.marker,"start_index":self.draft.start_index,"end_index":self.draft.end_index,"include_start":self.draft.include_start,"start_frame":self.draft.start_frame,"end_frame":self.draft.end_frame,"basis":self.draft.basis,"min_bpm":self.draft.min_bpm,"max_bpm":self.draft.max_bpm},"selected":self.selected,"target":self.target,"pulse_page":self.pulse_page,"context_epoch":self.epoch,"commit_authorized":self.commit_authorized,"verified_waiting":self.verified.is_some(),"error":self.error,"notice":self.notice,"current":current,"use_plan":self.use_plan.as_ref().map(|(_,_,p)|serde_json::json!({"id":p.id(),"summary":p.summary(),"candidate":p.candidate()})),"preview":self.ready.as_ref().map(|r|serde_json::json!({"id":r.preview.id(),"analysis":r.preview.analysis(),"points":r.preview.points(),"revision":r.revision}))})
    }
}

impl TempoUi {
    pub(crate) fn cancel_commit(&mut self) {
        self.commit_authorized = false;
        self.verified = None;
    }
    fn committed(&mut self, result: Result<(), String>, before: u64, after: u64) {
        match result {
            Ok(()) => {
                self.error = None;
                self.notice=if before==after {"That BPM was already stored; no history entry added."}else{"Stored only the selected marker set's source BPM (one undo). Markers/locks, phase origin, Song BPM/LPB and notes are unchanged."}.into();
            }
            Err(error) => self.fail(error),
        }
    }
    fn finish_verified(
        &mut self,
        editor: &mut Editor,
        source: SampleId,
        asset: &SampleAsset,
        onset: &OnsetConfig,
        enabled: bool,
    ) {
        if !enabled || !editor.edit_mode {
            self.cancel_commit();
            return;
        }
        let Some(verified) = self.verified.take() else {
            return;
        };
        let current = self.commit_authorized
            && self.current(editor, source, asset, onset)
            && verified.revision == editor.revision()
            && verified.source == source
            && verified.asset == *asset
            && verified.draft == self.draft
            && verified.epoch == self.epoch
            && verified.onset == *onset
            && self.selected == Some(verified.selected)
            && self.target == Some(verified.target)
            && self
                .ready
                .as_ref()
                .is_some_and(|r| r.preview.id() == verified.tempo_id);
        self.commit_authorized = false;
        if !current {
            self.fail(
                "Cached BPM application became stale; analyze/review again. No BPM was changed."
                    .into(),
            );
            return;
        }
        let before = editor.revision();
        let result = editor.commit_tempo_preview(verified.revision, &verified.plan);
        self.committed(result, before, editor.revision());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        onset_service,
        song::{MarkerSet, MarkerSetId},
    };

    fn fixture() -> (Editor, SampleAsset) {
        let mut editor = Editor::default();
        let mut song = editor.song.clone();
        let asset = SampleAsset {
            path: "/__tempo_ui_metadata_only_missing__.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: 48_000,
            channels: 1,
            frames: 192_000,
        };
        song.register_sample(SampleId(1), asset.clone()).unwrap();
        song.upsert_marker_set(
            MarkerSetId(1),
            MarkerSet {
                name: "Accepted periodic".into(),
                sample: SampleId(1),
                source_bpm: None,
                markers: (0..=8).map(|i| i * 24_000).collect(),
                locked_frames: vec![48_000],
            },
        )
        .unwrap();
        song.upsert_marker_set(
            MarkerSetId(2),
            MarkerSet {
                name: "Same source target".into(),
                sample: SampleId(1),
                source_bpm: Some(91.25),
                markers: vec![0, 48_000, 96_000, 144_000, 192_000],
                locked_frames: vec![96_000],
            },
        )
        .unwrap();
        editor.replace_document(song).unwrap();
        editor.edit_mode = true;
        (editor, asset)
    }
    fn review(editor: &Editor, asset: &SampleAsset) -> TempoUi {
        let mut ui = TempoUi::default();
        ui.sync(editor, SampleId(1), asset);
        ui.draft.marker = Some(MarkerSetId(1));
        ui.draft.end_index = "8".into();
        ui.draft.include_start = true;
        ui.draft.min_bpm = "119.9".into();
        ui.draft.max_bpm = "120.1".into();
        ui.target = Some(MarkerSetId(2));
        let request = ui
            .request(
                editor,
                SampleId(1),
                asset,
                None,
                &onset_service::experimental_config(crate::onset_v2::Preset::Balanced),
                asset.path.clone(),
            )
            .unwrap();
        ui.finish_analysis(analyze(request).unwrap());
        ui
    }
    fn onset() -> OnsetConfig {
        onset_service::experimental_config(crate::onset_v2::Preset::Balanced)
    }
    fn draw(
        ctx: &egui::Context,
        ui: &mut TempoUi,
        editor: &mut Editor,
        asset: &SampleAsset,
        size: egui::Vec2,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> (egui::FullOutput, Option<WorkRequest>) {
        let mut request = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                modifiers: events
                    .iter()
                    .rev()
                    .find_map(|e| match e {
                        egui::Event::Key { modifiers, .. }
                        | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                        _ => None,
                    })
                    .unwrap_or_default(),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |parent| {
                    parent.add_enabled_ui(enabled, |parent| {
                        egui::ScrollArea::vertical()
                            .id_salt("tempo-test-outer")
                            .show(parent, |parent| {
                                request = ui.show(
                                    parent,
                                    editor,
                                    Context {
                                        source: SampleId(1),
                                        asset,
                                        cache: None,
                                        onset: &onset(),
                                        path: asset.path.clone(),
                                        busy: false,
                                    },
                                );
                            });
                    });
                });
            },
        );
        (output, request)
    }
    fn visible(output: &egui::FullOutput, predicate: impl Fn(&str) -> bool) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if predicate(&text.galley.job.text) => {
                let rect = text.visual_bounding_rect();
                (shape.clip_rect.contains_rect(rect) && !text.galley.elided).then_some(rect)
            }
            _ => None,
        })
    }
    fn control(
        ctx: &egui::Context,
        ui: &mut TempoUi,
        editor: &mut Editor,
        asset: &SampleAsset,
        size: egui::Vec2,
        label: &str,
        down: bool,
    ) -> egui::Pos2 {
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..150 {
            let (output, _) = draw(ctx, ui, editor, asset, size, vec![], true);
            if let Some(rect) = visible(&output, |s| {
                s == label || (label == "candidate120" && s.trim_start().starts_with("120.0   "))
            }) {
                let point = rect.center();
                if previous.is_some_and(|old: egui::Pos2| old.distance(point) < 0.02) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                previous = Some(point);
                if stable >= 3 {
                    return point;
                }
                continue;
            }
            previous = None;
            stable = 0;
            draw(
                ctx,
                ui,
                editor,
                asset,
                size,
                vec![
                    egui::Event::PointerMoved(egui::pos2(size.x - 20.0, size.y * 0.5)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, if down { -85.0 } else { 85.0 }),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        panic!("Visible tempo control missing: {label}")
    }
    fn click(
        ctx: &egui::Context,
        ui: &mut TempoUi,
        editor: &mut Editor,
        asset: &SampleAsset,
        size: egui::Vec2,
        pos: egui::Pos2,
        enabled: bool,
    ) -> Option<WorkRequest> {
        draw(
            ctx,
            ui,
            editor,
            asset,
            size,
            vec![egui::Event::PointerMoved(pos)],
            enabled,
        );
        let mut request = None;
        for pressed in [true, false] {
            let (_, next) = draw(
                ctx,
                ui,
                editor,
                asset,
                size,
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
            if next.is_some() {
                request = next;
            }
        }
        request
    }
    #[test]
    fn exact_decimal_and_source_integer_inputs_reject_without_rounding_or_clamping() {
        for (value, expected) in [
            ("20", 200),
            ("120.0", 1200),
            ("120.1", 1201),
            ("400", 4000),
            (" 99.9 ", 999),
        ] {
            assert_eq!(bpm_tenths(value, "BPM").unwrap(), expected);
        }
        for value in [
            "120.05", "NaN", "inf", "-0", "1e2", "120.", "65536", "6553.6", "", ".5",
        ] {
            assert!(bpm_tenths(value, "BPM").is_err(), "{value}");
        }
        for value in ["19.9", "400.1", "0"] {
            let draft = Draft {
                min_bpm: value.into(),
                ..Draft::default()
            };
            assert!(draft.config().is_err());
        }
        assert!(Draft {
            min_bpm: "121".into(),
            max_bpm: "120".into(),
            ..Draft::default()
        }
        .config()
        .is_err());
        assert_eq!(
            integer::<u64>("18446744073709551615", "Frame").unwrap(),
            u64::MAX
        );
        for value in ["18446744073709551616", "-1", "1.5", "NaN"] {
            assert!(integer::<u64>(value, "Frame").is_err());
        }
    }
    #[test]
    fn metadata_review_is_nonmutating_and_native_selected_apply_is_one_bpm_only_undo() {
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let (mut editor, asset) = fixture();
            let base = editor.song.clone();
            let revision = editor.revision();
            let mut ui = review(&editor, &asset);
            let ctx = egui::Context::default();
            assert!(ui.selected.is_none());
            assert_eq!(editor.song, base);
            assert_eq!(editor.revision(), revision);
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "candidate120",
                true,
            );
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            assert!(ui.selected.is_some());
            assert!(ui.use_plan.is_some());
            let expected = ui.use_plan.as_ref().unwrap().2.candidate().clone();
            assert_eq!(editor.song, base);
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "Use selected BPM only",
                true,
            );
            click(&ctx, &mut ui, &mut editor, &asset, size, p, false);
            assert_eq!(editor.song, base);
            editor.edit_mode = false;
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            assert_eq!(editor.song, base);
            editor.edit_mode = true;
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "Use selected BPM only",
                true,
            );
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            assert_eq!(editor.song, expected);
            assert_eq!(editor.revision(), revision + 1);
            let mut only = base.clone();
            only.marker_sets
                .get_mut(&MarkerSetId(2))
                .unwrap()
                .source_bpm = Some(120.0);
            assert_eq!(editor.song, only);
            editor.undo();
            assert_eq!(editor.song, base);
            assert!(!ui.current(&editor, SampleId(1), &asset, &onset()));
            editor.redo();
            assert_eq!(editor.song, expected);
        }
    }
    #[test]
    fn draft_revision_cache_epoch_and_source_changes_make_review_stale_without_recreating_markers()
    {
        let (mut editor, asset) = fixture();
        let mut ui = review(&editor, &asset);
        assert!(ui.current(&editor, SampleId(1), &asset, &onset()));
        ui.draft.min_bpm = "120.05".into();
        assert!(!ui.current(&editor, SampleId(1), &asset, &onset()));
        ui.draft.min_bpm = "119.9".into();
        assert!(ui.current(&editor, SampleId(1), &asset, &onset()));
        ui.invalidate_cached();
        assert!(
            ui.current(&editor, SampleId(1), &asset, &onset()),
            "Accepted-marker evidence is independent of onset cache epochs"
        );
        let mut ui = review(&editor, &asset);
        let mut changed = asset.clone();
        changed.sha256 = "b".repeat(64);
        assert!(!ui.current(&editor, SampleId(1), &changed, &onset()));
        editor.remove_marker_set(MarkerSetId(2)).unwrap_err(); // locked targets cannot disappear implicitly
        editor
            .set_marker_lock(MarkerSetId(2), 96_000, false)
            .unwrap();
        editor.remove_marker_set(MarkerSetId(2)).unwrap();
        ui.sync(&editor, SampleId(1), &asset);
        assert!(ui.target.is_none());
        assert!(!ui.current(&editor, SampleId(1), &asset, &onset()));
        ui.reset();
        assert!(ui.ready.is_none());
        assert!(ui.use_plan.is_none());
    }
    #[test]
    fn insufficient_evidence_has_no_candidate_no_automatic_bpm_and_explicit_error_is_retained() {
        let (editor, asset) = fixture();
        let mut ui = TempoUi::default();
        ui.sync(&editor, SampleId(1), &asset);
        ui.draft.marker = Some(MarkerSetId(1));
        ui.draft.end_index = "3".into();
        let request = ui
            .request(
                &editor,
                SampleId(1),
                &asset,
                None,
                &onset(),
                asset.path.clone(),
            )
            .unwrap();
        ui.finish_analysis(analyze(request).unwrap());
        assert!(ui
            .ready
            .as_ref()
            .unwrap()
            .preview
            .analysis()
            .candidates
            .is_empty());
        assert!(ui
            .ready
            .as_ref()
            .unwrap()
            .preview
            .analysis()
            .insufficient_reason
            .is_some());
        assert!(ui.selected.is_none());
        ui.draft.mode = EvidenceMode::Cached;
        let error = ui
            .request(
                &editor,
                SampleId(1),
                &asset,
                None,
                &onset(),
                asset.path.clone(),
            )
            .err()
            .unwrap();
        ui.fail(error);
        assert!(ui.error.as_ref().unwrap().contains("Analyze v2 first"));
        assert!(ui.notice.is_empty());
    }
    #[test]
    fn actual_decimal_input_preserves_invalid_text_and_refuses_analysis_without_note_leakage() {
        let (mut editor, asset) = fixture();
        let base = editor.song.clone();
        let revision = editor.revision();
        let mut ui = review(&editor, &asset);
        let ctx = egui::Context::default();
        let size = egui::vec2(900.0, 620.0);
        for value in ["120.05", "NaN", "19.9", "400.1", "65536"] {
            control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "BPM minimum",
                false,
            );
            let rect = ctx
                .read_response(egui::Id::new(("tempo-number", "BPM minimum")))
                .unwrap()
                .rect;
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
            click(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                rect.center(),
                true,
            );
            for pressed in [true, false] {
                draw(
                    &ctx,
                    &mut ui,
                    &mut editor,
                    &asset,
                    size,
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
            draw(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                vec![egui::Event::Text(value.into())],
                true,
            );
            assert_eq!(ui.draft.min_bpm, value);
            assert!(!ui.current(&editor, SampleId(1), &asset, &onset()));
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "Analyze tempo / pulse",
                true,
            );
            assert!(click(&ctx, &mut ui, &mut editor, &asset, size, p, true).is_none());
            assert!(ui.error.is_some());
            assert_eq!(editor.song, base);
            assert_eq!(editor.revision(), revision);
            let snapshot = ui.capture_state(&editor, Some(SampleId(1)), Some(&asset), &onset());
            assert_eq!(snapshot["draft"]["min_bpm"], value);
            assert_eq!(snapshot["current"], false);
        }
    }
    fn menu_choice(
        ctx: &egui::Context,
        ui: &mut TempoUi,
        editor: &mut Editor,
        asset: &SampleAsset,
        size: egui::Vec2,
        label: &str,
    ) {
        let mut previous = None;
        let mut stable = 0;
        for _ in 0..180 {
            let (out, _) = draw(ctx, ui, editor, asset, size, vec![], true);
            assert!(
                out.shapes.iter().all(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) => text.galley.job.text.len() < 2048,
                    _ => true,
                }),
                "A multi-megabyte marker name must never enter text shaping"
            );
            if let Some(rect) = visible(&out, |text| text == label) {
                let pos = rect.center();
                if previous.is_some_and(|old: egui::Pos2| old.distance(pos) < 0.02) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                previous = Some(pos);
                if stable >= 3 {
                    click(ctx, ui, editor, asset, size, pos, true);
                    return;
                }
                continue;
            }
            previous = None;
            stable = 0;
            let clip = out
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text)
                        if text.galley.job.text.contains(" · ")
                            && text
                                .galley
                                .job
                                .text
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_digit())
                            && shape.clip_rect.height() > 60.0
                            && shape.clip_rect.height() < 260.0 =>
                    {
                        Some((shape.clip_rect, text.visual_bounding_rect().center().x))
                    }
                    _ => None,
                })
                .expect("Native bounded marker popup clip");
            draw(
                ctx,
                ui,
                editor,
                asset,
                size,
                vec![
                    egui::Event::PointerMoved(egui::pos2(clip.1, clip.0.center().y)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -140.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        let (out, _) = draw(ctx, ui, editor, asset, size, vec![], true);
        let details: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some((
                    text.galley.job.text.clone(),
                    text.visual_bounding_rect(),
                    shape.clip_rect,
                )),
                _ => None,
            })
            .collect();
        panic!("Last marker popup item not reachable: {label}; {details:?}");
    }
    #[test]
    fn long_unicode_names_and_128_marker_choices_are_bounded_and_reachable_at_both_sizes() {
        assert!(marker_label(MarkerSetId(7), &"界".repeat(10000)).len() < 160);
        assert!(marker_label(MarkerSetId(7), &"界".repeat(10000)).ends_with('…'));
        assert!(marker_hint(MarkerSetId(7), &"x".repeat(100000)).len() < 200);
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let (mut editor, asset) = fixture();
            let mut song = editor.song.clone();
            let template = song.marker_sets[&MarkerSetId(1)].clone();
            song.marker_sets.clear();
            for id in 1..=128 {
                let mut set = template.clone();
                set.name = if id == 1 {
                    "X".repeat(6_000_000)
                } else {
                    format!("Map{id} {}", "long ".repeat(160))
                };
                song.marker_sets.insert(MarkerSetId(id), set);
            }
            crate::project::check_serialized_size(&song).unwrap();
            editor.replace_document(song).unwrap();
            editor.edit_mode = true;
            let mut ui = TempoUi::default();
            ui.sync(&editor, SampleId(1), &asset);
            ui.draft.min_bpm = "119.9".into();
            ui.draft.max_bpm = "120.1".into();
            let ctx = egui::Context::default();
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "Choose accepted markers",
                false,
            );
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            let label = marker_label(
                MarkerSetId(128),
                &editor.song.marker_sets[&MarkerSetId(128)].name,
            );
            menu_choice(&ctx, &mut ui, &mut editor, &asset, size, &label);
            assert_eq!(ui.draft.marker, Some(MarkerSetId(128)));
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "Analyze tempo / pulse",
                true,
            );
            let work = click(&ctx, &mut ui, &mut editor, &asset, size, p, true).unwrap();
            ui.finish(execute(work).unwrap());
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "candidate120",
                true,
            );
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "BPM target: markers 128",
                true,
            );
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            let label = marker_label(
                MarkerSetId(127),
                &editor.song.marker_sets[&MarkerSetId(127)].name,
            );
            menu_choice(&ctx, &mut ui, &mut editor, &asset, size, &label);
            assert_eq!(ui.target, Some(MarkerSetId(127)));
            let p = control(
                &ctx,
                &mut ui,
                &mut editor,
                &asset,
                size,
                "Use selected BPM only",
                true,
            );
            let before = editor.song.clone();
            click(&ctx, &mut ui, &mut editor, &asset, size, p, true);
            let mut expected = before.clone();
            expected
                .marker_sets
                .get_mut(&MarkerSetId(127))
                .unwrap()
                .source_bpm = Some(120.0);
            assert_eq!(editor.song, expected);
            editor.undo();
            assert_eq!(editor.song, before);
        }
    }
}
