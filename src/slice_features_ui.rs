//! Bounded native descriptor review. Measurements and user-authored roles are
//! separate metadata; neither is playback authority or an automatic classifier.
use crate::{
    editor::Editor,
    onset_service::CachedSource,
    sample_asset,
    slice_features::{
        self, Articulation, CatalogPreview, FeaturePreview, FeatureRequest, FeatureSnapshot,
        PercussionRole, RoleChoice, RoleLabel,
    },
    song::{RegionId, SampleAsset, SampleId, Song},
    tracker_ui::AQUA,
};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};
const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const WARN: Color32 = Color32::from_rgb(229, 185, 112);

pub(crate) struct AnalyzeRequest {
    song: Box<Song>,
    revision: u64,
    request: FeatureRequest,
    path: PathBuf,
    cache: Option<Arc<CachedSource>>,
}
pub(crate) struct Measured {
    revision: u64,
    preview: Arc<FeaturePreview>,
    reused_cache: bool,
}
pub(crate) struct Verification {
    revision: u64,
    preview_id: String,
    selection: Vec<RegionId>,
    asset: SampleAsset,
    path: PathBuf,
    plan: Arc<CatalogPreview>,
}
pub(crate) enum WorkRequest {
    Analyze(Box<AnalyzeRequest>),
    Verify(Box<Verification>),
}
pub(crate) enum WorkResult {
    Measured(Box<Measured>),
    Verified(Box<Verification>),
}
pub(crate) fn execute(request: WorkRequest) -> Result<WorkResult, String> {
    match request {
        WorkRequest::Analyze(request) => {
            let plan = slice_features::prepare(&request.song, request.request)?;
            let reused_cache = request.cache.is_some();
            let preview = if let Some(cache) = request.cache {
                sample_asset::verify_identity(plan.asset(), &request.path)?;
                slice_features::analyze_cached(&plan, &cache)?
            } else {
                slice_features::analyze_verified(&plan, &request.path)?
            };
            Ok(WorkResult::Measured(Box::new(Measured {
                revision: request.revision,
                preview: Arc::new(preview),
                reused_cache,
            })))
        }
        WorkRequest::Verify(request) => {
            sample_asset::verify_identity(&request.asset, &request.path)?;
            Ok(WorkResult::Verified(request))
        }
    }
}
struct RoleDraft {
    region: RegionId,
    revision: u64,
    base: Arc<Song>,
    label: RoleLabel,
}
#[derive(Default)]
pub(crate) struct SliceFeaturesUi {
    source: Option<SampleId>,
    selection: BTreeSet<RegionId>,
    focused: Option<RegionId>,
    measured: Option<Measured>,
    choices: BTreeMap<RegionId, RoleLabel>,
    catalog: Option<(Vec<RoleChoice>, Arc<CatalogPreview>)>,
    plan_key: Option<Vec<RoleChoice>>,
    role: Option<RoleDraft>,
    authorized: bool,
    verified: Option<Verification>,
    error: Option<String>,
    notice: String,
}
pub(crate) struct Context<'a> {
    pub source: SampleId,
    pub asset: &'a SampleAsset,
    pub path: PathBuf,
    pub cache: Option<&'a Arc<CachedSource>>,
    pub busy: bool,
}
impl SliceFeaturesUi {
    pub(crate) fn agent_action(
        &mut self,
        action: &serde_json::Value,
        editor: &mut Editor,
        context: Context<'_>,
    ) -> Result<Option<WorkRequest>, String> {
        let op = action
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .ok_or("catalog operation required")?;
        if self.source != Some(context.source) {
            self.reset();
            self.source = Some(context.source)
        }
        match op {
            "select" => {
                let id = RegionId(
                    action
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("id must be unsigned")?,
                );
                if editor
                    .song
                    .regions
                    .get(&id)
                    .is_none_or(|r| r.sample != context.source)
                {
                    return Err("region missing or belongs to another source".into());
                };
                let selected = action
                    .get("selected")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                if selected {
                    if self.selection.len() >= slice_features::MAX_REGIONS
                        && !self.selection.contains(&id)
                    {
                        return Err(format!(
                            "at most {} regions per analysis",
                            slice_features::MAX_REGIONS
                        ));
                    };
                    self.selection.insert(id);
                } else {
                    self.selection.remove(&id);
                }
                self.cancel_commit();
                Ok(None)
            }
            "select_all" => {
                let selection: BTreeSet<_> = editor
                    .song
                    .regions
                    .iter()
                    .filter_map(|(&id, r)| (r.sample == context.source).then_some(id))
                    .collect();
                if selection.len() > slice_features::MAX_REGIONS {
                    return Err("too many regions for one analysis; select a subset".into());
                };
                self.selection = selection;
                self.cancel_commit();
                Ok(None)
            }
            "clear_selection" => {
                self.selection.clear();
                self.cancel_commit();
                Ok(None)
            }
            "focus" => {
                let id = RegionId(
                    action
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("id must be unsigned")?,
                );
                if editor
                    .song
                    .regions
                    .get(&id)
                    .is_none_or(|r| r.sample != context.source)
                {
                    return Err("region missing or belongs to another source".into());
                };
                self.focus(editor, id);
                Ok(None)
            }
            "clear_role" | "clear_descriptors" => {
                if !editor.edit_mode {
                    return Err("edit mode required".into());
                };
                let id = RegionId(
                    action
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("id must be unsigned")?,
                );
                let inspection = slice_features::inspect_catalog(&editor.song, id)?;
                if inspection.current_binding.sample != context.source {
                    return Err("region belongs to another source".into());
                };
                self.cancel_commit();
                if op == "clear_role" {
                    editor.set_region_role(id, None)?
                } else {
                    editor.clear_region_features(id)?
                };
                self.focus(editor, id);
                Ok(None)
            }
            "analyze" => {
                if context.busy || self.selection.is_empty() {
                    return Err("catalog analysis needs an idle worker and selected regions".into());
                };
                let request = FeatureRequest {
                    version: slice_features::VERSION,
                    sample: context.source,
                    regions: self.selected_ids(),
                };
                self.cancel_commit();
                self.measured = None;
                self.catalog = None;
                self.plan_key = None;
                self.choices.clear();
                self.error = None;
                self.notice =
                    "Analyzing native descriptors; document and playback remain unchanged.".into();
                Ok(Some(WorkRequest::Analyze(Box::new(AnalyzeRequest {
                    song: Box::new(editor.song.clone()),
                    revision: editor.revision(),
                    request,
                    path: context.path,
                    cache: context.cache.cloned(),
                }))))
            }
            "set_role" => {
                let id = RegionId(
                    action
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("id must be unsigned")?,
                );
                let label: RoleLabel = serde_json::from_value(
                    action
                        .get("label")
                        .cloned()
                        .ok_or("label object required")?,
                )
                .map_err(|e| e.to_string())?;
                if editor
                    .song
                    .regions
                    .get(&id)
                    .is_none_or(|r| r.sample != context.source)
                {
                    return Err("region missing or belongs to another source".into());
                };
                self.choices.insert(id, label.clone());
                self.role = Some(RoleDraft {
                    region: id,
                    revision: editor.revision(),
                    base: Arc::new(editor.song.clone()),
                    label,
                });
                self.cancel_commit();
                self.update_plan();
                Ok(None)
            }
            "save_role" => {
                if !editor.edit_mode {
                    return Err("edit mode required".into());
                };
                let d = self
                    .role
                    .as_ref()
                    .ok_or("set a reviewed role draft first")?;
                if d.revision != editor.revision() || d.base.as_ref() != &editor.song {
                    return Err("role draft is stale".into());
                };
                editor.set_region_role(d.region, Some(d.label.clone()))?;
                self.focus(editor, d.region);
                Ok(None)
            }
            "store" => {
                if !editor.edit_mode
                    || context.busy
                    || !self.current(editor, context.source, context.asset)
                {
                    return Err(
                        "store requires edit mode, idle worker and current measurements".into(),
                    );
                };
                self.update_plan();
                let (_, plan) = self
                    .catalog
                    .as_ref()
                    .ok_or("catalog store preview unavailable")?;
                self.authorized = true;
                self.verified = None;
                Ok(Some(WorkRequest::Verify(Box::new(Verification {
                    revision: editor.revision(),
                    preview_id: self
                        .measured
                        .as_ref()
                        .ok_or("measurements missing")?
                        .preview
                        .id()
                        .into(),
                    selection: self.selected_ids(),
                    asset: context.asset.clone(),
                    path: context.path,
                    plan: Arc::clone(plan),
                }))))
            }
            "commit_verified" => {
                let verified = self
                    .verified
                    .take()
                    .ok_or("no verified catalog store waiting")?;
                let valid = self.authorized
                    && editor.edit_mode
                    && self.current(editor, context.source, context.asset)
                    && verified.revision == editor.revision()
                    && verified.asset == *context.asset
                    && verified.selection == self.selected_ids()
                    && self
                        .measured
                        .as_ref()
                        .is_some_and(|m| m.preview.id() == verified.preview_id)
                    && self
                        .catalog
                        .as_ref()
                        .is_some_and(|(_, p)| p.id() == verified.plan.id());
                self.authorized = false;
                if !valid {
                    return Err("catalog verification is stale or canceled".into());
                };
                editor.commit_region_catalog_preview(verified.revision, &verified.plan)?;
                self.notice =
                    "Stored reviewed catalog metadata (one undo); playback is unchanged.".into();
                Ok(None)
            }
            "cancel" => {
                self.cancel_commit();
                Ok(None)
            }
            _ => Err(format!("unsupported catalog operation: {op}")),
        }
    }
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn cancel_commit(&mut self) {
        self.authorized = false;
        self.verified = None;
    }
    pub(crate) fn fail(&mut self, error: String) {
        self.cancel_commit();
        self.error = Some(error);
        self.notice.clear();
    }
    pub(crate) fn finish(&mut self, result: WorkResult) {
        match result {
            WorkResult::Measured(measured) => {
                self.cancel_commit();
                self.measured = Some(*measured);
                self.catalog = None;
                self.plan_key = None;
                self.choices.clear();
                self.error = None;
                self.notice="Measurements ready for review only. No descriptors or human roles have been stored.".into();
            }
            WorkResult::Verified(verified) => {
                if self.authorized {
                    self.verified = Some(*verified);
                }
            }
        }
    }
    fn selected_ids(&self) -> Vec<RegionId> {
        self.selection.iter().copied().collect()
    }
    fn current(&self, editor: &Editor, source: SampleId, asset: &SampleAsset) -> bool {
        self.measured.as_ref().is_some_and(|m| {
            m.revision == editor.revision()
                && m.preview.base() == &editor.song
                && m.preview.request().sample == source
                && m.preview.asset() == asset
                && m.preview.request().regions == self.selected_ids()
        })
    }
    fn focus(&mut self, editor: &Editor, id: RegionId) {
        self.focused = Some(id);
        let label = self
            .choices
            .get(&id)
            .cloned()
            .or_else(|| {
                slice_features::inspect_catalog(&editor.song, id)
                    .ok()
                    .and_then(|i| i.entry)
                    .and_then(|e| e.reviewed_role)
                    .map(|r| r.label)
            })
            .unwrap_or(RoleLabel {
                role: PercussionRole::Unknown,
                articulation: Articulation::Unspecified,
                note: String::new(),
            });
        self.role = Some(RoleDraft {
            region: id,
            revision: editor.revision(),
            base: Arc::new(editor.song.clone()),
            label,
        });
    }
    fn update_plan(&mut self) {
        let Some(measured) = &self.measured else {
            return;
        };
        let choices = self
            .choices
            .iter()
            .map(|(&region, label)| RoleChoice {
                region,
                label: label.clone(),
            })
            .collect::<Vec<_>>();
        if self.plan_key.as_ref() == Some(&choices) {
            return;
        }
        self.plan_key = Some(choices.clone());
        match measured.preview.catalog_preview(choices.clone()) {
            Ok(plan) => {
                self.catalog = Some((choices, Arc::new(plan)));
                self.error = None;
            }
            Err(error) => {
                self.catalog = None;
                self.error = Some(error);
                self.notice.clear();
            }
        }
    }
    fn finish_verified(&mut self, editor: &mut Editor, context: &Context<'_>, enabled: bool) {
        if !enabled || !editor.edit_mode {
            self.cancel_commit();
            return;
        }
        let Some(verified) = self.verified.take() else {
            return;
        };
        let current = self.authorized
            && self.current(editor, context.source, context.asset)
            && verified.revision == editor.revision()
            && verified.asset == *context.asset
            && verified.selection == self.selected_ids()
            && self
                .measured
                .as_ref()
                .is_some_and(|m| m.preview.id() == verified.preview_id)
            && self
                .catalog
                .as_ref()
                .is_some_and(|(_, p)| p.id() == verified.plan.id());
        self.authorized = false;
        if !current {
            self.fail(
                "Catalog Store became stale; analyze and review again. No metadata was stored."
                    .into(),
            );
            return;
        }
        let before = editor.revision();
        match editor.commit_region_catalog_preview(verified.revision, &verified.plan) {
            Ok(()) => {
                self.error = None;
                self.notice=if before==editor.revision(){"Catalog already matched; no history entry added."}else{"Stored native descriptors and only explicitly reviewed labels (one undo). Source, regions and playback are unchanged."}.into();
            }
            Err(error) => self.fail(error),
        }
    }
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        context: Context<'_>,
    ) -> Option<WorkRequest> {
        if self.source != Some(context.source) {
            self.reset();
            self.source = Some(context.source);
        }
        self.finish_verified(editor, &context, ui.is_enabled());
        let mut request = None;
        let available = editor
            .song
            .regions
            .iter()
            .filter_map(|(&id, r)| (r.sample == context.source).then_some(id))
            .collect::<Vec<_>>();
        self.selection.retain(|id| available.contains(id));
        if self.focused.is_some_and(|id| !available.contains(&id)) {
            self.focused = None;
            self.role = None;
        }
        ui.label(
            RichText::new("SLICE CATALOG · native descriptors / explicit human review")
                .color(AQUA)
                .strong(),
        );
        ui.label(RichText::new("Advisory metadata, not a trained classifier. Mixed / Unknown are normal; Ghost is manual only. No reviewed role is assigned or stored automatically; playback is unchanged.").small().color(WARN));
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "S{} · {} regions · selected {} / {}",
                context.source.0,
                available.len(),
                self.selection.len(),
                slice_features::MAX_REGIONS
            ));
            if ui
                .add_enabled(
                    available.len() <= slice_features::MAX_REGIONS,
                    egui::Button::new("Select all regions"),
                )
                .clicked()
            {
                self.selection = available.iter().copied().collect();
                self.cancel_commit();
            }
            if ui.button("Clear selection").clicked() {
                self.selection.clear();
                self.cancel_commit();
            }
            if ui
                .add_enabled(
                    !context.busy && !self.selection.is_empty(),
                    egui::Button::new("Analyze selected regions"),
                )
                .clicked()
            {
                let feature_request = FeatureRequest {
                    version: slice_features::VERSION,
                    sample: context.source,
                    regions: self.selected_ids(),
                };
                self.cancel_commit();
                self.measured = None;
                self.catalog = None;
                self.plan_key = None;
                self.choices.clear();
                self.error = None;
                self.notice =
                    "Analyzing on the source worker; document and playback remain unchanged."
                        .into();
                request = Some(WorkRequest::Analyze(Box::new(AnalyzeRequest {
                    song: Box::new(editor.song.clone()),
                    revision: editor.revision(),
                    request: feature_request,
                    path: context.path.clone(),
                    cache: context.cache.cloned(),
                })));
            }
        });
        if context.busy {
            ui.label(RichText::new(if self.authorized{"Checking source identity; guarded completion stores only the reviewed catalog metadata."}else{"Source worker running. No catalog Store is authorized."}).small().color(WARN));
            ui.ctx().request_repaint();
        }
        let current = self.current(editor, context.source, context.asset);
        let body_height = (ui.ctx().content_rect().height() - 430.0).clamp(150.0, 450.0);
        egui::ScrollArea::vertical().id_salt("slice-catalog-body").max_height(body_height).show(ui,|ui|{
            ui.label(RichText::new("Committed regions · check to measure; select R-ID to inspect").small().color(MUTED));
            egui::ScrollArea::vertical().id_salt("slice-catalog-regions").max_height(122.0).show_rows(ui,22.0,available.len(),|ui,rows|{for index in rows{
                let id=available[index];let region=&editor.song.regions[&id];let mut checked=self.selection.contains(&id);
                ui.horizontal(|ui|{
                    if ui.checkbox(&mut checked,"").changed(){if checked{if self.selection.len()>=slice_features::MAX_REGIONS{self.fail(format!("At most {} regions per job; selection was not truncated.",slice_features::MAX_REGIONS));}else{self.selection.insert(id);self.cancel_commit();}}else{self.selection.remove(&id);self.cancel_commit();}}
                    if ui.selectable_label(self.focused==Some(id),format!("R{}",id.0)).clicked(){self.focus(editor,id);}
                    ui.add(egui::Label::new(format!("[{}, {}) · {}",region.start_frame,region.end_frame,bounded_name(&region.name))).truncate()).on_hover_text(format!("Region {} · source {} · exact native frame bounds [{}, {})\nName has {} UTF-8 bytes; long labels are explicitly elided.",id.0,region.sample.0,region.start_frame,region.end_frame,region.name.len()));
                });
            }});
            if let Some(measured)=&self.measured{
                ui.separator();ui.label(RichText::new(format!("{} · {} measured regions · revision {} · {}",if current{"MEASURED REVIEW"}else{"STALE MEASURED REVIEW"},measured.preview.records().len(),measured.revision,if measured.reused_cache{"native PCM cache reused; new feature FFTs"}else{"strictly loaded native source; new feature FFTs"})).color(if current{AQUA}else{WARN}));
                egui::CollapsingHeader::new("Measurement identity / layout / full warnings").show(ui,|ui|{
                    let summary=measured.preview.summary();ui.monospace(format!("{} · S{} · {} Hz / {} channels",summary.algorithm,summary.sample.0,summary.source_rate,summary.channels));
                    ui.monospace(format!("Source SHA-256 {}",summary.source_sha256));ui.monospace(format!("Preview {}",measured.preview.id()));ui.label(format!("Visited {} native frames; {} FFT windows; estimated work {}",summary.visited_frames,summary.fft_windows,summary.estimated_work));
                    layout(ui,&summary.layout,summary.source_rate);for warning in &summary.warnings{ui.label(RichText::new(warning).small().color(WARN));}
                });
            }
            if let Some(id)=self.focused {self.region_details(ui,editor,id,current);}
            else{ui.label("Select a committed R-ID to inspect native metrics, suggestions and the separate human-reviewed role.");}
            self.update_plan();
            if let Some((_,plan))=&self.catalog{
                egui::CollapsingHeader::new("Catalog Store details").show(ui,|ui|{
                    ui.monospace(format!("Plan {}",plan.id()));
                    egui::ScrollArea::vertical().id_salt("catalog-store-regions").max_height(100.0).show_rows(ui,18.0,plan.summary().stored_features.len(),|ui,rows|{for row in rows{let id=plan.summary().stored_features[row];let role=self.choices.get(&id).map_or_else(||"preserve prior human review".into(),|label|format!("explicit {} / {}",role_name(label.role),articulation_name(label.articulation)));ui.monospace(format!("R{} · store measured descriptors · {}",id.0,role));}});
                    for warning in &plan.summary().warnings{ui.label(RichText::new(warning).small().color(WARN));}
                });
            }
        });
        let current = self.current(editor, context.source, context.asset);
        self.update_plan();
        ui.separator();
        if let Some(id) = self.focused {
            if let Ok(inspection) = slice_features::inspect_catalog(&editor.song, id) {
                ui.label(RichText::new(format!("Focused R{} / S{} · stored descriptors: {} · human review: {}",id.0,inspection.current_binding.sample.0,binding_status(inspection.features_current_match),binding_status(inspection.reviewed_role_current_match))).small().color(AQUA)).on_hover_text(format!("Current model bounds [{}, {}) SOURCE frames. Status is model-binding equality, not a fresh external-file check.",inspection.current_binding.start_frame,inspection.current_binding.end_frame));
            }
        }
        if let Some((_, plan)) = &self.catalog {
            ui.label(RichText::new(format!("{} · store {} descriptor(s), {} explicit reviewed role(s); preserve {} existing review(s)",if current{"READY"}else{"STALE"},plan.summary().stored_features.len(),plan.summary().reviewed_roles.len(),plan.summary().preserved_review_count)).color(if current{AQUA}else{WARN}));
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    editor.edit_mode && current && !context.busy && self.catalog.is_some(),
                    egui::Button::new("Store reviewed catalog"),
                )
                .clicked()
            {
                self.authorized = true;
                self.verified = None;
                self.error = None;
                request = Some(WorkRequest::Verify(Box::new(Verification {
                    revision: editor.revision(),
                    preview_id: self.measured.as_ref().unwrap().preview.id().into(),
                    selection: self.selected_ids(),
                    asset: context.asset.clone(),
                    path: context.path,
                    plan: Arc::clone(&self.catalog.as_ref().unwrap().1),
                })));
            }
            if self.authorized && ui.button("Cancel catalog Store").clicked() {
                self.cancel_commit();
                self.notice =
                    "Catalog Store canceled; worker remains tracked until completion.".into();
            }
            ui.label(
                RichText::new(
                    "Store rechecks source bytes off-thread. One undo; no audio or note changes.",
                )
                .small()
                .color(MUTED),
            );
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if !self.notice.is_empty() {
            ui.label(RichText::new(&self.notice).small().color(MUTED));
        }
        request
    }
    fn region_details(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: RegionId,
        current: bool,
    ) {
        let inspection = match slice_features::inspect_catalog(&editor.song, id) {
            Ok(value) => value,
            Err(error) => {
                ui.colored_label(Color32::LIGHT_RED, error);
                return;
            }
        };
        ui.separator();
        ui.label(
            RichText::new(format!(
                "R{} · S{} · native source [{}, {})",
                id.0,
                inspection.current_binding.sample.0,
                inspection.current_binding.start_frame,
                inspection.current_binding.end_frame
            ))
            .color(AQUA),
        );
        ui.label(
            RichText::new(format!(
                "Stored descriptors: {} · reviewed role: {}",
                binding_status(inspection.features_current_match),
                binding_status(inspection.reviewed_role_current_match)
            ))
            .small()
            .color(WARN),
        );
        ui.label(RichText::new("Current-match means registered source / bounds match only; not a fresh file check or proof of classification. Source / bounds edits retain historical evidence.").small().color(MUTED));
        for warning in &inspection.warnings {
            ui.label(RichText::new(warning).small().color(WARN));
        }
        if let Some(role) = inspection
            .entry
            .as_ref()
            .and_then(|entry| entry.reviewed_role.as_ref())
        {
            ui.label(format!(
                "PERSISTED HUMAN REVIEW: {} / {}",
                role_name(role.label.role),
                articulation_name(role.label.articulation)
            ));
            ui.label(
                RichText::new(if role.based_on_feature_sha256.is_some() {
                    "Explicit review linked to a historical feature snapshot."
                } else {
                    "Manual model-bound annotation; no measured-source verification claim."
                })
                .small()
                .color(MUTED),
            );
            if !role.label.note.is_empty() {
                ui.label(&role.label.note);
            }
        } else {
            ui.label("PERSISTED HUMAN REVIEW: not reviewed (not an automatic Unknown assignment)");
        }
        let measured = self
            .measured
            .as_ref()
            .and_then(|m| {
                m.preview
                    .records()
                    .iter()
                    .find(|record| record.region == id)
            })
            .map(|r| r.snapshot.clone());
        let snapshot = measured.as_ref().or_else(|| {
            inspection
                .entry
                .as_ref()
                .and_then(|entry| entry.features.as_ref())
        });
        if let Some(snapshot) = snapshot {
            ui.label(
                RichText::new(if measured.is_some() {
                    if current {
                        "UNSTORED MEASUREMENT REVIEW"
                    } else {
                        "STALE MEASUREMENT REVIEW"
                    }
                } else {
                    "STORED HISTORICAL DESCRIPTORS"
                })
                .color(AQUA),
            );
            metrics(ui, snapshot);
            egui::CollapsingHeader::new("Heuristic role suggestions · never human labels").default_open(true).show(ui,|ui|{
                ui.label(RichText::new(format!("Status {:?}; rule support is a handcrafted rule fraction, not probability or calibrated confidence. No automatic Ghost detection.",snapshot.suggestion_status)).small().color(WARN));
                for suggestion in &snapshot.suggestions{ui.horizontal_wrapped(|ui|{
                    ui.label(format!("{} · rule support {:.3}",role_name(suggestion.role),suggestion.rule_support)).on_hover_text(format!("Exact rule fraction {:?}",suggestion.rule_support));
                    if ui.button(format!("Review {}",role_name(suggestion.role))).clicked(){if let Some(draft)=&mut self.role{draft.label.role=suggestion.role;}}
                });egui::CollapsingHeader::new(format!("{} rule evidence",role_name(suggestion.role))).show(ui,|ui|{for rule in &suggestion.rules{ui.label(format!("{} · {}",if rule.matched{"matched"}else{"not matched"},rule.rule));}});}
                for warning in &snapshot.warnings{ui.label(RichText::new(warning).small().color(WARN));}
            });
        } else {
            ui.label(
                "No native feature snapshot for this region. Manual review remains available.",
            );
        }
        ui.separator();
        ui.label(RichText::new("EXPLICIT HUMAN ROLE DRAFT · no automatic acceptance").strong());
        if self.role.as_ref().is_none_or(|draft| draft.region != id) {
            self.focus(editor, id);
        }
        let draft = self.role.as_mut().unwrap();
        let role_current =
            draft.revision == editor.revision() && draft.base.as_ref() == &editor.song;
        if !role_current {
            ui.label(
                RichText::new(
                    "Role draft is stale; reload it before saving or including it in Store.",
                )
                .color(WARN),
            );
        }
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("slice-role")
                .selected_text(role_name(draft.label.role))
                .show_ui(ui, |ui| {
                    for value in [
                        PercussionRole::Unknown,
                        PercussionRole::Mixed,
                        PercussionRole::Kick,
                        PercussionRole::Snare,
                        PercussionRole::ClosedHat,
                        PercussionRole::OpenHatRide,
                        PercussionRole::Crash,
                    ] {
                        ui.selectable_value(&mut draft.label.role, value, role_name(value));
                    }
                });
            egui::ComboBox::from_id_salt("slice-articulation")
                .selected_text(articulation_name(draft.label.articulation))
                .show_ui(ui, |ui| {
                    for value in [
                        Articulation::Unspecified,
                        Articulation::Normal,
                        Articulation::Ghost,
                    ] {
                        ui.selectable_value(
                            &mut draft.label.articulation,
                            value,
                            articulation_name(value),
                        );
                    }
                });
            ui.label(
                RichText::new(
                    "Ghost is an explicit contextual human choice, not inferred from low RMS.",
                )
                .small()
                .color(MUTED),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Review note");
            ui.add(
                egui::TextEdit::singleline(&mut draft.label.note)
                    .id(egui::Id::new("slice-review-note"))
                    .desired_width(350.0),
            );
        });
        let mut include = self.choices.contains_key(&id);
        let inclusion = ui.add_enabled(
            role_current && current && measured.is_some(),
            egui::Checkbox::new(&mut include, "Include this explicit role in catalog Store"),
        );
        if role_current && current && measured.is_some() {
            if include {
                self.choices.insert(id, draft.label.clone());
            } else if inclusion.changed() {
                self.choices.remove(&id);
            }
        }
        let label = draft.label.clone();
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Reload role draft").clicked() {
                action = Some(0);
            }
            if ui
                .add_enabled(
                    editor.edit_mode && role_current,
                    egui::Button::new("Save manual role only"),
                )
                .clicked()
            {
                action = Some(1);
            }
            if ui
                .add_enabled(
                    editor.edit_mode
                        && role_current
                        && inspection.reviewed_role_current_match.is_some(),
                    egui::Button::new("Clear human role"),
                )
                .clicked()
            {
                action = Some(2);
            }
            if ui
                .add_enabled(
                    editor.edit_mode && role_current && inspection.features_current_match.is_some(),
                    egui::Button::new("Clear descriptors only"),
                )
                .clicked()
            {
                action = Some(3);
            }
        });
        if let Some(action) = action {
            self.cancel_commit();
            let before_revision = editor.revision();
            let result = match action {
                0 => Ok(()),
                1 => editor.set_region_role(id, Some(label)),
                2 => editor.set_region_role(id, None),
                _ => editor.clear_region_features(id),
            };
            match result {
                Ok(()) => {
                    self.error = None;
                    self.notice=match action{0=>"Role draft reloaded; no document change.",1=>"Saved explicit manual role only (one undo). No source verification or playback change.",2=>"Cleared human role only; descriptors were preserved.",_=>"Cleared descriptors only; the human role was preserved."}.into();
                    if action != 0 && before_revision == editor.revision() {
                        self.notice = "Metadata already matched; no history entry added.".into();
                    }
                    self.choices.remove(&id);
                    self.focus(editor, id);
                }
                Err(error) => self.fail(error),
            }
        }
    }
    pub(crate) fn capture_state(
        &self,
        editor: &Editor,
        source: Option<SampleId>,
        asset: Option<&SampleAsset>,
    ) -> serde_json::Value {
        serde_json::json!({"selection":self.selected_ids(),"focused":self.focused,"current":source.zip(asset).is_some_and(|(source,asset)|self.current(editor,source,asset)),"authorized":self.authorized,"verified_waiting":self.verified.is_some(),"error":self.error,"notice":self.notice,"role_draft":self.role.as_ref().map(|draft|serde_json::json!({"region":draft.region,"revision":draft.revision,"label":draft.label,"current":draft.revision==editor.revision()&&draft.base.as_ref()==&editor.song})),"choices":self.choices,"measured":self.measured.as_ref().map(|m|serde_json::json!({"id":m.preview.id(),"revision":m.revision,"request":m.preview.request(),"summary":m.preview.summary(),"records":m.preview.records(),"reused_cache":m.reused_cache})),"catalog_preview":self.catalog.as_ref().map(|(_,p)|serde_json::json!({"id":p.id(),"summary":p.summary(),"candidate":p.candidate()})),"inspection":self.focused.and_then(|id|slice_features::inspect_catalog(&editor.song,id).ok())})
    }
}
fn bounded_name(name: &str) -> String {
    let mut chars = name.chars();
    let prefix: String = chars.by_ref().take(48).collect();
    format!("{prefix}{}", if chars.next().is_some() { "…" } else { "" })
}
fn role_name(role: PercussionRole) -> &'static str {
    match role {
        PercussionRole::Kick => "Kick",
        PercussionRole::Snare => "Snare",
        PercussionRole::ClosedHat => "Closed hat",
        PercussionRole::OpenHatRide => "Open hat / ride",
        PercussionRole::Crash => "Crash",
        PercussionRole::Mixed => "Mixed",
        PercussionRole::Unknown => "Unknown",
    }
}
fn articulation_name(articulation: Articulation) -> &'static str {
    match articulation {
        Articulation::Unspecified => "Unspecified",
        Articulation::Normal => "Normal",
        Articulation::Ghost => "Ghost · manual",
    }
}
fn binding_status(value: Option<bool>) -> &'static str {
    match value {
        None => "none",
        Some(true) => "current model match",
        Some(false) => "STALE / historical",
    }
}
fn band_label(value: &slice_features::FeatureLayout, rate: u32) -> String {
    let nyquist = f64::from(rate) / 2.0;
    let upper = f64::from(value.high_hz).min(nyquist);
    format!("Bands [{}, {}) / [{}, {}) / [{}, {}] Hz; high upper included, capped at min({}, native Nyquist {}).",value.low_hz,value.low_mid_hz,value.low_mid_hz,value.mid_high_hz,value.mid_high_hz,upper,value.high_hz,nyquist)
}
fn layout(ui: &mut egui::Ui, value: &slice_features::FeatureLayout, rate: u32) {
    ui.label(format!(
        "FFT {} / hop {} native frames · envelope bin {} native frames · {}",
        value.fft_size, value.hop_frames, value.envelope_bin_frames, value.window
    ));
    ui.label(band_label(value, rate));
    ui.label(format!(
        "Flatness floor relative {:?}; shape-support band/total AC floor {:?} (not absolute loudness)",
        value.flatness_floor_relative, value.shape_support_floor_relative
    ));
}
fn metrics(ui: &mut egui::Ui, snapshot: &FeatureSnapshot) {
    let m = &snapshot.metrics;
    let rate = f64::from(snapshot.binding.source_rate);
    ui.label(RichText::new("NATIVE METRICS · before pitch, gain, ADSR, loop or mix").strong());
    ui.label(
        RichText::new(
            "Displayed decimals are rounded; exact values and layout are available below.",
        )
        .small()
        .color(MUTED),
    );
    egui::Grid::new("slice-native-metrics")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            let mut row = |name: &str, value: String| {
                ui.label(name);
                ui.monospace(value);
                ui.end_row();
            };
            row(
                "Duration",
                format!("{} frames / {:.6} s", m.frames, m.frames as f64 / rate),
            );
            row(
                "RMS / peak · linear amplitude",
                format!(
                    "{} / {} · peak SOURCE frame {}",
                    scalar(m.rms),
                    scalar(m.peak),
                    m.peak_frame
                ),
            );
            row(
                "DC per channel · linear amplitude",
                format!("{:?}", m.dc_per_channel),
            );
            row("Crest · linear ratio", optional(m.crest_factor));
            row("Analysed AC power", optional(m.analysed_power));
            row(
                "Low / mid / high power fractions",
                format!(
                    "{} / {} / {}",
                    optional(m.low_fraction),
                    optional(m.mid_fraction),
                    optional(m.high_fraction)
                ),
            );
            row(
                "Centroid · Hz / flatness · ratio",
                format!("{} / {}", optional(m.centroid_hz), optional(m.flatness)),
            );
            row(
                "Spectral shape change · L1 mean / max",
                format!(
                    "{} / {}",
                    optional(m.shape_change_mean),
                    optional(m.shape_change_max)
                ),
            );
            row(
                "FFT / raw positive / shape-supported / pairs",
                format!(
                    "{} / {} / {} / {}",
                    m.spectral_windows,
                    m.nonzero_spectral_windows,
                    m.supported_shape_windows,
                    m.shape_comparisons
                ),
            );
            row(
                "Envelope peak · SOURCE frame",
                m.envelope_peak_frame
                    .map_or_else(|| "unavailable".into(), |v| v.to_string()),
            );
            row(
                "Attack 10–90% · native frames / ms",
                duration(m.attack_10_90_frames, rate),
            );
            row("Attack left-censored", m.attack_left_censored.to_string());
            row(
                "Tail ≥−20 dB activity · frames / ms",
                duration(m.tail_20db_frames, rate),
            );
            row(
                "Tail right-censored / lower bound",
                format!(
                    "{} / {}",
                    m.tail_right_censored,
                    duration(m.tail_lower_bound_frames, rate)
                ),
            );
            row(
                "Envelope rises · not proven hits",
                m.envelope_rises.to_string(),
            );
        });
    ui.label(RichText::new("Tail is activity after the global envelope peak, not clean decay or RT60. Censored bounds are not exact durations; unavailable values are not zero.").small().color(WARN));
    egui::CollapsingHeader::new("Exact descriptor values / native layout").show(ui, |ui| {
        layout(ui, &snapshot.layout, snapshot.binding.source_rate);
        ui.monospace(format!(
            "Binding SHA-256 {} · S{} / {} Hz / {} channels / [{}, {})",
            snapshot.binding.source_sha256,
            snapshot.binding.sample.0,
            snapshot.binding.source_rate,
            snapshot.binding.channels,
            snapshot.binding.start_frame,
            snapshot.binding.end_frame
        ));
        ui.monospace(format!("{:#?}", snapshot.metrics));
    });
}
fn scalar(value: f64) -> String {
    if value != 0.0 && (value.abs() < 0.000001 || value.abs() >= 1_000_000.0) {
        format!("{value:.8e}")
    } else {
        format!("{value:.8}")
    }
}
fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "unavailable".into(), scalar)
}
fn duration(value: Option<u64>, rate: f64) -> String {
    value.map_or_else(
        || "unavailable".into(),
        |v| format!("{v} / {:.6} ms", v as f64 * 1000.0 / rate),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{Cell, InstrumentId, Note, SampleRegion};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        editor: Editor,
        asset: SampleAsset,
        path: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    fn fixture() -> Fixture {
        let path = std::env::temp_dir().join(format!(
            "slice-ui-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for frame in 0..12000 {
            let t = frame as f64 / 48000.0;
            let value = if frame < 6000 {
                (std::f64::consts::TAU * 90.0 * t).sin() * (-t * 25.0).exp() * 0.6
            } else {
                0.0
            };
            writer.write_sample(value as f32).unwrap();
        }
        writer.finalize().unwrap();
        let asset = sample_asset::import(&path).unwrap().asset;
        let mut editor = Editor {
            edit_mode: true,
            edit_step: 0,
            ..Default::default()
        };
        editor
            .song
            .register_sample(SampleId(1), asset.clone())
            .unwrap();
        for (id, start, end) in [(1, 0, 6000), (2, 6000, 12000)] {
            editor
                .song
                .upsert_region(
                    RegionId(id),
                    SampleRegion {
                        name: format!("Region{id}"),
                        sample: SampleId(1),
                        start_frame: start,
                        end_frame: end,
                        root_note: 60,
                        amp_envelope: None,
                        forward_loop: None,
                    },
                )
                .unwrap();
        }
        for pattern in editor.song.patterns.values_mut() {
            pattern.tracks.clear();
        }
        editor
            .song
            .assign_region(InstrumentId(1), Some(RegionId(1)))
            .unwrap();
        let pattern = editor.pattern_id();
        let track = editor.track;
        editor
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
                    ..Default::default()
                },
            );
        editor.song.validate().unwrap();
        Fixture {
            editor,
            asset,
            path,
        }
    }
    fn analysis(f: &Fixture) -> WorkResult {
        execute(WorkRequest::Analyze(Box::new(AnalyzeRequest {
            song: Box::new(f.editor.song.clone()),
            revision: f.editor.revision(),
            request: FeatureRequest {
                version: 1,
                sample: SampleId(1),
                regions: vec![RegionId(1), RegionId(2)],
            },
            path: f.path.clone(),
            cache: None,
        })))
        .unwrap()
    }
    fn ready(f: &Fixture) -> SliceFeaturesUi {
        let mut view = SliceFeaturesUi {
            source: Some(SampleId(1)),
            selection: BTreeSet::from([RegionId(1), RegionId(2)]),
            ..Default::default()
        };
        view.finish(analysis(f));
        view.update_plan();
        view
    }
    fn verification(view: &SliceFeaturesUi, f: &Fixture) -> WorkRequest {
        WorkRequest::Verify(Box::new(Verification {
            revision: f.editor.revision(),
            preview_id: view.measured.as_ref().unwrap().preview.id().into(),
            selection: view.selected_ids(),
            asset: f.asset.clone(),
            path: f.path.clone(),
            plan: Arc::clone(&view.catalog.as_ref().unwrap().1),
        }))
    }
    #[test]
    fn measurement_review_store_and_manual_roles_are_separate_metadata_one_undo() {
        let mut f = fixture();
        let before = f.editor.song.clone();
        let cursor = (
            f.editor.occurrence,
            f.editor.track,
            f.editor.row,
            f.editor.column,
            f.editor.field,
        );
        let manifest = crate::song_render::evaluate(&before, None).unwrap();
        let mut view = ready(&f);
        assert_eq!(f.editor.song, before);
        assert!(view.current(&f.editor, SampleId(1), &f.asset));
        assert!(view.choices.is_empty());
        view.authorized = true;
        view.finish(execute(verification(&view, &f)).unwrap());
        let context = Context {
            source: SampleId(1),
            asset: &f.asset,
            path: f.path.clone(),
            cache: None,
            busy: false,
        };
        view.finish_verified(&mut f.editor, &context, true);
        assert_eq!(f.editor.song.region_catalog.len(), 2);
        assert!(f
            .editor
            .song
            .region_catalog
            .values()
            .all(|entry| entry.reviewed_role.is_none()));
        assert_eq!(
            (
                f.editor.occurrence,
                f.editor.track,
                f.editor.row,
                f.editor.column,
                f.editor.field
            ),
            cursor
        );
        assert_eq!(
            crate::song_render::evaluate(&f.editor.song, None).unwrap(),
            manifest
        );
        f.editor.undo();
        assert_eq!(f.editor.song, before);
        f.editor.redo();
        let stored = f.editor.song.clone();
        f.editor
            .set_region_role(
                RegionId(1),
                Some(RoleLabel {
                    role: PercussionRole::Mixed,
                    articulation: Articulation::Ghost,
                    note: "Explicit contextual decision".into(),
                }),
            )
            .unwrap();
        let inspection = slice_features::inspect_catalog(&f.editor.song, RegionId(1)).unwrap();
        assert_eq!(inspection.reviewed_role_current_match, Some(true));
        assert!(inspection
            .entry
            .unwrap()
            .reviewed_role
            .unwrap()
            .based_on_feature_sha256
            .is_none());
        f.editor.undo();
        assert_eq!(f.editor.song, stored);
    }
    #[test]
    fn explicit_feature_review_choices_preserve_other_roles_and_invalid_note_is_atomic() {
        let mut f = fixture();
        f.editor
            .set_region_role(
                RegionId(2),
                Some(RoleLabel {
                    role: PercussionRole::Unknown,
                    articulation: Articulation::Normal,
                    note: "Manual silence review".into(),
                }),
            )
            .unwrap();
        let before = f.editor.song.clone();
        let mut view = ready(&f);
        view.choices.insert(
            RegionId(1),
            RoleLabel {
                role: PercussionRole::Kick,
                articulation: Articulation::Ghost,
                note: "User reviewed, not low-level inferred".into(),
            },
        );
        view.update_plan();
        let plan = &view.catalog.as_ref().unwrap().1;
        assert_eq!(
            plan.candidate().region_catalog[&RegionId(2)].reviewed_role,
            before.region_catalog[&RegionId(2)].reviewed_role
        );
        assert!(plan.candidate().region_catalog[&RegionId(1)]
            .reviewed_role
            .as_ref()
            .unwrap()
            .based_on_feature_sha256
            .is_some());
        assert_eq!(f.editor.song, before);
        view.choices.get_mut(&RegionId(1)).unwrap().note = "x".repeat(257);
        view.update_plan();
        assert!(view.catalog.is_none());
        assert!(view.error.is_some());
        let key = view.plan_key.clone();
        view.update_plan();
        assert_eq!(view.plan_key, key);
        assert_eq!(f.editor.song, before);
    }
    #[test]
    fn late_verification_guards_revision_selection_read_mode_cancel_and_tamper() {
        for scenario in 0..5 {
            let mut f = fixture();
            let mut view = ready(&f);
            let request = verification(&view, &f);
            view.authorized = true;
            match scenario {
                0 => {
                    f.editor.song.title.push('x');
                }
                1 => {
                    view.selection.remove(&RegionId(2));
                }
                2 => {
                    f.editor.edit_mode = false;
                }
                3 => view.cancel_commit(),
                _ => {
                    let mut bytes = std::fs::read(&f.path).unwrap();
                    *bytes.last_mut().unwrap() ^= 1;
                    std::fs::write(&f.path, bytes).unwrap();
                }
            }
            let before = f.editor.song.clone();
            match execute(request) {
                Ok(result) => view.finish(result),
                Err(error) => view.fail(error),
            };
            let context = Context {
                source: SampleId(1),
                asset: &f.asset,
                path: f.path.clone(),
                cache: None,
                busy: false,
            };
            view.finish_verified(&mut f.editor, &context, true);
            assert_eq!(f.editor.song, before);
            assert!(f.editor.song.region_catalog.is_empty());
        }
    }
    #[test]
    fn shared_inspection_marks_bounds_stale_but_preserves_roles_and_native_options() {
        let mut f = fixture();
        let view = ready(&f);
        f.editor
            .commit_region_catalog_preview(f.editor.revision(), &view.catalog.as_ref().unwrap().1)
            .unwrap();
        f.editor
            .set_region_role(
                RegionId(1),
                Some(RoleLabel {
                    role: PercussionRole::Unknown,
                    articulation: Articulation::Unspecified,
                    note: String::new(),
                }),
            )
            .unwrap();
        let stored = f.editor.song.region_catalog[&RegionId(1)].clone();
        let mut region = f.editor.song.regions[&RegionId(1)].clone();
        region.name = "Renamed".into();
        region.root_note = 72;
        f.editor.upsert_region(RegionId(1), region.clone()).unwrap();
        assert_eq!(
            slice_features::inspect_catalog(&f.editor.song, RegionId(1))
                .unwrap()
                .features_current_match,
            Some(true)
        );
        region.start_frame += 1;
        f.editor.upsert_region(RegionId(1), region).unwrap();
        let inspection = slice_features::inspect_catalog(&f.editor.song, RegionId(1)).unwrap();
        assert_eq!(inspection.features_current_match, Some(false));
        assert_eq!(inspection.reviewed_role_current_match, Some(false));
        assert_eq!(inspection.entry, Some(stored));
    }
    #[test]
    fn display_units_missing_values_and_long_names_are_explicit_and_bounded() {
        assert_eq!(optional(None), "unavailable");
        assert!(optional(Some(1e-20)).contains("e-20"));
        assert_eq!(duration(None, 48000.0), "unavailable");
        assert_eq!(duration(Some(48), 48000.0), "48 / 1.000000 ms");
        let text = "音".repeat(2_000_000);
        assert_eq!(bounded_name(&text).chars().count(), 49);
        assert!(bounded_name(&text).ends_with('…'));
        assert_eq!(role_name(PercussionRole::Mixed), "Mixed");
        assert_eq!(articulation_name(Articulation::Ghost), "Ghost · manual");
    }
    #[test]
    fn cached_native_reuse_matches_verified_features_and_result_is_picker_independent() {
        let f = fixture();
        let config = crate::onset_service::experimental_config(crate::onset_v2::Preset::Balanced);
        let cache = crate::onset_service::prepare_verified(&f.asset, &f.path, &config)
            .unwrap()
            .cache;
        let plan = slice_features::prepare(
            &f.editor.song,
            FeatureRequest {
                version: 1,
                sample: SampleId(1),
                regions: vec![RegionId(1), RegionId(2)],
            },
        )
        .unwrap();
        let direct = slice_features::analyze_verified(&plan, &f.path).unwrap();
        let reused = slice_features::analyze_cached(&plan, &cache).unwrap();
        assert_eq!(direct.id(), reused.id());
        assert_eq!(direct.records(), reused.records());
        let mut view = SliceFeaturesUi {
            source: Some(SampleId(1)),
            selection: BTreeSet::from([RegionId(1), RegionId(2)]),
            ..Default::default()
        };
        view.finish(
            execute(WorkRequest::Analyze(Box::new(AnalyzeRequest {
                song: Box::new(f.editor.song.clone()),
                revision: f.editor.revision(),
                request: plan.request().clone(),
                path: f.path.clone(),
                cache: Some(Arc::clone(&cache)),
            })))
            .unwrap(),
        );
        assert!(view.measured.as_ref().unwrap().reused_cache);
        drop(cache);
        assert!(view.current(&f.editor, SampleId(1), &f.asset));
        assert_eq!(view.measured.as_ref().unwrap().preview.id(), direct.id());
    }
    #[test]
    fn moving_region_focus_preserves_explicit_pending_choice_without_auto_acceptance() {
        let f = fixture();
        let mut view = ready(&f);
        let label = RoleLabel {
            role: PercussionRole::Mixed,
            articulation: Articulation::Ghost,
            note: "Keep this explicit choice".into(),
        };
        view.choices.insert(RegionId(1), label.clone());
        view.focus(&f.editor, RegionId(2));
        assert!(!view.choices.contains_key(&RegionId(2)));
        view.focus(&f.editor, RegionId(1));
        assert_eq!(view.role.as_ref().unwrap().label, label);
        assert_eq!(f.editor.song.region_catalog.len(), 0);
    }
    #[test]
    fn native_layout_bounds_long_names_and_refuses_bulk_over_64_without_truncation() {
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let mut f = fixture();
            let template = f.editor.song.regions[&RegionId(2)].clone();
            for id in 3..=65 {
                f.editor
                    .song
                    .upsert_region(RegionId(id), template.clone())
                    .unwrap();
            }
            f.editor.song.regions.get_mut(&RegionId(1)).unwrap().name = "音".repeat(2_000_000);
            f.editor.song.validate().unwrap();
            let ctx = egui::Context::default();
            crate::tracker_ui::install_theme(&ctx);
            let mut view = SliceFeaturesUi::default();
            let draw = |view: &mut SliceFeaturesUi, editor: &mut Editor, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            assert!(view
                                .show(
                                    ui,
                                    editor,
                                    Context {
                                        source: SampleId(1),
                                        asset: &f.asset,
                                        path: f.path.clone(),
                                        cache: None,
                                        busy: false
                                    }
                                )
                                .is_none());
                        });
                    },
                )
            };
            let mut output = draw(&mut view, &mut f.editor, vec![]);
            for _ in 0..3 {
                output = draw(&mut view, &mut f.editor, vec![]);
            }
            let find = |output: &egui::FullOutput, label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                            let rect = text.visual_bounding_rect();
                            (shape.clip_rect.contains_rect(rect) && !text.galley.elided)
                                .then_some(rect.center())
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let point = find(&output, "Select all regions");
            for pressed in [true, false] {
                draw(
                    &mut view,
                    &mut f.editor,
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
            assert!(view.selection.is_empty());
            f.editor.song.regions.remove(&RegionId(65));
            output = draw(&mut view, &mut f.editor, vec![]);
            let point = find(&output, "Select all regions");
            for pressed in [true, false] {
                draw(
                    &mut view,
                    &mut f.editor,
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
            assert_eq!(view.selection.len(), 64);
            output = draw(&mut view, &mut f.editor, vec![]);
            find(&output, "Store reviewed catalog");
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    assert!(
                        text.galley.job.text.len() < 4096,
                        "Unbounded name was shaped"
                    );
                    if text.galley.job.text.starts_with('[') {
                        assert_eq!(
                            text.galley.rows.len(),
                            1,
                            "Virtualized region label wrapped into another row"
                        );
                        assert!(text.visual_bounding_rect().height() <= 22.0);
                    }
                }
            }
            let label = band_label(
                &slice_features::prepare(
                    &fixture().editor.song,
                    FeatureRequest {
                        version: 1,
                        sample: SampleId(1),
                        regions: vec![RegionId(1)],
                    },
                )
                .unwrap()
                .summary()
                .layout,
                8000,
            );
            assert!(label.contains("[2000, 4000]"));
            assert!(label.contains("Nyquist 4000"));
        }
    }
}
