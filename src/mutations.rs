//! Bounded seeded thinning, baked into ordinary cells. Reviewed human roles only;
//! no live randomness, source IO, inferred anchors or genre-composition claim.
use crate::{
    slice_features::{PercussionRole, RoleLabel},
    song::*,
    song_render::{self, Command, Manifest},
    transforms::TransformScope,
};
use pattrns::{gates::ProbabilityGate, Gate, RhythmEvent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "reviewed-role-pattrns-thin-v1";
pub const GATE_ALGORITHM: &str = crate::generation::GATE_ALGORITHM;
pub const MAX_RECORDS: usize = 4096;
pub const MAX_CONTEXTS: usize = 4096;
pub const MAX_PATH_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_STARTS: usize = 2048;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinConfig {
    pub version: u32,
    pub scope: TransformScope,
    pub seed: u64,
    pub removal_percent: u8,
    pub max_removed: u16,
    /// Unique concrete roles, normally Kick + Snare. Empty explicitly disables
    /// these protections; unreviewed/stale/Mixed/Unknown remain protected.
    pub protected_roles: Vec<PercussionRole>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinReason {
    NotOn,
    Phrase,
    Effects,
    /// No evaluated unsuppressed ON; not a measurement of audible sound.
    NoAudibleUse,
    UnknownSource,
    Unreviewed,
    StaleReview,
    MixedOrUnknown,
    ProtectedRole,
    GateKept,
    Removed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThinSource {
    Legacy,
    Region {
        region: RegionId,
        root_note: u8,
    },
    DrumPad {
        kit: DrumKitId,
        pad: PadId,
        region: RegionId,
        root_note: u8,
        layer: Option<PadLayerId>,
        zone_min: Option<u8>,
        zone_max: Option<u8>,
        round_robin_index: Option<u8>,
    },
    Multisample {
        map: MultisampleId,
        zone: KeyZoneId,
        region: RegionId,
        root_note: u8,
    },
}
impl ThinSource {
    fn region(&self) -> Option<RegionId> {
        match *self {
            Self::Legacy => None,
            Self::Region { region, .. }
            | Self::DrumPad { region, .. }
            | Self::Multisample { region, .. } => Some(region),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinDecision {
    pub row: u32,
    pub column: u16,
    pub before: Cell,
    pub uses: usize,
    pub reason: ThinReason,
    /// Canonical eligible-cell gate-call ordinal; None for excluded cells or 0%.
    pub gate_index: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinUse {
    pub decision: usize,
    pub occurrence: usize,
    pub track: TrackId,
    pub row: u32,
    pub column: u16,
    pub frame: u64,
    pub instrument: InstrumentId,
    pub source: ThinSource,
    pub pitch: u8,
    pub velocity: f32,
    pub pan: f32,
    pub selection_velocity: Option<f32>,
    pub velocity_bin: Option<u8>,
    pub reviewed_role: Option<RoleLabel>,
    pub review_current_match: bool,
    pub retention: Option<ThinReason>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThinContext {
    pub occurrence: usize,
    pub pattern: PatternId,
    pub track: TrackId,
    pub muted: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinChange {
    pub row: u32,
    pub column: u16,
    pub before: Cell,
    pub after: Option<Cell>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinSummary {
    pub algorithm: String,
    pub gate_algorithm: String,
    pub pattrns_revision: String,
    pub selected_stored_cells: usize,
    pub reviewed_cells: usize,
    pub empty_cells: usize,
    pub eligible_cells: usize,
    pub removed_cells: usize,
    pub reason_counts: BTreeMap<ThinReason, usize>,
    pub effective_probability: f32,
    pub gate_calls: usize,
    pub random_draws: usize,
    pub base_starts: usize,
    pub candidate_starts: usize,
    pub removed_starts: usize,
    pub base_controls: usize,
    pub candidate_controls: usize,
    pub removed_controls: usize,
    pub affected_occurrences: usize,
    pub affected_blocks: usize,
    pub affected_aliases: usize,
    pub max_source_path_bytes: usize,
    pub estimated_path_payload: usize,
    pub warnings: Vec<String>,
}
/// Transient reversible record header. Exact before->None data are separately
/// paginated through changes(); hashes bind them. No persisted/live recipe.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinRecord {
    pub algorithm: String,
    pub version: u32,
    pub gate_algorithm: String,
    pub pattrns_revision: String,
    pub config: ThinConfig,
    pub base_sha256: String,
    pub candidate_sha256: String,
    pub decisions_sha256: String,
    pub uses_sha256: String,
    pub changes_sha256: String,
    pub retained_proof_sha256: String,
    pub change_count: usize,
}
#[derive(Debug, Clone)]
pub struct ThinPreview {
    id: String,
    base: Song,
    candidate: Song,
    config: ThinConfig,
    summary: ThinSummary,
    decisions: Vec<ThinDecision>,
    uses: Vec<ThinUse>,
    contexts: Vec<ThinContext>,
    changes: Vec<ThinChange>,
    record: ThinRecord,
}
impl ThinPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &ThinConfig {
        &self.config
    }
    pub fn summary(&self) -> &ThinSummary {
        &self.summary
    }
    pub fn decisions(&self) -> &[ThinDecision] {
        &self.decisions
    }
    pub fn uses(&self) -> &[ThinUse] {
        &self.uses
    }
    pub fn contexts(&self) -> &[ThinContext] {
        &self.contexts
    }
    pub fn changes(&self) -> &[ThinChange] {
        &self.changes
    }
    pub fn record(&self) -> &ThinRecord {
        &self.record
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base || hash(current)? != self.record.base_sha256 {
            // Structural float equality alone treats +0/-0 as equal. The
            // captured record binds exact serialized source representations.
            Err("THIN preview is stale; prepare and review again".into())
        } else {
            Ok(self.candidate.clone())
        }
    }
}
fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|b| crate::sample_asset::content_sha256(&b))
        .map_err(|e| e.to_string())
}

fn preflight(song: &Song, config: &ThinConfig) -> Result<(usize, usize), String> {
    if config.version != VERSION
        || config.removal_percent > 100
        || usize::from(config.max_removed) > MAX_RECORDS
    {
        return Err(
            "THIN requires version 1, integer removal percent 0..100 and max_removed 0..4096"
                .into(),
        );
    }
    if config.protected_roles.len() > 5
        || config.protected_roles.iter().enumerate().any(|(i, r)| {
            matches!(r, PercussionRole::Mixed | PercussionRole::Unknown)
                || config.protected_roles[..i].contains(r)
        })
    {
        return Err(
            "Protected roles must be unique concrete roles; Mixed/Unknown are always retained"
                .into(),
        );
    }
    crate::project::check_serialized_size(song)?;
    crate::generation::check_work(song, false)
        .map_err(|e| format!("THIN model work preflight: {e}"))?;
    // Bound the existing evaluator's per-voice path clones BEFORE it runs.
    // This is replicated path payload, not a global memory/allocator guarantee.
    let max_path = song
        .samples
        .values()
        .map(|a| a.path.as_os_str().as_encoded_bytes().len())
        .chain(
            song.instruments
                .values()
                .filter_map(|i| i.sample.as_ref())
                .map(|p| p.as_os_str().as_encoded_bytes().len()),
        )
        .max()
        .unwrap_or(0);
    let payload = max_path
        .checked_mul(MAX_STARTS)
        .ok_or("THIN path provenance payload overflow")?;
    if payload > MAX_PATH_PAYLOAD {
        return Err(format!("THIN replicated path payload {payload} exceeds {MAX_PATH_PAYLOAD} bytes (2048 starts × max declared path {max_path} bytes, including unused paths); shorten source-path metadata before preview"));
    }
    song.validate()?;
    let s = &config.scope;
    song.require_local_track(s.pattern, s.track)?;
    if s.row_start >= s.row_end || s.row_end > song.patterns[&s.pattern].rows {
        return Err("THIN rows require a nonempty [start,end) range inside the pattern".into());
    }
    if s.columns.is_empty()
        || s.columns.len() > 256
        || s.columns.windows(2).any(|c| c[0] >= c[1])
        || s.columns
            .iter()
            .any(|&c| c >= song.tracks[&s.track].note_columns)
    {
        return Err(
            "THIN columns must be nonempty, strictly ascending, unique and inside the track".into(),
        );
    }
    Ok((max_path, payload))
}
fn contexts(song: &Song, scope: &TransformScope) -> Result<(Vec<ThinContext>, usize), String> {
    let mut source_blocks = BTreeSet::from([(scope.pattern, scope.track)]);
    let mut aliases = 0;
    for (&pattern, p) in &song.patterns {
        for &track in p.track_aliases.keys() {
            if song.resolve_track_source(pattern, track)? == (scope.pattern, scope.track) {
                source_blocks.insert((pattern, track));
                aliases += 1;
            }
        }
    }
    let mut contexts = Vec::new();
    for (occurrence, o) in song.order.iter().enumerate() {
        for &track in &song.track_order {
            if source_blocks.contains(&(o.pattern, track)) {
                if contexts.len() >= MAX_CONTEXTS {
                    return Err("THIN affects more than 4096 ordered track blocks; use a less repeated/shared source".into());
                }
                contexts.push(ThinContext {
                    occurrence,
                    pattern: o.pattern,
                    track,
                    muted: o.muted_tracks.contains(&track),
                });
            }
        }
    }
    Ok((contexts, aliases))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct CellOrigin {
    pattern: PatternId,
    track: TrackId,
    row: u32,
    column: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct PhraseOrigin {
    phrase: PhraseId,
    column: u16,
    cycle: Option<u16>,
    row: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Origin {
    occurrence: usize,
    track: TrackId,
    row: u32,
    column: u16,
    phrase: Option<PhraseOrigin>,
    ordinal: u32,
}
#[derive(Debug, Clone, Serialize)]
struct OnProof {
    origin: Origin,
    cell: CellOrigin,
    frame: u64,
    instrument: InstrumentId,
    pitch: u8,
    velocity: f32,
    pan: f32,
    source: ThinSource,
    selection_velocity: Option<f32>,
    velocity_bin: Option<u8>,
}
impl PartialEq for OnProof {
    fn eq(&self, b: &Self) -> bool {
        self.origin == b.origin
            && self.cell == b.cell
            && self.frame == b.frame
            && self.instrument == b.instrument
            && self.pitch == b.pitch
            && self.source == b.source
            && self.velocity.to_bits() == b.velocity.to_bits()
            && self.pan.to_bits() == b.pan.to_bits()
            && self.selection_velocity.map(f32::to_bits) == b.selection_velocity.map(f32::to_bits)
            && self.velocity_bin == b.velocity_bin
    }
}
fn proofs(song: &Song, manifest: &Manifest) -> Result<Vec<OnProof>, String> {
    let mut proofs = Vec::new();
    let mut ordinals = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    for event in &manifest.events {
        let Command::On {
            voice,
            pitch,
            volume,
            pan,
        } = event.command
        else {
            continue;
        };
        if proofs.len() >= MAX_STARTS {
            return Err("THIN proof exceeds 2048 evaluated ON starts".into());
        }
        let (occurrence, row) = event
            .position
            .ok_or("THIN requires a stable stored-cell origin for every ON")?;
        let pattern = song
            .order
            .get(occurrence)
            .ok_or("Invalid evaluated occurrence origin")?
            .pattern;
        let terminal = if let Some(value) = terminals.get(&(pattern, event.track)) {
            *value
        } else {
            let value = song.resolve_track_source(pattern, event.track)?;
            terminals.insert((pattern, event.track), value);
            value
        };
        let phrase = event.phrase.map(|p| PhraseOrigin {
            phrase: p.phrase,
            column: p.column,
            cycle: p.cycle,
            row: p.row,
        });
        let ordinal = ordinals
            .entry((occurrence, event.track, row, event.column, phrase))
            .or_insert(0u32);
        let origin = Origin {
            occurrence,
            track: event.track,
            row,
            column: event.column,
            phrase,
            ordinal: *ordinal,
        };
        *ordinal = ordinal.checked_add(1).ok_or("THIN origin count overflow")?;
        let (source, selection_velocity, velocity_bin) =
            if let Some(pad) = manifest.pad_voices.get(&voice) {
                let l = pad.layer_selection.as_ref();
                (
                    ThinSource::DrumPad {
                        kit: pad.kit,
                        pad: pad.pad,
                        region: pad.region.region,
                        root_note: pad.region.root_note,
                        layer: l.map(|v| v.layer),
                        zone_min: l.map(|v| v.min_velocity),
                        zone_max: l.map(|v| v.max_velocity),
                        round_robin_index: l.map(|v| v.round_robin_index),
                    },
                    l.map(|v| v.input_velocity),
                    l.map(|v| v.velocity_bin),
                )
            } else if let Some(map) = manifest.multisample_voices.get(&voice) {
                (
                    ThinSource::Multisample {
                        map: map.map,
                        zone: map.zone,
                        region: map.region.region,
                        root_note: map.region.root_note,
                    },
                    Some(map.input_velocity),
                    Some(map.velocity_bin),
                )
            } else if let Some(region) = manifest.regions.get(&event.instrument) {
                (
                    ThinSource::Region {
                        region: region.region,
                        root_note: region.root_note,
                    },
                    None,
                    None,
                )
            } else {
                (ThinSource::Legacy, None, None)
            };
        proofs.push(OnProof {
            origin,
            cell: CellOrigin {
                pattern: terminal.0,
                track: terminal.1,
                row,
                column: event.column,
            },
            frame: event.frame,
            instrument: event.instrument,
            pitch,
            velocity: volume,
            pan,
            source,
            selection_velocity,
            velocity_bin,
        });
    }
    Ok(proofs)
}
#[derive(Debug, Clone, Serialize)]
enum ControlAction {
    Volume(f32),
    Pan(f32),
    Glide(u8, f32),
    SampleStart(Option<u64>, Direction),
    Offset(u64),
    Direction(Direction),
    Cut,
}
impl PartialEq for ControlAction {
    fn eq(&self, b: &Self) -> bool {
        match (self, b) {
            (Self::Volume(a), Self::Volume(b)) | (Self::Pan(a), Self::Pan(b)) => {
                a.to_bits() == b.to_bits()
            }
            (Self::Glide(a, x), Self::Glide(b, y)) => a == b && x.to_bits() == y.to_bits(),
            (Self::SampleStart(a, x), Self::SampleStart(b, y)) => a == b && x == y,
            (Self::Offset(a), Self::Offset(b)) => a == b,
            (Self::Direction(a), Self::Direction(b)) => a == b,
            (Self::Cut, Self::Cut) => true,
            _ => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
struct ControlProof {
    frame: u64,
    track: TrackId,
    column: u16,
    position: Option<(usize, u32)>,
    phrase: Option<PhraseOrigin>,
    target: Origin,
    action: ControlAction,
}
fn control_proofs(manifest: &Manifest, starts: &[OnProof]) -> Result<Vec<ControlProof>, String> {
    let mut voices = BTreeMap::new();
    let mut on_index = 0;
    for event in &manifest.events {
        if let Command::On { voice, .. } = event.command {
            let start = starts
                .get(on_index)
                .ok_or("THIN ON proof/voice count mismatch")?;
            voices.insert(voice, start.origin);
            on_index += 1;
        }
    }
    let mut result = Vec::new();
    for event in &manifest.events {
        let (voice, action) = match event.command {
            Command::On { .. } | Command::Off { .. } => continue,
            Command::Volume { voice, value } => (voice, ControlAction::Volume(value)),
            Command::Pan { voice, value } => (voice, ControlAction::Pan(value)),
            Command::Glide {
                voice,
                target,
                semitones_per_second,
            } => (voice, ControlAction::Glide(target, semitones_per_second)),
            Command::SampleStart {
                voice,
                offset,
                direction,
            } => (voice, ControlAction::SampleStart(offset, direction)),
            Command::Offset { voice, frame } => (voice, ControlAction::Offset(frame)),
            Command::Direction { voice, direction } => (voice, ControlAction::Direction(direction)),
            Command::Cut { voice } => (voice, ControlAction::Cut),
        };
        if result.len() >= 65536 {
            return Err("THIN control proof exceeds 65536 commands".into());
        }
        let target = *voices
            .get(&voice)
            .ok_or("THIN control has no retained stable ON origin")?;
        result.push(ControlProof {
            frame: event.frame,
            track: event.track,
            column: event.column,
            position: event.position,
            phrase: event.phrase.map(|p| PhraseOrigin {
                phrase: p.phrase,
                column: p.column,
                cycle: p.cycle,
                row: p.row,
            }),
            target,
            action,
        });
    }
    Ok(result)
}
type RoleStatus = (Option<RoleLabel>, bool, Option<ThinReason>);
fn role_status(
    song: &Song,
    region: Option<RegionId>,
    protected: &[PercussionRole],
    cache: &mut BTreeMap<RegionId, RoleStatus>,
) -> Result<RoleStatus, String> {
    let Some(id) = region else {
        return Ok((None, false, Some(ThinReason::UnknownSource)));
    };
    if let Some(status) = cache.get(&id) {
        return Ok(status.clone());
    }
    let inspection = crate::slice_features::inspect_catalog(song, id)?;
    let label = inspection
        .entry
        .and_then(|e| e.reviewed_role.map(|r| r.label));
    let current = inspection.reviewed_role_current_match == Some(true);
    let reason = match &label {
        None => Some(ThinReason::Unreviewed),
        Some(_) if !current => Some(ThinReason::StaleReview),
        Some(r) if matches!(r.role, PercussionRole::Mixed | PercussionRole::Unknown) => {
            Some(ThinReason::MixedOrUnknown)
        }
        Some(r) if protected.contains(&r.role) => Some(ThinReason::ProtectedRole),
        Some(_) => None,
    };
    let status = (label, current, reason);
    cache.insert(id, status.clone());
    Ok(status)
}
fn evaluate(song: &Song, stage: &str) -> Result<Manifest, String> {
    song_render::evaluate_model(song).map_err(|e|format!("THIN {stage} must be model-evaluable with all used sources assigned (no file IO or fabricated assignments): {e}"))
}
/// Pure preview. Uses the existing model evaluator before/after, not a partial
/// scheduler. File existence/hash/decode and actual PCM are not established.
pub fn preview(song: &Song, config: ThinConfig) -> Result<ThinPreview, String> {
    let (max_path, payload) = preflight(song, &config)?;
    let (contexts, aliases) = contexts(song, &config.scope)?;
    let mut selected_stored = 0;
    let mut empty = 0;
    let mut count = 0;
    if let Some(rows) = song.patterns[&config.scope.pattern]
        .tracks
        .get(&config.scope.track)
    {
        let mut visits = 0usize;
        for (_, columns) in rows.range(config.scope.row_start..config.scope.row_end) {
            visits = visits
                .checked_add(1)
                .and_then(|v| v.checked_add(columns.len()))
                .ok_or("THIN selected-source scan overflow")?;
            if visits > 65536 {
                return Err("THIN selected-source scan exceeds 65536 row-container/cell visits, including unselected columns and stored defaults; narrow the scope".into());
            }
        }
    }
    if let Some(rows) = song.patterns[&config.scope.pattern]
        .tracks
        .get(&config.scope.track)
    {
        for (_, columns) in rows.range(config.scope.row_start..config.scope.row_end) {
            for (&column, cell) in columns {
                if config.scope.columns.binary_search(&column).is_ok() {
                    selected_stored += 1;
                    if cell == &Cell::default() {
                        empty += 1;
                    } else {
                        count += 1;
                        if count > MAX_RECORDS {
                            return Err("THIN selection exceeds 4096 nondefault cells before decision allocation; narrow rows/columns".into());
                        }
                    }
                }
            }
        }
    }
    let mut decisions = Vec::with_capacity(count);
    let mut indices = BTreeMap::new();
    if let Some(rows) = song.patterns[&config.scope.pattern]
        .tracks
        .get(&config.scope.track)
    {
        for (&row, columns) in rows.range(config.scope.row_start..config.scope.row_end) {
            for (&column, cell) in columns {
                if config.scope.columns.binary_search(&column).is_ok() && cell != &Cell::default() {
                    let reason = if !matches!(cell.note, Note::Note(_)) {
                        ThinReason::NotOn
                    } else if cell.phrase.is_some() {
                        ThinReason::Phrase
                    } else if cell.effects != StepEffects::default() {
                        ThinReason::Effects
                    } else {
                        ThinReason::NoAudibleUse
                    };
                    indices.insert((row, column), decisions.len());
                    decisions.push(ThinDecision {
                        row,
                        column,
                        before: cell.clone(),
                        uses: 0,
                        reason,
                        gate_index: None,
                    });
                }
            }
        }
    }
    let base_manifest = evaluate(song, "base")?;
    let base_proofs = proofs(song, &base_manifest)?;
    let base_controls = control_proofs(&base_manifest, &base_proofs)?;
    // Only compact metadata escapes the evaluator; never clone paths per proof.
    drop(base_manifest);
    let mut uses = Vec::new();
    let mut role_cache = BTreeMap::new();
    let mut blocked = vec![None; decisions.len()];
    for proof in &base_proofs {
        if proof.cell.pattern != config.scope.pattern
            || proof.cell.track != config.scope.track
            || proof.origin.phrase.is_some()
        {
            continue;
        }
        let Some(&index) = indices.get(&(proof.cell.row, proof.cell.column)) else {
            continue;
        };
        let (label, current, retention) = role_status(
            song,
            proof.source.region(),
            &config.protected_roles,
            &mut role_cache,
        )?;
        decisions[index].uses += 1;
        if let Some(reason) = retention {
            blocked[index] = Some(blocked[index].map_or(reason, |old: ThinReason| old.min(reason)));
        }
        uses.push(ThinUse {
            decision: index,
            occurrence: proof.origin.occurrence,
            track: proof.origin.track,
            row: proof.origin.row,
            column: proof.origin.column,
            frame: proof.frame,
            instrument: proof.instrument,
            source: proof.source.clone(),
            pitch: proof.pitch,
            velocity: proof.velocity,
            pan: proof.pan,
            selection_velocity: proof.selection_velocity,
            velocity_bin: proof.velocity_bin,
            reviewed_role: label,
            review_current_match: current,
            retention,
        });
    }
    let probability = (f64::from(config.removal_percent) / 100.0) as f32;
    let mut gate = ProbabilityGate::new(Some(config.seed));
    let pulse = RhythmEvent {
        value: probability,
        step_time: 1.0,
    };
    let mut calls = 0;
    let mut eligible = 0;
    let mut changes = Vec::new();
    let mut removed = BTreeSet::new();
    for (index, decision) in decisions.iter_mut().enumerate() {
        if decision.reason != ThinReason::NoAudibleUse || decision.uses == 0 {
            continue;
        }
        if let Some(reason) = blocked[index] {
            decision.reason = reason;
            continue;
        }
        eligible += 1;
        let remove = if config.removal_percent == 0 {
            false
        } else {
            decision.gate_index = Some(calls);
            calls += 1;
            gate.run(&pulse)
        };
        decision.reason = if remove {
            ThinReason::Removed
        } else {
            ThinReason::GateKept
        };
        if remove {
            removed.insert(CellOrigin {
                pattern: config.scope.pattern,
                track: config.scope.track,
                row: decision.row,
                column: decision.column,
            });
            changes.push(ThinChange {
                row: decision.row,
                column: decision.column,
                before: decision.before.clone(),
                after: None,
            });
        }
    }
    if changes.len() > usize::from(config.max_removed) {
        return Err(format!("THIN seeded decisions request {} removals, exceeding max_removed {}; preview rejected without first-N truncation",changes.len(),config.max_removed));
    }
    let mut candidate = song.clone();
    for change in &changes {
        candidate
            .patterns
            .get_mut(&config.scope.pattern)
            .unwrap()
            .tracks
            .get_mut(&config.scope.track)
            .unwrap()
            .get_mut(&change.row)
            .unwrap()
            .remove(&change.column);
    }
    // Preserve empty containers, other fields and historical generation/catalog
    // records exactly. Timing never changes; no normalization/materialization.
    candidate.validate()?;
    crate::project::check_serialized_size(&candidate)?;
    let wanted: Vec<_> = base_proofs
        .iter()
        .filter(|p| p.origin.phrase.is_some() || !removed.contains(&p.cell))
        .collect();
    let removed_origins: BTreeSet<_> = base_proofs
        .iter()
        .filter(|p| p.origin.phrase.is_none() && removed.contains(&p.cell))
        .map(|p| p.origin)
        .collect();
    let wanted_controls: Vec<_> = base_controls
        .iter()
        .filter(|p| !removed_origins.contains(&p.target))
        .collect();
    let (candidate_starts, candidate_controls) = if changes.is_empty() {
        (base_proofs.len(), base_controls.len())
    } else {
        let after = evaluate(&candidate, "candidate")?;
        let actual = proofs(&candidate, &after)?;
        if actual.len() != wanted.len() || actual.iter().zip(&wanted).any(|(a, b)| a != *b) {
            let mismatch = actual
                .iter()
                .zip(&wanted)
                .position(|(a, b)| a != *b)
                .unwrap_or(actual.len().min(wanted.len()));
            return Err(format!("THIN retained-ON proof failed at ordered start {mismatch}: expected {} starts, found {}; removal changes retained origin/order, source/zone/layer/root, frame/pitch/gain/pan or reveals/cancels phrase starts. Choose another seed/scope or make selection memory explicit; no partial result or fallback",wanted.len(),actual.len()));
        }
        let controls = control_proofs(&after, &actual)?;
        if controls.len() != wanted_controls.len()
            || controls.iter().zip(&wanted_controls).any(|(a, b)| a != *b)
        {
            return Err("THIN retained-control proof failed: Volume/Pan/Glide/Offset/Direction/SampleStart/Cut changes origin, target voice, frame/value or retained order. No control retargeting or partial fallback is accepted".into());
        }
        (actual.len(), controls.len())
    };
    let mut reason_counts = BTreeMap::new();
    for d in &decisions {
        *reason_counts.entry(d.reason).or_insert(0) += 1;
    }
    let warnings=vec![
  "Whole ordinary-cell deletion only. Unsupported phrase/FX/non-ON cells and cells with no evaluated unsuppressed current ON use are retained and reported. Empty row/track containers and all unrelated stored metadata are preserved.".into(),
  "Only explicit currently model-bound human roles authorize eligibility; suggestions and feature scores are never role authority. Any protected, stale, unreviewed, Mixed/Unknown or unknown-source use retains the shared cell. Normal/Ghost articulation does not override protection.".into(),
  "All current repeated and alias uses are aggregated before gate calls. One canonical row/column gate call per eligible stored cell, not per occurrence. Changing eligibility/base can shift later seeded choices; reproducibility means identical base and config, not repeated-apply idempotence.".into(),
  "Actual pinned pattrns ProbabilityGate uses the disclosed f32 probability and strict comparison; endpoints consume no RNG draws. Zero percent bypasses gate/edits but still validates full model, scope and budgets. max_removed is a rejection cap, never truncation.".into(),
  "Both whole documents must be model-evaluable with assigned used sources. No paths are opened, no source hashes/PCM are verified, and clipping/audibility/render success is not guaranteed. Zero-gain or silent-source ONs still count; a no-use reason does not measure audibility. Current role matches are model metadata equality only.".into(),
  "Every retained ON preserves stable origin/order, current selected source/zone/layer/root and exact frame/pitch/gain/pan. Voice IDs and RR provenance ordinals may renumber. Retained Volume/Pan/Glide/Offset/Direction/SampleStart/Cut keep origin, target, frame/value and order. Only OFF/release/replacement/retrigger/choke lifetimes may differ; this is NOT whole-audio or anchor-tail invariance.".into(),
  "No live probability, genre preset, structural phrase generator or durable audit log is created. Review record and exact deletion pages are transient; existing Editor Undo restores the document, and saving stores ordinary cells only.".into(),
 ];
    let summary = ThinSummary {
        algorithm: ALGORITHM.into(),
        gate_algorithm: GATE_ALGORITHM.into(),
        pattrns_revision: crate::pattern::REVISION.into(),
        selected_stored_cells: selected_stored,
        reviewed_cells: decisions.len(),
        empty_cells: empty,
        eligible_cells: eligible,
        removed_cells: changes.len(),
        reason_counts,
        effective_probability: probability,
        gate_calls: calls,
        random_draws: if config.removal_percent < 100 {
            calls
        } else {
            0
        },
        base_starts: base_proofs.len(),
        candidate_starts,
        removed_starts: base_proofs.len() - candidate_starts,
        base_controls: base_controls.len(),
        candidate_controls,
        removed_controls: base_controls.len() - candidate_controls,
        affected_occurrences: contexts
            .iter()
            .map(|c| c.occurrence)
            .collect::<BTreeSet<_>>()
            .len(),
        affected_blocks: contexts.len(),
        affected_aliases: aliases,
        max_source_path_bytes: max_path,
        estimated_path_payload: payload,
        warnings,
    };
    let record = ThinRecord {
        algorithm: ALGORITHM.into(),
        version: VERSION,
        gate_algorithm: GATE_ALGORITHM.into(),
        pattrns_revision: crate::pattern::REVISION.into(),
        config: config.clone(),
        base_sha256: hash(song)?,
        candidate_sha256: hash(&candidate)?,
        decisions_sha256: hash(&decisions)?,
        uses_sha256: hash(&uses)?,
        changes_sha256: hash(&changes)?,
        retained_proof_sha256: hash(&(&wanted, &wanted_controls))?,
        change_count: changes.len(),
    };
    let id = hash(&record)?;
    Ok(ThinPreview {
        id,
        base: song.clone(),
        candidate,
        config,
        summary,
        decisions,
        uses,
        contexts,
        changes,
        record,
    })
}
