//! Native-region acoustic descriptors and explicitly user-reviewed roles.
//! Advisory measurements only: no source edits, playback effects or automatic labels.
use crate::{onset_service::CachedSource, song::*};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "native-region-acoustic-catalog-v1";
pub const MAX_REGIONS: usize = 64;
pub const MAX_CATALOG: usize = 256;
pub const MAX_REGION_SECONDS: u64 = 30;
pub const MAX_VISITED_FRAMES: u64 = 16 * 1024 * 1024;
pub const MAX_WORK: u64 = 500_000_000;
pub const MAX_PAGE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRequest {
    pub version: u32,
    pub sample: SampleId,
    /// Explicit strictly ascending IDs; one source, no hidden selection/truncation.
    pub regions: Vec<RegionId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionSourceBinding {
    pub sample: SampleId,
    pub source_sha256: String,
    pub source_rate: u32,
    pub channels: u16,
    pub source_frames: u64,
    pub start_frame: u64,
    pub end_frame: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureLayout {
    pub fft_size: u32,
    pub hop_frames: u32,
    pub envelope_bin_frames: u32,
    pub window: String,
    pub low_hz: u32,
    pub low_mid_hz: u32,
    pub mid_high_hz: u32,
    pub high_hz: u32,
    pub flatness_floor_relative: f64,
    /// Per-window analysed-band/total AC power floor for shape normalisation.
    /// Numerical support only, never an absolute source-level threshold.
    pub shape_support_floor_relative: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMetrics {
    pub frames: u64,
    /// Full native source levels, not normalised, mixed, pitched or envelope-processed.
    pub rms: f64,
    pub peak: f64,
    /// Absolute native source frame of the first maximum absolute sample.
    pub peak_frame: u64,
    pub dc_per_channel: Vec<f64>,
    pub crest_factor: Option<f64>,
    /// Hann-squared-weighted native AC power within the analysed spectral band.
    pub analysed_power: Option<f64>,
    pub low_fraction: Option<f64>,
    pub mid_fraction: Option<f64>,
    pub high_fraction: Option<f64>,
    pub centroid_hz: Option<f64>,
    pub flatness: Option<f64>,
    /// Positive L1 difference of consecutive nonzero unit-sum spectra, not amplitude flux.
    pub shape_change_mean: Option<f64>,
    pub shape_change_max: Option<f64>,
    pub spectral_windows: u32,
    pub nonzero_spectral_windows: u32,
    /// Analysed-band power strictly exceeds total post-demean power times
    /// shape_support_floor_relative. Raw aggregate power remains unchanged.
    pub supported_shape_windows: u32,
    pub shape_comparisons: u32,
    /// Envelope bins are anchored at start_frame + i*envelope_bin_frames.
    pub envelope_peak_frame: Option<u64>,
    pub attack_10_90_frames: Option<u64>,
    pub attack_left_censored: bool,
    /// Time after global envelope peak to last >= -20dB activity; NOT clean decay/RT60.
    pub tail_20db_frames: Option<u64>,
    pub tail_right_censored: bool,
    /// Last active bin START minus peak-bin START; only when right-censored.
    pub tail_lower_bound_frames: Option<u64>,
    /// Hysteretic envelope rises, not a count of proven musical hits.
    pub envelope_rises: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PercussionRole {
    Kick,
    Snare,
    ClosedHat,
    OpenHatRide,
    Crash,
    Mixed,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Articulation {
    Unspecified,
    Normal,
    Ghost,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleLabel {
    pub role: PercussionRole,
    /// Explicit user-authored/contextual choice. Never inferred from absolute level.
    pub articulation: Articulation,
    pub note: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleChoice {
    pub region: RegionId,
    pub label: RoleLabel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Unknown,
    Mixed,
    SingleHypothesis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleEvidence {
    pub rule: String,
    pub matched: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSuggestion {
    pub role: PercussionRole,
    /// Fraction of named handcrafted rules satisfied, NOT a class probability.
    pub rule_support: f64,
    pub rules: Vec<RuleEvidence>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSnapshot {
    pub algorithm: String,
    pub version: u32,
    pub binding: RegionSourceBinding,
    pub layout: FeatureLayout,
    pub metrics: NativeMetrics,
    pub suggestion_status: SuggestionStatus,
    pub suggestions: Vec<RoleSuggestion>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRole {
    pub binding: RegionSourceBinding,
    pub label: RoleLabel,
    /// Historical evidence identity; None means manual model-bound annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on_feature_sha256: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionCatalogEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<FeatureSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_role: Option<ReviewedRole>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeatureRecord {
    pub region: RegionId,
    pub snapshot: FeatureSnapshot,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeatureSummary {
    pub algorithm: String,
    pub sample: SampleId,
    pub source_sha256: String,
    pub source_rate: u32,
    pub channels: u16,
    pub region_count: usize,
    pub visited_frames: u64,
    pub fft_windows: u64,
    pub estimated_work: u64,
    pub layout: FeatureLayout,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogSummary {
    pub stored_features: Vec<RegionId>,
    pub reviewed_roles: Vec<RegionId>,
    pub preserved_review_count: usize,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogInspection {
    pub region: RegionId,
    pub current_binding: RegionSourceBinding,
    pub entry: Option<RegionCatalogEntry>,
    /// Model binding equality ONLY. This is not verification of current external bytes.
    pub features_current_match: Option<bool>,
    pub reviewed_role_current_match: Option<bool>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct FeaturePlan {
    id: String,
    base: Song,
    request: FeatureRequest,
    summary: FeatureSummary,
    bindings: Vec<(RegionId, RegionSourceBinding)>,
    asset: SampleAsset,
}
impl FeaturePlan {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn request(&self) -> &FeatureRequest {
        &self.request
    }
    pub fn summary(&self) -> &FeatureSummary {
        &self.summary
    }
    pub fn asset(&self) -> &SampleAsset {
        &self.asset
    }
}
#[derive(Debug, Clone)]
pub struct FeaturePreview {
    id: String,
    plan: FeaturePlan,
    records: Vec<FeatureRecord>,
}
impl FeaturePreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        self.plan.base()
    }
    pub fn request(&self) -> &FeatureRequest {
        self.plan.request()
    }
    pub fn summary(&self) -> &FeatureSummary {
        self.plan.summary()
    }
    pub fn asset(&self) -> &SampleAsset {
        self.plan.asset()
    }
    pub fn records(&self) -> &[FeatureRecord] {
        &self.records
    }
    /// Store every measured descriptor, preserving old labels unless explicitly
    /// replaced by one of these selected-region, user-authored choices.
    pub fn catalog_preview(&self, choices: Vec<RoleChoice>) -> Result<CatalogPreview, String> {
        build_catalog_preview(self, choices)
    }
}
#[derive(Debug, Clone)]
pub struct CatalogPreview {
    id: String,
    base: Song,
    candidate: Song,
    summary: CatalogSummary,
}
impl CatalogPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn summary(&self) -> &CatalogSummary {
        &self.summary
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base {
            return Err("Slice catalog preview is stale; analyze and review again".into());
        }
        Ok(self.candidate.clone())
    }
}

fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|b| crate::sample_asset::content_sha256(&b))
        .map_err(|e| e.to_string())
}
fn binding(song: &Song, region: RegionId) -> Result<RegionSourceBinding, String> {
    let region = song.regions.get(&region).ok_or("Unknown region")?;
    let asset = song
        .samples
        .get(&region.sample)
        .ok_or("Unknown region source")?;
    Ok(RegionSourceBinding {
        sample: region.sample,
        source_sha256: asset.sha256.clone(),
        source_rate: asset.sample_rate,
        channels: asset.channels,
        source_frames: asset.frames,
        start_frame: region.start_frame,
        end_frame: region.end_frame,
    })
}
fn layout(rate: u32) -> FeatureLayout {
    let n = (u64::from(rate) * 32)
        .div_ceil(1000)
        .next_power_of_two()
        .clamp(256, 16384) as u32;
    FeatureLayout {
        fft_size: n,
        hop_frames: n / 4,
        envelope_bin_frames: (rate + 500) / 1000,
        window: "centered_periodic_hann".into(),
        low_hz: 20,
        low_mid_hz: 180,
        mid_high_hz: 2000,
        high_hz: 20000,
        flatness_floor_relative: 1e-12,
        shape_support_floor_relative: 1e-24,
    }
}
fn preflight(song: &Song) -> Result<(), String> {
    crate::project::check_serialized_size(song)?;
    crate::generation::check_work(song, false)
        .map_err(|e| format!("Slice feature model work preflight: {e}"))?;
    song.validate()
}
/// Pure selection/resource preflight. No source path is opened or resolved.
pub fn prepare(song: &Song, request: FeatureRequest) -> Result<FeaturePlan, String> {
    if request.version != VERSION
        || request.regions.is_empty()
        || request.regions.len() > MAX_REGIONS
        || request.regions.windows(2).any(|p| p[0] >= p[1])
    {
        return Err(
            "Feature version must be 1; select 1..64 strictly ascending unique RegionIds".into(),
        );
    }
    preflight(song)?;
    let asset = song
        .samples
        .get(&request.sample)
        .ok_or("Unknown feature source")?
        .clone();
    if !(8000..=384000).contains(&asset.sample_rate) {
        return Err("Native acoustic features support 8000..384000 Hz only".into());
    }
    let layout = layout(asset.sample_rate);
    let mut visited = 0u64;
    let mut windows = 0u64;
    let mut work = 0u64;
    let mut bindings = vec![];
    let mut count = song.region_catalog.len();
    for &id in &request.regions {
        let b = binding(song, id)?;
        if b.sample != request.sample {
            return Err("One feature job must use regions from exactly one selected source".into());
        }
        let frames = b.end_frame - b.start_frame;
        if frames > u64::from(asset.sample_rate) * MAX_REGION_SECONDS {
            return Err(format!(
                "Region {id:?} exceeds 30 seconds; select a shorter region, never silently truncate"
            ));
        }
        visited = visited
            .checked_add(frames)
            .ok_or("Feature frame work overflow")?;
        let w = frames.div_ceil(u64::from(layout.hop_frames));
        windows = windows
            .checked_add(w)
            .ok_or("Feature window count overflow")?;
        let fft = w
            .checked_mul(u64::from(asset.channels))
            .and_then(|v| v.checked_mul(u64::from(layout.fft_size)))
            .and_then(|v| v.checked_mul(u64::from(layout.fft_size.ilog2() + 2)))
            .ok_or("Feature FFT work overflow")?;
        work = work
            .checked_add(fft)
            .and_then(|v| v.checked_add(frames * u64::from(asset.channels)))
            .ok_or("Feature work overflow")?;
        if visited > MAX_VISITED_FRAMES || work > MAX_WORK {
            return Err(format!("Feature work exceeds full-scan cap: {visited}/{MAX_VISITED_FRAMES} visited frames, {work}/{MAX_WORK} units; use fewer/shorter regions (overlap is counted repeatedly)"));
        }
        count += usize::from(!song.region_catalog.contains_key(&id));
        bindings.push((id, b));
    }
    if count > MAX_CATALOG {
        return Err(
            "Feature catalog exceeds 256 regions; clear an entry before adding more".into(),
        );
    }
    let summary=FeatureSummary {algorithm:ALGORITHM.into(),sample:request.sample,source_sha256:asset.sha256.clone(),source_rate:asset.sample_rate,channels:asset.channels,region_count:bindings.len(),visited_frames:visited,fft_windows:windows,estimated_work:work,layout,warnings:vec![
        "Native source-region descriptors only; NOT pitched, envelope/loop-processed or mixed playback. Source audio and existing playback remain unchanged.".into(),
        "New feature FFTs are computed even when opaque source PCM is reused. Full selected bounds are scanned; overlaps count repeatedly; no first-N sampling or hidden truncation.".into(),
        "Rule support is a transparent handcrafted heuristic, NOT calibrated confidence or a clean drum classification. Mixed/Unknown, short/censored/contaminated evidence are ordinary outcomes.".into(),
        "Ghost is an explicit contextual human articulation, never inferred from absolute source level. Storing descriptors does not automatically assign any reviewed role.".into(),
        "Stored snapshots and manual labels are historical metadata, not signed attestations. Current-match means model-binding equality only, not current external file verification. Adapters must reauthorize and hash-check before Store.".into(),
        "Source/bounds changes retain visibly stale snapshots/labels. Names, roots and playback options do not stale these pre-playback measurements. Future grammar must require explicitly reviewed, current-bound roles.".into(),
    ]};
    let id = hash(&(song, &request, &summary))?;
    Ok(FeaturePlan {
        id,
        base: song.clone(),
        request,
        summary,
        bindings,
        asset,
    })
}
/// Caller authorizes this explicit absolute path before entry. Strict verified
/// decode is bounded by the registered native asset, never an implicit CWD lookup.
pub fn analyze_verified(
    plan: &FeaturePlan,
    path: &std::path::Path,
) -> Result<FeaturePreview, String> {
    if !path.is_absolute() {
        return Err(
            "Feature source path must be absolute and explicitly authorized; no CWD resolution"
                .into(),
        );
    }
    let pcm = crate::sample_asset::load_verified(
        &plan.asset,
        path,
        crate::sample_asset::MAX_SAMPLE_VALUES,
    )?;
    let len = plan.asset.frames as usize * plan.asset.channels as usize;
    analyze_native(
        plan,
        pcm.buffer()
            .get(..len)
            .ok_or("Source PCM is shorter than registered bounds")?,
    )
}
/// Reuses opaque verified native PCM, but computes NEW feature FFTs. Callers
/// authorize and verify current source identity before reuse and before Store.
/// The resulting compact measurements do not depend on later onset picker epochs.
pub fn analyze_cached(plan: &FeaturePlan, cache: &CachedSource) -> Result<FeaturePreview, String> {
    let b = cache.binding();
    let a = &plan.asset;
    if b.source_sha256 != a.sha256
        || b.source_rate != a.sample_rate
        || b.channels != a.channels
        || b.source_frames != a.frames
    {
        return Err("Feature cache source identity/native metadata mismatch".into());
    }
    analyze_native(plan, cache.pcm())
}
fn analyze_native(plan: &FeaturePlan, pcm: &[f32]) -> Result<FeaturePreview, String> {
    let mut records = vec![];
    let mut planner = realfft::RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(plan.summary.layout.fft_size as usize);
    let hann: Vec<_> = (0..plan.summary.layout.fft_size)
        .map(|i| {
            0.5 - 0.5
                * (std::f64::consts::TAU * f64::from(i) / f64::from(plan.summary.layout.fft_size))
                    .cos()
        })
        .collect();
    for (id, b) in &plan.bindings {
        let metrics = measure(pcm, b, &plan.summary.layout, &*fft, &hann)?;
        let (suggestion_status, suggestions, warnings) = suggest(&metrics, b, &plan.summary.layout);
        records.push(FeatureRecord {
            region: *id,
            snapshot: FeatureSnapshot {
                algorithm: ALGORITHM.into(),
                version: VERSION,
                binding: b.clone(),
                layout: plan.summary.layout.clone(),
                metrics,
                suggestion_status,
                suggestions,
                warnings,
            },
        });
    }
    let preview = FeaturePreview {
        id: hash(&(&plan.id, &records))?,
        plan: plan.clone(),
        records,
    };
    // Do not expose a descriptor plan that is already known to be unsaveable.
    preview.catalog_preview(vec![])?;
    Ok(preview)
}
fn build_catalog_preview(
    preview: &FeaturePreview,
    choices: Vec<RoleChoice>,
) -> Result<CatalogPreview, String> {
    if choices.len() > MAX_REGIONS {
        return Err("At most 64 explicit reviewed role choices".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for choice in &choices {
        if !seen.insert(choice.region) || !preview.records.iter().any(|r| r.region == choice.region)
        {
            return Err("Reviewed choices must be unique regions in this analysis".into());
        }
        validate_label(&choice.label)?;
    }
    let mut candidate = preview.base().clone();
    let mut preserved = 0;
    for record in &preview.records {
        let entry = candidate.region_catalog.entry(record.region).or_default();
        entry.features = Some(record.snapshot.clone());
        if let Some(choice) = choices.iter().find(|c| c.region == record.region) {
            entry.reviewed_role = Some(ReviewedRole {
                binding: record.snapshot.binding.clone(),
                label: choice.label.clone(),
                based_on_feature_sha256: Some(hash(&record.snapshot)?),
            });
        } else {
            preserved += usize::from(entry.reviewed_role.is_some());
        }
    }
    candidate.validate()?;
    crate::project::check_serialized_size(&candidate)?;
    let summary=CatalogSummary {stored_features:preview.records.iter().map(|r|r.region).collect(),reviewed_roles:choices.iter().map(|c|c.region).collect(),preserved_review_count:preserved,warnings:vec![
        "Descriptors are measured snapshots; role choices are explicitly user-authored, not calibrated classifier outputs. Unselected existing labels and their original evidence identity are preserved.".into(),
        "This pure metadata commit does not verify files; adapters must reauthorize and hash-check the captured source before Store. No BPM, markers, mappings, source bytes or playback settings change.".into(),
    ]};
    Ok(CatalogPreview {
        id: hash(&(&preview.id, &choices, &candidate))?,
        base: preview.base().clone(),
        candidate,
        summary,
    })
}
pub fn inspect_catalog(song: &Song, region: RegionId) -> Result<CatalogInspection, String> {
    let current_binding = binding(song, region)?;
    if !valid_binding(&current_binding) || song.region_catalog.len() > MAX_CATALOG {
        return Err("Invalid current catalog binding/count".into());
    }
    if let Some(entry) = song.region_catalog.get(&region) {
        if let Some(f) = &entry.features {
            validate_snapshot(f)?;
        }
        if let Some(r) = &entry.reviewed_role {
            validate_label(&r.label)?;
            if !valid_binding(&r.binding)
                || r.based_on_feature_sha256.as_ref().is_some_and(|s| !sha(s))
            {
                return Err("Invalid historical reviewed role binding".into());
            }
        }
        if entry.features.is_none() && entry.reviewed_role.is_none() {
            return Err("Empty catalog entry".into());
        }
    }
    let entry = song.region_catalog.get(&region).cloned();
    Ok(CatalogInspection {region,features_current_match:entry.as_ref().and_then(|e|e.features.as_ref().map(|f|f.binding==current_binding)),reviewed_role_current_match:entry.as_ref().and_then(|e|e.reviewed_role.as_ref().map(|r|r.binding==current_binding)),current_binding,entry,warnings:vec!["Binding matches are document metadata equality, NOT fresh external-byte verification or proof that loaded feature numbers were computed. Measured snapshots and user labels retain separate historical provenance.".into()]})
}
impl Song {
    /// Explicit user-authored model-bound annotation; does not read/verify audio.
    pub fn set_region_role(
        &mut self,
        region: RegionId,
        label: Option<RoleLabel>,
    ) -> Result<(), String> {
        let b = binding(self, region)?;
        if let Some(label) = &label {
            validate_label(label)?;
            if self
                .region_catalog
                .get(&region)
                .and_then(|e| e.reviewed_role.as_ref())
                .is_some_and(|r| r.binding == b && &r.label == label)
            {
                // Identical manual values are a no-op, not a request to erase
                // the existing historical evidence identity. Direct callers
                // still receive the same model/storage validation as edits.
                self.validate()?;
                crate::project::check_serialized_size(self)?;
                return Ok(());
            }
        }
        let mut candidate = self.clone();
        if let Some(label) = label {
            candidate
                .region_catalog
                .entry(region)
                .or_default()
                .reviewed_role = Some(ReviewedRole {
                binding: b,
                label,
                based_on_feature_sha256: None,
            });
        } else if let Some(entry) = candidate.region_catalog.get_mut(&region) {
            entry.reviewed_role = None;
        }
        prune(&mut candidate, region);
        candidate.validate()?;
        crate::project::check_serialized_size(&candidate)?;
        *self = candidate;
        Ok(())
    }
    pub fn clear_region_features(&mut self, region: RegionId) -> Result<(), String> {
        if !self.regions.contains_key(&region) {
            return Err("Unknown region".into());
        }
        let mut candidate = self.clone();
        if let Some(entry) = candidate.region_catalog.get_mut(&region) {
            entry.features = None;
        }
        prune(&mut candidate, region);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}
fn prune(song: &mut Song, region: RegionId) {
    if song
        .region_catalog
        .get(&region)
        .is_some_and(|e| e.features.is_none() && e.reviewed_role.is_none())
    {
        song.region_catalog.remove(&region);
    }
}
fn validate_label(label: &RoleLabel) -> Result<(), String> {
    if label.note.len() > 256 {
        Err("Manual role note exceeds 256 UTF-8 bytes".into())
    } else {
        Ok(())
    }
}
fn sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn valid_binding(b: &RegionSourceBinding) -> bool {
    b.sample.0 != 0
        && sha(&b.source_sha256)
        && (1..=384000).contains(&b.source_rate)
        && (1..=2).contains(&b.channels)
        && b.source_frames > 0
        && b.source_frames
            .saturating_add(1)
            .saturating_mul(u64::from(b.channels))
            <= 16 * 1024 * 1024
        && b.start_frame < b.end_frame
        && b.end_frame <= b.source_frames
}
pub(crate) fn validate_catalog(song: &Song) -> Result<(), String> {
    if song.region_catalog.len() > MAX_CATALOG {
        return Err("Region catalog exceeds 256 entries".into());
    }
    for (id, entry) in &song.region_catalog {
        if !song.regions.contains_key(id)
            || entry.features.is_none() && entry.reviewed_role.is_none()
        {
            return Err(
                "Catalog entry needs an existing region and features or a reviewed role".into(),
            );
        }
        if let Some(r) = &entry.reviewed_role {
            validate_label(&r.label)?;
            if !valid_binding(&r.binding)
                || r.based_on_feature_sha256.as_ref().is_some_and(|s| !sha(s))
            {
                return Err("Invalid historical manual role binding/evidence identity".into());
            }
        }
        if let Some(f) = &entry.features {
            validate_snapshot(f)?;
        }
    }
    Ok(())
}

fn measure(
    pcm: &[f32],
    b: &RegionSourceBinding,
    l: &FeatureLayout,
    fft: &dyn realfft::RealToComplex<f64>,
    hann: &[f64],
) -> Result<NativeMetrics, String> {
    let channels = usize::from(b.channels);
    let frames = (b.end_frame - b.start_frame) as usize;
    let start = b.start_frame as usize;
    let source = pcm
        .get(start * channels..b.end_frame as usize * channels)
        .ok_or("Feature region exceeds native PCM")?;
    if source.iter().any(|v| !v.is_finite()) {
        return Err("Non-finite native source sample".into());
    }
    let mut sumsq = 0.0;
    let mut energy_error = 0.0;
    let mut dc = vec![0.0; channels];
    let mut peak = 0.0;
    let mut peak_frame = b.start_frame;
    let bin = l.envelope_bin_frames as usize;
    let mut envelope = vec![0.0; frames.div_ceil(bin)];
    for (f, values) in source.chunks_exact(channels).enumerate() {
        for (c, &value) in values.iter().enumerate() {
            let x = f64::from(value);
            let p = x * x;
            // Compensated sum keeps long, finite high-level regions from
            // falsely exceeding their own peak through accumulation error.
            let y = p - energy_error;
            let t = sumsq + y;
            energy_error = (t - sumsq) - y;
            sumsq = t;
            dc[c] += x;
            envelope[f / bin] += p;
            if x.abs() > peak {
                peak = x.abs();
                peak_frame = b.start_frame + f as u64;
            }
        }
    }
    let rms = (sumsq / (frames * channels) as f64).sqrt();
    for v in &mut dc {
        *v /= frames as f64;
    }
    for (i, e) in envelope.iter_mut().enumerate() {
        *e /= (bin.min(frames - i * bin) * channels) as f64;
    }
    let mut epi = 0;
    for i in 1..envelope.len() {
        if envelope[i] > envelope[epi] {
            epi = i;
        }
    }
    let ep = envelope[epi];
    let (envelope_peak_frame, attack, aleft, tail, tright, lower, rises) = if ep == 0.0 {
        (None, None, false, None, false, None, 0)
    } else {
        let i10 = envelope.iter().position(|&p| p >= ep * 0.01).unwrap();
        let i90 = envelope.iter().position(|&p| p >= ep * 0.81).unwrap();
        let left = i10 == 0;
        let last = envelope.iter().rposition(|&p| p >= ep * 0.01).unwrap();
        let right = last + 1 == envelope.len();
        let mut active = false;
        let mut rises = 0;
        for &p in &envelope {
            if active && p < ep * 0.0625 {
                active = false;
            }
            if !active && p >= ep * 0.25 {
                active = true;
                rises += 1;
            }
        }
        (
            Some(b.start_frame + (epi * bin) as u64),
            (!left).then_some(((i90 - i10) * bin) as u64),
            left,
            (!right).then_some(((last + 1 - epi) * bin) as u64),
            right,
            right.then_some(((last - epi) * bin) as u64),
            rises,
        )
    };
    let n = l.fft_size as usize;
    let hop = l.hop_frames as usize;
    let bins = n / 2 + 1;
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut aggregate = vec![0.0; bins];
    let mut total_weight = 0.0;
    let mut window_power = vec![0.0; bins];
    let upper = 20000.0f64.min(f64::from(b.source_rate) / 2.0);
    let frequencies: Vec<_> = (0..bins)
        .map(|k| k as f64 * f64::from(b.source_rate) / n as f64)
        .collect();
    let domain: Vec<_> = (0..bins)
        .filter(|&k| frequencies[k] >= 20.0 && frequencies[k] <= upper)
        .collect();
    let mut previous: Option<Vec<f64>> = None;
    let mut change_sum = 0.0;
    let mut change_max = 0.0f64;
    let mut comparisons = 0;
    let mut nonzero = 0;
    let mut supported = 0;
    let mut windows = 0;
    for center in (0..frames).step_by(hop) {
        windows += 1;
        window_power.fill(0.0);
        let first = (center as i64 - n as i64 / 2).max(0) as usize;
        let end = (center + n / 2).min(frames);
        let mut mean = vec![0.0; channels];
        for values in source[first * channels..end * channels].chunks_exact(channels) {
            for (c, &x) in values.iter().enumerate() {
                mean[c] += f64::from(x);
            }
        }
        for v in &mut mean {
            *v /= (end - first) as f64;
        }
        let mut valid_weight = 0.0;
        for (i, &w) in hann.iter().enumerate() {
            let f = center as i64 + i as i64 - n as i64 / 2;
            if f >= 0 && (f as usize) < frames {
                valid_weight += w * w;
            }
        }
        total_weight += valid_weight * channels as f64;
        for c in 0..channels {
            for (i, value) in input.iter_mut().enumerate() {
                let f = center as i64 + i as i64 - n as i64 / 2;
                *value = if f >= 0 && (f as usize) < frames {
                    (f64::from(source[f as usize * channels + c]) - mean[c]) * hann[i]
                } else {
                    0.0
                };
            }
            fft.process_with_scratch(&mut input, &mut output, &mut scratch)
                .map_err(|e| e.to_string())?;
            for (k, z) in output.iter().enumerate() {
                let factor = if k == 0 || k == n / 2 { 1.0 } else { 2.0 };
                window_power[k] += z.norm_sqr() * factor / n as f64;
            }
        }
        for (sum, p) in aggregate.iter_mut().zip(&window_power) {
            *sum += *p;
        }
        let power: f64 = domain.iter().map(|&k| window_power[k]).sum();
        if power > 0.0 {
            nonzero += 1;
        }
        // Do not turn roundoff from an overwhelmingly out-of-band window
        // into a unit-sum shape. Preserve its raw aggregate contribution.
        let total_power: f64 = window_power.iter().sum();
        if power > total_power * l.shape_support_floor_relative {
            supported += 1;
            let current: Vec<_> = domain.iter().map(|&k| window_power[k] / power).collect();
            if let Some(old) = &previous {
                let change: f64 = current.iter().zip(old).map(|(a, b)| (a - b).max(0.0)).sum();
                change_sum += change;
                change_max = change_max.max(change);
                comparisons += 1;
            }
            previous = Some(current);
        } else {
            previous = None;
        }
    }
    let power: f64 = domain.iter().map(|&k| aggregate[k]).sum();
    let (analysed_power, low, mid, high, centroid, flatness) = if power > 0.0 {
        let mut bands = [0.0; 3];
        let mut centroid = 0.0;
        let mut maximum = 0.0f64;
        for &k in &domain {
            bands[if frequencies[k] < 180.0 {
                0
            } else if frequencies[k] < 2000.0 {
                1
            } else {
                2
            }] += aggregate[k];
            centroid += frequencies[k] * aggregate[k];
            maximum = maximum.max(aggregate[k]);
        }
        let floor = maximum * l.flatness_floor_relative;
        let log_mean = domain
            .iter()
            .map(|&k| aggregate[k].max(floor).ln())
            .sum::<f64>()
            / domain.len() as f64;
        (
            Some(power / total_weight),
            Some(bands[0] / power),
            Some(bands[1] / power),
            Some(bands[2] / power),
            Some(centroid / power),
            Some(log_mean.exp() / (power / domain.len() as f64)),
        )
    } else {
        (None, None, None, None, None, None)
    };
    Ok(NativeMetrics {
        frames: frames as u64,
        rms,
        peak,
        peak_frame,
        dc_per_channel: dc,
        crest_factor: (rms > 0.0).then_some(peak / rms),
        analysed_power,
        low_fraction: low,
        mid_fraction: mid,
        high_fraction: high,
        centroid_hz: centroid,
        flatness,
        shape_change_mean: (comparisons > 0).then_some(change_sum / comparisons as f64),
        shape_change_max: (comparisons > 0).then_some(change_max),
        spectral_windows: windows,
        nonzero_spectral_windows: nonzero,
        supported_shape_windows: supported,
        shape_comparisons: comparisons,
        envelope_peak_frame,
        attack_10_90_frames: attack,
        attack_left_censored: aleft,
        tail_20db_frames: tail,
        tail_right_censored: tright,
        tail_lower_bound_frames: lower,
        envelope_rises: rises,
    })
}
fn suggest(
    m: &NativeMetrics,
    b: &RegionSourceBinding,
    l: &FeatureLayout,
) -> (SuggestionStatus, Vec<RoleSuggestion>, Vec<String>) {
    let at_most = |frames: Option<u64>, ms: u64| {
        frames.is_some_and(|f| u128::from(f) * 1000 <= u128::from(b.source_rate) * u128::from(ms))
    };
    let at_least = |frames: Option<u64>, ms: u64| {
        frames.is_some_and(|f| u128::from(f) * 1000 >= u128::from(b.source_rate) * u128::from(ms))
    };
    let ge = |v: Option<f64>, x: f64| v.is_some_and(|v| v >= x);
    let le = |v: Option<f64>, x: f64| v.is_some_and(|v| v <= x);
    let long_tail = m.tail_20db_frames.or(m.tail_lower_bound_frames);
    let definitions = [
        (
            PercussionRole::Kick,
            [
                ("low fraction >=0.55", ge(m.low_fraction, 0.55)),
                ("centroid <=800Hz", le(m.centroid_hz, 800.0)),
                (
                    "uncensored attack <=25ms",
                    at_most(m.attack_10_90_frames, 25),
                ),
                (
                    "completed activity tail <=700ms",
                    at_most(m.tail_20db_frames, 700),
                ),
            ],
        ),
        (
            PercussionRole::Snare,
            [
                ("mid fraction >=0.20", ge(m.mid_fraction, 0.2)),
                ("high fraction >=0.20", ge(m.high_fraction, 0.2)),
                ("flatness >=0.10", ge(m.flatness, 0.1)),
                (
                    "uncensored attack <=30ms",
                    at_most(m.attack_10_90_frames, 30),
                ),
            ],
        ),
        (
            PercussionRole::ClosedHat,
            [
                ("high fraction >=0.65", ge(m.high_fraction, 0.65)),
                ("centroid >=2500Hz", ge(m.centroid_hz, 2500.0)),
                (
                    "completed activity tail <=250ms",
                    at_most(m.tail_20db_frames, 250),
                ),
                (
                    "uncensored attack <=20ms",
                    at_most(m.attack_10_90_frames, 20),
                ),
            ],
        ),
        (
            PercussionRole::OpenHatRide,
            [
                ("high fraction >=0.50", ge(m.high_fraction, 0.5)),
                ("centroid >=2000Hz", ge(m.centroid_hz, 2000.0)),
                (
                    "activity tail or lower bound >=150ms",
                    at_least(long_tail, 150),
                ),
                (
                    "uncensored attack <=30ms",
                    at_most(m.attack_10_90_frames, 30),
                ),
            ],
        ),
        (
            PercussionRole::Crash,
            [
                ("high fraction >=0.40", ge(m.high_fraction, 0.4)),
                ("centroid >=2000Hz", ge(m.centroid_hz, 2000.0)),
                (
                    "activity tail or lower bound >=700ms",
                    at_least(long_tail, 700),
                ),
                ("flatness >=0.05", ge(m.flatness, 0.05)),
            ],
        ),
    ];
    let mut hints: Vec<_> = definitions
        .into_iter()
        .map(|(role, rules)| RoleSuggestion {
            role,
            rule_support: rules.iter().filter(|r| r.1).count() as f64 / 4.0,
            rules: rules
                .into_iter()
                .map(|(rule, matched)| RuleEvidence {
                    rule: rule.into(),
                    matched,
                })
                .collect(),
        })
        .collect();
    // Stable ties retain declared role order; these are rule fractions, not posteriors.
    hints.sort_by(|a, b| b.rule_support.total_cmp(&a.rule_support));
    let qualifying = hints.iter().filter(|h| h.rule_support >= 0.75).count();
    let status = if m.rms == 0.0 || m.frames < u64::from(l.fft_size) || m.analysed_power.is_none() {
        SuggestionStatus::Unknown
    } else if m.envelope_rises > 1 || qualifying > 1 {
        SuggestionStatus::Mixed
    } else if qualifying == 1 {
        SuggestionStatus::SingleHypothesis
    } else {
        SuggestionStatus::Unknown
    };
    let mut warnings=vec!["Handcrafted rule fractions are not calibrated probabilities, verified instrument labels or evidence that bleed is absent. A single hypothesis is still advisory; only explicit user review authors a role.".into(),"Tail is last above-threshold activity after the global envelope peak, not proof of a single clean decay. Spectral change measures normalised shape, not amplitude flux strength.".into()];
    if m.rms == 0.0 {
        warnings.push("Silent region: no attack, spectral role or ghost inference.".into());
    }
    if m.frames < u64::from(l.fft_size) {
        warnings.push("Region shorter than one FFT window: zero-padded spectral interpretation is insufficient for role suggestion status.".into());
    }
    if m.attack_left_censored {
        warnings.push("Attack is left-censored by the region boundary; no completed 10-to-90% duration is invented.".into());
    }
    if m.tail_right_censored {
        warnings.push("Final envelope bin is active: end crossing is unresolved; lower bound reaches the last active bin START, not the exact last sample.".into());
    }
    if m.envelope_rises > 1 {
        warnings.push("Multiple envelope rises: this may be a phrase, repeated hit, modulation or mixed/contaminated audio, not a proven clean single drum.".into());
    }
    if m.analysed_power.is_none() {
        warnings.push("No nonzero analysed-band AC power; centroid, flatness and band fractions are unavailable.".into());
    }
    if m.nonzero_spectral_windows > m.supported_shape_windows {
        warnings.push("Some raw nonzero spectra lack numerical shape support: analysed-band power must exceed total post-demean power times the declared relative floor. Their raw aggregate power is retained, but they break shape-comparison adjacency; this is not a loudness threshold.".into());
    }
    if m.shape_comparisons == 0 {
        warnings.push("No pair of consecutive numerically supported spectra; spectral-shape change is unavailable, not a confident zero.".into());
    }
    (status, hints, warnings)
}
fn validate_snapshot(f: &FeatureSnapshot) -> Result<(), String> {
    let b = &f.binding;
    let m = &f.metrics;
    let bad = || {
        "Invalid stored acoustic feature snapshot; metrics are historical metadata, not signed measurements".to_string()
    };
    if f.version != VERSION
        || f.algorithm != ALGORITHM
        || !valid_binding(b)
        || !(8000..=384000).contains(&b.source_rate)
        || b.end_frame - b.start_frame > u64::from(b.source_rate) * MAX_REGION_SECONDS
        || f.layout != layout(b.source_rate)
    {
        return Err(bad());
    }
    let frames = b.end_frame - b.start_frame;
    let bin = u64::from(f.layout.envelope_bin_frames);
    let windows = frames.div_ceil(u64::from(f.layout.hop_frames));
    if m.frames != frames
        || m.peak_frame < b.start_frame
        || m.peak_frame >= b.end_frame
        || !m.rms.is_finite()
        || !m.peak.is_finite()
        || m.rms < 0.0
        || m.peak < 0.0
        || m.peak > f64::from(f32::MAX)
        || m.rms > m.peak * (1.0 + 1e-12)
        || m.dc_per_channel.len() != usize::from(b.channels)
        || m.dc_per_channel
            .iter()
            .any(|v| !v.is_finite() || v.abs() > m.peak * (1.0 + 1e-12))
    {
        return Err(bad());
    }
    if (m.rms == 0.0) != (m.peak == 0.0)
        || m.crest_factor.is_some() != (m.rms > 0.0)
        || m.crest_factor.is_some_and(|v| {
            !v.is_finite() || (v - m.peak / m.rms).abs() > 1e-10 * (m.peak / m.rms).max(1.0)
        })
    {
        return Err(bad());
    }
    if u64::from(m.spectral_windows) != windows
        || m.nonzero_spectral_windows > m.spectral_windows
        || m.shape_comparisons > m.spectral_windows.saturating_sub(1)
        || m.supported_shape_windows > m.nonzero_spectral_windows
        || m.shape_comparisons > m.supported_shape_windows.saturating_sub(1)
    {
        return Err(bad());
    }
    let spectrum = [
        m.low_fraction,
        m.mid_fraction,
        m.high_fraction,
        m.centroid_hz,
        m.flatness,
    ];
    if m.analysed_power.is_some() != (m.nonzero_spectral_windows > 0)
        || spectrum
            .iter()
            .any(|v| v.is_some() != m.analysed_power.is_some())
        || m.analysed_power.is_some_and(|v| {
            !v.is_finite() || v <= 0.0 || v > 4.0 * m.peak * m.peak * (1.0 + 1e-10)
        })
    {
        return Err(bad());
    }
    if let (Some(low), Some(mid), Some(high), Some(centroid), Some(flatness)) = (
        m.low_fraction,
        m.mid_fraction,
        m.high_fraction,
        m.centroid_hz,
        m.flatness,
    ) {
        if [low, mid, high, flatness]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0 || *v > 1.0 + 1e-10)
            || (low + mid + high - 1.0).abs() > 1e-10
            || !centroid.is_finite()
            || centroid < 20.0 - 1e-8
            || centroid > 20000.0f64.min(f64::from(b.source_rate) / 2.0) + 1e-8
        {
            return Err(bad());
        }
    }
    if m.shape_change_mean.is_some() != (m.shape_comparisons > 0)
        || m.shape_change_max.is_some() != (m.shape_comparisons > 0)
    {
        return Err(bad());
    }
    if let (Some(mean), Some(max)) = (m.shape_change_mean, m.shape_change_max) {
        if !mean.is_finite()
            || !max.is_finite()
            || mean < 0.0
            || max < 0.0
            || mean > max + 1e-12
            || max > 1.0 + 1e-10
        {
            return Err(bad());
        }
    }
    if m.envelope_peak_frame.is_some() != (m.rms > 0.0)
        || u64::from(m.envelope_rises) > frames.div_ceil(bin)
    {
        return Err(bad());
    }
    if let Some(peak) = m.envelope_peak_frame {
        if peak < b.start_frame
            || peak >= b.end_frame
            || !(peak - b.start_frame).is_multiple_of(bin)
            || m.envelope_rises == 0
            || m.attack_10_90_frames.is_some() == m.attack_left_censored
            || m.tail_20db_frames.is_some() == m.tail_right_censored
            || m.tail_lower_bound_frames.is_some() != m.tail_right_censored
        {
            return Err(bad());
        }
        if m.attack_10_90_frames
            .is_some_and(|v| v > peak - b.start_frame || !v.is_multiple_of(bin))
            || [m.tail_20db_frames, m.tail_lower_bound_frames]
                .iter()
                .flatten()
                .any(|v| *v > b.end_frame - peak || !v.is_multiple_of(bin))
        {
            return Err(bad());
        }
    } else if m.attack_10_90_frames.is_some()
        || m.attack_left_censored
        || m.tail_20db_frames.is_some()
        || m.tail_right_censored
        || m.tail_lower_bound_frames.is_some()
        || m.envelope_rises != 0
    {
        return Err(bad());
    }
    if f.suggestions.len() != 5
        || f.warnings.len() > 10
        || f.warnings.iter().any(|w| w.len() > 512)
        || f.suggestions.iter().any(|s| {
            s.rules.len() != 4
                || s.rules.iter().any(|r| r.rule.len() > 128)
                || !s.rule_support.is_finite()
        })
    {
        return Err(bad());
    }
    let (status, hints, warnings) = suggest(m, b, &f.layout);
    if status != f.suggestion_status || hints != f.suggestions || warnings != f.warnings {
        return Err("Stored acoustic heuristic rules/status do not match the versioned metric interpretation".into());
    }
    Ok(())
}
