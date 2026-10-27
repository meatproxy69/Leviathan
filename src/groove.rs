//! Frozen accepted-marker timing templates, baked through the existing exact
//! whole-cell transform authority. No audio IO, inference, accents or runtime DSP.
use crate::{
    break_compiler::{timing::Rational, Subdivision},
    song::*,
    transforms::{self, CellChange, TransformScope},
    RATE,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "accepted-marker-groove-v1";
pub const MAX_TEMPLATES: usize = 64;
pub const MAX_ANCHORS: usize = 256;
pub const MAX_VISITS: usize = 65536;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfig {
    pub version: u32,
    pub name: String,
    pub marker_set: MarkerSetId,
    pub start_marker: u16,
    pub end_marker: u16,
    pub include_start_anchor: bool,
    pub division: Subdivision,
    pub cycle_steps: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrooveAnchor {
    pub step: u16,
    pub source_frame_delta: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrooveProvenance {
    pub sample: SampleId,
    pub source_sha256: String,
    pub source_frames: u64,
    pub marker_set: MarkerSetId,
    pub marker_set_sha256: String,
    pub start_marker: u16,
    pub end_marker: u16,
    pub window_start_frame: u64,
    pub window_end_frame: u64,
    pub include_start_anchor: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrooveTemplate {
    pub version: u32,
    pub name: String,
    pub division: Subdivision,
    pub cycle_steps: u16,
    /// Frozen numeric timing authority, not a live asset/marker lookup.
    pub source_bpm: f64,
    pub source_rate: u32,
    pub anchors: Vec<GrooveAnchor>,
    /// Historical declared identity. Capture does not read or reverify files.
    pub provenance: GrooveProvenance,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GrooveAnchorInfo {
    pub step: u16,
    pub source_frame_delta: u64,
    /// Approximation for display only. The stored integer/f64 inputs are exact authority.
    pub offset_beats: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyConfig {
    pub groove: GrooveId,
    pub scope: TransformScope,
    pub strength_percent: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaptureSummary {
    pub algorithm: String,
    pub groove: GrooveId,
    pub anchor_count: usize,
    pub cycle_steps: u16,
    pub window_start_frame: u64,
    pub window_end_frame: u64,
    pub source_bpm: f64,
    pub source_sha256: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplySummary {
    pub algorithm: String,
    pub groove: GrooveId,
    pub template_sha256: String,
    pub selected_notes: usize,
    pub matched_notes: usize,
    pub unmatched_notes: usize,
    pub changed_cells: usize,
    pub moved_cells: usize,
    pub zero_frame_moves: usize,
    pub affected_occurrences: Vec<usize>,
    pub affected_aliases: Vec<TrackAlias>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct CapturePreview {
    id: String,
    base: Song,
    candidate: Song,
    config: CaptureConfig,
    summary: CaptureSummary,
    template: GrooveTemplate,
}
impl CapturePreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &CaptureConfig {
        &self.config
    }
    pub fn summary(&self) -> &CaptureSummary {
        &self.summary
    }
    pub fn template(&self) -> &GrooveTemplate {
        &self.template
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        commit(current, &self.base, &self.candidate)
    }
}
#[derive(Debug, Clone)]
pub struct GroovePreview {
    id: String,
    base: Song,
    candidate: Song,
    config: ApplyConfig,
    summary: ApplySummary,
    changes: Vec<CellChange>,
}
impl GroovePreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &ApplyConfig {
        &self.config
    }
    pub fn summary(&self) -> &ApplySummary {
        &self.summary
    }
    pub fn changes(&self) -> &[CellChange] {
        &self.changes
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        commit(current, &self.base, &self.candidate)
    }
}
fn commit(current: &Song, base: &Song, candidate: &Song) -> Result<Song, String> {
    if current != base {
        return Err("Groove preview is stale; prepare a new preview".into());
    }
    Ok(candidate.clone())
}
fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| crate::sample_asset::content_sha256(&bytes))
        .map_err(|e| e.to_string())
}
fn preflight(song: &Song) -> Result<(), String> {
    crate::project::check_serialized_size(song)?;
    crate::generation::check_work(song, false)
        .map_err(|e| format!("Groove work preflight: {e}"))?;
    song.validate()
}
fn step_beats(division: Subdivision) -> Rational {
    Rational::new(4, u128::from(division.denominator()))
}
fn source_beat(delta: u64, bpm: f64, rate: u32) -> Result<Rational, String> {
    Rational::new(u128::from(delta), u128::from(rate) * 60).mul(Rational::bpm(bpm))
}
fn sha_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl GrooveTemplate {
    pub fn validate(&self) -> Result<(), String> {
        let p = &self.provenance;
        if self.version != VERSION
            || self.name.trim().is_empty()
            || self.name.len() > 256
            || !(1..=1024).contains(&self.cycle_steps)
            || self.anchors.is_empty()
            || self.anchors.len() > MAX_ANCHORS
            || !self.source_bpm.is_finite()
            || !(20.0..=400.0).contains(&self.source_bpm)
            || !(1..=384000).contains(&self.source_rate)
            || p.sample.0 == 0
            || p.marker_set.0 == 0
            || !sha_valid(&p.source_sha256)
            || !sha_valid(&p.marker_set_sha256)
            || p.source_frames == 0
            || p.source_frames > u64::from(u32::MAX)
            || p.window_start_frame >= p.window_end_frame
            || p.window_end_frame > p.source_frames
            || p.start_marker >= p.end_marker
            || p.end_marker > 256
        {
            return Err(
                "Invalid groove version/name/cycle/anchors or frozen source provenance".into(),
            );
        }
        let expected =
            usize::from(p.end_marker - p.start_marker - 1) + usize::from(p.include_start_anchor);
        if self.anchors.len() != expected
            || (self.anchors[0].source_frame_delta == 0) != p.include_start_anchor
        {
            return Err(
                "Groove anchor count/start policy does not match its explicit marker window".into(),
            );
        }
        let mut previous = None;
        for anchor in &self.anchors {
            if anchor.source_frame_delta >= p.window_end_frame - p.window_start_frame
                || anchor.step >= self.cycle_steps
                || previous.is_some_and(|(step, delta)| {
                    anchor.step <= step || anchor.source_frame_delta <= delta
                })
                || source_beat(anchor.source_frame_delta, self.source_bpm, self.source_rate)?
                    .div(step_beats(self.division))?
                    .round()?
                    != u64::from(anchor.step)
            {
                return Err("Groove anchors must be increasing unique nearest grid slots inside the explicit cycle/window; no wrap or merge".into());
            }
            previous = Some((anchor.step, anchor.source_frame_delta));
        }
        Ok(())
    }
}
impl Song {
    pub fn next_groove_id(&self) -> Result<GrooveId, String> {
        if self.grooves.len() >= MAX_TEMPLATES {
            return Err("Song exceeds 64 groove templates".into());
        }
        self.grooves
            .keys()
            .next_back()
            .map_or(Some(1), |id| id.0.checked_add(1))
            .map(GrooveId)
            .ok_or("Groove ID allocation overflow".into())
    }
    pub fn register_groove(
        &mut self,
        id: GrooveId,
        template: GrooveTemplate,
    ) -> Result<(), String> {
        template.validate()?;
        if id.0 == 0 {
            return Err("Groove ID must be nonzero".into());
        }
        if let Some(old) = self.grooves.get(&id) {
            return if old == &template {
                Ok(())
            } else {
                Err("Groove ID is immutable; capture under a fresh ID".into())
            };
        }
        if self.grooves.len() >= MAX_TEMPLATES {
            return Err("Song exceeds 64 groove templates".into());
        }
        let mut candidate = self.clone();
        candidate.grooves.insert(id, template);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_groove(&mut self, id: GrooveId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate
            .grooves
            .remove(&id)
            .ok_or("Unknown groove template")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}
/// Snapshot explicitly accepted boundary positions. No file access or inference.
pub fn capture(song: &Song, config: CaptureConfig) -> Result<GrooveTemplate, String> {
    preflight(song)?;
    if config.version != VERSION
        || config.name.trim().is_empty()
        || config.name.len() > 256
        || !(1..=1024).contains(&config.cycle_steps)
    {
        return Err(
            "Groove capture requires version1, nonblank name <=256 bytes and cycle_steps1..1024"
                .into(),
        );
    }
    let set = song
        .marker_sets
        .get(&config.marker_set)
        .ok_or("Unknown marker set")?;
    let bpm = set.source_bpm.ok_or(
        "Groove capture requires explicit source BPM on the accepted marker set; no inference",
    )?;
    let start = usize::from(config.start_marker);
    let end = usize::from(config.end_marker);
    if start >= end || end >= set.markers.len() {
        return Err(
            "Groove capture requires start_marker < end_marker within accepted marker indices"
                .into(),
        );
    }
    let origin = set.markers[start];
    let first = start + usize::from(!config.include_start_anchor);
    let source = &song.samples[&set.sample];
    let mut anchors = Vec::new();
    for &frame in &set.markers[first..end] {
        let delta = frame - origin;
        let step = source_beat(delta, bpm, source.sample_rate)?
            .div(step_beats(config.division))?
            .round()?;
        if step >= u64::from(config.cycle_steps) {
            return Err("Captured anchor maps outside the declared cycle (including excluded end); choose a longer cycle or explicit narrower marker window".into());
        }
        if anchors
            .last()
            .is_some_and(|a: &GrooveAnchor| u64::from(a.step) == step)
        {
            return Err("Two accepted markers map to the same nearest groove slot; review markers or choose a finer grid, no merging".into());
        }
        anchors.push(GrooveAnchor {
            step: step as u16,
            source_frame_delta: delta,
        });
    }
    if anchors.is_empty() {
        return Err("Groove capture has no declared anchors; source end is structural and start requires explicit opt-in".into());
    }
    let template = GrooveTemplate {
        version: VERSION,
        name: config.name,
        division: config.division,
        cycle_steps: config.cycle_steps,
        source_bpm: bpm,
        source_rate: source.sample_rate,
        anchors,
        provenance: GrooveProvenance {
            sample: set.sample,
            source_sha256: source.sha256.clone(),
            source_frames: source.frames,
            marker_set: config.marker_set,
            marker_set_sha256: hash(set)?,
            start_marker: config.start_marker,
            end_marker: config.end_marker,
            window_start_frame: origin,
            window_end_frame: set.markers[end],
            include_start_anchor: config.include_start_anchor,
        },
    };
    template.validate()?;
    Ok(template)
}
pub fn inspect(template: &GrooveTemplate) -> Result<Vec<GrooveAnchorInfo>, String> {
    template.validate()?;
    template
        .anchors
        .iter()
        .map(|a| {
            let actual = source_beat(
                a.source_frame_delta,
                template.source_bpm,
                template.source_rate,
            )?;
            let grid = step_beats(template.division).scale(u64::from(a.step))?;
            let offset = if actual.less(grid)? {
                -grid.sub(actual)?.as_f64()
            } else {
                actual.sub(grid)?.as_f64()
            };
            Ok(GrooveAnchorInfo {
                step: a.step,
                source_frame_delta: a.source_frame_delta,
                offset_beats: offset,
            })
        })
        .collect()
}
pub fn capture_preview(
    song: &Song,
    id: GrooveId,
    config: CaptureConfig,
) -> Result<CapturePreview, String> {
    if id.0 == 0 || song.grooves.contains_key(&id) {
        return Err("Capture preview requires a fresh nonzero GrooveId".into());
    }
    let template = capture(song, config.clone())?;
    let mut candidate = song.clone();
    candidate.register_groove(id, template.clone())?;
    crate::project::check_serialized_size(&candidate)?;
    let summary=CaptureSummary {algorithm:ALGORITHM.into(),groove:id,anchor_count:template.anchors.len(),cycle_steps:template.cycle_steps,
        window_start_frame:template.provenance.window_start_frame,window_end_frame:template.provenance.window_end_frame,
        source_bpm:template.source_bpm,source_sha256:template.provenance.source_sha256.clone(),warnings:vec![
            "Timing anchors are explicitly accepted boundaries, not inferred attacks. Window end is excluded; window start is an anchor only when opted in".into(),
            "Source SHA/IDs are historical declared provenance, not freshly verified. This frozen template has no live source dependency or decoder IO".into(),
            "Cycle length is explicitly declared, not inferred from file duration. Sparse slots never invent notes".into()]};
    Ok(CapturePreview {
        id: hash(&(ALGORITHM, song, &config, &candidate))?,
        base: song.clone(),
        candidate,
        config,
        summary,
        template,
    })
}
#[derive(Clone)]
struct Context {
    clock: transforms::Use,
    tracks: BTreeSet<TrackId>,
}
fn contexts(song: &Song, scope: &TransformScope, per_row: f64) -> Result<Vec<Context>, String> {
    let mut targets = BTreeMap::from([(scope.pattern, BTreeSet::from([scope.track]))]);
    for (&pattern, p) in &song.patterns {
        for &track in p.track_aliases.keys() {
            if song.resolve_track_source(pattern, track)? == (scope.pattern, scope.track) {
                targets.entry(pattern).or_default().insert(track);
            }
        }
    }
    let end = (song
        .order
        .iter()
        .map(|o| u64::from(song.patterns[&o.pattern].rows))
        .sum::<u64>() as f64
        * per_row)
        .round() as u64;
    let mut result = Vec::new();
    let mut prefix = 0;
    for occurrence in &song.order {
        if let Some(tracks) = targets.get(&occurrence.pattern) {
            if result.len() >= 4096 {
                return Err("Groove affects more than 4096 ordered uses".into());
            }
            result.push(Context {
                clock: transforms::Use { prefix, end },
                tracks: tracks.clone(),
            });
        }
        prefix += u64::from(song.patterns[&occurrence.pattern].rows);
    }
    if result.is_empty() {
        result.push(Context {
            clock: transforms::Use {
                prefix: 0,
                end: (f64::from(song.patterns[&scope.pattern].rows) * per_row).round() as u64,
            },
            tracks: BTreeSet::new(),
        });
    }
    Ok(result)
}
// Inputs are validated stored cell coordinates. D is a power of two, so
// delay*D is exact even for subnormals. Flooring the fractional numerator
// cannot change the integer half-up threshold. Do not sum row+delay in f64.
fn nearest_tick(row: u32, origin: u32, delay: f64, lpb: u16, division: Subdivision) -> u64 {
    let d = u64::from(division.denominator());
    let denominator = 4 * u64::from(lpb);
    (u64::from(row - origin) * d + (delay * d as f64).floor() as u64 + denominator / 2)
        / denominator
}
fn tick(checks: &mut usize) -> Result<(), String> {
    *checks += 1;
    if *checks > MAX_VISITS {
        Err(
            "Groove exceeds 65536 timing/action/context checks; narrow selection or arrangement"
                .into(),
        )
    } else {
        Ok(())
    }
}
/// Blend current note phase toward stored template phase; bake whole cells with
/// the same exact encoder, collision rules and order proof as scoped transforms.
pub fn preview(song: &Song, config: ApplyConfig) -> Result<GroovePreview, String> {
    preflight(song)?;
    if config.strength_percent > 100 {
        return Err("Groove strength must be 0..100 percent".into());
    }
    transforms::validate_config(
        song,
        &transforms::TransformConfig {
            scope: config.scope.clone(),
            fields: Default::default(),
            transpose_semitones: 0,
            velocity_scale: 1.0,
            nudge_frames: 0,
            swing: None,
        },
    )?;
    let template = song
        .grooves
        .get(&config.groove)
        .ok_or("Unknown groove template")?;
    let scope = &config.scope;
    let per_row = f64::from(RATE) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
    let contexts = contexts(song, scope, per_row)?;
    let mut summary = ApplySummary {
        algorithm: ALGORITHM.into(),
        groove: config.groove,
        template_sha256: hash(template)?,
        selected_notes: 0,
        matched_notes: 0,
        unmatched_notes: 0,
        changed_cells: 0,
        moved_cells: 0,
        zero_frame_moves: 0,
        affected_occurrences: Vec::new(),
        affected_aliases: Vec::new(),
        warnings: Vec::new(),
    };
    let lpb = u128::from(song.lines_per_beat);
    let scope_beats = Rational::new(u128::from(scope.row_end - scope.row_start), lpb);
    let origin = Rational::new(u128::from(scope.row_start), lpb);
    let step = step_beats(template.division);
    let cycle = step.scale(u64::from(template.cycle_steps))?;
    let anchors: BTreeMap<_, _> = template
        .anchors
        .iter()
        .map(|a| (u64::from(a.step), a))
        .collect();
    let mut moves = BTreeMap::new();
    let mut intended = BTreeMap::new();
    let mut slots = BTreeSet::new();
    let mut visited = 0usize;
    let mut checks = 0usize;
    if let Some(rows) = song.patterns[&scope.pattern].tracks.get(&scope.track) {
        for (&row, columns) in rows.range(scope.row_start..scope.row_end) {
            for (&column, cell) in columns {
                visited += 1;
                if visited > MAX_VISITS {
                    return Err("Groove selection scan exceeds 65536 stored cells".into());
                }
                if scope.columns.binary_search(&column).is_err()
                    || !matches!(cell.note, Note::Note(_))
                {
                    continue;
                }
                summary.selected_notes += 1;
                if summary.selected_notes > 4096 {
                    return Err("Groove selection exceeds 4096 notes".into());
                }
                if config.strength_percent == 0 {
                    continue;
                }
                let k = nearest_tick(
                    row,
                    scope.row_start,
                    cell.delay.unwrap_or(0.0),
                    song.lines_per_beat,
                    template.division,
                );
                let Some(anchor) = anchors.get(&(k % u64::from(template.cycle_steps))) else {
                    summary.unmatched_notes += 1;
                    continue;
                };
                summary.matched_notes += 1;
                if !step.scale(k)?.less(scope_beats)? {
                    return Err(format!("Groove nearest target anchor for row {row} column {column} is at/outside exclusive selection end; no wrapping"));
                }
                if !slots.insert((column, k)) {
                    return Err(format!("Multiple selected notes in column {column} map to groove tick {k}; no merging"));
                }
                let target = cycle
                    .scale(k / u64::from(template.cycle_steps))?
                    .add(source_beat(
                        anchor.source_frame_delta,
                        template.source_bpm,
                        template.source_rate,
                    )?)?;
                let blended = if config.strength_percent == 100 {
                    target
                } else {
                    let current = Rational::new(u128::from(row - scope.row_start), 1)
                        .add(Rational::from_nonnegative_f64(cell.delay.unwrap_or(0.0))?)?
                        .div(Rational::new(lpb, 1))?;
                    current
                        .scale(u64::from(100 - config.strength_percent))?
                        .add(target.scale(u64::from(config.strength_percent))?)?
                        .div(Rational::new(100, 1))?
                };
                if !blended.less(scope_beats)? {
                    return Err("Groove ideal destination leaves selected [start,end) interval; no clamping".into());
                }
                let absolute = origin.add(blended)?;
                let new_row = absolute.scale(u64::from(song.lines_per_beat))?.floor()?;
                let mut common = None;
                for context in &contexts {
                    tick(&mut checks)?;
                    let wanted = Rational::new(u128::from(context.clock.prefix), lpb)
                        .add(absolute)?
                        .scale(u64::from(RATE) * 60)?
                        .div(Rational::bpm(song.bpm))?
                        .round()?;
                    if wanted >= context.clock.end {
                        return Err("Groove destination reaches/exceeds an ordered musical end; no clamping".into());
                    }
                    let old =
                        transforms::frame(context.clock, row, cell.delay.unwrap_or(0.0), per_row);
                    let delta = i64::try_from(i128::from(wanted) - i128::from(old))
                        .map_err(|_| "Groove frame displacement overflow")?;
                    if common.is_some_and(|prior| prior != delta) {
                        return Err(format!("Groove row {row} column {column} requires different integer displacements across uses; not representable by timing v1"));
                    }
                    common = Some(delta);
                }
                let delta = common.unwrap();
                if delta == 0 {
                    summary.zero_frame_moves += 1;
                    continue;
                }
                moves.insert((row, column), delta);
                intended.insert((row, column), (new_row, k));
            }
        }
    }
    let plan = transforms::preview_frame_moves(song, scope.clone(), &moves)?;
    let mutes = FrameMutes::new(song, RATE);
    for change in plan.changes() {
        let (intended_row, intended_tick) = intended[&(change.from_row, change.column)];
        if u64::from(change.to_row) != intended_row {
            return Err("Groove frame encoder would cross the ideal rational destination row; no boundary squeezing".into());
        }
        if nearest_tick(
            change.to_row,
            scope.row_start,
            change.after.delay.unwrap_or(0.0),
            song.lines_per_beat,
            template.division,
        ) != intended_tick
        {
            return Err("Groove frame encoder would cross the intended nearest-slot boundary; timing v1 cannot represent this transfer without retargeting".into());
        }
        let before = transforms::fractions(&change.before);
        let after = transforms::fractions(&change.after);
        for context in &contexts {
            for &track in &context.tracks {
                for (&old, &new) in before.iter().zip(&after) {
                    tick(&mut checks)?;
                    if mutes.contains(
                        track,
                        transforms::frame(context.clock, change.from_row, old, per_row),
                    ) != mutes.contains(
                        track,
                        transforms::frame(context.clock, change.to_row, new, per_row),
                    ) {
                        return Err("Groove changes a scheduled note/retrigger/row-local action's occurrence-mute eligibility; no hidden suppression or revival".into());
                    }
                }
            }
        }
    }
    let candidate = plan.candidate().clone();
    crate::project::check_serialized_size(&candidate)?;
    summary.changed_cells = plan.changes().len();
    summary.moved_cells = plan.summary().moved_cells;
    summary.affected_occurrences = plan.summary().affected_occurrences.clone();
    summary.affected_aliases = plan.summary().affected_aliases.clone();
    summary.warnings=vec![
        "Strength blends existing phase toward stored template phase, not an additive nudge. 100% may straighten an off-grid note; unmatched slots are unchanged".into(),
        "Timing only: whole ordinary Note cells move, including kit selectors unchanged. OFF/scalar cells remain order barriers; changed phrase triggers are unsupported".into(),
        "Proof is for captured BPM/LPB and all current repeat/alias uses, requiring one integer displacement per changed cell. 100% means exact rounded rational target subject to checked-u128/32-ULP representability and preserved nearest-slot identity; later clock edits rescale baked row/delay values".into(),
        "Zero strength bypasses classification/encoding; zero-output-frame moves also retain exact stored representation. No source reading, inference, accents, new stored notes, stretch or runtime groove".into(),
        "This edits shared pattern content across every listed repeat/alias; Make unique first for a local variation. Declared cycles repeat, including a partial last cycle, without wrapping excluded-end anchors".into(),
        "Ordinary evaluator policies are recompiled after timing edits: cross-column/track interleaving can change round-robin source choices, choke order, phrase replacement timing and voice overlap even though stored note/pad mappings are unchanged".into(),
        "Moving a replacement note can expose/cancel pending phrase children. Preview proves stored-cell/timing constraints, not full renderability or unchanged expanded event count; source assignment/decoding and child/action budgets are checked by evaluation/export".into()];
    Ok(GroovePreview {
        id: hash(&(ALGORITHM, song, &config, &candidate))?,
        base: song.clone(),
        candidate,
        config,
        summary,
        changes: plan.changes().to_vec(),
    })
}
