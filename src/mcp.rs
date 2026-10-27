//! Local, headless MCP adapter. All musical validation, history and rendering use
//! the same Song / Editor / project / evaluator implementations as the GUI.
use crate::{
    editor::{DocumentOwner, Editor},
    project,
    session::{Action, Session},
    song::*,
    song_render,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, BufRead, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub const PROTOCOL_VERSION: &str = "2025-11-25";
pub const API_VERSION: u32 = 1;
const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_OPERATIONS: usize = 256;
const MAX_DOCUMENT: usize = 8 * 1024 * 1024;
const MAX_ANALYSIS_SECONDS: u64 = 60;
const MAX_ANALYSIS_SUGGESTIONS: usize = 4096;
const MAX_ANALYSIS_BINS: usize = 2048;
const MAX_PREVIEW_BYTES: usize = 256 * 1024;
const MAX_ONSET_SECONDS: u64 = 15;
const MAX_ONSET_SUGGESTIONS: usize = 4096;
const MAX_ONSET_PAGE: usize = 256;
const MAX_ONSET_RESULT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ONSET_RESULT_PAGE: usize = 64;
const MAX_TRANSFORM_PAGE: usize = 64;
const MAX_GENERATION_PAGE: usize = 64;
const MAX_SOURCE_INSPECTION_PAGE: usize = 128;
const MAX_GROOVE_PAGE: usize = 64;
const MAX_KEYZONE_PAGE: usize = 64;
const MAX_RESAMPLE_PAGE: usize = 64;
const MAX_TEMPO_PAGE: usize = 64;
const MAX_FEATURE_PAGE: usize = 16;
const MAX_THIN_PAGE: usize = 64;
const MAX_SAMPLE_PEAK_PAGE: usize = 4;

#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
    details: Value,
}
type Outcome<T> = Result<T, Failure>;
impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: json!({}),
        }
    }
}
fn validation(message: String) -> Failure {
    Failure::new("validation_error", message)
}
fn io_failure(message: impl ToString) -> Failure {
    Failure::new("io_error", message.to_string())
}
fn decode<T: DeserializeOwned>(arguments: Value) -> Outcome<T> {
    serde_json::from_value(arguments).map_err(|e| Failure::new("invalid_arguments", e.to_string()))
}

/// Explicit authority boundary, fixed at process startup. This is a pathname
/// policy, not an OS sandbox against a hostile process concurrently renaming files.
struct Workspace {
    root: PathBuf,
}
impl Workspace {
    fn new(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("Workspace root: {e}"))?;
        let root = project::normalize_canonical_path(root);
        if root.to_str().is_none() {
            return Err("Workspace root must be representable as UTF-8 JSON text".into());
        }
        if !root.is_dir() {
            return Err("Workspace root must be an existing directory".into());
        }
        Ok(Self { root })
    }
    fn resolve(&self, relative: &str, must_exist: bool) -> Outcome<PathBuf> {
        let path = Path::new(relative);
        if relative.is_empty()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Failure::new(
                "path_denied",
                "Use a nonempty workspace-relative path without . or .. components",
            ));
        }
        self.check(&self.root.join(path), must_exist)
    }
    fn check(&self, absolute: &Path, must_exist: bool) -> Outcome<PathBuf> {
        let relative = relative_under_root(&self.root, absolute).ok_or_else(|| {
            Failure::new("path_denied", "Path is outside the startup workspace root")
        })?;
        let components: Vec<_> = relative.components().collect();
        if components.is_empty()
            || components
                .iter()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Failure::new(
                "path_denied",
                "Traversal and directory-only paths are not allowed",
            ));
        }
        // Keep the spelling used by the caller after proving that it names
        // this root. A long verbatim input must stay verbatim for the file
        // system calls below.
        let mut current = PathBuf::new();
        for component in absolute.components().take(self.root.components().count()) {
            current.push(component.as_os_str());
        }
        for (index, component) in components.iter().enumerate() {
            current.push(component.as_os_str());
            let last = index + 1 == components.len();
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(Failure::new(
                            "path_denied",
                            "Symlinks are not allowed in project, sample or output paths",
                        ));
                    }
                    if (last && !metadata.is_file()) || (!last && !metadata.is_dir()) {
                        return Err(Failure::new(
                            "path_denied",
                            "Expected a regular file beneath existing workspace directories",
                        ));
                    }
                }
                Err(error) if last && !must_exist && error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(io_failure(error)),
            }
        }
        let parent = current
            .parent()
            .unwrap()
            .canonicalize()
            .map(project::normalize_canonical_path)
            .map_err(io_failure)?;
        if relative_under_root(&self.root, &parent).is_none() {
            return Err(Failure::new(
                "path_denied",
                "Resolved parent escaped the workspace",
            ));
        }
        Ok(current)
    }
    fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }
    fn assets(&self, song: &Song) -> Outcome<()> {
        for instrument in song.instruments.values() {
            if let Some(sample) = &instrument.sample {
                self.check(sample, true)?;
            }
        }
        for sample in song.samples.values() {
            self.check(&sample.path, true)?;
        }
        Ok(())
    }
}

/// Return a path's lexical suffix beneath the startup root. Windows may spell
/// the same drive prefix as either `C:` or `\\?\C:`; accept that one prefix
/// equivalence while comparing every remaining component exactly. This keeps
/// long verbatim paths intact and does not normalize reserved names or device
/// namespaces into ordinary DOS paths.
fn relative_under_root(root: &Path, path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::path::Prefix;

        let root_components: Vec<_> = root.components().collect();
        let path_components: Vec<_> = path.components().collect();
        if path_components.len() < root_components.len() {
            return None;
        }
        for (index, (root_component, path_component)) in root_components
            .iter()
            .zip(path_components.iter())
            .enumerate()
        {
            let equal = if index == 0 {
                match (root_component, path_component) {
                    (Component::Prefix(root_prefix), Component::Prefix(path_prefix)) => {
                        match (root_prefix.kind(), path_prefix.kind()) {
                            (Prefix::Disk(root_drive), Prefix::VerbatimDisk(path_drive))
                            | (Prefix::VerbatimDisk(root_drive), Prefix::Disk(path_drive)) => {
                                root_drive.eq_ignore_ascii_case(&path_drive)
                            }
                            (
                                Prefix::UNC(root_server, root_share),
                                Prefix::VerbatimUNC(path_server, path_share),
                            )
                            | (
                                Prefix::VerbatimUNC(root_server, root_share),
                                Prefix::UNC(path_server, path_share),
                            ) => root_server == path_server && root_share == path_share,
                            _ => root_component == path_component,
                        }
                    }
                    _ => root_component == path_component,
                }
            } else {
                root_component == path_component
            };
            if !equal {
                return None;
            }
        }

        let mut relative = PathBuf::new();
        for component in path_components.iter().skip(root_components.len()) {
            relative.push(component.as_os_str());
        }
        Some(relative)
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(root).ok().map(Path::to_owned)
    }
}
fn extension(path: &Path, suffix: &str) -> Outcome<()> {
    if !path.to_string_lossy().ends_with(suffix) {
        return Err(Failure::new(
            "invalid_path_type",
            format!("Path must end with {suffix}"),
        ));
    }
    Ok(())
}

/// Typed, protocol-independent edit vocabulary. Each operation is validated using
/// Song; the final candidate is committed once by Editor::apply_document_edit.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditOperation {
    DrumStepToggle {
        projection_token: String,
        pad_id: u64,
        step: u8,
    },
    DrumStepSetVelocity {
        projection_token: String,
        pad_id: u64,
        row: u32,
        column: u16,
        #[serde(deserialize_with = "required_source_bpm")]
        value: Option<f64>,
    },
    DrumStepSetDelay {
        projection_token: String,
        pad_id: u64,
        row: u32,
        column: u16,
        #[serde(deserialize_with = "required_source_bpm")]
        value: Option<f64>,
    },
    DrumStepSetRetrigger {
        projection_token: String,
        pad_id: u64,
        row: u32,
        column: u16,
        #[serde(deserialize_with = "required_retrigger")]
        value: Option<Retrigger>,
    },
    SetSong {
        title: Option<String>,
        bpm: Option<f64>,
        lines_per_beat: Option<u16>,
    },
    SetTrackMix {
        config: crate::track_mix::TrackMixConfig,
    },
    UpsertTrack {
        id: u64,
        name: String,
        note_columns: u16,
    },
    RemoveTrack {
        id: u64,
        #[serde(default)]
        clear_cells: bool,
    },
    SetTrackOrder {
        track_ids: Vec<u64>,
    },
    UpsertInstrument {
        id: u64,
        name: String,
        #[serde(deserialize_with = "required_sample")]
        sample: Option<String>,
    },
    RemoveInstrument {
        id: u64,
    },
    UpsertPattern {
        id: u64,
        name: String,
        rows: u32,
    },
    RemovePattern {
        id: u64,
    },
    ClonePattern {
        source_id: u64,
        new_id: u64,
        name: Option<String>,
    },
    SetOrder {
        pattern_ids: Vec<u64>,
    },
    AppendRepeat {
        pattern_id: u64,
    },
    SetTrackAlias {
        pattern_id: u64,
        track_id: u64,
        #[serde(deserialize_with = "required_alias")]
        source: Option<TrackAlias>,
    },
    MakeTrackUnique {
        pattern_id: u64,
        track_id: u64,
    },
    SetOccurrenceTrackMute {
        occurrence_index: usize,
        track_id: u64,
        muted: bool,
    },
    MoveOccurrence {
        from_index: usize,
        to_index: usize,
    },
    SetCell {
        pattern_id: u64,
        track_id: u64,
        row: u32,
        column: u16,
        cell: Cell,
    },
    ClearCell {
        pattern_id: u64,
        track_id: u64,
        row: u32,
        column: u16,
    },
    SetEffects {
        pattern_id: u64,
        track_id: u64,
        row: u32,
        column: u16,
        effects: StepEffects,
    },
    ClearPatternTrack {
        pattern_id: u64,
        track_id: u64,
    },
    UpsertPhrase {
        id: u64,
        name: String,
        instrument_id: u64,
        rows: u32,
        lines_per_beat: u16,
        note_columns: u16,
        root_note: u8,
    },
    RemovePhrase {
        id: u64,
    },
    RemoveGroove {
        id: u64,
    },
    CreateMultisampleInstrument {
        name: String,
    },
    UpsertMultisample {
        id: u64,
        name: String,
    },
    SetKeyzones {
        map_id: u64,
        zones: Vec<KeyZoneInput>,
    },
    RemoveMultisample {
        id: u64,
    },
    AssignMultisample {
        instrument_id: u64,
        #[serde(deserialize_with = "required_region")]
        map_id: Option<u64>,
    },
    CreateDrumKitInstrument {
        name: String,
    },
    UpsertDrumKit {
        id: u64,
        name: String,
    },
    RemoveDrumKit {
        id: u64,
    },
    UpsertDrumPad {
        kit_id: u64,
        id: u64,
        slot: u8,
        name: String,
        note: u8,
        #[serde(deserialize_with = "required_region")]
        region_id: Option<u64>,
        root_note: u8,
        #[serde(default)]
        tune_semitones: i8,
        gain: f64,
        pan: f64,
        mode: PadMode,
        new_note: PadNewNote,
        #[serde(deserialize_with = "required_choke_group")]
        choke_group: Option<u8>,
    },
    RemoveDrumPad {
        kit_id: u64,
        id: u64,
    },
    SetPadSources {
        kit_id: u64,
        pad_id: u64,
        #[serde(deserialize_with = "required_region")]
        region_id: Option<u64>,
        velocity_zones: Vec<VelocityZone>,
    },
    AssignKit {
        instrument_id: u64,
        #[serde(deserialize_with = "required_region")]
        kit_id: Option<u64>,
    },
    SetPhraseCell {
        phrase_id: u64,
        row: u32,
        column: u16,
        cell: Cell,
    },
    ClearPhraseCell {
        phrase_id: u64,
        row: u32,
        column: u16,
    },
    UpsertMarkerSet {
        id: u64,
        name: String,
        sample_id: u64,
        #[serde(deserialize_with = "required_source_bpm")]
        source_bpm: Option<f64>,
        markers: Vec<u64>,
    },
    RemoveMarkerSet {
        id: u64,
    },
    SetMarkerLock {
        marker_set_id: u64,
        frame: u64,
        locked: bool,
    },
    AddMarker {
        marker_set_id: u64,
        frame: u64,
    },
    MoveMarker {
        marker_set_id: u64,
        from_frame: u64,
        to_frame: u64,
    },
    RemoveMarker {
        marker_set_id: u64,
        frame: u64,
    },
    RemoveSample {
        id: u64,
    },
    UpsertRegion {
        id: u64,
        name: String,
        sample_id: u64,
        start_frame: u64,
        end_frame: u64,
        root_note: u8,
    },
    SetRegionPlayback {
        region_id: u64,
        #[serde(deserialize_with = "required_amp_envelope")]
        amp_envelope: Option<AmpEnvelope>,
        #[serde(deserialize_with = "required_forward_loop")]
        forward_loop: Option<ForwardLoop>,
    },
    SetRegionRole {
        region_id: u64,
        #[serde(deserialize_with = "required_role_label")]
        label: Option<crate::slice_features::RoleLabel>,
    },
    ClearRegionFeatures {
        region_id: u64,
    },
    RemoveRegion {
        id: u64,
    },
    AssignRegion {
        instrument_id: u64,
        #[serde(deserialize_with = "required_region")]
        region_id: Option<u64>,
    },
    SetPhraseTrigger {
        pattern_id: u64,
        track_id: u64,
        row: u32,
        column: u16,
        #[serde(deserialize_with = "required_trigger")]
        trigger: Option<PhraseTrigger>,
    },
}
fn required_role_label<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::slice_features::RoleLabel>, D::Error> {
    Option::<crate::slice_features::RoleLabel>::deserialize(deserializer)
}
fn required_sample<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
fn required_source_bpm<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    Option::<f64>::deserialize(deserializer)
}
fn required_region<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(deserializer)
}
fn required_amp_envelope<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<AmpEnvelope>, D::Error> {
    Option::<AmpEnvelope>::deserialize(deserializer)
}
fn required_forward_loop<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ForwardLoop>, D::Error> {
    Option::<ForwardLoop>::deserialize(deserializer)
}
fn required_choke_group<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u8>, D::Error> {
    Option::<u8>::deserialize(deserializer)
}
fn required_retrigger<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Retrigger>, D::Error> {
    Option::<Retrigger>::deserialize(deserializer)
}
fn required_alias<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TrackAlias>, D::Error> {
    Option::<TrackAlias>::deserialize(deserializer)
}
fn required_trigger<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PhraseTrigger>, D::Error> {
    Option::<PhraseTrigger>::deserialize(deserializer)
}
fn set_cell(
    song: &mut Song,
    pattern_id: u64,
    track_id: u64,
    row: u32,
    column: u16,
    cell: Cell,
) -> Outcome<()> {
    song.require_local_track(PatternId(pattern_id), TrackId(track_id))
        .map_err(validation)?;
    let track = song
        .tracks
        .get(&TrackId(track_id))
        .ok_or_else(|| Failure::new("not_found", "Unknown track ID"))?;
    let pattern = song
        .patterns
        .get_mut(&PatternId(pattern_id))
        .ok_or_else(|| Failure::new("not_found", "Unknown pattern ID"))?;
    if row >= pattern.rows || column >= track.note_columns {
        return Err(Failure::new(
            "validation_error",
            "Cell row/column is outside pattern/track bounds",
        ));
    }
    if cell == Cell::default() {
        if let Some(rows) = pattern.tracks.get_mut(&TrackId(track_id)) {
            if let Some(columns) = rows.get_mut(&row) {
                columns.remove(&column);
                if columns.is_empty() {
                    rows.remove(&row);
                }
            }
            if rows.is_empty() {
                pattern.tracks.remove(&TrackId(track_id));
            }
        }
    } else {
        pattern
            .tracks
            .entry(TrackId(track_id))
            .or_default()
            .entry(row)
            .or_default()
            .insert(column, cell);
    }
    Ok(())
}
impl EditOperation {
    fn step_token(&self) -> Option<&str> {
        match self {
            Self::DrumStepToggle {
                projection_token, ..
            }
            | Self::DrumStepSetVelocity {
                projection_token, ..
            }
            | Self::DrumStepSetDelay {
                projection_token, ..
            }
            | Self::DrumStepSetRetrigger {
                projection_token, ..
            } => Some(projection_token),
            _ => None,
        }
    }
    fn apply(
        &self,
        song: &mut Song,
        workspace: &Workspace,
        steps: Option<&PendingDrumSteps>,
    ) -> Outcome<()> {
        match self {
            Self::DrumStepToggle { .. }
            | Self::DrumStepSetVelocity { .. }
            | Self::DrumStepSetDelay { .. }
            | Self::DrumStepSetRetrigger { .. } => {
                apply_drum_step(
                    song,
                    steps.ok_or_else(|| {
                        Failure::new(
                            "drum_step_projection_unavailable",
                            "Inspect the exact drum-step view before editing",
                        )
                    })?,
                    self,
                )?;
            }
            Self::SetSong {
                title,
                bpm,
                lines_per_beat,
            } => {
                if let Some(value) = title {
                    song.title = value.clone();
                }
                if let Some(value) = bpm {
                    song.bpm = *value;
                }
                if let Some(value) = lines_per_beat {
                    song.lines_per_beat = *value;
                }
            }
            Self::SetTrackMix { config } => {
                song.set_track_mix(config.clone()).map_err(validation)?;
            }
            Self::UpsertTrack {
                id,
                name,
                note_columns,
            } => {
                if song
                    .tracks
                    .insert(
                        TrackId(*id),
                        Track {
                            name: name.clone(),
                            note_columns: *note_columns,
                        },
                    )
                    .is_none()
                {
                    song.track_order.push(TrackId(*id));
                }
            }
            Self::RemoveTrack { id, clear_cells } => {
                if song.track_mix.contains_key(&TrackId(*id)) {
                    return Err(Failure::new("track_mix_reference", "Track has persisted balance settings. First use set_track_mix with the complete retained set excluding this track; clear_cells does not reset mixer settings"));
                }
                song.tracks
                    .remove(&TrackId(*id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown track ID"))?;
                song.track_order.retain(|track| track.0 != *id);
                if *clear_cells {
                    for pattern in song.patterns.values_mut() {
                        pattern.tracks.remove(&TrackId(*id));
                        pattern.track_aliases.remove(&TrackId(*id));
                    }
                }
            }
            Self::SetTrackOrder { track_ids } => {
                song.track_order = track_ids.iter().map(|id| TrackId(*id)).collect()
            }
            Self::UpsertInstrument { id, name, sample } => {
                let sample = sample
                    .as_ref()
                    .map(|path| workspace.resolve(path, true))
                    .transpose()?;
                if let Some(instrument) = song.instruments.get_mut(&InstrumentId(*id)) {
                    // An explicit legacy path switches the source; null updates
                    // legacy assignment only and preserves region/kit routing.
                    instrument.name = name.clone();
                    if sample.is_some() {
                        instrument.region = None;
                        instrument.kind = InstrumentKind::Pitched;
                    }
                    instrument.sample = sample;
                } else {
                    song.instruments.insert(
                        InstrumentId(*id),
                        Instrument {
                            name: name.clone(),
                            sample,
                            region: None,
                            kind: InstrumentKind::Pitched,
                        },
                    );
                }
            }
            Self::RemoveInstrument { id } => {
                song.instruments
                    .remove(&InstrumentId(*id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown instrument ID"))?;
            }
            Self::UpsertPattern { id, name, rows } => {
                let pattern = song
                    .patterns
                    .entry(PatternId(*id))
                    .or_insert_with(|| Pattern {
                        name: name.clone(),
                        rows: *rows,
                        tracks: BTreeMap::new(),
                        track_aliases: BTreeMap::new(),
                        generation: None,
                    });
                pattern.name = name.clone();
                pattern.rows = *rows;
            }
            Self::RemovePattern { id } => {
                song.patterns
                    .remove(&PatternId(*id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown pattern ID"))?;
            }
            Self::ClonePattern {
                source_id,
                new_id,
                name,
            } => {
                song.clone_pattern(PatternId(*source_id), PatternId(*new_id))
                    .map_err(validation)?;
                if let Some(name) = name {
                    song.patterns.get_mut(&PatternId(*new_id)).unwrap().name = name.clone();
                }
            }
            Self::SetOrder { pattern_ids } => {
                song.order = pattern_ids
                    .iter()
                    .map(|id| Occurrence {
                        pattern: PatternId(*id),
                        muted_tracks: BTreeSet::new(),
                        section: None,
                    })
                    .collect()
            }
            Self::AppendRepeat { pattern_id } => song
                .append_repeat(PatternId(*pattern_id))
                .map_err(validation)?,
            Self::SetTrackAlias {
                pattern_id,
                track_id,
                source,
            } => song
                .set_track_alias(PatternId(*pattern_id), TrackId(*track_id), *source)
                .map_err(validation)?,
            Self::MakeTrackUnique {
                pattern_id,
                track_id,
            } => song
                .make_track_unique(PatternId(*pattern_id), TrackId(*track_id))
                .map_err(validation)?,
            Self::SetOccurrenceTrackMute {
                occurrence_index,
                track_id,
                muted,
            } => song
                .set_occurrence_track_mute(*occurrence_index, TrackId(*track_id), *muted)
                .map_err(validation)?,
            Self::MoveOccurrence {
                from_index,
                to_index,
            } => song
                .move_occurrence(*from_index, *to_index)
                .map_err(validation)?,
            Self::SetCell {
                pattern_id,
                track_id,
                row,
                column,
                cell,
            } => set_cell(song, *pattern_id, *track_id, *row, *column, cell.clone())?,
            Self::ClearCell {
                pattern_id,
                track_id,
                row,
                column,
            } => set_cell(song, *pattern_id, *track_id, *row, *column, Cell::default())?,
            Self::SetEffects {
                pattern_id,
                track_id,
                row,
                column,
                effects,
            } => {
                let mut cell = song
                    .patterns
                    .get(&PatternId(*pattern_id))
                    .and_then(|p| p.tracks.get(&TrackId(*track_id)))
                    .and_then(|r| r.get(row))
                    .and_then(|c| c.get(column))
                    .cloned()
                    .unwrap_or_default();
                cell.effects = effects.clone();
                set_cell(song, *pattern_id, *track_id, *row, *column, cell)?;
            }
            Self::ClearPatternTrack {
                pattern_id,
                track_id,
            } => {
                song.require_local_track(PatternId(*pattern_id), TrackId(*track_id))
                    .map_err(validation)?;
                if !song.tracks.contains_key(&TrackId(*track_id)) {
                    return Err(Failure::new("not_found", "Unknown track ID"));
                }
                song.patterns
                    .get_mut(&PatternId(*pattern_id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown pattern ID"))?
                    .tracks
                    .remove(&TrackId(*track_id));
            }
            Self::UpsertPhrase {
                id,
                name,
                instrument_id,
                rows,
                lines_per_beat,
                note_columns,
                root_note,
            } => {
                let mut phrase = song
                    .phrases
                    .get(&PhraseId(*id))
                    .cloned()
                    .unwrap_or_else(|| Phrase {
                        name: name.clone(),
                        instrument: InstrumentId(*instrument_id),
                        rows: *rows,
                        lines_per_beat: *lines_per_beat,
                        note_columns: *note_columns,
                        root_note: *root_note,
                        cells: BTreeMap::new(),
                    });
                phrase.name = name.clone();
                phrase.instrument = InstrumentId(*instrument_id);
                phrase.rows = *rows;
                phrase.lines_per_beat = *lines_per_beat;
                phrase.note_columns = *note_columns;
                phrase.root_note = *root_note;
                song.upsert_phrase(PhraseId(*id), phrase)
                    .map_err(validation)?;
            }
            Self::RemovePhrase { id } => song.remove_phrase(PhraseId(*id)).map_err(validation)?,
            Self::RemoveGroove { id } => song.remove_groove(GrooveId(*id)).map_err(validation)?,
            Self::CreateMultisampleInstrument { name } => {
                song.create_multisample_instrument(name.clone())
                    .map_err(validation)?;
            }
            Self::UpsertMultisample { id, name } => {
                let zones = song
                    .multisamples
                    .get(&MultisampleId(*id))
                    .map(|map| map.zones.clone())
                    .unwrap_or_default();
                song.upsert_multisample(
                    MultisampleId(*id),
                    MultisampleInstrument {
                        name: name.clone(),
                        zones,
                    },
                )
                .map_err(validation)?;
            }
            Self::SetKeyzones { map_id, zones } => {
                if zones.len() > 128 {
                    return Err(validation(
                        "A multisample map has at most128 key zones".into(),
                    ));
                }
                let mut mapped = BTreeMap::new();
                for zone in zones {
                    let value = KeyZone {
                        name: zone.name.clone(),
                        note_min: zone.note_min,
                        note_max: zone.note_max,
                        velocity_min: zone.velocity_min,
                        velocity_max: zone.velocity_max,
                        region: zone.region,
                        root_note: zone.root_note,
                    };
                    if mapped.insert(KeyZoneId(zone.id), value).is_some() {
                        return Err(validation(format!(
                            "Duplicate key-zone ID {}; no zones replaced",
                            zone.id
                        )));
                    }
                }
                song.set_keyzones(MultisampleId(*map_id), mapped)
                    .map_err(validation)?;
            }
            Self::RemoveMultisample { id } => song
                .remove_multisample(MultisampleId(*id))
                .map_err(validation)?,
            Self::AssignMultisample {
                instrument_id,
                map_id,
            } => song
                .assign_multisample(InstrumentId(*instrument_id), map_id.map(MultisampleId))
                .map_err(validation)?,
            Self::CreateDrumKitInstrument { name } => {
                song.create_drum_kit_instrument(name.clone())
                    .map_err(validation)?;
            }
            Self::UpsertDrumKit { id, name } => {
                let pads = song
                    .drum_kits
                    .get(&DrumKitId(*id))
                    .map(|kit| kit.pads.clone())
                    .unwrap_or_default();
                song.upsert_drum_kit(
                    DrumKitId(*id),
                    DrumKitInstrument {
                        name: name.clone(),
                        pads,
                    },
                )
                .map_err(validation)?;
            }
            Self::RemoveDrumKit { id } => {
                song.remove_drum_kit(DrumKitId(*id)).map_err(validation)?
            }
            Self::UpsertDrumPad {
                kit_id,
                id,
                slot,
                name,
                note,
                region_id,
                root_note,
                tune_semitones,
                gain,
                pan,
                mode,
                new_note,
                choke_group,
            } => {
                // Validate the JSON number before narrowing to the model's f32.
                // Otherwise a slightly out-of-range value could round into range.
                if !(0.0..=1.0).contains(gain) || !(-1.0..=1.0).contains(pan) {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Pad gain must be 0..1 and pan -1..1 before f32 conversion",
                    ));
                }
                let velocity_zones = song
                    .drum_kits
                    .get(&DrumKitId(*kit_id))
                    .and_then(|kit| kit.pads.get(&PadId(*id)))
                    .map(|pad| pad.velocity_zones.clone())
                    .unwrap_or_default();
                if region_id.is_some() && !velocity_zones.is_empty() {
                    return Err(validation(
                        "Use set_pad_sources to switch source mode; upsert_drum_pad cannot replace existing velocity zones with a single region".into(),
                    ));
                }
                song.upsert_drum_pad(
                    DrumKitId(*kit_id),
                    PadId(*id),
                    DrumPad {
                        slot: *slot,
                        name: name.clone(),
                        note: *note,
                        region: region_id.map(RegionId),
                        velocity_zones,
                        root_note: *root_note,
                        tune_semitones: *tune_semitones,
                        gain: *gain as f32,
                        pan: *pan as f32,
                        mode: *mode,
                        new_note: *new_note,
                        choke_group: *choke_group,
                    },
                )
                .map_err(validation)?;
            }
            Self::RemoveDrumPad { kit_id, id } => song
                .remove_drum_pad(DrumKitId(*kit_id), PadId(*id))
                .map_err(validation)?,
            Self::SetPadSources {
                kit_id,
                pad_id,
                region_id,
                velocity_zones,
            } => song
                .set_pad_sources(
                    DrumKitId(*kit_id),
                    PadId(*pad_id),
                    region_id.map(RegionId),
                    velocity_zones.clone(),
                )
                .map_err(validation)?,
            Self::AssignKit {
                instrument_id,
                kit_id,
            } => song
                .assign_kit(InstrumentId(*instrument_id), kit_id.map(DrumKitId))
                .map_err(validation)?,
            Self::SetPhraseCell {
                phrase_id,
                row,
                column,
                cell,
            } => {
                song.set_phrase_cell(PhraseId(*phrase_id), *row, *column, cell.clone())
                    .map_err(validation)?;
            }
            Self::ClearPhraseCell {
                phrase_id,
                row,
                column,
            } => {
                song.set_phrase_cell(PhraseId(*phrase_id), *row, *column, Cell::default())
                    .map_err(validation)?;
            }
            Self::UpsertMarkerSet {
                id,
                name,
                sample_id,
                source_bpm,
                markers,
            } => {
                song.upsert_marker_set(
                    MarkerSetId(*id),
                    MarkerSet {
                        name: name.clone(),
                        sample: SampleId(*sample_id),
                        source_bpm: *source_bpm,
                        markers: markers.clone(),
                        locked_frames: song
                            .marker_sets
                            .get(&MarkerSetId(*id))
                            .map(|set| set.locked_frames.clone())
                            .unwrap_or_default(),
                    },
                )
                .map_err(validation)?;
            }
            Self::RemoveMarkerSet { id } => song
                .remove_marker_set(MarkerSetId(*id))
                .map_err(validation)?,
            Self::SetMarkerLock {
                marker_set_id,
                frame,
                locked,
            } => song
                .set_marker_lock(MarkerSetId(*marker_set_id), *frame, *locked)
                .map_err(validation)?,
            Self::AddMarker {
                marker_set_id,
                frame,
            } => song
                .add_marker(MarkerSetId(*marker_set_id), *frame)
                .map_err(validation)?,
            Self::MoveMarker {
                marker_set_id,
                from_frame,
                to_frame,
            } => song
                .move_marker(MarkerSetId(*marker_set_id), *from_frame, *to_frame)
                .map_err(validation)?,
            Self::RemoveMarker {
                marker_set_id,
                frame,
            } => song
                .remove_marker(MarkerSetId(*marker_set_id), *frame)
                .map_err(validation)?,
            Self::RemoveSample { id } => song.remove_sample(SampleId(*id)).map_err(validation)?,
            Self::UpsertRegion {
                id,
                name,
                sample_id,
                start_frame,
                end_frame,
                root_note,
            } => {
                let existing = song.regions.get(&RegionId(*id));
                song.upsert_region(
                    RegionId(*id),
                    SampleRegion {
                        name: name.clone(),
                        sample: SampleId(*sample_id),
                        start_frame: *start_frame,
                        end_frame: *end_frame,
                        root_note: *root_note,
                        amp_envelope: existing.and_then(|region| region.amp_envelope),
                        forward_loop: existing.and_then(|region| region.forward_loop),
                    },
                )
                .map_err(validation)?;
            }
            Self::SetRegionPlayback {
                region_id,
                amp_envelope,
                forward_loop,
            } => song
                .set_region_playback(RegionId(*region_id), *amp_envelope, *forward_loop)
                .map_err(validation)?,
            Self::SetRegionRole { region_id, label } => song
                .set_region_role(RegionId(*region_id), label.clone())
                .map_err(validation)?,
            Self::ClearRegionFeatures { region_id } => song
                .clear_region_features(RegionId(*region_id))
                .map_err(validation)?,
            Self::RemoveRegion { id } => song.remove_region(RegionId(*id)).map_err(validation)?,
            Self::AssignRegion {
                instrument_id,
                region_id,
            } => {
                song.assign_region(InstrumentId(*instrument_id), region_id.map(RegionId))
                    .map_err(validation)?;
            }
            Self::SetPhraseTrigger {
                pattern_id,
                track_id,
                row,
                column,
                trigger,
            } => {
                let mut cell = song
                    .patterns
                    .get(&PatternId(*pattern_id))
                    .and_then(|p| p.tracks.get(&TrackId(*track_id)))
                    .and_then(|r| r.get(row))
                    .and_then(|c| c.get(column))
                    .cloned()
                    .unwrap_or_default();
                cell.phrase = trigger.clone();
                set_cell(song, *pattern_id, *track_id, *row, *column, cell)?;
            }
        }
        song.validate().map_err(validation)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewProject {
    expected_revision: u64,
    #[serde(default)]
    discard_unsaved: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyZoneInput {
    pub id: u64,
    pub name: String,
    pub note_min: u8,
    pub note_max: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    pub region: RegionId,
    pub root_note: u8,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectMultisample {
    expected_revision: u64,
    map_id: u64,
    #[serde(default)]
    zones_start: usize,
    #[serde(default = "default_source_inspection_page")]
    zones_limit: usize,
    #[serde(default)]
    instruments_start: usize,
    #[serde(default = "default_source_inspection_page")]
    instruments_limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenProject {
    expected_revision: u64,
    path: String,
    #[serde(default)]
    discard_unsaved: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveProject {
    expected_revision: u64,
    path: Option<String>,
    overwrite_token: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathRequest {
    expected_revision: u64,
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadProject {
    section: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    expected_revision: u64,
    operations: Vec<EditOperation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportSample {
    expected_revision: u64,
    sample_id: u64,
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleRequest {
    expected_revision: u64,
    sample_id: u64,
    #[serde(default)]
    regions_start: usize,
    #[serde(default = "default_source_inspection_page")]
    regions_limit: usize,
    #[serde(default)]
    mappings_start: usize,
    #[serde(default = "default_source_inspection_page")]
    mappings_limit: usize,
    #[serde(default)]
    keyzones_start: usize,
    #[serde(default = "default_source_inspection_page")]
    keyzones_limit: usize,
}
fn default_source_inspection_page() -> usize {
    64
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalysisOptions {
    sensitivity: Option<f32>,
    min_spacing_ms: Option<f32>,
    minimum_rms: Option<f32>,
    max_suggestions: Option<usize>,
    waveform_bins: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeSample {
    expected_revision: u64,
    sample_id: u64,
    #[serde(default)]
    config: AnalysisOptions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupMarkers {
    expected_revision: u64,
    sample_id: u64,
    candidates: Vec<u64>,
    minimum_slice_frames: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileRequest {
    expected_revision: u64,
    config: crate::break_compiler::CompileConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewMarkerBank {
    expected_revision: u64,
    config: crate::marker_bank::MarkerBankConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyCellBlock {
    expected_revision: u64,
    scope: crate::cell_blocks::RectScope,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeSamplePeaks {
    expected_revision: u64,
    config: crate::meters::MeterAnalysisConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectSamplePeaks {
    expected_revision: u64,
    analysis_token: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_sample_peak_page")]
    limit: usize,
}
fn default_sample_peak_page() -> usize {
    MAX_SAMPLE_PEAK_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewTrackMix {
    expected_revision: u64,
    config: crate::track_mix::TrackMixConfig,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CellBlockOperation {
    PasteReplace {
        clipboard_token: String,
        destination: crate::cell_blocks::PasteDestination,
    },
    Clear {
        scope: crate::cell_blocks::RectScope,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewCellBlock {
    expected_revision: u64,
    operation: CellBlockOperation,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InspectCellBlock {
    Clipboard {
        expected_revision: u64,
        clipboard_token: String,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_cell_block_page")]
        limit: usize,
    },
    Changes {
        expected_revision: u64,
        preview_token: String,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_cell_block_page")]
        limit: usize,
    },
    Contexts {
        expected_revision: u64,
        preview_token: String,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_cell_block_page")]
        limit: usize,
    },
    Aliases {
        expected_revision: u64,
        preview_token: String,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_cell_block_page")]
        limit: usize,
    },
}
fn default_cell_block_page() -> usize {
    crate::cell_blocks::MAX_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitCompile {
    expected_revision: u64,
    preview_token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationNoteOptions {
    note: u8,
    velocity: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
enum GenerationPresetOptions {
    Euclidean {
        steps: u16,
        pulses: u16,
        rotation: u16,
        probability: f64,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationOptions {
    version: u32,
    name: String,
    bars: u16,
    instrument: InstrumentId,
    seed: u64,
    preset: GenerationPresetOptions,
    sequence: Vec<GenerationNoteOptions>,
}
impl GenerationOptions {
    fn effective(self) -> Outcome<crate::generation::GenerationConfig> {
        let GenerationPresetOptions::Euclidean {
            steps,
            pulses,
            rotation,
            probability,
        } = self.preset;
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return Err(Failure::new(
                "invalid_arguments",
                "Euclidean probability must be finite in 0..1 before f32 conversion",
            ));
        }
        if self.sequence.is_empty() || self.sequence.len() > 16 {
            return Err(Failure::new(
                "invalid_arguments",
                "Generation sequence must contain 1..16 explicit note/velocity entries",
            ));
        }
        let sequence = self
            .sequence
            .into_iter()
            .map(|entry| {
                if !entry.velocity.is_finite() || !(0.0..=1.0).contains(&entry.velocity) {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Every sequence velocity must be finite in 0..1 before f32 conversion",
                    ));
                }
                Ok(crate::generation::GenerationNote {
                    note: entry.note,
                    velocity: entry.velocity as f32,
                })
            })
            .collect::<Outcome<Vec<_>>>()?;
        Ok(crate::generation::GenerationConfig {
            version: self.version,
            name: self.name,
            bars: self.bars,
            instrument: self.instrument,
            seed: self.seed,
            preset: crate::generation::GenerationPreset::Euclidean {
                steps,
                pulses,
                rotation,
                probability: probability as f32,
            },
            sequence,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewThin {
    expected_revision: u64,
    config: crate::mutations::ThinConfig,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ThinPageKind {
    Decisions,
    Uses,
    Contexts,
    Changes,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectThin {
    expected_revision: u64,
    preview_token: String,
    kind: ThinPageKind,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_thin_page")]
    limit: usize,
}
fn default_thin_page() -> usize {
    MAX_THIN_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitThin {
    expected_revision: u64,
    preview_token: String,
}
struct PendingThin {
    token: String,
    revision: u64,
    editor_revision: u64,
    preview: crate::mutations::ThinPreview,
}
fn bounded_thin(data: Value) -> Outcome<Value> {
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new("thin_response_limit","THIN response exceeds256KiB; reduce page limit. No document mutation or pending-review replacement; never silently truncate"));
    }
    Ok(data)
}
fn thin_header(pending: &PendingThin) -> Outcome<Value> {
    bounded_thin(
        json!({"preview_token":pending.token,"preview_id":pending.preview.id(),"effective_config":pending.preview.config(),"summary":pending.preview.summary(),"record":pending.preview.record(),"counts":{"decisions":pending.preview.decisions().len(),"uses":pending.preview.uses().len(),"contexts":pending.preview.contexts().len(),"changes":pending.preview.changes().len()},"source_identity_verified":false,"document_changed":false,
        "notice":"Whole ordinary-cell deletion only, using real seeded pattrns gate once per eligible stored cell, never live or per-occurrence randomness. Current reviewed human roles and ALL resolved uses determine protection; Unknown/Mixed/stale/unreviewed remain retained. Shared full-model evaluability plus retained-ON and owned-control proof required even0%. This is model/source metadata authority, NOT current source-byte verification or unchanged full audio/lifetimes. MCP retains pathname authorization/existence checks. Inspect exact decisions/uses/contexts/changes before one-Undo commit; no inferred anchors, automatic roles or broad musical-quality guarantee."}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewGeneration {
    expected_revision: u64,
    config: GenerationOptions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewGrooveCapture {
    expected_revision: u64,
    groove_id: u64,
    config: crate::groove::CaptureConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewGrooveApply {
    expected_revision: u64,
    config: crate::groove::ApplyConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectGrooveTemplate {
    expected_revision: u64,
    groove_id: u64,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_groove_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectGroovePreview {
    expected_revision: u64,
    preview_token: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_groove_limit")]
    limit: usize,
}
fn default_groove_limit() -> usize {
    MAX_GROOVE_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectGeneration {
    expected_revision: u64,
    preview_token: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_generation_limit")]
    limit: usize,
}
fn default_generation_limit() -> usize {
    MAX_GENERATION_PAGE
}
/// Keep the wire scalar at f64 until bounds are checked: narrowing must not
/// round an out-of-range client value into the shared f32 parameter range.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformOptions {
    scope: crate::transforms::TransformScope,
    fields: crate::transforms::TransformFields,
    transpose_semitones: i8,
    velocity_scale: f64,
    nudge_frames: i32,
    #[serde(deserialize_with = "required_transform_swing")]
    swing: Option<crate::transforms::TransformSwing>,
}
fn required_transform_swing<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::transforms::TransformSwing>, D::Error> {
    Option::deserialize(deserializer)
}
impl TransformOptions {
    fn effective(self) -> Outcome<crate::transforms::TransformConfig> {
        if !self.velocity_scale.is_finite() || !(0.0..=4.0).contains(&self.velocity_scale) {
            return Err(Failure::new(
                "invalid_arguments",
                "velocity_scale must be finite and between 0 and 4 before f32 conversion",
            ));
        }
        Ok(crate::transforms::TransformConfig {
            scope: self.scope,
            fields: self.fields,
            transpose_semitones: self.transpose_semitones,
            velocity_scale: self.velocity_scale as f32,
            nudge_frames: self.nudge_frames,
            swing: self.swing,
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TempoEvidence {
    Markers {
        marker_set_id: u64,
        start_index: u16,
        end_index: u16,
        include_start: bool,
    },
    CachedOnsets {
        cache_token: String,
        start_frame: u64,
        end_frame: u64,
        basis: crate::tempo::PointBasis,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeTempo {
    expected_revision: u64,
    evidence: TempoEvidence,
    config: crate::tempo::TempoConfig,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InspectTempo {
    Points {
        expected_revision: u64,
        analysis_token: String,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_tempo_page")]
        limit: usize,
    },
    Pulses {
        expected_revision: u64,
        analysis_token: String,
        candidate_index: usize,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_tempo_page")]
        limit: usize,
    },
}
fn default_tempo_page() -> usize {
    MAX_TEMPO_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewTempoUse {
    expected_revision: u64,
    analysis_token: String,
    target_marker_set_id: u64,
    candidate_index: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitTempoUse {
    expected_revision: u64,
    preview_token: String,
}
struct PendingTempoUse {
    token: String,
    preview: crate::tempo::UseBpmPreview,
}
struct PendingTempo {
    token: String,
    revision: u64,
    editor_revision: u64,
    // No extra PCM/cache copies: only the current server-owned cache epoch.
    cache_token: Option<String>,
    preview: crate::tempo::TempoPreview,
    use_preview: Option<PendingTempoUse>,
}
fn tempo_header(pending: &PendingTempo) -> Outcome<Value> {
    let data = json!({"analysis_token":pending.token,"analysis_id":pending.preview.id(),"effective_config":pending.preview.config(),"analysis":pending.preview.analysis(),"point_count":pending.preview.points().len(),"cache_token":pending.cache_token,"source_identity_verified_now":pending.cache_token.is_some(),"document_changed":false,
        "notice":"Deterministic timing hypotheses, not calibrated probabilities or known source BPM. Review alternative/half-double pulse explanations. Phase is advisory only, never downbeat/meter/role/loop-length authority. Marker evidence uses stored unverified source metadata; cached evidence reauthorizes and hashes the current source, then shared trusted-cache repick performs no new FFT/decode. Use BPM requires an explicit candidate and same-source marker target; no automatic write."});
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new(
            "tempo_response_limit",
            "Tempo summary exceeds256KiB; previous analysis and document unchanged",
        ));
    }
    Ok(data)
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FeatureSource {
    VerifiedSource {},
    CachedOnsets { cache_token: String },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeFeatures {
    expected_revision: u64,
    request: crate::slice_features::FeatureRequest,
    source: FeatureSource,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InspectFeatures {
    Pending {
        expected_revision: u64,
        analysis_token: String,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_feature_page")]
        limit: usize,
    },
    Stored {
        expected_revision: u64,
        sample_id: u64,
        #[serde(default)]
        start: usize,
        #[serde(default = "default_feature_page")]
        limit: usize,
    },
}
fn default_feature_page() -> usize {
    MAX_FEATURE_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewCatalog {
    expected_revision: u64,
    analysis_token: String,
    roles: Vec<crate::slice_features::RoleChoice>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitCatalog {
    expected_revision: u64,
    preview_token: String,
}
struct PendingCatalog {
    token: String,
    preview: crate::slice_features::CatalogPreview,
}
struct PendingFeatures {
    token: String,
    revision: u64,
    editor_revision: u64,
    route: &'static str,
    // Compact descriptors own no PCM and no later onset-picker epoch dependency.
    preview: crate::slice_features::FeaturePreview,
    catalog: Option<PendingCatalog>,
}
fn bounded_features(data: Value) -> Outcome<Value> {
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new("feature_response_limit", "Slice feature response exceeds256KiB; reduce page limit or selection. No mutation or review replacement performed; never silently truncate"));
    }
    Ok(data)
}
fn feature_header(pending: &PendingFeatures, verified_now: bool) -> Outcome<Value> {
    bounded_features(
        json!({"analysis_token":pending.token,"analysis_id":pending.preview.id(),"request":pending.preview.request(),"summary":pending.preview.summary(),"source_route":pending.route,"record_count":pending.preview.records().len(),"source_identity_verified_now":verified_now,"metrics_recomputed_now":verified_now,"document_changed":false,
        "notice":"Native source/bounds measurements, not playback-processed audio. Suggestions are handcrafted rule support, NOT probabilities or reviewed roles. Cached PCM still requires NEW feature FFTs; completed measurements no longer depend on onset picker/cache epochs. Store requires explicit review plus fresh source authorization/hash. Historical metadata is not signed attestation."}),
    )
}
/// Metadata-only audition requests. A region draft never enters the document;
/// a pad always resolves its committed mapping through the shared evaluator.
#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AuditionTarget {
    RegionDraft {
        sample_id: u64,
        start_frame: u64,
        end_frame: u64,
        root_note: u8,
        #[serde(deserialize_with = "required_amp_envelope")]
        amp_envelope: Option<AmpEnvelope>,
        #[serde(deserialize_with = "required_forward_loop")]
        forward_loop: Option<ForwardLoop>,
    },
    Pad {
        instrument_id: u64,
        pad_id: u64,
    },
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrepareAudition {
    expected_revision: u64,
    target: AuditionTarget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResampleOptions {
    version: u32,
    occurrence: u32,
    name: String,
    root_note: u8,
    tail: crate::resample::TailPolicy,
    output: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareResample {
    expected_revision: u64,
    config: ResampleOptions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResampleRequest {
    expected_revision: u64,
    workflow_token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectResample {
    expected_revision: u64,
    workflow_token: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_resample_page")]
    limit: usize,
}
fn default_resample_page() -> usize {
    MAX_RESAMPLE_PAGE
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewTransform {
    expected_revision: u64,
    config: TransformOptions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectTransform {
    expected_revision: u64,
    preview_token: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_transform_limit")]
    limit: usize,
}
fn default_transform_limit() -> usize {
    MAX_TRANSFORM_PAGE
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnsetRefinement {
    enabled: Option<bool>,
    backtrack_ms: Option<f64>,
    lookahead_ms: Option<f64>,
    attack_fraction: Option<f64>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnsetPicker {
    enable_phase: Option<bool>,
    threshold: Option<f64>,
    mad_multiplier: Option<f64>,
    median_radius_ms: Option<f64>,
    min_gap_ms: Option<f64>,
    dense_gap_ms: Option<f64>,
    dense_evidence: Option<f64>,
    refinement: Option<OnsetRefinement>,
}
impl OnsetPicker {
    fn apply(&self, config: &mut crate::onset_v2::Config) {
        if let Some(v) = self.enable_phase {
            config.enable_phase = v;
        }
        if let Some(v) = self.threshold {
            config.threshold = v;
        }
        if let Some(v) = self.mad_multiplier {
            config.mad_multiplier = v;
        }
        if let Some(v) = self.median_radius_ms {
            config.median_radius_ms = v;
        }
        if let Some(v) = self.min_gap_ms {
            config.min_gap_ms = v;
        }
        if let Some(v) = self.dense_gap_ms {
            config.dense_gap_ms = v;
        }
        if let Some(v) = self.dense_evidence {
            config.dense_evidence = v;
        }
        if let Some(refinement) = &self.refinement {
            if let Some(v) = refinement.enabled {
                config.refinement.enabled = v;
            }
            if let Some(v) = refinement.backtrack_ms {
                config.refinement.backtrack_ms = v;
            }
            if let Some(v) = refinement.lookahead_ms {
                config.refinement.lookahead_ms = v;
            }
            if let Some(v) = refinement.attack_fraction {
                config.refinement.attack_fraction = v;
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareOnset {
    expected_revision: u64,
    sample_id: u64,
    engine: String,
    preset: crate::onset_v2::Preset,
    #[serde(default)]
    picker: OnsetPicker,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepickOnset {
    expected_revision: u64,
    cache_token: String,
    preset: Option<crate::onset_v2::Preset>,
    picker: OnsetPicker,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnsetPage {
    expected_revision: u64,
    cache_token: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_onset_page")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OnsetResultKind {
    Candidates,
    Suggestions,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnsetResultsPage {
    expected_revision: u64,
    cache_token: String,
    kind: OnsetResultKind,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_onset_result_page")]
    limit: usize,
}
fn default_onset_result_page() -> usize {
    MAX_ONSET_RESULT_PAGE
}
fn default_onset_page() -> usize {
    128
}
struct PendingOnset {
    token: String,
    revision: u64,
    sample_id: SampleId,
    asset: SampleAsset,
    cache: Arc<crate::onset_service::CachedSource>,
    config: crate::onset_v2::Config,
    config_sha256: String,
    result: crate::onset_service::SourceAnalysis,
}
enum ResampleWorkflow {
    Prepared(Box<crate::resample::ResamplePlan>),
    Printed(Box<crate::resample::ResamplePreview>),
}
impl ResampleWorkflow {
    fn phase(&self) -> &'static str {
        match self {
            Self::Prepared(_) => "prepared",
            Self::Printed(_) => "printed",
        }
    }
    fn base(&self) -> &Song {
        match self {
            Self::Prepared(p) => p.base(),
            Self::Printed(p) => p.base(),
        }
    }
    fn id(&self) -> &str {
        match self {
            Self::Prepared(p) => p.id(),
            Self::Printed(p) => p.id(),
        }
    }
    fn config(&self) -> &crate::resample::ResampleConfig {
        match self {
            Self::Prepared(p) => p.config(),
            Self::Printed(p) => p.config(),
        }
    }
    fn summary(&self) -> &crate::resample::ResampleSummary {
        match self {
            Self::Prepared(p) => p.summary(),
            Self::Printed(p) => p.summary(),
        }
    }
}
struct PendingResample {
    token: String,
    revision: u64,
    editor_revision: u64,
    workflow: ResampleWorkflow,
}
fn authorize_resample(
    workspace: &Workspace,
    access: crate::resample::Access<'_>,
) -> Result<(), String> {
    let checked = match access {
        crate::resample::Access::ReadSource(path) => workspace.check(path, true),
        crate::resample::Access::PublishOutput(path) => workspace.check(path, false),
    };
    checked
        .map(|_| ())
        .map_err(|e| format!("{}: {}", e.code, e.message))
}
fn resample_header(
    pending: &PendingResample,
    workspace: &Workspace,
    current: bool,
) -> Outcome<Value> {
    let (asset, receipt) = match &pending.workflow {
        ResampleWorkflow::Prepared(_) => (Value::Null, Value::Null),
        ResampleWorkflow::Printed(p) => {
            let mut receipt = serde_json::to_value(p.receipt()).map_err(io_failure)?;
            receipt.as_object_mut().unwrap().remove("source_identities");
            (
                serde_json::to_value(p.asset()).map_err(io_failure)?,
                receipt,
            )
        }
    };
    let data = json!({"workflow_token":pending.token,"phase":pending.workflow.phase(),"resample_id":pending.workflow.id(),"effective_config":pending.workflow.config(),"summary":pending.workflow.summary(),"output_path":workspace.display(&pending.workflow.config().output),"source_count":pending.workflow.summary().source_samples.len(),"asset":asset,"receipt":receipt,"document_changed":false,"review_current":current,"registration_current":current && matches!(pending.workflow,ResampleWorkflow::Printed(_)),"artifact_published":matches!(pending.workflow,ResampleWorkflow::Printed(_)),"artifact_identity_verified_now":false,"artifact_verified_before_publication":matches!(pending.workflow,ResampleWorkflow::Printed(_)),"source_identity_basis":if matches!(pending.workflow,ResampleWorkflow::Printed(_)){"verified_during_render"}else{"declared_metadata_only"},
        "notice":"One retained workflow. Prepared is pure metadata; Printed is a durable new WAV, not a document edit. Source identities in inspection are declared in prepared phase and verified during rendering in printed phase, not reverified by this read. Use register for fresh output/parent checks plus prospective all-source storage/identity preflight and one-Undo metadata registration. No deletion on supersession/stale/Undo. Printing bakes current effective track mix. Pre-master PCM×2 is fixed compensation, not normalization; only neutral effective track mix plus native-root/unity/center/no-options replay held through the WHOLE region without early OFF/replacement/SongEnd preserves printed Engine output. Stale printed receipts remain inspectable, never registerable."});
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new(
            "resample_response_limit",
            "Resample header exceeds256KiB; no source records are silently truncated",
        ));
    }
    Ok(data)
}
struct PendingGeneration {
    token: String,
    revision: u64,
    editor_revision: u64,
    preview: crate::generation::GenerationPreview,
}
enum GroovePlan {
    Capture(Box<crate::groove::CapturePreview>),
    Apply(Box<crate::groove::GroovePreview>),
}
impl GroovePlan {
    fn kind(&self) -> &'static str {
        match self {
            Self::Capture(_) => "capture",
            Self::Apply(_) => "apply",
        }
    }
    fn id(&self) -> &str {
        match self {
            Self::Capture(plan) => plan.id(),
            Self::Apply(plan) => plan.id(),
        }
    }
    fn base(&self) -> &Song {
        match self {
            Self::Capture(plan) => plan.base(),
            Self::Apply(plan) => plan.base(),
        }
    }
    fn candidate(&self) -> &Song {
        match self {
            Self::Capture(plan) => plan.candidate(),
            Self::Apply(plan) => plan.candidate(),
        }
    }
    fn config(&self) -> Outcome<Value> {
        match self {
            Self::Capture(plan) => serde_json::to_value(plan.config()),
            Self::Apply(plan) => serde_json::to_value(plan.config()),
        }
        .map_err(io_failure)
    }
    fn summary(&self) -> Outcome<Value> {
        match self {
            Self::Capture(plan) => serde_json::to_value(plan.summary()),
            Self::Apply(plan) => serde_json::to_value(plan.summary()),
        }
        .map_err(io_failure)
    }
    fn record_count(&self) -> usize {
        match self {
            Self::Capture(plan) => plan.template().anchors.len(),
            Self::Apply(plan) => plan.changes().len(),
        }
    }
}
struct PendingGroove {
    token: String,
    revision: u64,
    editor_revision: u64,
    plan: GroovePlan,
}
struct PendingTransform {
    token: String,
    revision: u64,
    editor_revision: u64,
    preview: crate::transforms::TransformPreview,
}
struct PendingCompile {
    token: String,
    revision: u64,
    editor_revision: u64,
    sample_id: SampleId,
    preview: crate::break_compiler::CompilePreview,
}
struct PendingMarkerBank {
    token: String,
    revision: u64,
    editor_revision: u64,
    preview: crate::marker_bank::MarkerBankPreview,
}
struct PendingCellClipboard {
    token: String,
    copied_at_revision: u64,
    clipboard: crate::editor::DocumentClipboard,
}
struct PendingCellBlock {
    token: String,
    revision: u64,
    preview: crate::editor::DocumentCellPreview,
}
struct PendingSamplePeaks {
    token: String,
    revision: u64,
    editor_revision: u64,
    owner: crate::editor::DocumentOwner,
    plan: crate::meters::MeterAnalysisPlan,
    analysis: crate::meters::MeterAnalysis,
}
fn bounded_sample_peaks(data: Value) -> Outcome<Value> {
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new("sample_peaks_response_limit", "Sample-peak response exceeds 256 KiB; no report is replaced and no records are silently truncated"));
    }
    Ok(data)
}
fn sample_peak_page(pending: &PendingSamplePeaks, start: usize, limit: usize) -> Outcome<Value> {
    let reports = &pending.analysis.reports;
    if limit == 0 || limit > MAX_SAMPLE_PEAK_PAGE || start > reports.len() {
        return Err(Failure::new("invalid_arguments", "Sample-peak page limit must be1..4 and start must not exceed total selected windows; equal total returns an empty page"));
    }
    let end = start.saturating_add(limit).min(reports.len());
    Ok(
        json!({"start":start,"limit":limit,"total":reports.len(),"next_start":(end<reports.len()).then_some(end),"windows":&reports[start..end]}),
    )
}
fn sample_peak_header(pending: &PendingSamplePeaks) -> Outcome<Value> {
    bounded_sample_peaks(json!({
        "analysis_token":pending.token,"analysis_id":pending.analysis.id,"plan_id":pending.plan.id(),
        "analysis_revision":pending.revision,"analysis_editor_revision":pending.editor_revision,
        "config":pending.analysis.config,"summary":pending.analysis.summary,
        "verified_sources_at_analysis":pending.analysis.verified_sources,
        "page":sample_peak_page(pending,0,MAX_SAMPLE_PEAK_PAGE)?,
        "document_changed":false,"source_identity_verified_now":pending.analysis.verified_sources>0,"pcm_rendered":true,
        "pcm_returned":false,"files_written":false,"audio_device_started":false,"live_gui_observed":false,
        "scope":"offline_headless_song_sample_peaks",
        "notice":"Completed device-free offline measurement, not live GUI/device/readback or heard audio. Full compiled Song context advances from frame0 through end_frame; range selects complete2400-frame windows, with only an exact natural-end partial. Units are48kHz Song/Engine frames, never native asset frames. Track sample peaks are post-balance/post-fixed-master sums in original f32 order; master peaks use final PCM. No RMS/loudness/true-peak claim. track_peaks slots match summary.layout.tracks; null means unavailable, not silence or a partial track total. All compiled used sources were authorized and verified at Analyze, including sources first used after this prefix; unused source files were not opened or byte-verified; full model and bounded metadata validation still applies. No retained PCM, file output, history change or device action."
    }))
}
struct PendingTrackMix {
    token: String,
    revision: u64,
    preview: crate::editor::DocumentTrackMixPreview,
}
fn bounded_track_mix(data: Value) -> Outcome<Value> {
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new("track_mix_response_limit", "Track-balance response exceeds 256 KiB; no settings, review or document changed and no records are silently truncated"));
    }
    Ok(data)
}
fn track_mix_header(pending: &PendingTrackMix) -> Outcome<Value> {
    let preview = pending.preview.preview();
    bounded_track_mix(json!({
        "preview_token":pending.token,"preview_id":preview.id(),
        "requested_config":preview.config(),"stored_settings":preview.candidate().track_mix,"summary":preview.summary(),
        "document_changed":false,"source_identity_verified_now":false,
        "audio_rendered":false,"live_update_applied":false,
        "session_scope":"isolated_headless_document",
        "notice":"Pure complete-set track-balance review in this headless Song, not a live GUI/device command. Shared layout status distinguishes available, unavailable and unsupported routing; metadata validity is not source-byte, playback or live readiness. Commit is separately guarded by Editor owner, both revisions and exact full base. Saved settings affect later starts and offline export. No path/file/hash/decode, incoming-memory rewrite, audio rendering or live mailbox access."
    }))
}
fn bounded_cell_block(data: Value) -> Outcome<Value> {
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new("cell_block_response_limit", "Rectangle response exceeds 256 KiB; reduce the page or selection. Nothing is silently truncated and no clipboard, review or document changed"));
    }
    Ok(data)
}
fn cell_clipboard_header(pending: &PendingCellClipboard) -> Outcome<Value> {
    let snapshot = pending.clipboard.snapshot();
    bounded_cell_block(json!({
        "clipboard_token":pending.token,"clipboard_id":snapshot.id(),
        "copied_at_revision":pending.copied_at_revision,
        "copied_at_editor_revision":pending.clipboard.revision(),
        "source_document_sha256":snapshot.source_document_sha256(),
        "summary":snapshot.summary(),"document_changed":false,
        "source_identity_verified_now":false,"audio_rendered":false,
        "notice":"Server-owned frozen whole cells in the current Editor document owner, not a system clipboard or live selection. Row-major positions include holes and explicit stored default cells separately. Raw IDs resolve against current document definitions at Paste. The source-document digest is historical metadata, not source-audio verification. Clipboard survives ordinary edits, Save and Undo/Redo; successful New/Open revokes it. A new successful Copy replaces it and retires the rectangle preview."
    }))
}
fn cell_block_header(pending: &PendingCellBlock) -> Outcome<Value> {
    let preview = pending.preview.preview();
    bounded_cell_block(json!({
        "preview_token":pending.token,"preview_id":preview.id(),
        "request":preview.request(),"summary":preview.summary(),
        "record_counts":{"changes":preview.changes().len(),"contexts":preview.contexts().len(),"aliases":preview.aliases().len()},
        "document_changed":false,"source_identity_verified_now":false,"audio_rendered":false,
        "notice":"Shared whole-cell Paste replace/Clear plan, guarded by Editor document owner, both revisions and exact base. Inspect exact changes and affected stored-content aliases/occurrences before Commit. No path/file access, source hash, decode, incoming-memory repair or audio-equivalence promise. Commit is a separate Edit-Mode transaction; a true no-op preserves history, dirty state, revision and redo."
    }))
}
fn cell_block_records<T: Serialize>(
    records: &[T],
    start: usize,
    limit: usize,
) -> Outcome<(Value, usize, usize)> {
    if start > records.len() {
        return Err(Failure::new(
            "invalid_arguments",
            "Rectangle page start exceeds this record kind's total; an empty end page is allowed",
        ));
    }
    let end = start.saturating_add(limit).min(records.len());
    Ok((
        serde_json::to_value(&records[start..end]).map_err(io_failure)?,
        records.len(),
        end,
    ))
}
fn bounded_marker_bank(data: Value) -> Outcome<Value> {
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new(
            "marker_bank_response_limit",
            "Marker-bank response exceeds 256 KiB; no records are silently truncated and no review or document changed",
        ));
    }
    Ok(data)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectDrumSteps {
    expected_revision: u64,
    pattern_id: u64,
    track_id: u64,
    instrument_id: u64,
    division: crate::drum_steps::StepDivision,
    steps: u8,
    #[serde(default)]
    page: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectMatrix {
    expected_revision: u64,
    #[serde(default)]
    occurrence_start: usize,
    #[serde(default = "default_matrix_limit")]
    occurrence_limit: usize,
    #[serde(default)]
    track_start: usize,
    #[serde(default = "default_matrix_limit")]
    track_limit: usize,
}
fn default_matrix_limit() -> usize {
    16
}
struct PendingDrumSteps {
    token: String,
    revision: u64,
    editor_revision: u64,
    projection: crate::drum_steps::StepProjection,
}
fn projected_step_event(
    projection: &crate::drum_steps::StepProjection,
    pad: PadId,
    row: u32,
    column: u16,
) -> Outcome<crate::drum_steps::StepEvent> {
    projection.lanes.iter().filter(|lane| lane.pad == pad)
        .flat_map(|lane| &lane.steps).flat_map(|slot| &slot.events)
        .find(|event| event.row == row && event.column == column)
        .cloned().ok_or_else(|| Failure::new("drum_step_event_unavailable", "Exact pad/source row/column is not owned by this inspected page; inspect again or use the tracker"))
}
fn apply_drum_step(
    song: &mut Song,
    pending: &PendingDrumSteps,
    operation: &EditOperation,
) -> Outcome<()> {
    use crate::drum_steps::{self, StepEdit};
    // The token/base is checked against the live document before each ordered
    // batch operation. Reproject the explicit candidate, never infer/quantize cells.
    let fresh = drum_steps::project(song, &pending.projection.view).map_err(validation)?;
    let event = |pad_id: u64, row: u32, column: u16| {
        // Only an event present in the reviewed page is addressable here. Later
        // field edits in the same batch use its updated exact candidate snapshot.
        projected_step_event(&pending.projection, PadId(pad_id), row, column)?;
        projected_step_event(&fresh, PadId(pad_id), row, column)
    };
    let edit = match operation {
        EditOperation::DrumStepToggle { pad_id, step, .. } => StepEdit::Toggle {
            pad: PadId(*pad_id),
            step: *step,
        },
        EditOperation::DrumStepSetVelocity {
            pad_id,
            row,
            column,
            value,
            ..
        } => {
            if value.is_some_and(|value| !(0.0..=1.0).contains(&value)) {
                return Err(Failure::new(
                    "invalid_arguments",
                    "Step velocity must be null or 0..1 before f32 conversion",
                ));
            }
            StepEdit::SetVelocity {
                event: event(*pad_id, *row, *column)?,
                value: value.map(|value| value as f32),
            }
        }
        EditOperation::DrumStepSetDelay {
            pad_id,
            row,
            column,
            value,
            ..
        } => StepEdit::SetDelay {
            event: event(*pad_id, *row, *column)?,
            value: *value,
        },
        EditOperation::DrumStepSetRetrigger {
            pad_id,
            row,
            column,
            value,
            ..
        } => StepEdit::SetRetrigger {
            event: event(*pad_id, *row, *column)?,
            value: value.clone(),
        },
        _ => unreachable!("only typed step operations reach the shared step adapter"),
    };
    *song = drum_steps::apply(song, &fresh, &edit).map_err(validation)?;
    Ok(())
}
struct Overwrite {
    token: String,
    revision: u64,
    path: PathBuf,
    bytes: Vec<u8>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    New,
    Initializing,
    Ready,
}

/// One isolated headless document. No existing GUI process or audio device is controlled.
pub struct Server {
    workspace: Workspace,
    editor: Editor,
    session: Session,
    revision: u64,
    bound_editor_revision: Option<u64>,
    bound_document_owner: Option<DocumentOwner>,
    live_document_context: bool,
    phase: Phase,
    overwrite: Option<Overwrite>,
    token_serial: u64,
    compile_preview: Option<PendingCompile>,
    marker_bank: Option<PendingMarkerBank>,
    cell_clipboard: Option<PendingCellClipboard>,
    cell_block: Option<PendingCellBlock>,
    track_mix: Option<PendingTrackMix>,
    sample_peaks: Option<PendingSamplePeaks>,
    transform_preview: Option<PendingTransform>,
    generation_preview: Option<PendingGeneration>,
    groove_preview: Option<PendingGroove>,
    resample: Option<PendingResample>,
    tempo: Option<PendingTempo>,
    features: Option<PendingFeatures>,
    thin: Option<PendingThin>,
    onset_cache: Option<PendingOnset>,
    drum_steps: Option<PendingDrumSteps>,
}
impl Server {
    pub fn new(root: &Path) -> Result<Self, String> {
        let workspace = Workspace::new(root)?;
        let mut editor = Editor::default();
        let mut session = Session::default();
        session.request(&mut editor, Action::New)?;
        editor.edit_mode = true;
        Ok(Self {
            workspace,
            editor,
            session,
            revision: 0,
            bound_editor_revision: None,
            bound_document_owner: None,
            live_document_context: false,
            phase: Phase::New,
            overwrite: None,
            token_serial: 0,
            compile_preview: None,
            marker_bank: None,
            cell_clipboard: None,
            cell_block: None,
            track_mix: None,
            sample_peaks: None,
            transform_preview: None,
            generation_preview: None,
            groove_preview: None,
            resample: None,
            tempo: None,
            features: None,
            thin: None,
            onset_cache: None,
            drum_steps: None,
        })
    }
    fn expect_revision(&self, expected: u64) -> Outcome<()> {
        if expected != self.revision {
            let mut error = Failure::new(
                "revision_conflict",
                "Read current state and retry against its revision; no changes applied",
            );
            error.details = json!({"expected_revision":expected,"actual_revision":self.revision});
            Err(error)
        } else {
            Ok(())
        }
    }
    fn advance(&mut self) {
        self.revision += 1;
        self.overwrite = None;
        self.compile_preview = None;
        self.marker_bank = None;
        self.cell_block = None;
        self.track_mix = None;
        self.sample_peaks = None;
        if self.cell_clipboard.as_ref().is_some_and(|pending| {
            !self
                .editor
                .cell_clipboard_is_current_document(&pending.clipboard)
        }) {
            self.cell_clipboard = None;
        }
        self.transform_preview = None;
        self.generation_preview = None;
        self.groove_preview = None;
        self.tempo = None;
        self.features = None;
        self.thin = None;
        if matches!(
            self.resample.as_ref().map(|p| &p.workflow),
            Some(ResampleWorkflow::Prepared(_))
        ) {
            self.resample = None;
        }
        self.onset_cache = None;
        self.drum_steps = None;
    }
    fn checked_sample_peaks(&self, token: &str) -> Outcome<&PendingSamplePeaks> {
        let pending = self.sample_peaks.as_ref().ok_or_else(|| {
            Failure::new(
                "sample_peaks_unavailable",
                "Analyze a bounded current Song interval before inspecting sample peaks",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new("invalid_sample_peaks_token", "Token is not the current completed sample-peak analysis; previous report is unchanged"));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || !self.editor.owns_document(&pending.owner)
            || !project::same_serialized_song(pending.plan.song(), &self.editor.song)
                .map_err(validation)?
        {
            return Err(Failure::new("sample_peaks_stale", "Analysis document owner, revision or exact Song including current track mix changed; analyze again"));
        }
        Ok(pending)
    }
    fn checked_track_mix(&self, token: &str) -> Outcome<&PendingTrackMix> {
        let pending = self.track_mix.as_ref().ok_or_else(|| {
            Failure::new(
                "track_mix_preview_unavailable",
                "Prepare a current track-balance review before Commit",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_track_mix_preview_token",
                "Token is not the current track-balance review; document and review unchanged",
            ));
        }
        if pending.revision != self.revision {
            return Err(Failure::new(
                "track_mix_preview_stale",
                "Track-balance review revision is stale; prepare again",
            ));
        }
        self.editor
            .validate_track_mix_preview(&pending.preview)
            .map_err(|message| Failure::new("track_mix_preview_stale", message))?;
        Ok(pending)
    }
    fn checked_cell_clipboard(&self, token: &str) -> Outcome<&PendingCellClipboard> {
        let pending = self.cell_clipboard.as_ref().ok_or_else(|| {
            Failure::new(
                "cell_block_clipboard_unavailable",
                "Copy an explicit rectangle in the current document first",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_cell_block_clipboard_token",
                "Token is not the current server-owned clipboard; clipboard and document unchanged",
            ));
        }
        if !self
            .editor
            .cell_clipboard_is_current_document(&pending.clipboard)
        {
            return Err(Failure::new(
                "cell_block_clipboard_owner_changed",
                "Clipboard belongs to a replaced Editor document; copy again after New/Open",
            ));
        }
        Ok(pending)
    }
    fn checked_cell_block(&self, token: &str) -> Outcome<&PendingCellBlock> {
        let pending = self.cell_block.as_ref().ok_or_else(|| {
            Failure::new(
                "cell_block_preview_unavailable",
                "Prepare a rectangle Paste replace or Clear preview first",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new("invalid_cell_block_preview_token", "Token is not the current rectangle review; clipboard, review and document unchanged"));
        }
        if pending.revision != self.revision {
            return Err(Failure::new(
                "cell_block_preview_stale",
                "MCP revision changed; prepare the rectangle again",
            ));
        }
        self.editor
            .validate_cell_preview(&pending.preview)
            .map_err(|message| Failure::new("cell_block_preview_stale", message))?;
        Ok(pending)
    }
    fn checked_marker_bank(&self, token: &str) -> Outcome<&PendingMarkerBank> {
        let pending = self.marker_bank.as_ref().ok_or_else(|| {
            Failure::new(
                "marker_bank_preview_unavailable",
                "Create and review a marker-bank preview at the current revision",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_marker_bank_token",
                "Token is not the current marker-bank review; document and current review unchanged",
            ));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || !pending
                .preview
                .is_current(&self.editor.song)
                .map_err(validation)?
        {
            return Err(Failure::new(
                "marker_bank_preview_stale",
                "Exact base Song or revision changed; create a fresh marker-bank preview",
            ));
        }
        Ok(pending)
    }
    fn checked_thin(&self, token: &str) -> Outcome<&PendingThin> {
        let pending = self.thin.as_ref().ok_or_else(|| {
            Failure::new(
                "thin_unavailable",
                "Create and review an explicit seeded THIN preview first",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_thin_token",
                "Token is not the current THIN review; current review and document unchanged",
            ));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || pending.preview.base() != &self.editor.song
        {
            return Err(Failure::new(
                "thin_stale",
                "Full base Song or revision changed; create a fresh THIN preview",
            ));
        }
        self.workspace.assets(&self.editor.song)?;
        Ok(pending)
    }
    fn checked_features(&self, token: &str, verify_source: bool) -> Outcome<&PendingFeatures> {
        let pending = self.features.as_ref().ok_or_else(|| {
            Failure::new(
                "features_unavailable",
                "Analyze explicit current source regions first",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_feature_token",
                "Token is not the retained feature analysis",
            ));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || pending.preview.base() != &self.editor.song
        {
            return Err(Failure::new(
                "features_stale",
                "Full base Song or revision changed; analyze again",
            ));
        }
        if verify_source {
            self.workspace.assets(&self.editor.song)?;
            let path = self.workspace.check(&pending.preview.asset().path, true)?;
            crate::sample_asset::verify_identity(pending.preview.asset(), &path)
                .map_err(validation)?;
        }
        Ok(pending)
    }
    fn checked_tempo(&self, token: &str) -> Outcome<&PendingTempo> {
        let pending = self.tempo.as_ref().ok_or_else(|| {
            Failure::new(
                "tempo_unavailable",
                "Analyze current accepted markers or a current server-owned onset cache first",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_tempo_token",
                "Token is not the retained tempo analysis",
            ));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || pending.preview.base() != &self.editor.song
        {
            return Err(Failure::new(
                "tempo_stale",
                "Full base Song or revision changed; analyze again",
            ));
        }
        self.workspace.assets(&self.editor.song)?;
        if let Some(cache_token) = &pending.cache_token {
            // This also reauthorizes the path and freshly verifies source bytes.
            // A repick/extraction epoch can change without any Song revision.
            self.checked_onset(cache_token)?;
        }
        Ok(pending)
    }
    fn resample_current(&self, pending: &PendingResample) -> bool {
        pending.revision == self.revision
            && pending.editor_revision == self.editor.revision()
            && pending.workflow.base() == &self.editor.song
    }
    fn resample_metadata(&self, token: &str) -> Outcome<&PendingResample> {
        let pending=self.resample.as_ref().ok_or_else(||Failure::new("resample_unavailable","Prepare a fresh resample workflow at the current revision; any published WAV remains on disk"))?;
        if pending.token != token {
            return Err(Failure::new("invalid_resample_token","Token is not the current workflow; document/current flow unchanged, published files retained"));
        }
        Ok(pending)
    }
    fn checked_resample(&self, token: &str) -> Outcome<&PendingResample> {
        let pending = self.resample_metadata(token)?;
        if !self.resample_current(pending) {
            return Err(Failure::new(
                "resample_stale",
                "Full base Song or revision changed; prepare again. Published WAV remains on disk",
            ));
        }
        Ok(pending)
    }
    fn checked_groove(&self, token: &str) -> Outcome<&PendingGroove> {
        let pending = self.groove_preview.as_ref().ok_or_else(|| {
            Failure::new(
                "groove_preview_unavailable",
                "Create and review a groove capture or apply preview at the current revision",
            )
        })?;
        if token != pending.token {
            return Err(Failure::new("invalid_groove_preview_token", "Token does not identify the current groove capture/apply plan; document and pending plan unchanged"));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || pending.plan.base() != &self.editor.song
        {
            return Err(Failure::new(
                "groove_preview_stale",
                "The reviewed full Song or revision changed; create a fresh groove plan",
            ));
        }
        Ok(pending)
    }
    fn publish_groove(&mut self, plan: GroovePlan) -> Outcome<Value> {
        if serde_json::to_vec(plan.candidate())
            .map_err(io_failure)?
            .len()
            > MAX_DOCUMENT
        {
            return Err(Failure::new(
                "document_too_large",
                "Groove candidate exceeds 8 MiB; previous plan and document unchanged",
            ));
        }
        self.workspace.assets(plan.candidate())?;
        let serial = self.token_serial.checked_add(1).ok_or_else(|| {
            Failure::new(
                "token_exhausted",
                "No fresh review tokens remain in this session",
            )
        })?;
        let token = format!("groove-{}-{serial}-revision-{}", plan.kind(), self.revision);
        let data = json!({"preview_token":token,"kind":plan.kind(),"groove_preview_id":plan.id(),"effective_config":plan.config()?,"summary":plan.summary()?,"record_count":plan.record_count(),"document_changed":false,"source_identity_verified":false,"notice":"Shared pure timing recipe/cell plan; no audio decode/hash or inference. Source SHA is historical declared identity. Inspect exact records before explicit commit. One active groove plan: a successful capture OR apply preview supersedes the previous groove token; failures preserve it. No client candidate accepted."});
        if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
            return Err(Failure::new(
                "preview_too_large",
                "Groove summary exceeds 256 KiB; previous plan and document unchanged",
            ));
        }
        self.token_serial = serial;
        self.groove_preview = Some(PendingGroove {
            token,
            revision: self.revision,
            editor_revision: self.editor.revision(),
            plan,
        });
        Ok(data)
    }
    fn checked_generation(&self, token: &str) -> Outcome<&PendingGeneration> {
        let pending = self.generation_preview.as_ref().ok_or_else(|| {
            Failure::new(
                "generation_preview_unavailable",
                "Create and review a seeded generation preview at the current revision",
            )
        })?;
        if token != pending.token {
            return Err(Failure::new("invalid_generation_preview_token", "Token does not identify the current generation plan; document and pending plan unchanged"));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || pending.preview.base() != &self.editor.song
        {
            return Err(Failure::new(
                "generation_preview_stale",
                "The reviewed full Song or revision changed; prepare a fresh generation plan",
            ));
        }
        Ok(pending)
    }
    fn checked_transform(&self, token: &str) -> Outcome<&PendingTransform> {
        let pending = self.transform_preview.as_ref().ok_or_else(|| {
            Failure::new(
                "transform_preview_unavailable",
                "Create and review a transform preview at the current revision",
            )
        })?;
        if token != pending.token {
            return Err(Failure::new("invalid_transform_preview_token", "Token does not identify the current transform preview; document and current preview are unchanged"));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || pending.preview.base() != &self.editor.song
        {
            return Err(Failure::new(
                "transform_preview_stale",
                "The reviewed exact Song or revision changed; create a fresh transform preview",
            ));
        }
        Ok(pending)
    }
    fn guard_discard(&self, allowed: bool) -> Outcome<()> {
        if self.editor.dirty && !allowed {
            Err(Failure::new(
                "unsaved_changes",
                "Save first, or explicitly pass discard_unsaved:true with the current revision",
            ))
        } else {
            Ok(())
        }
    }
    fn checked_onset(&self, token: &str) -> Outcome<&PendingOnset> {
        let cached = self.onset_cache.as_ref().ok_or_else(|| {
            Failure::new(
                "onset_cache_unavailable",
                "Prepare an explicit experimental v2 cache at the current revision",
            )
        })?;
        if cached.token != token || cached.revision != self.revision {
            return Err(Failure::new(
                "invalid_onset_cache_token",
                "Token does not match the current source/config-bound cache",
            ));
        }
        let current = self
            .editor
            .song
            .samples
            .get(&cached.sample_id)
            .ok_or_else(|| {
                Failure::new("onset_cache_stale", "Cached source is no longer registered")
            })?;
        if current != &cached.asset {
            return Err(Failure::new(
                "onset_cache_stale",
                "Registered source metadata differs from the immutable cached source",
            ));
        }
        let path = self.workspace.check(&current.path, true)?;
        crate::sample_asset::verify_identity(current, &path).map_err(validation)?;
        Ok(cached)
    }
    fn checked_drum_steps(&self, token: &str) -> Outcome<&PendingDrumSteps> {
        let pending = self.drum_steps.as_ref().ok_or_else(|| {
            Failure::new(
                "drum_step_projection_unavailable",
                "Inspect the drum-step view at the current revision first",
            )
        })?;
        if pending.token != token {
            return Err(Failure::new(
                "invalid_drum_step_token",
                "Token belongs to a different/superseded page or view; inspect again",
            ));
        }
        if pending.revision != self.revision
            || pending.editor_revision != self.editor.revision()
            || !pending.projection.is_current(&self.editor.song)
        {
            return Err(Failure::new(
                "drum_step_projection_stale",
                "Revision or exact projected Song changed; inspect again before editing",
            ));
        }
        Ok(pending)
    }
    fn state(&self) -> Value {
        json!({"scope":if self.live_document_context {"bound_live_gui_document"} else {"isolated_headless_document"},"path":self.session.path.as_ref().map(|p| self.workspace.display(p)),"dirty":self.editor.dirty,
            "can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo(),"title":self.editor.song.title,
            "track_count":self.editor.song.tracks.len(),"instrument_count":self.editor.song.instruments.len(),
            "pattern_count":self.editor.song.patterns.len(),"generated_pattern_count":self.editor.song.patterns.values().filter(|pattern|pattern.generation.is_some()).count(),"occurrence_count":self.editor.song.order.len(),
            "region_catalog_count":self.editor.song.region_catalog.len(),"sample_count":self.editor.song.samples.len(),"region_count":self.editor.song.regions.len(),"marker_set_count":self.editor.song.marker_sets.len(),
            "phrase_count":self.editor.song.phrases.len(),"groove_count":self.editor.song.grooves.len(),
            "multisample_count":self.editor.song.multisamples.len(),"keyzone_count":self.editor.song.multisamples.values().map(|map|map.zones.len()).sum::<usize>(),
            "track_alias_count":self.editor.song.patterns.values().map(|pattern|pattern.track_aliases.len()).sum::<usize>(),"occurrence_track_mute_count":self.editor.song.order.iter().map(|occurrence|occurrence.muted_tracks.len()).sum::<usize>(),
            "drum_kit_count":self.editor.song.drum_kits.len(),"drum_pad_count":self.editor.song.drum_kits.values().map(|kit|kit.pads.len()).sum::<usize>(),
            "phrase_trigger_cell_count":self.editor.song.patterns.values().flat_map(|pattern| pattern.tracks.values()).flat_map(|rows| rows.values()).flat_map(|columns| columns.values()).filter(|cell| cell.phrase.is_some()).count()})
    }
    fn capabilities(&self) -> Value {
        let sample_peaks = json!({"version":crate::meters::VERSION,"algorithm":crate::meters::ALGORITHM,"tools":["leviathan_sample_peaks_analyze","leviathan_sample_peaks_inspect"],"scope":"Device-free offline measurements of this isolated headless Song; no live GUI/device observation or control","range":"Explicit half-open48kHz Song/Engine frames. Start2400-aligned, end2400-aligned or exact natural musical+tail terminal, at most120000 processed frames. No rounding. Full compiled context advances from0; range only filters observations; only natural-end partial is retained","measurement":"Post-track-balance AND post-fixed-master sample peaks from existing Engine f32 contributions; master is actual final PCM. Track slot order follows shared layout; null is unavailable, never silence or a partial track total. Not RMS/loudness/true peak/heard audio","sources":"Shared pure used-route scheduling preflight; all full-compiled used sources must be registered region routes, including future notes past this prefix. Path-only legacy routes reject even if same pathname is registered elsewhere. Selected original absolute lexical paths authorized before canonicalization/hash/decode; Engine verifies source identities. Unused source files are not opened or byte-verified; full model and bounded metadata validation and normal Open/Save/export policies remain unchanged","retention":"One complete result plus shared bounded base plan, no PCM or token history. Exact Editor owner/both revisions/full serialized Song guard inspection. Successful analysis supersedes previous result; every failed replacement preserves it. Inspect is metadata-only and does not reverify current source bytes. Save/changed edits/Undo/Redo/New/Open invalidate; exact no-op preserves","execution":"Synchronous bounded analysis, no async cancellation or timing SLA; ordinary clipping/nonfinite/render failures return no incomplete result. No files or audio device opened for output","limits":{"processed_song_frames":crate::meters::MAX_ANALYSIS_FRAMES,"window_song_frames":crate::meters::WINDOW_FRAMES,"tracks_per_window":crate::meters::MAX_TRACKS,"windows_per_page":MAX_SAMPLE_PEAK_PAGE,"conservative_voice_frames":crate::meters::MAX_VOICE_FRAMES,"base_project_bytes":MAX_DOCUMENT,"completed_result_bytes":crate::meters::MAX_REPORT_BYTES,"structured_payload_bytes":MAX_PREVIEW_BYTES,"encoded_bytes_per_source":crate::sample_asset::MAX_ENCODED_BYTES,"aggregate_encoded_bytes":crate::sample_asset::MAX_TOTAL_ENCODED_BYTES,"aggregate_decoded_sample_values":crate::sample_asset::MAX_SAMPLE_VALUES},"unavailable":["live_gui_meter_readback","audio_device_control","physical_output_measurement","rms_loudness_true_peak","meter_files","pcm_payload","arbitrary_unaligned_ranges","legacy_path_decode"]});
        let track_mix = json!({"version":crate::track_mix::VERSION,"algorithm":crate::track_mix::ALGORITHM,"tools":["leviathan_track_mix_preview","leviathan_track_mix_commit"],"read_section":"track_mix","edit_operation":"set_track_mix","scope":"Persisted gain/balance/mute/solo in this isolated headless Song only; no GUI/device/live-mailbox control","settings":"Complete strict unique-TrackId array, at most64 entries; finite gain0..1 and balance-1..1 validated before f32 storage, required mute/solo booleans, empty resets neutral defaults. Mute wins; a solo on a silent track still gates non-solo tracks","routing":"Shared pure compiled-layout review. Legacy Instrument-owned voices sharing multiple tracks require equal effective L/R; controlled voices route independently. Available/Unavailable/Unsupported status is explicit; model-valid unrenderable drafts may retain saved settings, never implying live readiness","workflow":"One bounded private Editor-owner/both-revision/exact-base review; failed requests preserve it, true no-op preserves redo/dirty/revisions, changed commit one Undo. No source/path/hash/decode or audio IO. Normal edit_batch source-path policy is unchanged","playback":"Persisted settings are consumed from frame zero by later starts and ordinary offline export, not a replay of native live ramps. Neutral PCM/manifest behavior is unchanged. Native live updates are separate and only for a matching currently playing snapshot; no MCP tool applies them","nonneutral_manifest_schema":11,"limits":{"settings":crate::track_mix::MAX_TRACKS,"structured_payload_bytes":MAX_PREVIEW_BYTES,"candidate_compact_bytes":MAX_DOCUMENT,"retained_reviews":1,"contributing_tracks":crate::track_mix::MAX_TRACKS,"source_path_bytes":crate::track_mix::MAX_PATH_BYTES},"unavailable":["live_gui_ipc","cross_session_mixer_control","live_gui_meter_readback","automation","fx","stems","midi","recording"]});
        let cell_blocks = json!({"algorithm":crate::cell_blocks::ALGORITHM,"version":crate::cell_blocks::VERSION,"tools":["leviathan_cell_block_copy","leviathan_cell_block_preview","leviathan_cell_block_inspect","leviathan_cell_block_commit"],"scope":"One explicit pattern/TrackId and nonempty half-open row/column bounds. Paste may use another pattern but must retain the copied logical TrackId; no growth/cropping/merge or system clipboard","copy":"Frozen row-major Option<Cell> includes None holes and Some(default) separately, exact signed-zero/scalars/effects/phrase fields; aliases resolve for read-only Copy. No caller cells or inter-document authority","ownership":"Opaque Editor-owned document identity, not a caller or MCP-invented ID. One clipboard survives ordinary edits, Save and Undo/Redo but successful New/Open revokes it. Successful Copy replaces it and retires the rectangle preview; failed Copy preserves both","preview":"One owner/both-revision/exact-base-guarded shared Paste replace or Clear candidate. Destination aliases reject; raw copied IDs resolve against current document definitions. Metadata only, Copy/Preview allowed without Edit Mode; Commit requires Edit Mode","commit":"Actual Editor revision delta reports change, including signed-zero-only edits. One Undo for changes, true no-op preserves redo/dirty/revisions and consumes only its review; clipboard remains. No file/path/hash/decode/audio/MIDI work or incoming-memory/audio-equivalence promise","limits":{"positions_including_holes":crate::cell_blocks::MAX_POSITIONS,"clipboard_bytes":crate::cell_blocks::MAX_CLIPBOARD_BYTES,"diff_bytes":crate::cell_blocks::MAX_DIFF_BYTES,"page_records":crate::cell_blocks::MAX_PAGE,"structured_payload_bytes":MAX_PREVIEW_BYTES,"candidate_compact_bytes":MAX_DOCUMENT,"resolved_visits":crate::cell_blocks::MAX_VISITS,"phrase_cells":crate::cell_blocks::MAX_PHRASE_CELLS,"prospective_validation_units":crate::cell_blocks::MAX_VALIDATION_UNITS},"unavailable":["system_clipboard","client_supplied_cells","inter_document_paste","cross_track_paste","alias_destination_write","clipboard_resource_remapping"]});
        let marker_bank = json!({"algorithm":crate::marker_bank::ALGORITHM,"version":crate::marker_bank::VERSION,"tools":["leviathan_marker_bank_preview","leviathan_marker_bank_commit"],"selection":"Persisted accepted marker set, explicit strictly ascending unique 1..16 interval indices; each adjacent endpoint pair becomes one exact native-source-frame region. Sparse choices allowed, no BPM requirement or inferred roles","allocation":"Shared checked fresh region/kit/instrument IDs and safe pad defaults, no overwrite or adapter-side allocation; creates one new kit and newly assigned instrument, preserving existing patterns/order/clock/markers/audio","preview":"Pure IO-free metadata plan, complete bounded summary and one opaque retained token; successful new preview supersedes only the prior marker-bank review, failed preview preserves it","commit":"Selected source lexical path authorization plus fresh bounded SHA check, then rechecked exact serialized base and both MCP/Editor revisions before one shared Editor Undo transaction. No all-source verification, decode, file writes, live transport or GUI IPC","limits":{"intervals":crate::marker_bank::MAX_INTERVALS,"name_utf8_bytes":256,"structured_payload_bytes":MAX_PREVIEW_BYTES,"candidate_compact_bytes":MAX_DOCUMENT,"selected_source_encoded_bytes":crate::sample_asset::MAX_ENCODED_BYTES,"retained_marker_bank_plans":1,"resolved_visits":crate::marker_bank::MAX_VISITS,"phrase_cells":crate::marker_bank::MAX_PHRASE_CELLS,"prospective_validation_units":crate::marker_bank::MAX_VALIDATION_UNITS},"unavailable":["automatic_slice_selection","automatic_role_classification","automatic_tempo_assignment","pattern_generation","midi_input"]});
        let audition = json!({"version":crate::audition::VERSION,"tool":"leviathan_audition_prepare","scope":"headless_metadata_only","targets":["region_draft","pad"],"clock":{"sample_rate":crate::RATE,"musical_frames":crate::audition::MUSICAL_FRAMES,"tail_frames":crate::TAIL,"total_frames":crate::audition::TOTAL_FRAMES,"note_starts":1},"region":"Explicit uncommitted bounds/root/ADSR/forward-loop draft on one registered sample; native pitch, unity input and center pan","pad":"Committed instrument/pad mapping, actual gain/pan/tune/root/mode, unity incoming velocity1.0/bin127, chosen velocity zone and first RR alternative; fresh isolated context on each prepare","verification":"Path authorization for every source retained in the selected snapshot, including unused alternatives of its selected pad; stricter than native selected-only loading. No source hash/decode or rendered audio. Source SHA is declared metadata","review_ref":"Deterministic exact base/request/both-revisions/shared-snapshot digest for provenance only; no retained workflow or capability","limits":{"structured_payload_bytes":MAX_PREVIEW_BYTES},"unavailable":["audition_audio_render_tool","audition_file_publication","physical_audio_playback","live_gui_ipc","cross_session_device_control","midi_input"]});
        let thin = json!({"algorithm":crate::mutations::ALGORITHM,"version":crate::mutations::VERSION,"gate_algorithm":crate::mutations::GATE_ALGORITHM,"operation":"Explicit version/scope/seed/integer removal_percent/max_removed/protected_roles. Whole stored-cell deletion baked into ordinary editable Song; no runtime generator or arbitrary code","gate":"Actual pinned pattrns ProbabilityGate, one canonical call per eligible stored cell, not per repeated occurrence. Shared summary publishes effective f32 probability, calls/draws and reasons; no hidden reseeding or fallback","protection":"Every resolved unsuppressed ON use must have current explicitly reviewed concrete region role and pass shared eligibility. Unknown/Mixed/unreviewed/stale/protected uses are always retained; alias targets reject. Empty protected_roles disables only the five optional concrete-role protections, not mandatory uncertainty protection","proof":"Whole model must evaluate before preview, including0% and max_removed0. Candidate must pass shared retained-ON and owned-control proof across all uses; conflicting selection/velocity/RR or control-target changes reject atomically, not silently repair. Retained starts are not a claim of identical full audio or voice lifetimes","authority":"Model-binding current human metadata, not feature-suggestion authority or fresh source-file verification. No hash/decode/audioIO in shared core. MCP retains normal all-referenced-path authorization/existence checks, so missing/unauthorized files can reject earlier than pure UI/core","review":"One opaque plan with explicit config/algorithm/version and full base/candidate/decision/use/change hashes. Independent bounded decisions/uses/contexts/changes pages. Both revisions/full base guard one Editor commit; no caller candidate, failed preview preserves prior review and true no-op preserves redo","limits":{"records_per_kind":crate::mutations::MAX_RECORDS,"contexts":crate::mutations::MAX_CONTEXTS,"source_path_payload_bytes":crate::mutations::MAX_PATH_PAYLOAD,"page_records":MAX_THIN_PAGE,"structured_bytes":MAX_PREVIEW_BYTES},"unavailable":["rotation_or_rearrangement","automatic_anchor_or_role_inference","per_occurrence_randomization","audio_processing_or_fresh_identity","arbitrary_code","musical_quality_or_full_audio_preservation_guarantee"]});
        let slice_features = json!({"algorithm":crate::slice_features::ALGORITHM,"version":crate::slice_features::VERSION,"analysis":"Explicit sorted unique regions of one registered source. Strict verified native decode or authorized/hash-verified current opaque onset PCM; both compute NEW feature FFTs, no first-N truncation","measurements":"Native pre-playback RMS/peak/DC/crest, spectral power/band fractions/centroid/flatness, unit-spectrum shape change with explicit relative spectral support floor/count, censored attack/tail and envelope rises. Source-frame units and effective layout included; no BPM, pitch, envelope/loop processing or musical-hit-count inference","suggestions":"Transparent named handcrafted rules; rule_support NOT probability. Mixed/Unknown are ordinary outcomes. Ghost articulation is explicit human context, never absolute-level inference","review":"Separate Store preview and commit; all measured descriptors stored, only explicit role choices replace labels. Empty roles preserves old labels. Manual set_region_role and clear_region_features are separate typed operations, not metric injection","authority":"One compact result plus nested Store review, both revisions/full base and fresh authorized source SHA at review/commit. Completed feature results detach from later onset picker epochs. Failed requests preserve prior review","history":"Source/bounds edits retain historical snapshots/labels with false current-match; names/roots/playback options do not stale native measurements. Region removal cleans its entry. Parsed metadata validates shape, not truth or signed computation; model match is not fresh-byte verification","inspection":"Stored catalog pages are metadata-only and remain readable with missing/tampered media; no file IO or fresh identity claim. Pending pages are also metadata-only, guarded by full base/both revisions; verification occurred at Analyze, not inspection. Independent exact totals/end pages; no silent truncation","execution":"Bounded synchronous requests, no cancellation/latency SLA; no retained extra PCM or automatic document changes","limits":{"regions_per_analysis":crate::slice_features::MAX_REGIONS,"catalog_entries":crate::slice_features::MAX_CATALOG,"seconds_per_region":crate::slice_features::MAX_REGION_SECONDS,"visited_frames":crate::slice_features::MAX_VISITED_FRAMES,"work_units":crate::slice_features::MAX_WORK,"page_records":MAX_FEATURE_PAGE,"structured_bytes":MAX_PREVIEW_BYTES,"source_rate_min":8000,"source_rate_max":384000},"unavailable":["ML_or_calibrated_classifier","automatic_reviewed_roles","ghost_from_level","arbitrary_feature_injection","playback_or_BPM_changes"]});
        let tempo = json!({"algorithm":crate::tempo::ALGORITHM,"version":crate::tempo::VERSION,"evidence":"Explicit accepted-marker index window (end excluded, include_start explicit), or current server-owned v2 cache window using peak/refined_attack points; no client points, audio decoding or new FFT","timing":"Native source frames; integer200..4000 tenths=20..400BPM, exact rational pulse phase relative to window start plus approximate display. Phase is advisory, not downbeat/meter/loop-length or compiler-origin authority","scores":"Versioned heuristic component scores/strength, NOT probability/calibrated confidence. All returned candidates, shortlisting disclosure and half/double/ratio aliases remain visible; insufficient evidence has no fallback BPM","use":"Separate explicit candidate + same-source target MarkerSet preview/commit changes source_bpm only. Preserves markers/locks/name/sample, Song clock, source bytes and existing baked output. Changed commit one Undo; same-value commit consumes Use token but preserves revision/redo","authority":"Both revisions and full base Song; cached mode ALSO binds exact current onset token/config epoch. Successful repick or started extraction retirement invalidates cached analysis even without Song edit; failed repick retaining cache may retain it. Marker analysis independent of optional onset cache","source_policy":"All tempo requests authorize referenced paths. Marker-only analysis/inspection/Use does not hash/decode; identity remains stored metadata. Cached requests additionally reauthorize and freshly hash current source through existing onset policy; shared scoring reuses trusted-cache repick, no additional PCM/cache copies","execution":"Bounded synchronous MCP request, no background job or latency promise; one analysis and one nested Use preview, failed new analysis/review retains prior state unless its cache was independently retired","limits":{"points":crate::tempo::MAX_POINTS,"cached_scan_points":crate::tempo::MAX_CACHED_POINTS,"shortlist":crate::tempo::MAX_SHORTLIST,"candidates":crate::tempo::MAX_CANDIDATES,"pulses_per_candidate":crate::tempo::MAX_PULSES,"page_records":MAX_TEMPO_PAGE,"structured_bytes":MAX_PREVIEW_BYTES,"window_seconds":crate::tempo::MAX_WINDOW_SECONDS,"source_rate_min":8000,"source_rate_max":384000,"work_units":crate::tempo::MAX_WORK},"unavailable":["arbitrary_evidence_injection","automatic_BPM_assignment","downbeat_or_meter_inference","role_or_loop_length_classification","phase_application","audio_stretch","ML_or_dataset_claims"]});
        let resample = json!({"algorithm":crate::resample::ALGORITHM,"version":crate::resample::VERSION,"publication_supported":crate::resample::PUBLICATION_SUPPORTED,"platform":"Guarded publication is Unix-only in v1; other platforms fail closed before decode/temp creation","scope":"One explicit occurrence, all tracks, isolated cold frame0; retained aliases/mutes, no incoming voices/column memory, fresh phrase instances/RR counters, not an arrangement crop","source_policy":"Actually used sources must be registered regions (including kit/keyzones). Used legacy sample paths reject with import/register-region guidance; no silent DSP conversion. MCP additionally authorizes all referenced paths, used sources are hash-verified by Engine","gain":"Fixed pre-master float32 asset=existing Engine PCM×2 at master0.5; NOT normalization. Values may exceed1. Printing bakes the current effective track mix. Exact output replay only with neutral effective track mix, native root, unity velocity/gain, center pan and no extra playback options/effects, held through the WHOLE printed region without early OFF/replacement/SongEnd","output":"Fresh48kHz stereo float32 WAV only, no events sidecar. Owned sibling temporary is strictly imported/verified then atomically hard-linked without clobber; parent identity and output hash rechecked before metadata registration, together with all prospective source paths/identities and existing64MiB/file,256MiB aggregate and8MiB pretty-project storage limits","tail":"Explicit MusicalOnly truncation (may click) or ReleaseTail exactly24000 frames/500ms; entire rendered extent becomes whole new region","registration":"Separate one-Undo fresh SampleId+RegionId+Pitched InstrumentId, unchanged old arrangement. Published WAV survives stale tokens, supersession, closing, Undo and failed registration; Redo restores references only","workflow":"One server-owned Prepared→Printed workflow token, full-base and MCP/Editor revision guarded; same token survives rendering for recoverable inspect/register; stale Printed metadata survives document advances for inspection only, registration_current=false. Failed new prepare/render retains prior phase; repeated render refuses","inspection":"Metadata-only bounded source identity pages; no retained event snapshot or fresh-byte verification claim. Prepared source identities are declared metadata; Printed receipt identities were verified during render","execution":"Bounded synchronous rendering, not a latency SLA or cancellable background job. Terminating process is not transactional rollback of an already-published file","limits":{"printed_frames":crate::resample::MAX_PRINTED_FRAMES,"voice_frames":crate::resample::MAX_VOICE_FRAMES,"source_page_records":MAX_RESAMPLE_PAGE,"structured_bytes":MAX_PREVIEW_BYTES,"project_pretty_wrapper_bytes":MAX_DOCUMENT},"unavailable":["arrangement_crop_with_incoming_state","used_legacy_sample_paths","automatic_normalization","automatic_replacement_of_old_notes","durable_artifact_delete_via_undo","live_recording_or_audio_device"]});
        let multisamples = json!({"storage":"Song.multisamples keyed by stable MultisampleId; KeyZoneIds are stable within each map","mapping":"Inclusive MIDI note/velocity rectangles0..127; overlap in both axes rejects. Gaps and empty maps are valid drafts; an uncovered actual evaluated ON rejects before decode/output","selection":"Exactly one region selected once at ON by audible MIDI note and round(127*incoming f32 ON velocity) in f64 before source gain. Actual amplitude remains continuous. Later gain/glide do not select another zone; no counters/random/stacking","pitch":"Ordinary note is audible chromatic pitch, not a pad selector. Zone root_note is the explicit native reference overriding region.root_note; mapped region supplies source frames and optional envelope/forward loop","sharing":"Maps are mutable shared definitions: zone edits affect all assigned instruments and future rendering of existing notes; duplicate map explicitly for independence","editing":"Full set_keyzones replacement uses stable IDs, permits adjacent rectangle edits atomically, and protects every region reference including unused zones. Metadata upsert preserves zones. Required assign map_id:null detaches only a current Multisample; other kinds are unchanged","inspection":"Bounded independent map zone/instrument pages and source multisample-mapping page; no inferred selector or hidden source reads in map inspection","manifest_schema":10,"limits":{"maps":64,"zones_per_map":128,"zones_total":4096,"map_name_bytes":256,"zone_name_bytes":128,"inspection_page_records":MAX_KEYZONE_PAGE,"source_mapping_page_records":MAX_SOURCE_INSPECTION_PAGE,"structured_bytes":MAX_PREVIEW_BYTES},"unavailable":["overlapping_or_crossfaded_layers","random_or_round_robin_keyzone_selection","automatic_root_pitch_detection","live_audition","GUI_instance_control"]});
        let groove = json!({"algorithm":crate::groove::ALGORITHM,"version":crate::groove::VERSION,"capture":"Accepted marker window, explicit stored source BPM and declared subdivision/cycle; no detector rerun or inference. Fresh explicit GrooveId; historical immutable raw-anchor snapshot","history":"Stored source/marker IDs and SHA are provenance, not live references or current-byte verification; later edits/deletion do not regenerate the recipe","apply":"Timing only on explicit pattern/track/half-open rows/sorted columns. Origin is scope.row_start; nearest rational tick half-up, repeated declared cycles, sparse missing slots unchanged. Strength0 exact stored no-op;100 targets exact captured phase at current tempo, never additive repeated offsets","preservation":"Ordinary cells only: notes/kit selectors/gain/pan/other fields retained; collisions/overtaking, alias-target writes, changed phrase triggers, out-of-scope and changed mute eligibility reject. Same integer frame displacement must hold across ordered/alias uses","review":"One active shared capture OR apply plan; successful new preview supersedes either kind, failed preview preserves it. Both revisions/full base/token/root guarded commit, one Undo; no-op consumes token without clearing redo","units":"Template raw source-frame delta and source rate/BPM are authoritative; inspected offset_beats is approximate display only. Baked row/delay timing follows future tempo/LPB/order changes","source_verification":"Path authorization only for preview/commit; no audio decode/hash. Historical template inspection performs no source access","audibility":"Stored-cell/timing proof is not full renderability or unchanged expanded event count. Recompiled interleaving can change round-robin choices, choke, phrase replacement and overlap; moving a replacement note can expose/cancel pending phrase children. Evaluation/export still checks source assignment/decoding and child/action budgets","limits":{"templates":crate::groove::MAX_TEMPLATES,"anchors":crate::groove::MAX_ANCHORS,"cycle_steps":1024,"selected_notes":4096,"affected_uses":4096,"ordered_blocks_rows_cells":crate::groove::MAX_VISITS,"timing_action_context_checks":crate::groove::MAX_VISITS,"encoding_frame_comparisons":4194304,"project_pretty_wrapper_bytes":MAX_DOCUMENT,"page_records":MAX_GROOVE_PAGE,"structured_bytes":MAX_PREVIEW_BYTES,"candidate_bytes":MAX_DOCUMENT},"unavailable":["automatic_beat_or_tempo_inference","accent_or_velocity_transfer","audio_stretch","runtime_global_groove","random_humanization","raw_template_injection"]});
        let sampler = json!({"storage":"Optional SampleRegion.amp_envelope and forward_loop; shared by every instrument/pad using that region, not destructive audio edits","envelope":"Explicit integer milliseconds at 48 kHz output clock: attack/decay 0..10000, sustain_percent 0..100, release 0..500; independent of BPM and playback pitch","legacy":"Both null means unchanged legacy region playback. Explicit A0/D0/S100/R50 is an opted-in envelope, not the legacy None path","loop":"Forward source-frame [start,end), at least two frames wholly inside region; prefix once then loop during sustain and release; no post-loop tail or crossfade","release":"Ordinary OFF/timed release use opted-in release_ms; Choke/PadReplaced/TrackMute use fixed 50 ms linear release; never restart an existing release. Hard cut remains immediate. R0 stops abruptly and may click","restrictions":"Used loop voices reject offset and Reverse before audio decode/output; envelope-only regions retain supported step effects. Existing OneShot/Gated OFF rules still apply","editing":"set_region_playback requires both nullable options; null clears that option. upsert_region preserves existing options, rejecting bounds that invalidate them","manifest_schema":8,"unavailable":["reverse_or_ping_pong_loops","loop_crossfade","embedded_file_loop_import","per_voice_independent_region_options","live_audition"]});
        let generation = json!({"algorithm":crate::generation::ALGORITHM,"version":crate::generation::VERSION,"sequencer":"Actual pinned pattrns 0.9.3","pattrns_revision":crate::pattern::REVISION,"gate_algorithm":crate::generation::GATE_ALGORITHM,"presets":["Euclidean"],"seed":"Required explicit u64, local deterministic seeded gate continuous across finite bars; no hidden random retry","sequence":"1..16 explicit MIDI12..127 note/velocity choices cycling on emitted hits only; every choice must be mapped/source-assigned even if gated out","output":"Fresh checked TrackId and PatternId plus appended unmuted occurrence; ordinary editable cells, unchanged song tempo/LPB and existing content","provenance":"Pattern.generation is historical seed/version/config/algorithm and output identity, not a live generator, current-reference authority or automatic regeneration recipe","review":"Opaque shared plan, exact generated-cell pagination and fullbase/both-revision/token/root guarded one-undo commit","units":{"step":"Global zero-based pattrns tick across bars","virtual_time":"step*1024 synthetic clock ticks at60BPM/1024Hz, not Song seconds or output frames","frame":"Absolute appended Song48kHz output frame"},"source_verification":"Path authorization only for pure generation; no decode/hash/audio output","unavailable":["arbitrary_code","Lua","user_scripts","custom_pattrns_programs","runtime_regeneration","unseeded_random_generation"],"limits":{"bars":16,"steps_per_bar":128,"raw_ticks":2048,"ordered_track_blocks":crate::generation::MAX_VISITS,"ordered_resolved_row_visits":crate::generation::MAX_VISITS,"ordered_resolved_cell_visits":crate::generation::MAX_VISITS,"work_guard":"Prospective (order_count+1)*(display_track_count+1), checked before model validation; resolved ordered row/cell counts include empty/default/muted entries, exact candidate rechecked","hits_before_existing_song_budget":2048,"sequence_choices":16,"page_cells":MAX_GENERATION_PAGE,"structured_bytes":MAX_PREVIEW_BYTES,"candidate_bytes":MAX_DOCUMENT,"project_pretty_wrapper_bytes":MAX_DOCUMENT,"project_size_policy":"Shared pure bounded pretty-Document preflight using current paths; conservative before Save path shortening"}});
        let transforms = json!({"algorithm":crate::transforms::ALGORITHM,"storage":"Ordinary stored parent cells; no recipe, runtime groove, DSP or random generation","scope":"Explicit PatternId/TrackId, half-open row range, sorted unique note columns and four independent field-mask booleans","operations":["pitched_note_transpose","explicit_glide_target_transpose","explicit_velocity_scale","integer_output_frame_nudge","offline_pattern_anchored_swing"],"timing":"48 kHz output frames; whole cells and commands move within selection only, exact across all occurrence and alias uses. Collisions/overtaking/unrepresentable contexts reject; no hidden quantization. Frame proof is raw scheduled timing: crossing occurrence mute boundaries can change audibility/event counts","aliases":"Alias targets are read-only; explicitly transform source IDs or make unique first. Source edits affect all repeats and aliases; inspect summary","preview":"One bounded opaque shared candidate, then paginated exact before/after CellChange records and revision/base/token-guarded one-undo commit","no_op":"Consumes review token, preserves document/revision/history/redo","seed":"Not accepted: deterministic operations only; random humanization is unavailable","source_verification":"Current root/regular-file/no-symlink pathname policy on preview/inspect/commit; no audio decode/hash claim","limits":{"selected_cells":4096,"affected_uses":4096,"event_use_timing_checks":65536,"ownership_order_scan_cells":65536,"encoding_frame_comparisons":4194304,"change_page_records":MAX_TRANSFORM_PAGE,"structured_payload_bytes":MAX_PREVIEW_BYTES,"candidate_bytes":MAX_DOCUMENT}});
        let arrangement = json!({"manifest_schema":7,"provenance":"Ordered alias source chains and occurrence mute frame intervals; unused aliases alone preserve the previous manifest path","aliases":"Per-pattern target TrackId links to PatternId/TrackId, including cross-track links; exact equal row and column geometry; scalar cells and phrase triggers only, not automation","resolution":"Shared bounded resolver, target-first terminal-last chain; sharing content never shares playback state","editing":"Alias targets are read-only; explicitly edit resolved source IDs or make_track_unique. source:null unlinks without copying; whole-pattern clone materializes aliases","occurrences":"Muted target TrackIds belong to each order occurrence; swaps carry full records, set_order and append_repeat create fresh unmuted occurrences","mute":"Suppress block cells and parameter-memory updates, cancel pending phrase actions, de-click release held track voices at muted boundary. Already-releasing tails may fade; not hard silence. Explicitly all-muted note arrangements export valid zero PCM without decoding unused assets; plain empty songs still reject","limits":{"alias_links":4096,"chain_edges":32,"resolved_alias_cells":65536,"resolved_alias_row_containers":65536,"ordered_alias_blocks":65536,"ordered_alias_cells":65536,"ordered_alias_row_containers":65536,"occurrence_mutes":65536,"matrix_page_occurrences":32,"matrix_page_tracks":32,"matrix_structured_bytes":MAX_PREVIEW_BYTES}});
        let pad_sources = json!({"modes":["single_region","velocity_zones_deterministic_round_robin","unassigned"],"simultaneous_sources_per_hit":1,"selection":"Nearest integer round(127 * incoming f32 ON volume), computed in f64 before pad gain; inclusive gapless sorted velocity bins 0..127; actual amplitude stays continuous","round_robin":"Alternative vector order, independent counter per InstrumentId/PadId/zone; every unsuppressed ON including retriggers and zero-volume hits advances. Full compilation resets counters, cold seek uses compiled choices and transport loops repeat them; no seed or random state","identity":"Stable nonzero PadLayerId unique across every zone within a pad; region references and alternative order are inspectable","replacement":"set_pad_sources requires both nullable region_id and velocity_zones array: one region + empty zones, null + full zones, or null + empty to explicitly unmap. A single region plus zones rejects","legacy_metadata":"upsert_drum_pad region_id:null preserves existing zones; non-null on a zoned pad rejects with an explicit set_pad_sources instruction. No silent source-mode switch","manifest_schema":9,"provenance":"Used zoned voices expose exact layer_selection in the shared evaluated manifest, including input velocity, rounded bin, zone bounds, stable layer ID, zero-based round-robin index and ordinal; no counters stored in Song","limits":{"zones_per_pad":8,"alternatives_per_zone":8,"alternatives_per_pad":32,"source_inspection_page_records":MAX_SOURCE_INSPECTION_PAGE,"source_inspection_structured_bytes":MAX_PREVIEW_BYTES},"unavailable":["random_alternative_selection","simultaneous_layer_stacking","velocity_crossfades","user_scripts"]});
        let drum_kits = json!({"storage":"Song.drum_kits keyed by stable DrumKitId; pad IDs are stable within a kit, slot and MIDI mapping are independently editable","layers":1,"layers_meaning":"One selected source per hit, not the number of stored velocity/RR alternatives","pads_per_kit":16,"manifest_schema":6,"creation":"create_drum_kit_instrument adds a new kit and new assigned instrument with shared checked fresh IDs; existing occupied instruments are preserved","source":"Each hit chooses one verified non-destructive region, from a single mapping or explicit velocity-zone/round-robin alternatives; original audio is unchanged","notes":"Ordinary tracker MIDI notes select pads, not chromatic pitch; use set_cell with the kit-assigned instrument. Per-pad root_note is the native reference; explicit tune_semitones -48..48 changes playback pitch, not the selector. Missing/unmapped hit selectors reject","modes":{"OneShot":"Ignores ordinary OFF/PhraseEnd; timed release and hard cut still apply","Gated":"Honors ordinary OFF/PhraseEnd; both modes ignore automatic column replacement OFF"},"new_note_policies":{"Overlap":"Earlier held same-pad voices may continue","ReleasePrevious":"Release older held same-pad voices before the new hit; already-releasing tails continue"},"choke_groups":"Optional 1..16, scoped to one InstrumentId across tracks/phrase instances; release older held members before the new hit, including zero-gain hits; already-releasing tails continue","audition":"leviathan_audition_prepare resolves a metadata-only shared snapshot; no live audio device, GUI control or audition export tool","unavailable":["simultaneous_layer_stacking","random_round_robin","pad_effect_chains","live_midi_input","transposed_kit_phrases"]});
        let drum_steps = json!({"storage":"Ordinary pattern cells only; no second sequencer","page_steps":[16,32],"origin":"pattern row 0","divisions":["Quarter","Eighth","Sixteenth","ThirtySecond","SixtyFourth","OneTwentyEighth"],"identity":"PatternId + TrackId + InstrumentId; shared kit aliases remain distinct","guard":"One inspection token bound to exact Song, both revisions and view; every changed document revision invalidates it","conflicts":"Ambiguous/off-grid/multiple-hit/phrase slots cannot toggle; exact owned source events can receive field edits without hidden snapping","units":"Delay/retrigger spacing in source-row fractions; None and explicit zero remain distinct","limits":{"scanned_pattern_cells":crate::drum_steps::MAX_SCAN_CELLS,"source_events_and_issues_per_page":crate::drum_steps::MAX_PAGE_EVENTS,"structured_projection_bytes":MAX_PREVIEW_BYTES}});
        let onset_analysis = json!({"default_engine":"v1 via leviathan_sample_analyze, unchanged","experimental_engine":"experimental_v2 requires explicit opt-in tool, engine and preset","experimental_algorithm":crate::onset_v2::ALGORITHM,"application_policy":crate::onset_service::POLICY,"phase_default":false,"phase_override":"Explicit enable_phase is optional supporting evidence, never a standalone trigger; actual effective config is returned","presets":["Conservative","Balanced","GhostHunter"],"confidence":"Heuristic evidence, not probability; false positives and missed onsets remain","cache":"One native-PCM/raw-evidence cache; source SHA/metadata + spectral algorithm/config key + full picker config SHA; any document revision invalidates it","repick":"Rehash current source, reuse shared immutable evidence without FFT or decode; successful repick replaces token","curves":"Paginated original-source-frame raw evidence, not normalized confidence or accepted markers","manual_edits":"Peak and refined attack stay separate; accepted markers change only through explicit shared locked-marker operations"});
        let mut limits = json!({"max_message_bytes":MAX_MESSAGE,"max_batch_operations":MAX_OPERATIONS,"max_document_bytes":MAX_DOCUMENT,"history_entries":64,"export_max_seconds":600,"max_phrase_definitions":128,"max_phrase_rows":1024,"max_phrase_columns":16,"max_phrase_cycles":64,"max_phrase_definition_cells":32768,"max_evaluated_phrase_triggers":256,"max_expanded_actions":65536,"max_note_starts":2048,"max_sample_sources":128,"max_sample_regions":4096,"max_encoded_sample_bytes":crate::sample_asset::MAX_ENCODED_BYTES,"max_decoded_sample_bytes":crate::sample_asset::MAX_SAMPLE_VALUES*4,"project_max_registered_encoded_bytes":256*1024*1024,"analysis_max_seconds":MAX_ANALYSIS_SECONDS,"analysis_max_suggestions":MAX_ANALYSIS_SUGGESTIONS,"analysis_max_waveform_bins":MAX_ANALYSIS_BINS,"max_marker_sets":128,"max_marker_boundaries":257,"max_compile_cycles":16,"max_compile_columns":256,"max_preview_bytes":MAX_PREVIEW_BYTES,"max_cleanup_candidates":MAX_ANALYSIS_SUGGESTIONS,"onset_v2_max_seconds":MAX_ONSET_SECONDS,"onset_v2_max_proposals":MAX_ONSET_SUGGESTIONS,"onset_v2_max_curve_page_bins":MAX_ONSET_PAGE,"onset_v2_max_serialized_result_bytes":MAX_ONSET_RESULT_BYTES,"onset_v2_max_result_page_records":MAX_ONSET_RESULT_PAGE,"onset_v2_max_retained_cache_bytes":crate::onset_service::MAX_RETAINED_BYTES,"onset_v2_max_work_units":500_000_000,"onset_v2_max_working_bytes":128*1024*1024});
        limits["max_drum_kits"] = json!(128);
        limits["max_pads_per_kit"] = json!(16);
        let mut capabilities = json!({"api_version":API_VERSION,"protocol_version":PROTOCOL_VERSION,"application_version":env!("CARGO_PKG_VERSION"),
            "scope":if self.live_document_context {"bound_live_gui_document"} else {"isolated_headless_document"},"workspace_root":self.workspace.root,"stable_ids":"nonzero u64; IDs survive reorder/rename; explicit IDs except shared compiler/fresh-kit allocation",
            "available":["project_read_create_open_save","atomic_revision_checked_edit_batches","tracks","instruments_and_root_confined_samples","patterns_and_cells","order_repeat_and_independent_clone","undo_redo","offline_evaluate_and_wav_export","demo_sample_generation","instrument_bound_phrases","verified_wav_flac_import","non_destructive_sample_regions","bounded_transient_suggestions","persistent_accepted_marker_sets","reviewable_marker_cleanup","editable_break_compile_preview_commit","persistent_marker_locks_and_precise_edits","experimental_onset_v2_cached_repick_and_evidence","single_layer_16_pad_drum_kits","guarded_drum_step_projection_and_exact_cell_edits","track_block_aliases_and_occurrence_mutes","reviewed_scoped_offline_cell_transforms","seeded_versioned_pattrns_euclidean_generation","optional_region_amp_envelopes_and_forward_loops","velocity_zones_and_deterministic_pad_round_robin","accepted_marker_groove_capture_and_timing_transfer","melodic_multisample_keyzones","isolated_occurrence_resample_to_new_asset","reviewable_tempo_and_pulse_phase_hypotheses","bounded_region_and_pad_audition_metadata","reviewed_accepted_marker_pad_bank","reviewed_whole_cell_rectangle_copy_paste_clear","persisted_track_balance_review_and_offline_render"],
            "step_effects":{"retrigger":"1..16 additional starts; spacing in row fractions; pitched note required","release":"row-local fraction initiates release: legacy de-click or opted-in region release_ms; zero milliseconds may stop abruptly","glide":"MIDI target 0..127; 0.01..120 semitones/second","cut":"absolute row fraction [0,1), at/after delayed starts; abrupt active-voice stop, may click; cannot combine release","offset":"zero-based source sample frame <=u32::MAX; decoded sample bounds checked at render","direction":"Forward or Reverse; reverse new notes start at last real source frame unless offset is given; advanced path uses linear interpolation"},
            "phrases":{"storage":"Song.phrases keyed by stable PhraseId; each definition binds one instrument","trigger":"Cell.phrase contains phrase ID and 1..64 total cycles; Pitched and melodic Multisample instruments transpose relative to root_note; kit phrases require neutral root-note triggers","timing":"Independent phrase LPB at song BPM; shared sample-frame evaluator","attributes":"Trigger gain multiplies internal velocity; pan is added and clamped to -1..1","nested_triggers":false,"manifest_schema":4},
            "sample_regions":{"storage":"Immutable SHA-256 SampleAsset identities and mutable SampleRegion views, keyed by stable IDs","bounds":"start_frame inclusive, end_frame exclusive, in original source frames; source audio unchanged","mapping":"assign_region selects a region for an instrument and clears its legacy path; explicit legacy sample assignment switches back","playback":"root_note plays native source rate regardless of song BPM; one-shot by default, optional explicit forward loop and amp envelope; no stretch","analysis":"Synchronous bounded suggestions only; deterministic adaptive energy/difference rises, not FFT or beat/tempo detection; confidence is heuristic, not probability"},
            "break_compiler":{"algorithm":crate::break_compiler::ALGORITHM,"markers":"Accepted persistent sets, strictly increasing original-source-frame boundaries including 0 and real source end; separate from analysis suggestions","source_tempo":"Explicit source BPM20..400 required, manually set or from reviewed Use BPM; compiler itself never infers tempo or applies advisory phase, origin source frame0","target_tempo":"Explicit target BPM replaces the global song BPM, affecting existing material; LPB is preserved","timing":["SourceRhythm","Grid"],"subdivisions":["Sixteenth","ThirtySecond","SixtyFourth","OneTwentyEighth"],"tail_policies":["Overlap","ReleasePrevious","CutPrevious"],"preview":"One bounded private candidate; review diff, then token/revision/base/source-hash-guarded one-undo commit","preview_byte_limit_scope":"Serialized data payload; the MCP mirrored text and JSON-RPC envelope add bounded wire overhead","output":"Ordinary editable regions, instruments, new track/pattern and appended order; no opaque playback recipe or random generation"},
            "onset_analysis":onset_analysis,
            "drum_kits":drum_kits,
            "pad_sources":pad_sources,
            "multisamples":multisamples,
            "resample":resample,
            "audition":audition,
            "marker_bank":marker_bank,
            "cell_blocks":cell_blocks,
            "track_mix":track_mix,
            "sample_peaks":sample_peaks,
            "tempo":tempo,
            "slice_features":slice_features,
            "thin":thin,
            "groove":groove,
            "drum_steps":drum_steps,
            "arrangement":arrangement,
            "transforms":transforms,
            "generation":generation,
            "sampler":sampler,
            "unavailable":{"gui_instance_control":"No GUI IPC; an already-open editor is not synchronized","live_transport":"No audio device is opened by this server","plugins":"Not implemented","phrase_aliases_nested":"Phrase aliases and nested triggers are not implemented","automatic_break_understanding":"Full raw-audio tempo/downbeat/meter analysis, automatic BPM assignment, authoritative drum classification, seeded/random break rearrangement beyond reviewed whole-cell THIN and time stretch are unavailable; explicit seeded THIN, native slice descriptors/heuristic role suggestions and bounded reviewed timing-point hypotheses are available, while accepted markers and Use BPM remain explicit","arbitrary_effect_codes":"Only typed supported step commands accepted","async_cancellation":"Synchronous tools; client may terminate child process","legacy_experiments":"pattrns experiment panel and probe utilities are not exposed","gui_cursor_edit_mode":"No GUI cursor/navigation or Edit Mode control; MCP edits are explicit authorized transactions"},
            "limits":limits,
            "filesystem_policy":"Relative musical-artifact paths only; no parent traversal, symlinks or nonregular files. Existing projects require a revision-bound overwrite token; export and sample generation never overwrite."});
        if self.live_document_context {
            capabilities["unavailable"]["gui_instance_control"] = json!("This dispatcher is bound to the live GUI Editor and Session document. The host bridge exposes typed application actions and runtime state; raw UI automation, screenshots and rendered-text inspection are not provided.");
            capabilities["unavailable"]["live_transport"] = json!("Live Play/Stop/Loop/Follow/seek controls are exposed by the host GUI bridge; this document dispatcher does not open or directly command the audio device.");
            capabilities["unavailable"]["gui_cursor_edit_mode"] = json!("This document API does not change cursor/navigation or Edit Mode. Document edits preserve the GUI's current Edit Mode and honor its edit guards; raw keyboard or mouse automation is not provided.");
            capabilities["track_mix"]["scope"] = json!("Persisted gain/balance/mute/solo changes in this live GUI document; immediate playback updates are handled by the host runtime bridge only when the active snapshot matches.");
            capabilities["track_mix"]["unavailable"] = json!([
                "live_gui_meter_readback",
                "automation",
                "fx",
                "stems",
                "midi",
                "recording"
            ]);
        }
        capabilities
    }
    fn execute(&mut self, name: &str, arguments: Value) -> Outcome<Value> {
        match name {
            "leviathan_capabilities" => {
                let _: Empty = decode(arguments)?;
                Ok(self.capabilities())
            }
            "leviathan_project_read" => {
                let request: ReadProject = decode(arguments)?;
                let section = request.section.as_deref().unwrap_or("summary");
                let data = match section {
                    "summary" | "history" => self.state(),
                    "document" => json!({"song":self.editor.song,"session":self.state()}),
                    "tracks" => {
                        json!({"tracks":self.editor.song.tracks,"track_order":self.editor.song.track_order})
                    }
                    "track_mix" => bounded_track_mix(json!({
                        "settings":self.editor.song.track_mix,
                        "defaults":{"gain":1.0,"balance":0.0,"mute":false,"solo":false},
                        "compiled_layout_checked":false,"live_update_applied":false,
                        "source_identity_verified_now":false,"audio_rendered":false,
                        "notice":if self.live_document_context {"Persisted settings in the bound live GUI document. Omitted tracks use neutral defaults; any solo gates non-solo tracks, including a solo on a silent track, and mute wins. This read does not compile routing or claim a runtime update; use track_mix_preview for a complete-set review. The host bridge may apply saved values to a matching active snapshot."} else {"Persisted settings in this isolated headless Song only. Omitted tracks use neutral defaults; any solo gates non-solo tracks, including a solo on a silent track, and mute wins. This read does not compile routing or claim source/render/live readiness. Use track_mix_preview for a complete-set routing review; saved settings affect later starts and offline export, never an already-open GUI."}
                    }))?,
                    "instruments" => json!({"instruments":self.editor.song.instruments}),
                    "patterns" => json!({"patterns":self.editor.song.patterns}),
                    "generation" => {
                        let patterns=self.editor.song.patterns.iter().filter(|(_,pattern)|pattern.generation.is_some()).map(|(id,pattern)| {
                            let matches=crate::generation::content_matches(pattern).map_err(validation)?;
                            Ok(json!({"pattern_id":id,"provenance":pattern.generation,"content_matches":matches}))
                        }).collect::<Outcome<Vec<_>>>()?;
                        json!({"patterns":patterns,"notice":"Historical recipe and digest, not active playback state. Shared content_matches hashes stored rows/aliases/cells (not names/history); false after editing is valid and never regenerates content or resolves historical instrument IDs."})
                    }
                    "order" => json!({"order":self.editor.song.order}),
                    "phrases" => json!({"phrases":self.editor.song.phrases}),
                    "drum_kits" => json!({"drum_kits":self.editor.song.drum_kits}),
                    "multisamples" => json!({"multisamples":self.editor.song.multisamples}),
                    "samples" => json!({"samples":self.editor.song.samples}),
                    "regions" => json!({"regions":self.editor.song.regions}),
                    "grooves" => json!({"grooves":self.editor.song.grooves}),
                    "marker_sets" => json!({"marker_sets":self.editor.song.marker_sets}),
                    _ => return Err(Failure::new("invalid_arguments", "Unknown read section")),
                };
                Ok(data)
            }
            "leviathan_project_new" => {
                let request: NewProject = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.guard_discard(request.discard_unsaved)?;
                self.session
                    .request(&mut self.editor, Action::New)
                    .map_err(validation)?;
                if self.session.pending.is_some() {
                    self.session.discard(&mut self.editor).map_err(validation)?;
                }
                self.editor.edit_mode = true;
                self.advance();
                Ok(self.state())
            }
            "leviathan_project_open" => {
                let request: OpenProject = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.guard_discard(request.discard_unsaved)?;
                let path = self.workspace.resolve(&request.path, true)?;
                extension(&path, ".leviathan.json")?;
                let (song, path) = project::load_with_asset_policy(&path, |asset| {
                    self.workspace
                        .check(asset, true)
                        .map(|_| ())
                        .map_err(|e| e.message)
                })
                .map_err(validation)?;
                self.workspace.assets(&song)?;
                self.editor.replace_document(song).map_err(validation)?;
                self.editor.edit_mode = true;
                self.session.path = Some(path);
                self.session.cancel();
                self.advance();
                Ok(self.state())
            }
            "leviathan_project_prepare_overwrite" => {
                let request: PathRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let path = self.workspace.resolve(&request.path, true)?;
                extension(&path, ".leviathan.json")?;
                let bytes = project::existing_bytes(&path).map_err(io_failure)?;
                self.token_serial += 1;
                let token = format!("overwrite-{}-revision-{}", self.token_serial, self.revision);
                let result = json!({"path":self.workspace.display(&path),"existing_bytes":bytes.len(),"overwrite_token":token,"notice":"Explicitly pass this single-use token to save after approving replacement. Changed file bytes or revision invalidate it."});
                self.overwrite = Some(Overwrite {
                    token,
                    revision: self.revision,
                    path,
                    bytes,
                });
                Ok(result)
            }
            "leviathan_project_save" => {
                let request: SaveProject = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let path = if let Some(path) = request.path {
                    self.workspace.resolve(&path, false)?
                } else {
                    self.workspace.check(
                        self.session.path.as_ref().ok_or_else(|| {
                            Failure::new("invalid_arguments", "First save requires a path")
                        })?,
                        false,
                    )?
                };
                extension(&path, ".leviathan.json")?;
                self.workspace.assets(&self.editor.song)?;
                let mode = if let Some(token) = request.overwrite_token {
                    let pending = self.overwrite.take().ok_or_else(|| {
                        Failure::new(
                            "overwrite_confirmation_required",
                            "Prepare and review a fresh overwrite token first",
                        )
                    })?;
                    if token != pending.token
                        || path != pending.path
                        || self.revision != pending.revision
                    {
                        return Err(Failure::new(
                            "invalid_overwrite_token",
                            "Token does not match this path and revision",
                        ));
                    }
                    project::SaveMode::ReplaceConfirmed(pending.bytes)
                } else {
                    if path.exists() {
                        return Err(Failure::new(
                            "overwrite_confirmation_required",
                            "Existing project is protected; prepare and review an overwrite token",
                        ));
                    }
                    project::SaveMode::CreateNew
                };
                let (path, warning) =
                    project::save(&self.editor.song, &path, mode).map_err(io_failure)?;
                self.session.path = Some(path);
                self.editor.mark_saved();
                self.advance();
                Ok(json!({"session":self.state(),"warning":warning}))
            }
            "leviathan_edit_batch" => {
                let request: Batch = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if request.operations.is_empty() || request.operations.len() > MAX_OPERATIONS {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Batch must contain 1..256 ordered operations",
                    ));
                }
                let mut candidate = self.editor.song.clone();
                for (index, operation) in request.operations.iter().enumerate() {
                    let attempt = operation
                        .step_token()
                        .map(|token| self.checked_drum_steps(token))
                        .transpose()
                        .and_then(|steps| operation.apply(&mut candidate, &self.workspace, steps))
                        .and_then(|()| {
                            // Check each step, not only the final result: many clones
                            // must not amplify a bounded document into gigabytes first.
                            if serde_json::to_vec(&candidate).map_err(io_failure)?.len()
                                > MAX_DOCUMENT
                            {
                                Err(Failure::new(
                                    "document_too_large",
                                    "Candidate exceeds 8 MiB; batch was not committed",
                                ))
                            } else {
                                Ok(())
                            }
                        });
                    attempt.map_err(|mut error| {
                        error.details = json!({"operation_index":index,"atomic":true});
                        error
                    })?;
                }
                self.workspace.assets(&candidate)?;
                let editor_revision = self.editor.revision();
                self.editor
                    .apply_document_edit(candidate)
                    .map_err(validation)?;
                // Editor history is authoritative, including signed-zero-only edits.
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                }
                Ok(
                    json!({"changed":changed,"applied_operations":request.operations.len(),"session":self.state()}),
                )
            }
            "leviathan_history_undo" | "leviathan_history_redo" => {
                let request: Revision = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let changed = if name.ends_with("undo") {
                    self.editor.can_undo()
                } else {
                    self.editor.can_redo()
                };
                if changed {
                    if name.ends_with("undo") {
                        self.editor.undo();
                    } else {
                        self.editor.redo();
                    }
                    self.advance();
                }
                Ok(json!({"changed":changed,"session":self.state()}))
            }
            "leviathan_song_evaluate" => {
                let request: Revision = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.workspace.assets(&self.editor.song)?;
                let manifest =
                    song_render::evaluate(&self.editor.song, None).map_err(validation)?;
                Ok(json!({"manifest":manifest,"source_identity_verified":false}))
            }
            "leviathan_drum_steps_inspect" => {
                let request: InspectDrumSteps = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let view = crate::drum_steps::StepView {
                    pattern: PatternId(request.pattern_id),
                    track: TrackId(request.track_id),
                    instrument: InstrumentId(request.instrument_id),
                    division: request.division,
                    steps: request.steps,
                    page: request.page,
                };
                let projection =
                    crate::drum_steps::project(&self.editor.song, &view).map_err(validation)?;
                let serial = self.token_serial + 1;
                let token = format!("drum-steps-{serial}-revision-{}", self.revision);
                let data = json!({"projection_token":token,"projection":projection,"document_changed":false,"grid_origin":"pattern row 0","delay_units":"absolute fraction of the source tracker row, not a step","source_identity_verified":false,"notice":"Read-only projection of ordinary cells; resolved alias source/read_only is explicit and all alias target edits require Make unique. Conflicted slots cannot toggle; exact owned source events may use field edits. No quantization, automatic column expansion or second pattern store."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("drum_step_projection_limit", "Projection exceeds 256 KiB structured payload; use a smaller page or the tracker. Previous token is preserved"));
                }
                self.token_serial = serial;
                self.drum_steps = Some(PendingDrumSteps {
                    token,
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    projection,
                });
                Ok(data)
            }
            "leviathan_matrix_inspect" => {
                let request: InspectMatrix = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let song = &self.editor.song;
                if !(1..=32).contains(&request.occurrence_limit)
                    || !(1..=32).contains(&request.track_limit)
                    || request.occurrence_start > song.order.len()
                    || request.track_start > song.track_order.len()
                {
                    return Err(Failure::new("invalid_arguments", "Matrix limits must be 1..32; starts may equal but not exceed current occurrence/track totals"));
                }
                let occurrence_end = request
                    .occurrence_start
                    .saturating_add(request.occurrence_limit)
                    .min(song.order.len());
                let track_end = request
                    .track_start
                    .saturating_add(request.track_limit)
                    .min(song.track_order.len());
                let mut blocks = Vec::new();
                for occurrence_index in request.occurrence_start..occurrence_end {
                    let occurrence = &song.order[occurrence_index];
                    let pattern = &song.patterns[&occurrence.pattern];
                    for &track_id in &song.track_order[request.track_start..track_end] {
                        let (source_pattern, source_track) = song
                            .resolve_track_source(occurrence.pattern, track_id)
                            .map_err(validation)?;
                        let source_chain = song
                            .resolve_track_chain(occurrence.pattern, track_id)
                            .map_err(validation)?;
                        let local_cells = pattern
                            .tracks
                            .get(&track_id)
                            .map_or(0, |rows| rows.values().map(BTreeMap::len).sum::<usize>());
                        let resolved_cells = song
                            .resolved_track(occurrence.pattern, track_id)
                            .map_err(validation)?
                            .map_or(0, |rows| rows.values().map(BTreeMap::len).sum::<usize>());
                        blocks.push(json!({"occurrence_index":occurrence_index,"pattern_id":occurrence.pattern,"track_id":track_id,"muted":occurrence.muted_tracks.contains(&track_id),"direct_alias":pattern.track_aliases.get(&track_id),"source_chain":source_chain,"resolved_source":{"pattern":source_pattern,"track":source_track},"rows":pattern.rows,"note_columns":song.tracks[&track_id].note_columns,"stored_local_cell_count":local_cells,"resolved_cell_count":resolved_cells,"local_editable":!pattern.track_aliases.contains_key(&track_id)}));
                    }
                }
                let data = json!({"occurrence_start":request.occurrence_start,"track_start":request.track_start,"occurrences_total":song.order.len(),"tracks_total":song.track_order.len(),"next_occurrence_start":(occurrence_end<song.order.len()).then_some(occurrence_end),"next_track_start":(track_end<song.track_order.len()).then_some(track_end),"blocks":blocks,"document_changed":false,"notice":"Indices address current order/display order, not stable occurrence IDs. Aliases share scalar cell content only; normal target edits require Make unique. Read stored source cells through project_read patterns using resolved source IDs."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("matrix_inspection_limit", "Matrix inspection exceeds 256 KiB structured payload; reduce either page limit"));
                }
                Ok(data)
            }
            "leviathan_song_export" => {
                let request: PathRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let path = self.workspace.resolve(&request.path, false)?;
                extension(&path, ".wav")?;
                let manifest = self
                    .workspace
                    .check(&path.with_extension("events.json"), false)?;
                if path.exists() || manifest.exists() {
                    return Err(Failure::new(
                        "output_exists",
                        "WAV and companion events.json must both be new",
                    ));
                }
                self.workspace.assets(&self.editor.song)?;
                let (stats, _) =
                    song_render::export(&self.editor.song, None, &path).map_err(validation)?;
                Ok(
                    json!({"wav_path":self.workspace.display(&path),"manifest_path":self.workspace.display(&manifest),"stats":stats,"sample_rate":crate::RATE,"channels":2}),
                )
            }
            "leviathan_sample_generate" => {
                let request: PathRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let path = self.workspace.resolve(&request.path, false)?;
                extension(&path, ".wav")?;
                if path.exists() {
                    return Err(Failure::new(
                        "output_exists",
                        "Sample generation never overwrites existing files",
                    ));
                }
                crate::generate_sample(&path).map_err(io_failure)?;
                Ok(
                    json!({"sample_path":self.workspace.display(&path),"sample_rate":crate::RATE,"frames":crate::RATE,"channels":2,"assigned":false}),
                )
            }
            "leviathan_sample_import" => {
                let request: ImportSample = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if request.sample_id == 0 {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Sample ID must be nonzero",
                    ));
                }
                let path = self.workspace.resolve(&request.path, true)?;
                let imported = crate::sample_asset::import(&path).map_err(validation)?;
                let mut candidate = self.editor.song.clone();
                candidate
                    .register_sample(SampleId(request.sample_id), imported.asset.clone())
                    .map_err(validation)?;
                if serde_json::to_vec(&candidate).map_err(io_failure)?.len() > MAX_DOCUMENT {
                    return Err(Failure::new(
                        "document_too_large",
                        "Candidate exceeds 8 MiB; import was not committed",
                    ));
                }
                self.workspace.assets(&candidate)?;
                let editor_revision = self.editor.revision();
                self.editor
                    .apply_document_edit(candidate)
                    .map_err(validation)?;
                // Editor history is authoritative, including signed-zero-only edits.
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                }
                Ok(
                    json!({"sample_id":request.sample_id,"sample":imported.asset,"changed":changed,"assigned":false,"session":self.state()}),
                )
            }
            "leviathan_multisample_inspect" => {
                let request: InspectMultisample = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if !(1..=MAX_KEYZONE_PAGE).contains(&request.zones_limit)
                    || !(1..=MAX_KEYZONE_PAGE).contains(&request.instruments_limit)
                {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Multisample zone/instrument page limits must each be1..64",
                    ));
                }
                let map = self
                    .editor
                    .song
                    .multisamples
                    .get(&MultisampleId(request.map_id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown multisample map ID"))?;
                let assigned = || {
                    self.editor.song.instruments.iter().filter(|(_,instrument)|
                    matches!(&instrument.kind, InstrumentKind::Multisample { map } if map.0 == request.map_id))
                };
                let instrument_total = assigned().count();
                if request.zones_start > map.zones.len()
                    || request.instruments_start > instrument_total
                {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Multisample page start exceeds its independent total",
                    ));
                }
                let zones: BTreeMap<_, _> = map
                    .zones
                    .iter()
                    .skip(request.zones_start)
                    .take(request.zones_limit)
                    .collect();
                let instruments: Vec<_> = assigned()
                    .skip(request.instruments_start)
                    .take(request.instruments_limit)
                    .map(|(id, instrument)| json!({"instrument_id":id,"name":instrument.name}))
                    .collect();
                let zone_end = request.zones_start + zones.len();
                let instrument_end = request.instruments_start + instruments.len();
                let data = json!({"map_id":request.map_id,"name":map.name,"zones":zones,"assigned_instruments":instruments,
                    "zones_page":{"start":request.zones_start,"total":map.zones.len(),"next_start":(zone_end<map.zones.len()).then_some(zone_end)},
                    "instruments_page":{"start":request.instruments_start,"total":instrument_total,"next_start":(instrument_end<instrument_total).then_some(instrument_end)},
                    "document_changed":false,"source_identity_verified":false,
                    "notice":"Mutable shared map: edits affect every assigned instrument and later render of existing notes. Inclusive disjoint note/velocity rectangles; gaps are valid drafts but an uncovered actual ON rejects in evaluation/export. MIDI note is audible pitch relative to explicit zone root, not a pad selector. No source access or independent selection here."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("inspection_too_large", "Multisample inspection exceeds256KiB; reduce page limits or oversized instrument names"));
                }
                Ok(data)
            }
            "leviathan_sample_inspect" => {
                let request: SampleRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if !(1..=MAX_SOURCE_INSPECTION_PAGE).contains(&request.regions_limit)
                    || !(1..=MAX_SOURCE_INSPECTION_PAGE).contains(&request.mappings_limit)
                    || !(1..=MAX_SOURCE_INSPECTION_PAGE).contains(&request.keyzones_limit)
                {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Source region/kit mapping/keyzone page limits must each be 1..128",
                    ));
                }
                let asset = self
                    .editor
                    .song
                    .samples
                    .get(&SampleId(request.sample_id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown sample ID"))?;
                let path = self.workspace.check(&asset.path, true)?;
                let _buffer = crate::sample_asset::load_verified(
                    asset,
                    &path,
                    crate::sample_asset::MAX_SAMPLE_VALUES,
                )
                .map_err(validation)?;
                let region_total = self
                    .editor
                    .song
                    .regions
                    .values()
                    .filter(|region| region.sample.0 == request.sample_id)
                    .count();
                if request.regions_start > region_total {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "regions_start exceeds the source region total",
                    ));
                }
                let regions: BTreeMap<_, _> = self
                    .editor
                    .song
                    .regions
                    .iter()
                    .filter(|(_, region)| region.sample.0 == request.sample_id)
                    .skip(request.regions_start)
                    .take(request.regions_limit)
                    .collect();
                let mut kit_pad_mappings = Vec::new();
                let mut mapping_total = 0;
                let matches_source = |region: &RegionId| {
                    self.editor
                        .song
                        .regions
                        .get(region)
                        .is_some_and(|region| region.sample.0 == request.sample_id)
                };
                for (kit_id, kit) in &self.editor.song.drum_kits {
                    for (pad_id, pad) in &kit.pads {
                        if let Some(region) = pad.region.filter(|region| matches_source(region)) {
                            if mapping_total >= request.mappings_start
                                && kit_pad_mappings.len() < request.mappings_limit
                            {
                                kit_pad_mappings.push(json!({"kit_id":kit_id,"pad_id":pad_id,"slot":pad.slot,"selector_note":pad.note,"region_id":region}));
                            }
                            mapping_total += 1;
                        }
                        for (zone_index, zone) in pad.velocity_zones.iter().enumerate() {
                            for (alternative_index, layer) in zone.alternatives.iter().enumerate() {
                                if matches_source(&layer.region) {
                                    if mapping_total >= request.mappings_start
                                        && kit_pad_mappings.len() < request.mappings_limit
                                    {
                                        kit_pad_mappings.push(json!({"kit_id":kit_id,"pad_id":pad_id,"slot":pad.slot,"selector_note":pad.note,"region_id":layer.region,"zone_index":zone_index,"min_velocity":zone.min_velocity,"max_velocity":zone.max_velocity,"layer_id":layer.id,"alternative_index":alternative_index}));
                                    }
                                    mapping_total += 1;
                                }
                            }
                        }
                    }
                }
                if request.mappings_start > mapping_total {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "mappings_start exceeds the source mapping total",
                    ));
                }
                let all_keyzones = || {
                    self.editor
                        .song
                        .multisamples
                        .iter()
                        .flat_map(|(map_id, map)| {
                            let matches_source = &matches_source;
                            map.zones.iter().filter_map(move |(zone_id, zone)| {
                                matches_source(&zone.region).then_some((map_id, zone_id, zone))
                            })
                        })
                };
                let keyzone_total = all_keyzones().count();
                if request.keyzones_start > keyzone_total {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "keyzones_start exceeds the source multisample mapping total",
                    ));
                }
                let multisample_mappings: Vec<_> = all_keyzones().skip(request.keyzones_start).take(request.keyzones_limit)
                    .map(|(map_id,zone_id,zone)|json!({"map_id":map_id,"zone_id":zone_id,"zone":zone})).collect();
                let keyzone_end = request.keyzones_start + multisample_mappings.len();
                let region_end = request.regions_start + regions.len();
                let mapping_end = request.mappings_start + kit_pad_mappings.len();
                let data = json!({"sample_id":request.sample_id,"sample":asset,"identity_and_metadata_verified":true,"regions":regions,"kit_pad_mappings":kit_pad_mappings,"multisample_mappings":multisample_mappings,
                    "regions_page":{"start":request.regions_start,"total":region_total,"next_start":(region_end < region_total).then_some(region_end)},
                    "mappings_page":{"start":request.mappings_start,"total":mapping_total,"next_start":(mapping_end < mapping_total).then_some(mapping_end)},
                    "keyzones_page":{"start":request.keyzones_start,"total":keyzone_total,"next_start":(keyzone_end < keyzone_total).then_some(keyzone_end)},
                    "notice":"Independent bounded pages: follow each next_start to inspect all regions and source mappings. Pad zone and alternative indices are zero-based vector positions; layer IDs are stable. Multisample map/keyzone IDs are stable, bounds inclusive; mapping membership is independent of the region page. No document changes."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("inspection_too_large", "Source inspection exceeds 256 KiB; reduce page limits or oversized region names"));
                }
                Ok(data)
            }
            "leviathan_sample_analyze" => {
                let request: AnalyzeSample = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let asset = self
                    .editor
                    .song
                    .samples
                    .get(&SampleId(request.sample_id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown sample ID"))?;
                let mut config = crate::break_analysis::AnalysisConfig {
                    max_frames: (u64::from(asset.sample_rate) * MAX_ANALYSIS_SECONDS)
                        .min(crate::sample_asset::MAX_SAMPLE_VALUES as u64),
                    ..Default::default()
                };
                if let Some(v) = request.config.sensitivity {
                    config.sensitivity = v;
                }
                if let Some(v) = request.config.min_spacing_ms {
                    config.min_spacing_ms = v;
                }
                if let Some(v) = request.config.minimum_rms {
                    config.minimum_rms = v;
                }
                if let Some(v) = request.config.max_suggestions {
                    config.max_suggestions = v;
                }
                if let Some(v) = request.config.waveform_bins {
                    config.waveform_bins = v;
                }
                if asset.frames > config.max_frames || !(8000..=384000).contains(&asset.sample_rate)
                {
                    return Err(Failure::new("analysis_limit", "Analysis requires a source at 8000..384000 Hz, at most 60 seconds and 64 MiB decoded; no truncation is performed"));
                }
                if !(0.0..=1.0).contains(&config.sensitivity)
                    || !(1.0..=1000.0).contains(&config.min_spacing_ms)
                    || !(0.0..=1.0).contains(&config.minimum_rms)
                    || !(1..=MAX_ANALYSIS_SUGGESTIONS).contains(&config.max_suggestions)
                    || config.waveform_bins > MAX_ANALYSIS_BINS
                {
                    return Err(Failure::new("invalid_arguments", "Analysis config exceeds advertised sensitivity/spacing/floor/output limits"));
                }
                let path = self.workspace.check(&asset.path, true)?;
                let buffer = crate::sample_asset::load_verified(
                    asset,
                    &path,
                    crate::sample_asset::MAX_SAMPLE_VALUES,
                )
                .map_err(validation)?;
                let values = (asset.frames * u64::from(asset.channels)) as usize;
                let analysis = crate::break_analysis::analyze_transients(
                    &buffer.buffer()[..values],
                    usize::from(asset.channels),
                    asset.sample_rate,
                    &config,
                )
                .map_err(|e| Failure::new("analysis_error", e.to_string()))?;
                Ok(
                    json!({"sample_id":request.sample_id,"source_sha256":asset.sha256,"analysis":analysis,"document_changed":false,"frame_units":"original_source_frames","notice":"Suggestions only, not accepted markers or calibrated probabilities. Use explicit region edits to accept boundaries; BPM and source audio are unchanged."}),
                )
            }
            "leviathan_marker_cleanup_preview" => {
                let request: CleanupMarkers = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if request.candidates.len() > MAX_ANALYSIS_SUGGESTIONS {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Cleanup accepts at most 4096 explicit candidate source frames",
                    ));
                }
                let asset = self
                    .editor
                    .song
                    .samples
                    .get(&SampleId(request.sample_id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown sample ID"))?;
                let path = self.workspace.check(&asset.path, true)?;
                drop(
                    crate::sample_asset::load_verified(
                        asset,
                        &path,
                        crate::sample_asset::MAX_SAMPLE_VALUES,
                    )
                    .map_err(validation)?,
                );
                let cleanup = crate::break_compiler::suggest_marker_cleanup(
                    asset,
                    &request.candidates,
                    request.minimum_slice_frames,
                )
                .map_err(validation)?;
                let data = json!({"sample_id":request.sample_id,"source_sha256":asset.sha256,"source_identity_verified":true,"cleanup":cleanup,"document_changed":false,"notice":"Review removals and proposed boundaries, then explicitly upsert an accepted marker set. No attacks are snapped or automatically accepted."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Cleanup response exceeds 256 KiB; no changes applied",
                    ));
                }
                Ok(data)
            }
            "leviathan_sample_peaks_analyze" => {
                let request: AnalyzeSamplePeaks = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new(
                        "token_exhausted",
                        "No fresh sample-peak analysis tokens remain",
                    )
                })?;
                let owner = self.editor.document_owner();
                let editor_revision = self.editor.revision();
                let plan = crate::meters::prepare(&self.editor.song, request.config)
                    .map_err(validation)?;
                let mut path_failure = None;
                let result = crate::meters::analyze(
                    &plan,
                    |path| {
                        self.workspace
                            .check(path, true)
                            .map(|_| ())
                            .map_err(|failure| {
                                let message = failure.message.clone();
                                path_failure = Some(failure);
                                message
                            })
                    },
                    || false,
                );
                let analysis = result
                    .map_err(|message| path_failure.unwrap_or_else(|| validation(message)))?;
                self.expect_revision(request.expected_revision)?;
                if editor_revision != self.editor.revision()
                    || !self.editor.owns_document(&owner)
                    || !project::same_serialized_song(plan.song(), &self.editor.song)
                        .map_err(validation)?
                {
                    return Err(Failure::new("sample_peaks_stale", "Document changed during offline analysis; previous completed report is unchanged"));
                }
                let pending = PendingSamplePeaks {
                    token: format!("sample-peaks-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    editor_revision,
                    owner,
                    plan,
                    analysis,
                };
                let data = sample_peak_header(&pending)?;
                self.token_serial = serial;
                self.sample_peaks = Some(pending);
                Ok(data)
            }
            "leviathan_sample_peaks_inspect" => {
                let request: InspectSamplePeaks = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_sample_peaks(&request.analysis_token)?;
                bounded_sample_peaks(json!({
                    "analysis_token":pending.token,"analysis_id":pending.analysis.id,"plan_id":pending.plan.id(),
                    "analysis_revision":pending.revision,"analysis_editor_revision":pending.editor_revision,
                    "config":pending.analysis.config,"sample_rate":crate::RATE,"layout":pending.analysis.summary.layout,
                    "verified_sources_at_analysis":pending.analysis.verified_sources,
                    "page":sample_peak_page(pending,request.start,request.limit)?,
                    "document_changed":false,"source_identity_verified_now":false,"pcm_rendered":false,
                    "pcm_returned":false,"files_written":false,"audio_device_started":false,"live_gui_observed":false,
                    "scope":"retained_offline_song_sample_peaks",
                    "notice":"Metadata-only page of the prior complete offline measurement; no path/hash/decode/render now and no promise that source files still match. Exact current document owner/revisions/full Song guard applies. Page start is a zero-based index into selected windows, while report_index and emitted/Song positions retain their full-context coordinates. track_peaks slots match layout.tracks; null is unavailable, not silence. No live GUI, device or heard-output claim."
                }))
            }
            "leviathan_track_mix_preview" => {
                let request: PreviewTrackMix = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let preview = self
                    .editor
                    .capture_track_mix_document()
                    .map_err(validation)?
                    .preview_track_mix(request.config)
                    .map_err(validation)?;
                self.editor
                    .validate_track_mix_preview(&preview)
                    .map_err(validation)?;
                if serde_json::to_vec(preview.preview().candidate())
                    .map_err(io_failure)?
                    .len()
                    > MAX_DOCUMENT
                {
                    return Err(Failure::new("document_too_large", "Track-balance candidate exceeds 8 MiB; document and prior review unchanged"));
                }
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new(
                        "token_exhausted",
                        "No fresh track-balance review tokens remain",
                    )
                })?;
                let pending = PendingTrackMix {
                    token: format!("track-mix-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    preview,
                };
                let data = track_mix_header(&pending)?;
                self.token_serial = serial;
                self.track_mix = Some(pending);
                Ok(data)
            }
            "leviathan_track_mix_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_track_mix(&request.preview_token)?;
                let preview = pending.preview.preview();
                // Pre-size the complete history-only receipt before mutation.
                // False is longer than true, so finalized booleans cannot grow it.
                let mut data = bounded_track_mix(json!({
                    "changed":false,"document_changed":false,"preview_consumed":true,
                    "preview_id":preview.id(),"requested_config":preview.config(),"stored_settings":preview.candidate().track_mix,"summary":preview.summary(),
                    "source_identity_verified_now":false,"audio_rendered":false,
                    "files_changed":false,"live_update_applied":false,
                    "session":{"dirty":false,"can_undo":false,"can_redo":false},
                    "session_scope":"history_only","omitted_session_fields":["title","path","document_counts"],
                    "notice":"Committed the reviewed complete persisted track-balance set in this headless Song only. Shared Editor owner/revision/exact-base guard and Edit Mode protection apply; changed settings are one Undo, a true no-op preserves redo/dirty/revision. No live mailbox, GUI, transport or audio-device update. Future starts and offline exports use saved/current settings subject to normal routing and source policies. No path/file/hash/decode/render work occurred."
                }))?;
                let editor_revision = self.editor.revision();
                let pending = self.track_mix.as_ref().unwrap();
                self.editor
                    .commit_track_mix_preview(&pending.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                } else {
                    self.track_mix = None;
                }
                data["changed"] = json!(changed);
                data["document_changed"] = json!(changed);
                data["session"] = json!({"dirty":self.editor.dirty,"can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo()});
                Ok(data)
            }
            "leviathan_cell_block_copy" => {
                let request: CopyCellBlock = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let captured = self.editor.capture_cell_document().map_err(validation)?;
                let clipboard = captured.copy_cells(&request.scope).map_err(validation)?;
                self.editor
                    .validate_cell_copy(&clipboard)
                    .map_err(validation)?;
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_exhausted", "No fresh clipboard/review tokens remain")
                })?;
                let pending = PendingCellClipboard {
                    token: format!("cell-clipboard-{serial}-revision-{}", self.revision),
                    copied_at_revision: self.revision,
                    clipboard,
                };
                let data = cell_clipboard_header(&pending)?;
                self.token_serial = serial;
                self.cell_clipboard = Some(pending);
                self.cell_block = None;
                Ok(data)
            }
            "leviathan_cell_block_preview" => {
                let request: PreviewCellBlock = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let preview = match request.operation {
                    CellBlockOperation::PasteReplace {
                        clipboard_token,
                        destination,
                    } => {
                        let pending = self.checked_cell_clipboard(&clipboard_token)?;
                        self.editor
                            .capture_cell_document()
                            .map_err(validation)?
                            .paste_cells_preview(&pending.clipboard, &destination)
                            .map_err(validation)?
                    }
                    CellBlockOperation::Clear { scope } => self
                        .editor
                        .capture_cell_document()
                        .map_err(validation)?
                        .clear_cells_preview(&scope)
                        .map_err(validation)?,
                };
                self.editor
                    .validate_cell_preview(&preview)
                    .map_err(validation)?;
                if serde_json::to_vec(preview.preview().candidate())
                    .map_err(io_failure)?
                    .len()
                    > MAX_DOCUMENT
                {
                    return Err(Failure::new("document_too_large", "Rectangle candidate exceeds 8 MiB; clipboard, document and previous review unchanged"));
                }
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_exhausted", "No fresh clipboard/review tokens remain")
                })?;
                let pending = PendingCellBlock {
                    token: format!("cell-block-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    preview,
                };
                let data = cell_block_header(&pending)?;
                self.token_serial = serial;
                self.cell_block = Some(pending);
                Ok(data)
            }
            "leviathan_cell_block_inspect" => {
                let request: InspectCellBlock = decode(arguments)?;
                let (revision, start, limit) = match &request {
                    InspectCellBlock::Clipboard {
                        expected_revision,
                        start,
                        limit,
                        ..
                    }
                    | InspectCellBlock::Changes {
                        expected_revision,
                        start,
                        limit,
                        ..
                    }
                    | InspectCellBlock::Contexts {
                        expected_revision,
                        start,
                        limit,
                        ..
                    }
                    | InspectCellBlock::Aliases {
                        expected_revision,
                        start,
                        limit,
                        ..
                    } => (*expected_revision, *start, *limit),
                };
                self.expect_revision(revision)?;
                if !(1..=crate::cell_blocks::MAX_PAGE).contains(&limit) {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Rectangle inspection requires a limit of 1..64 records",
                    ));
                }
                let (mut data, kind, records, total, end) = match request {
                    InspectCellBlock::Clipboard {
                        clipboard_token, ..
                    } => {
                        let pending = self.checked_cell_clipboard(&clipboard_token)?;
                        let snapshot = pending.clipboard.snapshot();
                        if start > snapshot.cells().len() {
                            return Err(Failure::new("invalid_arguments", "Clipboard page start exceeds the copied position count; empty end pages are allowed"));
                        }
                        let page = snapshot.page(start, limit).map_err(validation)?;
                        let end = page.start + page.cells.len();
                        (
                            cell_clipboard_header(pending)?,
                            "clipboard",
                            serde_json::to_value(page.cells).map_err(io_failure)?,
                            page.total,
                            end,
                        )
                    }
                    InspectCellBlock::Changes { preview_token, .. } => {
                        let pending = self.checked_cell_block(&preview_token)?;
                        let (records, total, end) =
                            cell_block_records(pending.preview.preview().changes(), start, limit)?;
                        (cell_block_header(pending)?, "changes", records, total, end)
                    }
                    InspectCellBlock::Contexts { preview_token, .. } => {
                        let pending = self.checked_cell_block(&preview_token)?;
                        let (records, total, end) =
                            cell_block_records(pending.preview.preview().contexts(), start, limit)?;
                        (cell_block_header(pending)?, "contexts", records, total, end)
                    }
                    InspectCellBlock::Aliases { preview_token, .. } => {
                        let pending = self.checked_cell_block(&preview_token)?;
                        let (records, total, end) =
                            cell_block_records(pending.preview.preview().aliases(), start, limit)?;
                        (cell_block_header(pending)?, "aliases", records, total, end)
                    }
                };
                data["kind"] = json!(kind);
                data["records"] = records;
                data["page"] =
                    json!({"start":start,"total":total,"next_start":(end<total).then_some(end)});
                bounded_cell_block(data)
            }
            "leviathan_cell_block_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_cell_block(&request.preview_token)?;
                let preview = pending.preview.preview();
                // False is the longer boolean spelling; bound the entire receipt
                // before mutation, without echoing arbitrary title/project paths.
                let mut data = bounded_cell_block(json!({
                    "changed":false,"document_changed":false,"preview_id":preview.id(),
                    "request":preview.request(),"summary":preview.summary(),"preview_consumed":true,
                    "source_identity_verified_now":false,"audio_rendered":false,"files_changed":false,
                    "session":{"dirty":false,"can_undo":false,"can_redo":false},
                    "session_scope":"history_only","omitted_session_fields":["title","path","document_counts"],
                    "notice":"Committed only the reviewed ordinary whole-cell replacement/clear through the shared Editor owner/revision/exact-base guard and Edit Mode protection. Changed content is one Undo entry. No-op consumes only the review and preserves revision/history/dirty/redo; the same-document frozen clipboard remains. No path/file access, audio/source verification, source-ID remapping or playback-equivalence claim."
                }))?;
                let editor_revision = self.editor.revision();
                let pending = self.cell_block.as_ref().unwrap();
                self.editor
                    .commit_cell_block_preview(&pending.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                } else {
                    self.cell_block = None;
                }
                data["changed"] = json!(changed);
                data["document_changed"] = json!(changed);
                data["session"] = json!({"dirty":self.editor.dirty,"can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo()});
                Ok(data)
            }
            "leviathan_marker_bank_preview" => {
                let request: PreviewMarkerBank = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                // Deliberately pure: missing/changed source files do not prevent
                // reviewing accepted metadata. No path preflight or source IO.
                let preview = crate::marker_bank::preview(&self.editor.song, &request.config)
                    .map_err(validation)?;
                if serde_json::to_vec(preview.candidate())
                    .map_err(io_failure)?
                    .len()
                    > MAX_DOCUMENT
                {
                    return Err(Failure::new("document_too_large", "Marker-bank candidate exceeds 8 MiB; document and previous review unchanged"));
                }
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_exhausted", "No fresh review tokens remain")
                })?;
                let token = format!("marker-bank-{serial}-revision-{}", self.revision);
                let data = bounded_marker_bank(json!({
                    "preview_token":token,"preview_id":preview.id(),
                    "effective_config":preview.config(),"summary":preview.summary(),
                    "source_path":preview.source().path,"source_identity_verified_now":false,
                    "source_path_authorized_now":false,"audio_decoded":false,"document_changed":false,
                    "notice":"Pure accepted-marker metadata preview; no source-file access, BPM inference or document mutation. Every selected index is one exact adjacent-boundary interval; sparse selections are allowed, never merged/sorted/truncated. Review all fresh region/kit/instrument IDs and safe pad defaults. One retained opaque plan: successful preview supersedes the previous marker-bank review, failure preserves it. Commit separately requires current token, both revisions, exact base and a fresh authorized SHA check of this selected source only. No audio files, patterns, order or clock are changed."
                }))?;
                self.token_serial = serial;
                self.marker_bank = Some(PendingMarkerBank {
                    token,
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    preview,
                });
                Ok(data)
            }
            "leviathan_marker_bank_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_marker_bank(&request.preview_token)?;
                // Pre-size a history-only receipt before IO/mutation; never echo
                // an unbounded document title/path after a successful commit.
                let mut data = bounded_marker_bank(json!({
                    "changed":true,"preview_id":pending.preview.id(),"summary":pending.preview.summary(),
                    "source_identity_verified_now":true,"source_path_authorized_now":true,
                    "all_source_identities_verified_now":false,"audio_decoded":false,
                    "source_files_changed":false,"preview_consumed":true,
                    "session":{"dirty":false,"can_undo":false,"can_redo":false},
                    "session_scope":"history_only","omitted_session_fields":["title","path","document_counts"],
                    "notice":"Committed only the reviewed fresh regions, kit/pads and assigned instrument as one Undo transaction. Selected source bytes were freshly SHA-verified without decoding; other sources were not inspected. The shared summary remains the original declared-metadata preview, not a whole-project save/render-readiness proof. Existing patterns, order, clock, markers, source metadata/audio and live transport are unchanged. No MIDI or automatic role/tempo inference."
                }))?;
                let source = pending.preview.source();
                let path = self.workspace.check(&source.path, true)?;
                crate::sample_asset::verify_identity(source, &path).map_err(validation)?;
                // Recheck after the control-side hash, before the shared pure
                // Editor transaction; no caller-supplied candidate is accepted.
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_marker_bank(&request.preview_token)?;
                let editor_revision = pending.editor_revision;
                let pending = self.marker_bank.as_ref().unwrap();
                self.editor
                    .commit_marker_bank_preview(editor_revision, &pending.preview)
                    .map_err(validation)?;
                self.advance();
                data["session"] = json!({"dirty":self.editor.dirty,"can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo()});
                Ok(data)
            }
            "leviathan_break_compile_preview" => {
                let request: CompileRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let marker_set = self
                    .editor
                    .song
                    .marker_sets
                    .get(&request.config.marker_set)
                    .ok_or_else(|| Failure::new("not_found", "Unknown accepted marker set ID"))?;
                let sample_id = marker_set.sample;
                let asset = &self.editor.song.samples[&sample_id];
                let path = self.workspace.check(&asset.path, true)?;
                // Check the actual immutable source metadata as well as bytes
                // before a pure compiler uses source-frame boundaries.
                drop(
                    crate::sample_asset::load_verified(
                        asset,
                        &path,
                        crate::sample_asset::MAX_SAMPLE_VALUES,
                    )
                    .map_err(validation)?,
                );
                let preview = crate::break_compiler::preview(&self.editor.song, &request.config)
                    .map_err(validation)?;
                if serde_json::to_vec(preview.candidate())
                    .map_err(io_failure)?
                    .len()
                    > MAX_DOCUMENT
                {
                    return Err(Failure::new(
                        "document_too_large",
                        "Compiled candidate exceeds 8 MiB; no preview or edit was accepted",
                    ));
                }
                self.workspace.assets(preview.candidate())?;
                let next_serial = self.token_serial + 1;
                let token = format!("compile-{next_serial}-revision-{}", self.revision);
                let data = json!({"preview_token":token,"compiler_preview_id":preview.id(),"config":preview.config(),"diff":preview.summary(),"sample_id":sample_id,"source_sha256":asset.sha256,"source_identity_verified":true,"document_changed":false,"notice":"Review the shared compiler diff before commit. This replaces the previous pending preview, not the document. Any successful revision change invalidates the token."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Preview exceeds 256 KiB; no preview or edit was accepted",
                    ));
                }
                self.token_serial = next_serial;
                self.compile_preview = Some(PendingCompile {
                    token,
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    sample_id,
                    preview,
                });
                Ok(data)
            }
            "leviathan_break_compile_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.compile_preview.as_ref().ok_or_else(|| {
                    Failure::new(
                        "preview_unavailable",
                        "Create and review a fresh compiler preview at the current revision",
                    )
                })?;
                if request.preview_token != pending.token || pending.revision != self.revision {
                    return Err(Failure::new("invalid_preview_token", "Token does not identify the current pending preview; document and current preview are unchanged"));
                }
                let asset = &pending.preview.base().samples[&pending.sample_id];
                let path = self.workspace.check(&asset.path, true)?;
                crate::sample_asset::verify_identity(asset, &path).map_err(validation)?;
                self.workspace.assets(pending.preview.candidate())?;
                let summary =
                    serde_json::to_value(pending.preview.summary()).map_err(io_failure)?;
                // Shared Editor generation + exact base guards reject ABA edits,
                // undo, New/Open and direct persistent-field mutations.
                self.editor
                    .commit_preview(pending.editor_revision, &pending.preview)
                    .map_err(validation)?;
                self.advance();
                Ok(
                    json!({"changed":true,"diff":summary,"session":self.state(),"preview_consumed":true}),
                )
            }
            "leviathan_thin_preview" => {
                let request: PreviewThin = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.workspace.assets(&self.editor.song)?;
                let preview = crate::mutations::preview(&self.editor.song, request.config)
                    .map_err(validation)?;
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_limit", "Session token counter exhausted")
                })?;
                let pending = PendingThin {
                    token: format!("thin-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    preview,
                };
                let data = thin_header(&pending)?;
                self.token_serial = serial;
                self.thin = Some(pending);
                Ok(data)
            }
            "leviathan_thin_inspect" => {
                let request: InspectThin = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if request.limit == 0 || request.limit > MAX_THIN_PAGE {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "THIN page limit must be1..64",
                    ));
                }
                let pending = self.checked_thin(&request.preview_token)?;
                let (kind, total) = match request.kind {
                    ThinPageKind::Decisions => ("decisions", pending.preview.decisions().len()),
                    ThinPageKind::Uses => ("uses", pending.preview.uses().len()),
                    ThinPageKind::Contexts => ("contexts", pending.preview.contexts().len()),
                    ThinPageKind::Changes => ("changes", pending.preview.changes().len()),
                };
                if request.start > total {
                    return Err(Failure::new("invalid_arguments","THIN start exceeds this record kind's total; equal total returns an empty page"));
                }
                let end = request.start.saturating_add(request.limit).min(total);
                let records = match request.kind {
                    ThinPageKind::Decisions => {
                        serde_json::to_value(&pending.preview.decisions()[request.start..end])
                    }
                    ThinPageKind::Uses => {
                        serde_json::to_value(&pending.preview.uses()[request.start..end])
                    }
                    ThinPageKind::Contexts => {
                        serde_json::to_value(&pending.preview.contexts()[request.start..end])
                    }
                    ThinPageKind::Changes => {
                        serde_json::to_value(&pending.preview.changes()[request.start..end])
                    }
                }
                .map_err(io_failure)?;
                let mut data = thin_header(pending)?;
                data["kind"] = json!(kind);
                data["records"] = records;
                data["page"] = json!({"start":request.start,"total":total,"next_start":(end<total).then_some(end)});
                bounded_thin(data)
            }
            "leviathan_thin_commit" => {
                let request: CommitThin = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_thin(&request.preview_token)?;
                // Preflight the complete bounded response before metadata mutation.
                let mut data = bounded_thin(
                    json!({"changed":false,"summary":pending.preview.summary(),"record":pending.preview.record(),"session":{"dirty":false,"can_undo":false,"can_redo":false},"session_scope":"history_only","omitted_session_fields":["title","path","document_counts"],"source_identity_verified":false,"preview_consumed":true,"notice":"Committed only the reviewed ordinary-cell deletions as one Editor transaction. Transient reversible record hashes bind paginated before→None changes; no persistent/live recipe or source-file mutation. True no-op preserves revision/dirty/history/redo. Model retained-ON proof is not full PCM/lifetime equivalence or fresh audio verification."}),
                )?;
                let editor_revision = pending.editor_revision;
                let pending = self.thin.as_ref().unwrap();
                self.editor
                    .commit_thin_preview(editor_revision, &pending.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                data["changed"] = json!(changed);
                if changed {
                    self.advance();
                } else {
                    self.thin = None;
                }
                data["session"] = json!({"dirty":self.editor.dirty,"can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo()});
                Ok(data)
            }
            "leviathan_slice_features_analyze" => {
                let request: AnalyzeFeatures = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.workspace.assets(&self.editor.song)?;
                let plan = crate::slice_features::prepare(&self.editor.song, request.request)
                    .map_err(validation)?;
                let (preview, route) = match request.source {
                    FeatureSource::VerifiedSource {} => {
                        let path = self.workspace.check(&plan.asset().path, true)?;
                        (
                            crate::slice_features::analyze_verified(&plan, &path)
                                .map_err(validation)?,
                            "verified_source",
                        )
                    }
                    FeatureSource::CachedOnsets { cache_token } => {
                        let cached = self.checked_onset(&cache_token)?;
                        if cached.sample_id != plan.request().sample {
                            return Err(Failure::new(
                                "feature_cache_mismatch",
                                "Cached source SampleId must equal the explicitly selected source",
                            ));
                        }
                        (
                            crate::slice_features::analyze_cached(&plan, &cached.cache)
                                .map_err(validation)?,
                            "cached_onsets",
                        )
                    }
                };
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_limit", "Session token counter exhausted")
                })?;
                let pending = PendingFeatures {
                    token: format!("features-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    route,
                    preview,
                    catalog: None,
                };
                let data = feature_header(&pending, true)?;
                self.token_serial = serial;
                self.features = Some(pending);
                Ok(data)
            }
            "leviathan_slice_features_inspect" => {
                let request: InspectFeatures = decode(arguments)?;
                let (revision, start, limit) = match &request {
                    InspectFeatures::Pending {
                        expected_revision,
                        start,
                        limit,
                        ..
                    }
                    | InspectFeatures::Stored {
                        expected_revision,
                        start,
                        limit,
                        ..
                    } => (*expected_revision, *start, *limit),
                };
                self.expect_revision(revision)?;
                if limit == 0 || limit > MAX_FEATURE_PAGE {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Feature page limit must be1..16",
                    ));
                }
                match request {
                    InspectFeatures::Pending { analysis_token, .. } => {
                        let pending = self.checked_features(&analysis_token, false)?;
                        let total = pending.preview.records().len();
                        if start > total {
                            return Err(Failure::new(
                                "invalid_arguments",
                                "Feature start exceeds total; equal total returns an empty page",
                            ));
                        }
                        let end = start.saturating_add(limit).min(total);
                        let mut data = feature_header(pending, false)?;
                        data["kind"] = json!("pending");
                        data["records"] =
                            serde_json::to_value(&pending.preview.records()[start..end])
                                .map_err(io_failure)?;
                        data["page"] = json!({"start":start,"total":total,"next_start":(end<total).then_some(end)});
                        bounded_features(data)
                    }
                    InspectFeatures::Stored { sample_id, .. } => {
                        let sample = SampleId(sample_id);
                        if !self.editor.song.samples.contains_key(&sample) {
                            return Err(validation("Unknown catalog source".into()));
                        }
                        let selected = || {
                            self.editor
                                .song
                                .regions
                                .iter()
                                .filter(|(_, region)| region.sample == sample)
                        };
                        let total = selected().count();
                        if start > total {
                            return Err(Failure::new(
                                "invalid_arguments",
                                "Catalog start exceeds total; equal total returns an empty page",
                            ));
                        }
                        let end = start.saturating_add(limit).min(total);
                        let records = selected()
                            .skip(start)
                            .take(limit)
                            .map(|(id, _)| {
                                crate::slice_features::inspect_catalog(&self.editor.song, *id)
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(validation)?;
                        bounded_features(
                            json!({"kind":"stored","sample_id":sample_id,"records":records,"page":{"start":start,"total":total,"next_start":(end<total).then_some(end)},"source_identity_verified_now":false,"metrics_recomputed_now":false,"document_changed":false,"notice":"Metadata-only inspection; no source-file IO. Current-match means stored source/bounds binding equals current model, NOT current file bytes, fresh computation or signed attestation. Historical snapshots/labels remain readable when media is missing or changed. Uncatalogued regions are included with entry:null."}),
                        )
                    }
                }
            }
            "leviathan_slice_catalog_preview" => {
                let request: PreviewCatalog = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_features(&request.analysis_token, true)?;
                let preview = pending
                    .preview
                    .catalog_preview(request.roles.clone())
                    .map_err(validation)?;
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_limit", "Session token counter exhausted")
                })?;
                let token = format!("catalog-{serial}-revision-{}", self.revision);
                let data = bounded_features(
                    json!({"analysis_token":pending.token,"preview_token":token,"preview_id":preview.id(),"summary":preview.summary(),"explicit_roles":request.roles,"source_identity_verified_now":true,"metrics_recomputed_now":false,"document_changed":false,"notice":"Store every measured descriptor; only explicit roles replace user-reviewed labels. Empty roles preserves all old labels and their original binding, including stale labels. No source bytes, playback, BPM or marker changes. Review before guarded one-Undo commit."}),
                )?;
                self.token_serial = serial;
                self.features.as_mut().unwrap().catalog = Some(PendingCatalog { token, preview });
                Ok(data)
            }
            "leviathan_slice_catalog_commit" => {
                let request: CommitCatalog = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let analysis_token = self
                    .features
                    .as_ref()
                    .ok_or_else(|| {
                        Failure::new(
                            "features_unavailable",
                            "Analyze and review catalog storage first",
                        )
                    })?
                    .token
                    .clone();
                let pending = self.checked_features(&analysis_token, true)?;
                let reviewed = pending.catalog.as_ref().ok_or_else(|| {
                    Failure::new(
                        "catalog_unavailable",
                        "Review explicit catalog storage first",
                    )
                })?;
                if reviewed.token != request.preview_token {
                    return Err(Failure::new(
                        "invalid_catalog_token",
                        "Token is not the current catalog Store preview",
                    ));
                }
                // Pre-size bounded history output before any mutation; false is longer than true.
                let mut data = bounded_features(
                    json!({"changed":false,"summary":reviewed.preview.summary(),"session":{"dirty":false,"can_undo":false,"can_redo":false},"session_scope":"history_only","omitted_session_fields":["title","path","document_counts"],"preview_consumed":true,"source_identity_verified_now":true,"metrics_recomputed_now":false,"notice":"Committed only reviewed catalog metadata. Descriptors and explicit user labels retain separate historical bindings. No playback/BPM/source changes; same-value commit preserves revision/history/redo."}),
                )?;
                let editor_revision = pending.editor_revision;
                let reviewed = self.features.as_ref().unwrap().catalog.as_ref().unwrap();
                self.editor
                    .commit_region_catalog_preview(editor_revision, &reviewed.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                data["changed"] = json!(changed);
                if changed {
                    self.advance();
                } else {
                    self.features.as_mut().unwrap().catalog = None;
                }
                data["session"] = json!({"dirty":self.editor.dirty,"can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo()});
                Ok(data)
            }
            "leviathan_tempo_analyze" => {
                let request: AnalyzeTempo = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.workspace.assets(&self.editor.song)?;
                let (preview, cache_token) = match request.evidence {
                    TempoEvidence::Markers {
                        marker_set_id,
                        start_index,
                        end_index,
                        include_start,
                    } => (
                        crate::tempo::analyze_markers(
                            &self.editor.song,
                            crate::tempo::MarkerRequest {
                                marker_set: MarkerSetId(marker_set_id),
                                start_index,
                                end_index,
                                include_start,
                            },
                            request.config,
                        )
                        .map_err(validation)?,
                        None,
                    ),
                    TempoEvidence::CachedOnsets {
                        cache_token,
                        start_frame,
                        end_frame,
                        basis,
                    } => {
                        let cached = self.checked_onset(&cache_token)?;
                        let preview = crate::tempo::analyze_cached(
                            &self.editor.song,
                            cached.sample_id,
                            &cached.cache,
                            &cached.config,
                            crate::tempo::CachedRequest {
                                start_frame,
                                end_frame,
                                basis,
                            },
                            request.config,
                        )
                        .map_err(validation)?;
                        (preview, Some(cache_token))
                    }
                };
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_limit", "Session token counter exhausted")
                })?;
                let pending = PendingTempo {
                    token: format!("tempo-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    cache_token,
                    preview,
                    use_preview: None,
                };
                let data = tempo_header(&pending)?;
                self.token_serial = serial;
                self.tempo = Some(pending);
                Ok(data)
            }
            "leviathan_tempo_inspect" => {
                let request: InspectTempo = decode(arguments)?;
                let (revision, token, start, limit, candidate) = match request {
                    InspectTempo::Points {
                        expected_revision,
                        analysis_token,
                        start,
                        limit,
                    } => (expected_revision, analysis_token, start, limit, None),
                    InspectTempo::Pulses {
                        expected_revision,
                        analysis_token,
                        candidate_index,
                        start,
                        limit,
                    } => (
                        expected_revision,
                        analysis_token,
                        start,
                        limit,
                        Some(candidate_index),
                    ),
                };
                self.expect_revision(revision)?;
                if !(1..=MAX_TEMPO_PAGE).contains(&limit) {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Tempo page limit must be1..64",
                    ));
                }
                let pending = self.checked_tempo(&token)?;
                let mut data = tempo_header(pending)?;
                if let Some(candidate) = candidate {
                    let page = pending
                        .preview
                        .pulse_page(candidate, start, limit)
                        .map_err(validation)?;
                    let end = page.start + page.frames.len();
                    let next = (end < page.total).then_some(end);
                    data["kind"] = json!("pulses");
                    data["candidate_index"] = json!(candidate);
                    data["page"] = serde_json::to_value(page).map_err(io_failure)?;
                    data["page"]["next_start"] = json!(next);
                } else {
                    let total = pending.preview.points().len();
                    if start > total {
                        return Err(Failure::new(
                            "invalid_arguments",
                            "Point start exceeds total; equal total returns an empty page",
                        ));
                    }
                    let end = start.saturating_add(limit).min(total);
                    data["kind"] = json!("points");
                    data["records"] = serde_json::to_value(&pending.preview.points()[start..end])
                        .map_err(io_failure)?;
                    data["page"] = json!({"start":start,"total":total,"next_start":(end<total).then_some(end)});
                }
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "tempo_response_limit",
                        "Tempo page exceeds256KiB; reduce limit, never silently truncate",
                    ));
                }
                Ok(data)
            }
            "leviathan_tempo_use_bpm_preview" => {
                let request: PreviewTempoUse = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_tempo(&request.analysis_token)?;
                let preview = pending
                    .preview
                    .use_bpm(
                        MarkerSetId(request.target_marker_set_id),
                        request.candidate_index,
                    )
                    .map_err(validation)?;
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_limit", "Session token counter exhausted")
                })?;
                let token = format!("tempo-use-{serial}-revision-{}", self.revision);
                let data = json!({"analysis_token":pending.token,"preview_token":token,"preview_id":preview.id(),"summary":preview.summary(),"source_identity_verified_now":pending.cache_token.is_some(),"document_changed":false,"notice":"Review this explicit source_bpm-only change. Phase, source frames, marker/lock contents, Song tempo and audio are not changed. Same-source target required. Commit remains full-base/both-revision/cache-epoch guarded."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "tempo_response_limit",
                        "Use-BPM preview exceeds256KiB; previous review retained",
                    ));
                }
                self.token_serial = serial;
                self.tempo.as_mut().unwrap().use_preview = Some(PendingTempoUse { token, preview });
                Ok(data)
            }
            "leviathan_tempo_use_bpm_commit" => {
                let request: CommitTempoUse = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let analysis_token = self
                    .tempo
                    .as_ref()
                    .ok_or_else(|| {
                        Failure::new(
                            "tempo_unavailable",
                            "Analyze and review an explicit Use-BPM change first",
                        )
                    })?
                    .token
                    .clone();
                let pending = self.checked_tempo(&analysis_token)?;
                let reviewed = pending.use_preview.as_ref().ok_or_else(|| {
                    Failure::new(
                        "tempo_use_unavailable",
                        "Review an explicit Use-BPM candidate and target first",
                    )
                })?;
                if reviewed.token != request.preview_token {
                    return Err(Failure::new(
                        "invalid_tempo_use_token",
                        "Token is not the current Use-BPM preview",
                    ));
                }
                let summary =
                    serde_json::to_value(reviewed.preview.summary()).map_err(io_failure)?;
                // Preflight the entire bounded result before mutation. False is
                // the longer boolean spelling, so the actual history replacement
                // cannot grow this serialized payload. Never echo unbounded title/path.
                let mut data = json!({"changed":false,"summary":summary,"session":{"dirty":false,"can_undo":false,"can_redo":false},"session_scope":"history_only","omitted_session_fields":["title","path","document_counts"],"preview_consumed":true,"notice":"Only the explicitly reviewed marker-set source_bpm was committed. Phase is advisory; compiler source origin remains frame0. Same-value commit preserves revision/dirty/history/redo. Session output intentionally contains bounded history indicators only; use project_read for full metadata."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("tempo_response_limit", "Use-BPM result would exceed256KiB; no mutation performed and review retained"));
                }
                let editor_revision = pending.editor_revision;
                let reviewed = self.tempo.as_ref().unwrap().use_preview.as_ref().unwrap();
                self.editor
                    .commit_tempo_preview(editor_revision, &reviewed.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                data["changed"] = json!(changed);
                if changed {
                    self.advance();
                } else {
                    self.tempo.as_mut().unwrap().use_preview = None;
                }
                data["session"] = json!({"dirty":self.editor.dirty,"can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo()});
                Ok(data)
            }
            "leviathan_audition_prepare" => {
                let request: PrepareAudition = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let preview = match &request.target {
                    AuditionTarget::RegionDraft {
                        sample_id,
                        start_frame,
                        end_frame,
                        root_note,
                        amp_envelope,
                        forward_loop,
                    } => {
                        let asset = self
                            .editor
                            .song
                            .samples
                            .get(&SampleId(*sample_id))
                            .ok_or_else(|| {
                                Failure::new("not_found", "Unknown registered sample ID")
                            })?;
                        let draft = SampleRegion {
                            name: "Audition region draft".into(),
                            sample: SampleId(*sample_id),
                            start_frame: *start_frame,
                            end_frame: *end_frame,
                            root_note: *root_note,
                            amp_envelope: *amp_envelope,
                            forward_loop: *forward_loop,
                        };
                        crate::audition::region_preview(asset, &draft)
                    }
                    AuditionTarget::Pad {
                        instrument_id,
                        pad_id,
                    } => crate::audition::pad_preview(
                        &self.editor.song,
                        InstrumentId(*instrument_id),
                        PadId(*pad_id),
                    ),
                }
                .map_err(validation)?;
                // Authorize the exact selected snapshot paths, before any future
                // Engine can canonicalize them. This is NOT byte/hash verification.
                self.workspace.assets(preview.song())?;
                // Provenance only: no retained workflow, consumable capability,
                // caller-supplied snapshot or device/export/commit authorization.
                let binding = serde_json::to_vec(&(
                    "leviathan-audition-review-v1",
                    self.revision,
                    self.editor.revision(),
                    &self.editor.song,
                    &request,
                    preview.id(),
                ))
                .map_err(io_failure)?;
                let review_ref = crate::sample_asset::content_sha256(&binding);
                let data = json!({
                    "review_ref":review_ref,
                    "preview_id":preview.id(),
                    "target":request.target,
                    "summary":preview.summary(),
                    "scope":"headless_metadata_only",
                    "source_identity_verified_now":false,
                    "source_paths_authorized_now":true,
                    "audio_rendered":false,
                    "audio_device_opened":false,
                    "document_changed":false,
                    "notice":"Prepared metadata only, not audible playback or an exported file. Shared snapshot resolves one ON in a fresh isolated context: 2 seconds plus 0.5 seconds release tail at 48 kHz. Region draft plays native pitch; pad uses committed mapping and unity input velocity/bin127 with first round-robin alternative. Source SHA is declared metadata, not freshly verified bytes. review_ref binds exact request/base/revisions/snapshot for provenance only; it is not a commit, render, device or GUI capability. No retained preview or cross-session control."
                });
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("audition_response_limit", "Audition metadata exceeds 256 KiB; no data silently truncated and no document/audio/file change"));
                }
                Ok(data)
            }
            "leviathan_resample_prepare" => {
                let request: PrepareResample = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let output = self.workspace.resolve(&request.config.output, false)?;
                extension(&output, ".wav")?;
                if output.exists() {
                    return Err(Failure::new(
                        "output_exists",
                        "Resampling only publishes a new WAV; choose an unused filename",
                    ));
                }
                self.workspace.assets(&self.editor.song)?;
                let plan = crate::resample::prepare(
                    &self.editor.song,
                    crate::resample::ResampleConfig {
                        version: request.config.version,
                        occurrence: request.config.occurrence,
                        name: request.config.name,
                        root_note: request.config.root_note,
                        tail: request.config.tail,
                        output,
                    },
                )
                .map_err(validation)?;
                let serial = self.token_serial.checked_add(1).ok_or_else(|| {
                    Failure::new("token_exhausted", "No fresh review tokens remain")
                })?;
                let pending = PendingResample {
                    token: format!("resample-{serial}-revision-{}", self.revision),
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    workflow: ResampleWorkflow::Prepared(Box::new(plan)),
                };
                let data = resample_header(&pending, &self.workspace, true)?;
                self.token_serial = serial;
                self.resample = Some(pending);
                Ok(data)
            }
            "leviathan_resample_render" => {
                let request: ResampleRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_resample(&request.workflow_token)?;
                let ResampleWorkflow::Prepared(plan) = &pending.workflow else {
                    return Err(Failure::new("resample_already_printed","This workflow already published a WAV; inspect/register with the same token, never render over it"));
                };
                self.workspace.assets(plan.base())?;
                let path = self.workspace.check(&plan.config().output, false)?;
                if path.exists() {
                    return Err(Failure::new("output_exists","Resample destination now exists; existing file and prepared plan unchanged"));
                }
                let printed = crate::resample::render(
                    plan,
                    |access| authorize_resample(&self.workspace, access),
                    || false,
                )
                .map_err(validation)?;
                // Publication is already durable: retain the receipt BEFORE any response
                // encoding. No post-publication error may hide/delete the artifact.
                self.resample.as_mut().unwrap().workflow =
                    ResampleWorkflow::Printed(Box::new(printed));
                let pending = self.resample.as_ref().unwrap();
                match resample_header(pending, &self.workspace, true) {
                    Ok(data) => Ok(data),
                    Err(mut error) => {
                        error.details = json!({"artifact_published":true,"output_path":self.workspace.display(&pending.workflow.config().output),"workflow_token":pending.token,"document_changed":false});
                        Err(error)
                    }
                }
            }
            "leviathan_resample_inspect" => {
                let request: InspectResample = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.resample_metadata(&request.workflow_token)?;
                let total = pending.workflow.summary().source_samples.len();
                if !(1..=MAX_RESAMPLE_PAGE).contains(&request.limit) || request.start > total {
                    return Err(Failure::new("invalid_arguments","Source page limit must be1..64; start may equal but not exceed source total"));
                }
                let end = request.start.saturating_add(request.limit).min(total);
                let records: Vec<_> = match &pending.workflow {
                    ResampleWorkflow::Prepared(plan) => plan.summary().source_samples
                        [request.start..end]
                        .iter()
                        .map(|id| json!({"sample":id,"asset":plan.base().samples[id]}))
                        .collect(),
                    ResampleWorkflow::Printed(preview) => preview.receipt().source_identities
                        [request.start..end]
                        .iter()
                        .map(|s| json!(s))
                        .collect(),
                };
                let mut data =
                    resample_header(pending, &self.workspace, self.resample_current(pending))?;
                data["source_records"] = json!(records);
                data["start"] = json!(request.start);
                data["total"] = json!(total);
                data["next_start"] = json!((end < total).then_some(end));
                data["inspection_source_access"] = json!(false);
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("resample_response_limit","Resample source page exceeds256KiB; reduce limit; workflow and artifact unchanged"));
                }
                Ok(data)
            }
            "leviathan_resample_register" => {
                let request: ResampleRequest = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_resample(&request.workflow_token)?;
                let ResampleWorkflow::Printed(preview) = &pending.workflow else {
                    return Err(Failure::new(
                        "resample_not_printed",
                        "Render and inspect the durable WAV before registration",
                    ));
                };
                self.workspace.assets(preview.candidate())?;
                crate::resample::verify_artifact(preview, |access| {
                    authorize_resample(&self.workspace, access)
                })
                .map_err(validation)?;
                let summary = serde_json::to_value(preview.summary()).map_err(io_failure)?;
                let asset = serde_json::to_value(preview.asset()).map_err(io_failure)?;
                let editor_revision = pending.editor_revision;
                let pending = self.resample.as_ref().unwrap();
                let ResampleWorkflow::Printed(preview) = &pending.workflow else {
                    unreachable!("printed phase checked above")
                };
                self.editor
                    .commit_resample_preview(editor_revision, preview)
                    .map_err(validation)?;
                self.resample = None;
                self.advance();
                Ok(
                    json!({"changed":true,"summary":summary,"asset":asset,"session":self.state(),"artifact_identity_verified_now":true,"workflow_consumed":true,"notice":"One Undo registration of fresh sample/whole region/Pitched instrument only. The external WAV remains after Undo; Redo restores references without rewriting bytes. Concurrent later filesystem changes remain outside document history."}),
                )
            }
            "leviathan_groove_capture_preview" => {
                let request: PreviewGrooveCapture = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.workspace.assets(&self.editor.song)?;
                let plan = crate::groove::capture_preview(
                    &self.editor.song,
                    GrooveId(request.groove_id),
                    request.config,
                )
                .map_err(validation)?;
                self.publish_groove(GroovePlan::Capture(Box::new(plan)))
            }
            "leviathan_groove_apply_preview" => {
                let request: PreviewGrooveApply = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                self.workspace.assets(&self.editor.song)?;
                let plan = crate::groove::preview(&self.editor.song, request.config)
                    .map_err(validation)?;
                self.publish_groove(GroovePlan::Apply(Box::new(plan)))
            }
            "leviathan_groove_template_inspect" => {
                let request: InspectGrooveTemplate = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let template = self
                    .editor
                    .song
                    .grooves
                    .get(&GrooveId(request.groove_id))
                    .ok_or_else(|| Failure::new("not_found", "Unknown groove template ID"))?;
                let records = crate::groove::inspect(template).map_err(validation)?;
                if !(1..=MAX_GROOVE_PAGE).contains(&request.limit) || request.start > records.len()
                {
                    return Err(Failure::new("invalid_arguments", "Groove anchor page limit must be 1..64; start may equal but not exceed anchor count"));
                }
                let end = request
                    .start
                    .saturating_add(request.limit)
                    .min(records.len());
                let mut metadata = serde_json::to_value(template).map_err(io_failure)?;
                metadata.as_object_mut().unwrap().remove("anchors");
                let data = json!({"groove_id":request.groove_id,"template":metadata,"start":request.start,"total":records.len(),"next_start":(end<records.len()).then_some(end),"anchors":&records[request.start..end],"document_changed":false,"source_identity_verified":false,"notice":"Immutable historical recipe: raw source-frame deltas are authoritative; offset_beats is approximate display only. Historical IDs/hash need not remain live. This read does not access source files, infer tempo or reverify audio bytes."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Groove template page exceeds 256 KiB; reduce limit",
                    ));
                }
                Ok(data)
            }
            "leviathan_groove_preview_inspect" => {
                let request: InspectGroovePreview = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_groove(&request.preview_token)?;
                self.workspace.assets(pending.plan.base())?;
                let total = pending.plan.record_count();
                if !(1..=MAX_GROOVE_PAGE).contains(&request.limit) || request.start > total {
                    return Err(Failure::new("invalid_arguments", "Groove preview page limit must be 1..64; start may equal but not exceed record_count"));
                }
                let end = request.start.saturating_add(request.limit).min(total);
                let (records, template) = match &pending.plan {
                    GroovePlan::Capture(plan) => {
                        let anchors =
                            crate::groove::inspect(plan.template()).map_err(validation)?;
                        let mut metadata =
                            serde_json::to_value(plan.template()).map_err(io_failure)?;
                        metadata.as_object_mut().unwrap().remove("anchors");
                        (
                            serde_json::to_value(&anchors[request.start..end])
                                .map_err(io_failure)?,
                            metadata,
                        )
                    }
                    GroovePlan::Apply(plan) => (
                        serde_json::to_value(&plan.changes()[request.start..end])
                            .map_err(io_failure)?,
                        Value::Null,
                    ),
                };
                let data = json!({"preview_token":pending.token,"kind":pending.plan.kind(),"groove_preview_id":pending.plan.id(),"effective_config":pending.plan.config()?,"summary":pending.plan.summary()?,"template":template,"start":request.start,"total":total,"next_start":(end<total).then_some(end),"records":records,"document_changed":false,"source_identity_verified":false,"notice":"Capture records are shared raw anchors with approximate display offsets; apply records are exact parent CellChange before/after values. Current full-base/token/revision/root guards, no audio-byte verification or document edits."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Groove preview page exceeds 256 KiB; reduce limit; plan unchanged",
                    ));
                }
                Ok(data)
            }
            "leviathan_groove_capture_commit" | "leviathan_groove_apply_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_groove(&request.preview_token)?;
                let expected_kind = if name == "leviathan_groove_capture_commit" {
                    "capture"
                } else {
                    "apply"
                };
                if pending.plan.kind() != expected_kind {
                    return Err(Failure::new("groove_preview_kind_mismatch", "Use the commit tool matching the reviewed capture/apply kind; plan and document unchanged"));
                }
                self.workspace.assets(pending.plan.candidate())?;
                let summary = pending.plan.summary()?;
                let editor_revision = pending.editor_revision;
                let pending = self.groove_preview.as_ref().unwrap();
                match &pending.plan {
                    GroovePlan::Capture(plan) => self
                        .editor
                        .commit_groove_capture_preview(editor_revision, plan),
                    GroovePlan::Apply(plan) => {
                        self.editor.commit_groove_preview(editor_revision, plan)
                    }
                }
                .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                } else {
                    self.groove_preview = None;
                }
                Ok(
                    json!({"kind":expected_kind,"changed":changed,"summary":summary,"session":self.state(),"preview_consumed":true,"source_identity_verified":false}),
                )
            }
            "leviathan_generation_preview" => {
                let request: PreviewGeneration = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let config = request.config.effective()?;
                self.workspace.assets(&self.editor.song)?;
                let preview =
                    crate::generation::preview(&self.editor.song, config).map_err(validation)?;
                if serde_json::to_vec(preview.candidate())
                    .map_err(io_failure)?
                    .len()
                    > MAX_DOCUMENT
                {
                    return Err(Failure::new(
                        "document_too_large",
                        "Generated candidate exceeds 8 MiB; previous plan and document unchanged",
                    ));
                }
                self.workspace.assets(preview.candidate())?;
                let serial = self.token_serial + 1;
                let token = format!("generation-{serial}-revision-{}", self.revision);
                let data = json!({"preview_token":token,"generation_preview_id":preview.id(),"effective_config":preview.config(),"summary":preview.summary(),"provenance":preview.provenance(),"generated_cell_count":preview.cells().len(),"document_changed":false,"source_identity_verified":false,"notice":"Actual pinned pattrns, typed Euclidean preset with explicit version/seed and finite bars; no arbitrary code/Lua or hidden random state. Inspect generated ordinary cells, then explicitly commit fresh track/pattern/order allocation as one undo. Source paths reauthorized, audio bytes not decoded or hashed. Failed previews preserve the current plan."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Generation summary exceeds 256 KiB; previous plan and document unchanged",
                    ));
                }
                self.token_serial = serial;
                self.generation_preview = Some(PendingGeneration {
                    token,
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    preview,
                });
                Ok(data)
            }
            "leviathan_generation_inspect" => {
                let request: InspectGeneration = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_generation(&request.preview_token)?;
                self.workspace.assets(pending.preview.base())?;
                let cells = pending.preview.cells();
                if !(1..=MAX_GENERATION_PAGE).contains(&request.limit)
                    || request.start > cells.len()
                {
                    return Err(Failure::new("invalid_arguments", "Generation page limit must be 1..64; start may equal but not exceed generated_cell_count"));
                }
                let end = request.start.saturating_add(request.limit).min(cells.len());
                let data = json!({"preview_token":pending.token,"generation_preview_id":pending.preview.id(),"effective_config":pending.preview.config(),"summary":pending.preview.summary(),"provenance":pending.preview.provenance(),"start":request.start,"total":cells.len(),"next_start":(end<cells.len()).then_some(end),"cells":&cells[request.start..end],"document_changed":false,"source_identity_verified":false,"notice":"Exact server-owned shared generated cells and pattrns timing/provenance, not client-authorized patches. No document/history/source-file changes; no audio-byte verification."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Generation page exceeds 256 KiB; reduce limit; plan unchanged",
                    ));
                }
                Ok(data)
            }
            "leviathan_generation_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_generation(&request.preview_token)?;
                self.workspace.assets(pending.preview.candidate())?;
                let summary =
                    serde_json::to_value(pending.preview.summary()).map_err(io_failure)?;
                let provenance =
                    serde_json::to_value(pending.preview.provenance()).map_err(io_failure)?;
                let editor_revision = pending.editor_revision;
                let pending = self.generation_preview.as_ref().unwrap();
                self.editor
                    .commit_generation_preview(editor_revision, &pending.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                } else {
                    self.generation_preview = None;
                }
                Ok(
                    json!({"changed":changed,"summary":summary,"provenance":provenance,"preview_consumed":true,"session":self.state(),"source_identity_verified":false}),
                )
            }
            "leviathan_transform_preview" => {
                let request: PreviewTransform = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let config = request.config.effective()?;
                self.workspace.assets(&self.editor.song)?;
                let preview =
                    crate::transforms::preview(&self.editor.song, config).map_err(validation)?;
                if serde_json::to_vec(preview.candidate())
                    .map_err(io_failure)?
                    .len()
                    > MAX_DOCUMENT
                {
                    return Err(Failure::new("document_too_large", "Transform candidate exceeds 8 MiB; previous preview and document unchanged"));
                }
                self.workspace.assets(preview.candidate())?;
                let serial = self.token_serial + 1;
                let token = format!("transform-{serial}-revision-{}", self.revision);
                let data = json!({"preview_token":token,"transform_preview_id":preview.id(),"effective_config":preview.config(),"summary":preview.summary(),"changed_cell_count":preview.changes().len(),"document_changed":false,"source_identity_verified":false,"notice":"Deterministic shared ordinary-cell transform, not runtime groove. Inspect exact before/after changes using this token, then explicitly commit. No random seed, audio decode/hash verification, clamping, alias-target edits or hidden broad selection. Raw scheduled-frame proof does not promise equal audible event counts when moves cross occurrence mute boundaries. New successful transform preview supersedes the old one; failed preview preserves it."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new("preview_too_large", "Transform summary exceeds 256 KiB; previous preview and document unchanged"));
                }
                self.token_serial = serial;
                self.transform_preview = Some(PendingTransform {
                    token,
                    revision: self.revision,
                    editor_revision: self.editor.revision(),
                    preview,
                });
                Ok(data)
            }
            "leviathan_transform_inspect" => {
                let request: InspectTransform = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_transform(&request.preview_token)?;
                self.workspace.assets(pending.preview.base())?;
                let changes = pending.preview.changes();
                if !(1..=MAX_TRANSFORM_PAGE).contains(&request.limit)
                    || request.start > changes.len()
                {
                    return Err(Failure::new("invalid_arguments", "Transform page limit must be 1..64; start may equal but not exceed changed_cell_count"));
                }
                let end = request
                    .start
                    .saturating_add(request.limit)
                    .min(changes.len());
                let data = json!({"preview_token":pending.token,"transform_preview_id":pending.preview.id(),"effective_config":pending.preview.config(),"start":request.start,"total":changes.len(),"next_start":(end<changes.len()).then_some(end),"changes":&changes[request.start..end],"document_changed":false,"source_identity_verified":false,"notice":"Exact shared before/after parent cells, source/destination rows and column; absent and explicit-zero fields remain distinct. Changes are stored-pattern edits affecting every repeat/alias use. No document mutation or current audio-byte verification."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Transform page exceeds 256 KiB; reduce limit; no changes applied",
                    ));
                }
                Ok(data)
            }
            "leviathan_transform_commit" => {
                let request: CommitCompile = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let pending = self.checked_transform(&request.preview_token)?;
                self.workspace.assets(pending.preview.candidate())?;
                let summary =
                    serde_json::to_value(pending.preview.summary()).map_err(io_failure)?;
                let editor_revision = pending.editor_revision;
                // Split the immutable plan borrow from Editor; no client candidate is accepted.
                let pending = self.transform_preview.as_ref().unwrap();
                self.editor
                    .commit_transform_preview(editor_revision, &pending.preview)
                    .map_err(validation)?;
                let changed = self.editor.revision() != editor_revision;
                if changed {
                    self.advance();
                } else {
                    self.transform_preview = None;
                }
                Ok(
                    json!({"changed":changed,"summary":summary,"session":self.state(),"preview_consumed":true,"source_identity_verified":false}),
                )
            }
            "leviathan_onset_v2_prepare" => {
                let request: PrepareOnset = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if request.engine != "experimental_v2" {
                    return Err(Failure::new("invalid_arguments", "This opt-in tool requires engine:experimental_v2; sample_analyze remains v1"));
                }
                let sample_id = SampleId(request.sample_id);
                let asset = self
                    .editor
                    .song
                    .samples
                    .get(&sample_id)
                    .ok_or_else(|| Failure::new("not_found", "Unknown sample ID"))?
                    .clone();
                if !(8000..=384000).contains(&asset.sample_rate)
                    || asset.frames > u64::from(asset.sample_rate) * MAX_ONSET_SECONDS
                {
                    return Err(Failure::new("analysis_limit", "Experimental MCP v2 requires 8000..384000 Hz and at most 15 seconds; input is never truncated"));
                }
                let path = self.workspace.check(&asset.path, true)?;
                let mut config = crate::onset_service::experimental_config(request.preset);
                config.max_frames = (u64::from(asset.sample_rate) * MAX_ONSET_SECONDS)
                    .min(crate::sample_asset::MAX_SAMPLE_VALUES as u64);
                config.max_suggestions = MAX_ONSET_SUGGESTIONS;
                config.max_work_units = 500_000_000;
                config.max_working_bytes = 128 * 1024 * 1024;
                config.retain_envelope = false;
                request.picker.apply(&mut config);
                // Retire before extraction, so retained native PCM/curve caches
                // cannot accumulate. A started prepare failure leaves no cache.
                self.onset_cache = None;
                let prepared = crate::onset_service::prepare_verified(&asset, &path, &config)
                    .map_err(|message| {
                        let mut error = Failure::new("analysis_error", message);
                        error.details =
                            json!({"previous_cache_retired":true,"document_changed":false});
                        error
                    })?;
                let token = format!("onset-{}-revision-{}", self.token_serial + 1, self.revision);
                let data = onset_result(
                    &prepared.result,
                    &config,
                    &token,
                    prepared.cache.retained_bytes(),
                    true,
                )
                .map_err(|mut error| {
                    error.details = json!({"previous_cache_retired":true,"document_changed":false});
                    error
                })?;
                self.token_serial += 1;
                self.onset_cache = Some(PendingOnset {
                    token,
                    revision: self.revision,
                    sample_id,
                    asset,
                    cache: prepared.cache,
                    config,
                    config_sha256: prepared.result.config_sha256.clone(),
                    result: prepared.result,
                });
                Ok(data)
            }
            "leviathan_onset_v2_repick" => {
                let request: RepickOnset = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                let cached = self.checked_onset(&request.cache_token)?;
                let mut config = if let Some(preset) = request.preset {
                    let mut config = crate::onset_service::experimental_config(preset);
                    config.max_frames = cached.config.max_frames;
                    config.max_suggestions = cached.config.max_suggestions;
                    config.max_work_units = cached.config.max_work_units;
                    config.max_working_bytes = cached.config.max_working_bytes;
                    config.retain_envelope = false;
                    config
                } else {
                    cached.config.clone()
                };
                request.picker.apply(&mut config);
                let result = crate::onset_service::repick(&cached.cache, &cached.asset, &config)
                    .map_err(|message| Failure::new("analysis_error", message))?;
                let token = format!("onset-{}-revision-{}", self.token_serial + 1, self.revision);
                let data = onset_result(
                    &result,
                    &config,
                    &token,
                    cached.cache.retained_bytes(),
                    false,
                )?;
                let next = PendingOnset {
                    token,
                    revision: self.revision,
                    sample_id: cached.sample_id,
                    asset: cached.asset.clone(),
                    cache: Arc::clone(&cached.cache),
                    config,
                    config_sha256: result.config_sha256.clone(),
                    result,
                };
                self.token_serial += 1;
                self.onset_cache = Some(next);
                Ok(data)
            }
            "leviathan_onset_v2_evidence" => {
                let request: OnsetPage = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if !(1..=MAX_ONSET_PAGE).contains(&request.limit) {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Evidence limit must be 1..256 curve bins",
                    ));
                }
                let cached = self.checked_onset(&request.cache_token)?;
                let page = cached
                    .cache
                    .curve_page(request.start, request.limit)
                    .map_err(validation)?;
                let end = page.start + page.points.len();
                let next = (end < page.total).then_some(end);
                let data = json!({"engine":"experimental_v2","policy":crate::onset_service::POLICY,"cache_token":cached.token,"config_sha256":cached.config_sha256,"page":page,"next_start":next,"current_source_sha_verified":true,"document_changed":false,"notice":"Raw unthresholded evidence, not calibrated probabilities or accepted markers. source_frame is original native PCM; start/next_start are curve-bin indices."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Evidence structured payload exceeds 256 KiB; request a smaller page",
                    ));
                }
                Ok(data)
            }
            "leviathan_onset_v2_results" => {
                let request: OnsetResultsPage = decode(arguments)?;
                self.expect_revision(request.expected_revision)?;
                if !(1..=MAX_ONSET_RESULT_PAGE).contains(&request.limit) {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Result limit must be 1..64 records",
                    ));
                }
                let cached = self.checked_onset(&request.cache_token)?;
                let (kind, total) = match request.kind {
                    OnsetResultKind::Candidates => (
                        "candidates",
                        cached.result.analysis.detection.candidates.len(),
                    ),
                    OnsetResultKind::Suggestions => {
                        ("suggestions", cached.result.analysis.suggestions.len())
                    }
                };
                if request.start > total {
                    return Err(Failure::new(
                        "invalid_arguments",
                        "Result start exceeds the current selected result count",
                    ));
                }
                let end = request.start.saturating_add(request.limit).min(total);
                let items = match request.kind {
                    OnsetResultKind::Candidates => serde_json::to_value(
                        &cached.result.analysis.detection.candidates[request.start..end],
                    ),
                    OnsetResultKind::Suggestions => serde_json::to_value(
                        &cached.result.analysis.suggestions[request.start..end],
                    ),
                }
                .map_err(io_failure)?;
                let data = json!({"engine":"experimental_v2","cache_token":cached.token,"binding":cached.cache.binding(),"config_sha256":cached.config_sha256,"kind":kind,"start":request.start,"total":total,"items":items,"next_start":(end<total).then_some(end),"current_source_sha_verified":true,"document_changed":false,"notice":"Server-owned coarse candidates or refined suggestions, not accepted markers. Peak and attack frames are separate original-source positions; confidence is heuristic, not probability."});
                if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
                    return Err(Failure::new(
                        "preview_too_large",
                        "Result page structured payload exceeds 256 KiB; request fewer records",
                    ));
                }
                Ok(data)
            }
            "leviathan_workspace_list" => {
                let _: Empty = decode(arguments)?;
                let mut files = Vec::new();
                let mut truncated = false;
                for (index, entry) in fs::read_dir(&self.workspace.root)
                    .map_err(io_failure)?
                    .enumerate()
                {
                    if index == 2048 {
                        truncated = true;
                        break;
                    }
                    let entry = entry.map_err(io_failure)?;
                    if !entry.file_type().map_err(io_failure)?.is_file() {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".wav")
                        || name.ends_with(".flac")
                        || name.ends_with(".leviathan.json")
                    {
                        files.push(name);
                    }
                }
                files.sort();
                Ok(json!({"files":files,"recursive":false,"truncated":truncated}))
            }
            _ => Err(Failure::new(
                "capability_unavailable",
                "Unknown or unavailable capability",
            )),
        }
    }
    fn tool_result(&mut self, name: &str, arguments: Value) -> Value {
        let data = self.agent_call(name, arguments);
        let failed = data.get("ok") == Some(&Value::Bool(false));
        json!({"content":[{"type":"text","text":data.to_string()}],"structuredContent":data,"isError":failed})
    }

    /// Dispatch one validated musical operation without an MCP transport envelope.
    /// The operation list, schemas, state and protections are shared with MCP.
    pub fn agent_call(&mut self, name: &str, arguments: Value) -> Value {
        let canonical = name.trim().to_ascii_lowercase();
        if !TOOL_NAMES.contains(&canonical.as_str()) {
            return json!({"ok":false,"revision":self.revision,"data":{},"error":{"code":"capability_unavailable","message":"Unknown or unavailable capability","details":{}}});
        }
        match self.execute(&canonical, arguments) {
            Ok(data) => json!({"ok":true,"revision":self.revision,"data":data}),
            Err(error) => {
                json!({"ok":false,"revision":self.revision,"data":{},"error":{"code":error.code,"message":error.message,"details":error.details}})
            }
        }
    }

    /// Process one direct-agent envelope. This is deliberately not JSON-RPC.
    pub fn agent_request(&mut self, request: Value) -> Value {
        let Some(object) = request.as_object() else {
            return agent_error("invalid_request", "Expected a JSON object");
        };
        match object.get("command").and_then(Value::as_str).map(str::trim) {
            Some("help") => {
                json!({"ok":true,"commands":["help","discover","call"],"protocol":"leviathan-agent/1","max_message_bytes":MAX_MESSAGE})
            }
            Some("discover") => {
                json!({"ok":true,"protocol":"leviathan-agent/1","tools":tool_catalog()})
            }
            Some("call") => {
                let Some(name) = object.get("name").and_then(Value::as_str) else {
                    return agent_error("invalid_request", "call requires a string name");
                };
                let arguments = match object.get("arguments") {
                    None => json!({}),
                    Some(value) if value.is_object() => value.clone(),
                    Some(_) => {
                        return agent_error("invalid_request", "arguments must be a JSON object")
                    }
                };
                self.agent_call(name, arguments)
            }
            _ => agent_error("invalid_request", "command must be help, discover, or call"),
        }
    }

    /// Dispatch against a live document without cloning or replacing its identity.
    /// Keep this Server bound to the same GUI document for the lifetime of any review tokens.
    pub fn agent_request_with_document(
        &mut self,
        editor: &mut Editor,
        session: &mut Session,
        request: Value,
    ) -> Value {
        std::mem::swap(&mut self.editor, editor);
        std::mem::swap(&mut self.session, session);
        let previous_live_context = self.live_document_context;
        self.live_document_context = true;
        let dispatched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let current_editor_revision = self.editor.revision();
            let current_owner = self.editor.document_owner();
            let owner_changed = self
                .bound_document_owner
                .as_ref()
                .is_some_and(|previous| !previous.same_owner(&current_owner));
            if owner_changed
                || self
                    .bound_editor_revision
                    .is_some_and(|previous| previous != current_editor_revision)
            {
                self.advance();
            }
            let response = self.agent_request(request);
            self.bound_editor_revision = Some(self.editor.revision());
            self.bound_document_owner = Some(self.editor.document_owner());
            response
        }));
        self.live_document_context = previous_live_context;
        std::mem::swap(&mut self.editor, editor);
        std::mem::swap(&mut self.session, session);
        match dispatched {
            Ok(response) => response,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Resolve a workspace artifact path with the same root, symlink and existence policy as operations.
    pub fn agent_resolve_path(&self, path: &str, must_exist: bool) -> Result<PathBuf, String> {
        self.workspace
            .resolve(path, must_exist)
            .map_err(|error| error.message)
    }

    /// Synchronize the bound live document and return this agent session's revision.
    pub fn agent_revision_with_document(
        &mut self,
        editor: &mut Editor,
        session: &mut Session,
    ) -> u64 {
        self.agent_request_with_document(
            editor,
            session,
            json!({"command":"call","name":"leviathan_project_read","arguments":{"section":"summary"}}),
        )["revision"].as_u64().unwrap_or(self.revision)
    }
    /// Process one JSON-RPC message. Notifications never produce responses.
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(rpc_error(
                Value::Null,
                -32600,
                "Expected a single JSON-RPC object",
            ));
        };
        let id = object.get("id").cloned();
        if object.get("jsonrpc") != Some(&json!("2.0"))
            || !object.get("method").is_some_and(Value::is_string)
            || id
                .as_ref()
                .is_some_and(|v| !(v.is_string() || v.as_i64().is_some() || v.as_u64().is_some()))
            || object.get("params").is_some_and(|v| !v.is_object())
        {
            return Some(rpc_error(
                id.filter(|v| v.is_string() || v.as_i64().is_some() || v.as_u64().is_some())
                    .unwrap_or(Value::Null),
                -32600,
                "Invalid JSON-RPC request",
            ));
        }
        let method = object["method"].as_str().unwrap();
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        let Some(id) = id else {
            if method == "notifications/initialized" && self.phase == Phase::Initializing {
                self.phase = Phase::Ready;
            }
            return None;
        };
        let result = match method {
            "ping" => Ok(json!({})),
            "initialize" if self.phase == Phase::New => {
                if !params.get("protocolVersion").is_some_and(Value::is_string)
                    || !params.get("capabilities").is_some_and(Value::is_object)
                    || !params
                        .get("clientInfo")
                        .and_then(|v| v.get("name"))
                        .is_some_and(Value::is_string)
                    || !params
                        .get("clientInfo")
                        .and_then(|v| v.get("version"))
                        .is_some_and(Value::is_string)
                {
                    Err((-32602, "Initialize needs protocolVersion, capabilities and clientInfo name/version"))
                } else {
                    self.phase = Phase::Initializing;
                    Ok(
                        json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{"listChanged":false}},
                        "serverInfo":{"name":"leviathan-local","title":"Leviathan — Headless Song Editor","version":env!("CARGO_PKG_VERSION")},
                        "instructions":"Use leviathan_capabilities first. This is a private headless document, NOT an open GUI instance. Read its revision before mutations; batch edits are atomic and one undo step. Paths are relative to the explicit startup root. No live audio, shell, network or arbitrary filesystem access."}),
                    )
                }
            }
            "initialize" => Err((-32600, "Already initialized")),
            _ if self.phase != Phase::Ready => Err((
                -32002,
                "Send initialize, then notifications/initialized before using tools",
            )),
            "tools/list" => {
                if params.get("cursor").is_some() {
                    Err((-32602, "This static single-page tool list has no cursor"))
                } else {
                    Ok(json!({"tools":tool_catalog()}))
                }
            }
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str);
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if !arguments.is_object() {
                    Err((-32602, "Tool arguments must be an object"))
                } else if let Some(name) = name.filter(|name| TOOL_NAMES.contains(name)) {
                    Ok(self.tool_result(name, arguments))
                } else {
                    Err((-32602, "Unknown tool name"))
                }
            }
            _ => Err((-32601, "Method not found")),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, text)) => rpc_error(id, code, text),
        })
    }
}
fn onset_result(
    result: &crate::onset_service::SourceAnalysis,
    config: &crate::onset_v2::Config,
    token: &str,
    retained_bytes: usize,
    fft_recomputed: bool,
) -> Outcome<Value> {
    let serialized_result_bytes = serde_json::to_vec(result).map_err(io_failure)?.len();
    if serialized_result_bytes > MAX_ONSET_RESULT_BYTES {
        return Err(Failure::new("preview_too_large", "Experimental retained analysis exceeds 8 MiB serialized; no result is silently truncated"));
    }
    let mut boundaries = BTreeSet::from([0, result.binding.source_frames]);
    boundaries.extend(
        result
            .analysis
            .suggestions
            .iter()
            .map(|suggestion| suggestion.attack_frame),
    );
    let summary = json!({"binding":result.binding,"config_sha256":result.config_sha256,"provenance":result.analysis.detection.provenance,"candidate_count":result.analysis.detection.candidates.len(),"suggestion_count":result.analysis.suggestions.len(),"refinement_collisions":result.analysis.refinement_collisions,"normalized_envelope_retained":false});
    let data = json!({"engine":"experimental_v2","policy":crate::onset_service::POLICY,"cache_token":token,"effective_config":config,"source_analysis_summary":summary,"retained_cache_bytes":retained_bytes,"serialized_result_bytes":serialized_result_bytes,"fft_recomputed":fft_recomputed,"current_source_sha_verified":true,"document_changed":false,"marker_capacity":{"max_full_source_boundaries":257,"proposed_full_source_boundary_count":boundaries.len(),"fits_one_marker_set":boundaries.len()<=257,"basis":"unique proposed refined attacks plus fixed endpoints only; not a union with existing manual markers","automatic_acceptance":false},"phase_enabled":config.enable_phase,"notice":"Experimental heuristic evidence with false positives and missed onsets, not probability/tempo inference/drum classification. Inspect paginated server-owned candidates/suggestions; peak_frame and refined attack_frame are distinct. Accepted marker sets above 257 boundaries are rejected, not truncated; review/select/cleanup explicitly."});
    if serde_json::to_vec(&data).map_err(io_failure)?.len() > MAX_PREVIEW_BYTES {
        return Err(Failure::new(
            "preview_too_large",
            "Experimental summary structured payload exceeds 256 KiB",
        ));
    }
    Ok(data)
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn agent_error(code: &str, message: &str) -> Value {
    json!({"ok":false,"error":{"code":code,"message":message}})
}

/// Persistent newline-delimited JSON interface for direct agent clients.
/// It uses the same validated dispatcher as MCP but has no JSON-RPC lifecycle.
pub fn serve_agent(
    root: &Path,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<(), String> {
    let mut server = Server::new(root)?;
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        loop {
            let available = input.fill_buf().map_err(|e| e.to_string())?;
            if available.is_empty() {
                break;
            }
            let length = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            let newline = available[length - 1] == b'\n';
            if line.len().saturating_add(length) <= MAX_MESSAGE && !oversized {
                line.extend_from_slice(&available[..length]);
            } else {
                oversized = true;
            }
            input.consume(length);
            if newline {
                break;
            }
        }
        if line.is_empty() && !oversized {
            return Ok(());
        }
        let response = if oversized {
            agent_error("message_too_large", "Message exceeds 1 MiB limit")
        } else {
            match serde_json::from_slice::<Value>(&line) {
                Ok(request) => server.agent_request(request),
                Err(_) => agent_error("parse_error", "Input line is not valid JSON"),
            }
        };
        serde_json::to_writer(&mut output, &response).map_err(|e| e.to_string())?;
        output.write_all(b"\n").map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
    }
}

/// Newline-delimited stdio; memory bounded independently of client framing.
/// EOF closes the session. No banners or diagnostics are written to stdout.
pub fn serve(root: &Path, mut input: impl BufRead, mut output: impl Write) -> Result<(), String> {
    let mut server = Server::new(root)?;
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        loop {
            let available = input.fill_buf().map_err(|e| e.to_string())?;
            if available.is_empty() {
                break;
            }
            let length = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            let newline = available[length - 1] == b'\n';
            if line.len() + length <= MAX_MESSAGE && !oversized {
                line.extend_from_slice(&available[..length]);
            } else {
                oversized = true;
            }
            input.consume(length);
            if newline {
                break;
            }
        }
        if line.is_empty() && !oversized {
            return Ok(());
        }
        let reply = if oversized {
            Some(rpc_error(
                Value::Null,
                -32600,
                "Message exceeds 1 MiB limit",
            ))
        } else {
            match serde_json::from_slice::<Value>(&line) {
                Ok(value) => server.handle(value),
                Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
            }
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut output, &reply).map_err(|e| e.to_string())?;
            output
                .write_all(b"\n")
                .and_then(|_| output.flush())
                .map_err(|e| e.to_string())?;
        }
    }
}

const TOOL_NAMES: &[&str] = &[
    "leviathan_capabilities",
    "leviathan_project_read",
    "leviathan_project_new",
    "leviathan_project_open",
    "leviathan_project_save",
    "leviathan_project_prepare_overwrite",
    "leviathan_edit_batch",
    "leviathan_history_undo",
    "leviathan_history_redo",
    "leviathan_song_evaluate",
    "leviathan_song_export",
    "leviathan_sample_generate",
    "leviathan_workspace_list",
    "leviathan_sample_import",
    "leviathan_sample_inspect",
    "leviathan_sample_analyze",
    "leviathan_break_compile_preview",
    "leviathan_break_compile_commit",
    "leviathan_marker_cleanup_preview",
    "leviathan_onset_v2_prepare",
    "leviathan_onset_v2_repick",
    "leviathan_onset_v2_evidence",
    "leviathan_onset_v2_results",
    "leviathan_drum_steps_inspect",
    "leviathan_matrix_inspect",
    "leviathan_transform_preview",
    "leviathan_transform_inspect",
    "leviathan_transform_commit",
    "leviathan_generation_preview",
    "leviathan_generation_inspect",
    "leviathan_generation_commit",
    "leviathan_groove_capture_preview",
    "leviathan_groove_capture_commit",
    "leviathan_groove_template_inspect",
    "leviathan_groove_apply_preview",
    "leviathan_groove_preview_inspect",
    "leviathan_groove_apply_commit",
    "leviathan_multisample_inspect",
    "leviathan_resample_prepare",
    "leviathan_resample_render",
    "leviathan_resample_inspect",
    "leviathan_resample_register",
    "leviathan_tempo_analyze",
    "leviathan_tempo_inspect",
    "leviathan_tempo_use_bpm_preview",
    "leviathan_tempo_use_bpm_commit",
    "leviathan_slice_features_analyze",
    "leviathan_slice_features_inspect",
    "leviathan_slice_catalog_preview",
    "leviathan_slice_catalog_commit",
    "leviathan_thin_preview",
    "leviathan_thin_inspect",
    "leviathan_thin_commit",
    "leviathan_audition_prepare",
    "leviathan_marker_bank_preview",
    "leviathan_marker_bank_commit",
    "leviathan_cell_block_copy",
    "leviathan_cell_block_preview",
    "leviathan_cell_block_inspect",
    "leviathan_cell_block_commit",
    "leviathan_track_mix_preview",
    "leviathan_track_mix_commit",
    "leviathan_sample_peaks_analyze",
    "leviathan_sample_peaks_inspect",
];
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn operation(name: &str, description: &str, mut properties: Value, required: &[&str]) -> Value {
    properties
        .as_object_mut()
        .unwrap()
        .insert("op".into(), json!({"const":name}));
    let mut fields = vec!["op"];
    fields.extend_from_slice(required);
    let mut schema = object(properties, &fields);
    schema["description"] = json!(description);
    schema
}
fn edit_schema() -> Value {
    let id = json!({"type":"integer","minimum":1,"maximum":u64::MAX,"description":"Stable nonzero document ID, not an array index"});
    let text = json!({"type":"string"});
    let address = json!({"pattern_id":id,"track_id":id,"row":{"type":"integer","minimum":0,"maximum":65535},"column":{"type":"integer","minimum":0,"maximum":255}});
    let mut cell = address.clone();
    cell["cell"] = json!({"$ref":"#/$defs/cell"});
    let mut effects = address.clone();
    effects["effects"] = json!({"$ref":"#/$defs/effects"});
    let mut trigger_address = address.clone();
    trigger_address["trigger"] = json!({"oneOf":[{"type":"null"},{"$ref":"#/$defs/phrase_trigger"}],"description":"Required: phrase reference and total cycles, or explicit null to clear only the trigger"});
    let phrase_address = json!({"phrase_id":id,"row":{"type":"integer","minimum":0,"maximum":1023},"column":{"type":"integer","minimum":0,"maximum":15}});
    let mut phrase_cell_address = phrase_address.clone();
    phrase_cell_address["cell"] = json!({"$ref":"#/$defs/phrase_cell"});
    let step_event = json!({"projection_token":{"type":"string","minLength":1},"pad_id":id,"row":{"type":"integer","minimum":0,"maximum":65535},"column":{"type":"integer","minimum":0,"maximum":255}});
    let mut step_velocity = step_event.clone();
    step_velocity["value"] = json!({"type":["number","null"],"minimum":0,"maximum":1,"description":"Required nullable velocity: null clears explicit value, zero is explicit silence"});
    let mut step_delay = step_event.clone();
    step_delay["value"] = json!({"type":["number","null"],"minimum":0,"exclusiveMaximum":1,"description":"Required nullable absolute source-row fraction, not a step fraction; no quantization or display rounding"});
    let mut step_retrigger = step_event;
    step_retrigger["value"] = json!({"$ref":"#/$defs/effects/properties/retrigger","description":"Required nullable parent-cell retrigger; children are not independent editable notes"});
    let operations = vec![
        operation("drum_step_toggle","Toggle one page-local pad slot through the shared guarded projection. Conflicts/off-grid/ambiguous/unsafe allocation reject; existing source-cell attributes are retained, only note/retrigger clear on removal. No new columns or hidden quantization",json!({"projection_token":{"type":"string","minLength":1},"pad_id":id,"step":{"type":"integer","minimum":0,"maximum":31}}),&["projection_token","pad_id","step"]),
        operation("drum_step_set_velocity","Change only an exact inspected parent event's velocity; off-grid/multiple-hit parents remain explicitly addressable. Original token/page/source row/column ownership is required",step_velocity,&["projection_token","pad_id","row","column","value"]),
        operation("drum_step_set_delay","Change only an exact inspected parent event's source-row delay, preserving the stored f64 and all other fields. Moving outside the inspected page requires a new inspection before another field edit",step_delay,&["projection_token","pad_id","row","column","value"]),
        operation("drum_step_set_retrigger","Replace/clear only one inspected parent cell's retrigger using existing row-fraction units and validation; no independent child-hit editing or unrelated-note deletion",step_retrigger,&["projection_token","pad_id","row","column","value"]),
        operation("set_song","Change title/tempo/lines per beat; omitted fields are preserved",json!({"title":{"type":["string","null"]},"bpm":{"type":["number","null"],"minimum":20,"maximum":400},"lines_per_beat":{"type":["integer","null"],"minimum":1,"maximum":256}}),&[]),
        operation("set_track_mix","Replace the complete persisted track gain/balance/mute/solo set via the same shared pure preview/validation as the review tools. Empty settings resets neutral defaults; known unequal shared-legacy owner coefficients reject. Model-valid unrenderable drafts can retain settings without a routing/live claim. Normal edit_batch all-source path policy remains; no GUI/mailbox/device control",json!({"config":track_mix_config_schema()}),&["config"]),
        operation("upsert_track","Create or update a track; new tracks append to display order. Shrinking never silently drops cells",json!({"id":id,"name":text,"note_columns":{"type":"integer","minimum":1,"maximum":256}}),&["id","name","note_columns"]),
        operation("remove_track","Remove a track; clear_cells:true explicitly deletes its cells from all patterns, otherwise references make it fail. Persisted track-mix references always require an earlier explicit full-set set_track_mix excluding that track; clear_cells does not reset mix settings",json!({"id":id,"clear_cells":{"type":"boolean","default":false}}),&["id"]),
        operation("set_track_order","Reorder tracks by stable IDs; must contain every track exactly once",json!({"track_ids":{"type":"array","items":id,"uniqueItems":true}}),&["track_ids"]),
        operation("upsert_instrument","Create/update instrument name and legacy sample. Required null clears only legacy assignment, preserving any region or kit. Explicit non-null path switches to the pitched legacy source",json!({"id":id,"name":text,"sample":{"type":["string","null"],"description":"Workspace-relative existing regular sample; no symlinks or parent traversal; null clears legacy sample but preserves region/kind; explicit path clears region and selects Pitched"}}),&["id","name","sample"]),
        operation("remove_instrument","Remove an unreferenced instrument; clear/change cell references first",json!({"id":id}),&["id"]),
        operation("upsert_pattern","Create empty pattern or rename/resize existing pattern without dropping cells; clear occupied rows before shrinking",json!({"id":id,"name":text,"rows":{"type":"integer","minimum":1,"maximum":65536}}),&["id","name","rows"]),
        operation("remove_pattern","Remove an unordered pattern; remove its occurrences first",json!({"id":id}),&["id"]),
        operation("clone_pattern","Deep-copy a pattern to a new stable ID, materializing every alias into independent local scalar content; does not append an order occurrence",json!({"source_id":id,"new_id":id,"name":{"type":["string","null"]}}),&["source_id","new_id"]),
        operation("set_track_alias","Link an empty target block to another pattern/track with identical rows and columns. Required source:{pattern,track} links; explicit null drops an existing alias to an empty target (does NOT copy), preserving an already-local block. Use make_track_unique to retain content. Shared cycle/depth/expanded-cell validation; never overwrites local cells implicitly",json!({"pattern_id":id,"track_id":id,"source":{"oneOf":[{"type":"null"},object(json!({"pattern":id,"track":id}),&["pattern","track"])]}}),&["pattern_id","track_id","source"]),
        operation("make_track_unique","Materialize the exact resolved scalar track block at its alias target, then detach it; source and other aliases remain shared. Existing local block is a no-op. Follow with ordinary edits in the same atomic batch",json!({"pattern_id":id,"track_id":id}),&["pattern_id","track_id"]),
        operation("set_occurrence_track_mute","Set one zero-based order occurrence's target-track mute, not a global track flag or pattern edit. Muted blocks suppress cells/parameter memory, cancel pending phrase actions and release held track voices at the boundary; existing release tails may fade, not hard silence",json!({"occurrence_index":{"type":"integer","minimum":0,"maximum":u64::MAX},"track_id":id,"muted":{"type":"boolean"}}),&["occurrence_index","track_id","muted"]),
        operation("move_occurrence","Swap two zero-based order indices via the shared order-arrow operation, carrying COMPLETE occurrence records including mutes. This is a swap, not insertion/rotation; equal indices are a no-op",json!({"from_index":{"type":"integer","minimum":0,"maximum":u64::MAX},"to_index":{"type":"integer","minimum":0,"maximum":u64::MAX}}),&["from_index","to_index"]),
        operation("set_order","Replace the complete order with FRESH UNMUTED occurrences; duplicate pattern IDs share content, empty order allowed. This explicitly resets occurrence mutes; use move_occurrence to preserve records",json!({"pattern_ids":{"type":"array","items":id}}),&["pattern_ids"]),
        operation("append_repeat","Append one NEW UNMUTED occurrence referencing existing shared pattern content",json!({"pattern_id":id}),&["pattern_id"]),
        operation("set_cell","Replace a sparse cell, including independent note/instrument/volume/pan/delay/effects; zero-based address",cell,&["pattern_id","track_id","row","column","cell"]),
        operation("clear_cell","Remove all fields/effects at this sparse cell",address,&["pattern_id","track_id","row","column"]),
        operation("set_effects","Replace only step effects; preserve note and independent cell fields",effects,&["pattern_id","track_id","row","column","effects"]),
        operation("clear_pattern_track","Explicitly clear every cell of one track in one pattern",json!({"pattern_id":id,"track_id":id}),&["pattern_id","track_id"]),
        operation("upsert_phrase","Create an empty instrument-bound phrase or change metadata while preserving cells. Shared validation rejects occupied-row/column shrink and dangling instrument/trigger references",json!({"id":id,"name":text,"instrument_id":id,"rows":{"type":"integer","minimum":1,"maximum":1024},"lines_per_beat":{"type":"integer","minimum":1,"maximum":256},"note_columns":{"type":"integer","minimum":1,"maximum":16},"root_note":{"type":"integer","minimum":0,"maximum":127}}),&["id","name","instrument_id","rows","lines_per_beat","note_columns","root_note"]),
        operation("remove_phrase","Remove a phrase only after all pattern trigger references have been cleared; no implicit destructive cascade",json!({"id":id}),&["id"]),
        operation("remove_groove","Remove one historical groove template without changing baked ordinary cells or source/marker records; no automatic regeneration or reverse transform",json!({"id":id}),&["id"]),
        operation("create_multisample_instrument","Create a NEW empty shared melodic map and a NEW assigned instrument using the shared checked fresh-ID allocator; both names equal name. Existing notes, instruments and cursor preserved; empty map is a valid draft but uncovered actual ON rejects in evaluation/export",json!({"name":{"type":"string","minLength":1,"maxLength":256}}),&["name"]),
        operation("upsert_multisample","Create an empty shared melodic map or rename while preserving all zones. Stable map ID; nonblank name at most256 UTF-8 bytes. Map edits affect every assigned instrument and later rendering of existing notes",json!({"id":id,"name":{"type":"string","minLength":1,"maxLength":256}}),&["id","name"]),
        operation("set_keyzones","Replace ALL key zones of one shared melodic map atomically. Empty array explicitly clears; omitted fields reject. Stable IDs unique within map; inclusive note/velocity rectangles must not overlap in both axes. Gaps allowed as drafts; actual uncovered ON rejects at evaluation/export. No hidden root inference, partial merge, cascade or selector logic",json!({"map_id":id,"zones":{"type":"array","maxItems":128,"items":keyzone_input_schema(),"description":"Complete replacement; all fields required. Region references include unused zones; at most4096 zones across64 maps. Shared map changes rerender existing notes for every assigned instrument"}}),&["map_id","zones"]),
        operation("remove_multisample","Remove only an unassigned melodic map; explicitly switch referencing instruments first. Regions and source files remain unchanged",json!({"id":id}),&["id"]),
        operation("assign_multisample","Assign a shared melodic map to an existing instrument, deliberately clearing its previous legacy/region/kit source. Required map_id:null detaches ONLY a currently Multisample instrument, leaving Pitched/DrumKit kinds and sources unchanged",json!({"instrument_id":id,"map_id":{"type":["integer","null"],"minimum":1,"maximum":u64::MAX}}),&["instrument_id","map_id"]),
        operation("create_drum_kit_instrument","Atomically create an empty kit and a NEW kit-assigned instrument with the same name using the shared checked fresh-ID allocator. Existing instruments, assignments, notes and cursor are untouched; unlike assigning an empty kit over an occupied instrument this works on existing songs. Read drum_kits/instruments afterward for allocated IDs; ID exhaustion or validation rolls back the entire batch",json!({"name":{"type":"string","minLength":1,"maxLength":256}}),&["name"]),
        operation("upsert_drum_kit","Create an empty 16-pad kit or rename it while preserving every pad. Name is at most 256 UTF-8 bytes; stable ID is independent of instrument ID",json!({"id":id,"name":{"type":"string","maxLength":256}}),&["id","name"]),
        operation("remove_drum_kit","Remove only a kit with no instrument assignment; explicitly unassign/switch referencing instruments first. Regions and source files are preserved",json!({"id":id}),&["id"]),
        operation("upsert_drum_pad","Create/replace one complete pad by stable ID; slot 0..15 and MIDI note 0..127 must each be unique within the kit. Required nullable region_id maps a single-region pad or leaves it unassigned; on an existing zoned pad, null PRESERVES all zones and non-null REJECTS (use set_pad_sources for an explicit mode switch). Required nullable choke_group disables or selects group 1..16. Other fields replace pad metadata; source files unchanged",json!({"kit_id":id,"id":id,"slot":{"type":"integer","minimum":0,"maximum":15},"name":{"type":"string","maxLength":128},"note":{"type":"integer","minimum":0,"maximum":127,"description":"MIDI pad selector only; does not transpose sample playback"},"region_id":{"type":["integer","null"],"minimum":1,"maximum":u64::MAX},"root_note":{"type":"integer","minimum":0,"maximum":127,"description":"Native playback reference, not the MIDI pad selector"},"tune_semitones":{"type":"integer","minimum":-48,"maximum":48,"default":0,"description":"Explicit source pitch offset; root_note+tune must remain MIDI 0..127; omitted resets to zero on full replacement"},"gain":{"type":"number","minimum":0,"maximum":1},"pan":{"type":"number","minimum":-1,"maximum":1},"mode":{"type":"string","enum":["OneShot","Gated"]},"new_note":{"type":"string","enum":["Overlap","ReleasePrevious"]},"choke_group":{"type":["integer","null"],"minimum":1,"maximum":16}}),&["kit_id","id","slot","name","note","region_id","root_note","gain","pan","mode","new_note","choke_group"]),
        operation("set_pad_sources","Atomically replace an existing pad source mode through shared validation. Both fields required: region_id plus [] for single-region, null plus complete velocity_zones for deterministic selection, null plus [] to unmap. Nonempty zones and region together reject. Each zone is an inclusive sorted gapless 0..127 partition; ordered alternatives rotate independently per instrument/pad/zone, one source per hit; all layer IDs stable and unique within pad",json!({"kit_id":id,"pad_id":id,"region_id":{"type":["integer","null"],"minimum":1,"maximum":u64::MAX},"velocity_zones":{"type":"array","maxItems":8,"description":"Empty for single-region/unassigned mode. Otherwise complete ordered partition 0..127; at most32 total alternatives per pad. Velocity bin is round(127*incoming ON volume) in f64, before pad gain; no quantization of audio gain","items":object(json!({"min_velocity":{"type":"integer","minimum":0,"maximum":127},"max_velocity":{"type":"integer","minimum":0,"maximum":127},"alternatives":{"type":"array","minItems":1,"maxItems":8,"items":object(json!({"id":id,"region":id}),&["id","region"]),"description":"Vector order is deterministic round-robin order; PadLayerId is nonzero and unique across all zones in this pad"}}),&["min_velocity","max_velocity","alternatives"])}}),&["kit_id","pad_id","region_id","velocity_zones"]),
        operation("remove_drum_pad","Remove one stable pad, rejecting an invalid remaining song mapping. Clear referencing note cells or explicitly switch the instrument first; no implicit cascade or region deletion",json!({"kit_id":id,"id":id}),&["kit_id","id"]),
        operation("assign_kit","Assign an existing drum kit to an instrument, clearing legacy sample/region. Required kit_id:null selects Pitched without clearing an already pitched source; ordinary note cells select mapped MIDI pads",json!({"instrument_id":id,"kit_id":{"type":["integer","null"],"minimum":1,"maximum":u64::MAX}}),&["instrument_id","kit_id"]),
        operation("set_phrase_cell","Replace one phrase cell using the same validated musical fields/effects. Phrase rows/columns are zero-based; nested phrase triggers are forbidden",phrase_cell_address,&["phrase_id","row","column","cell"]),
        operation("clear_phrase_cell","Remove every musical field/effect from one phrase cell",phrase_address,&["phrase_id","row","column"]),
        operation("set_phrase_trigger","Set or explicitly clear only a pattern cell's phrase reference, preserving its other fields. A trigger requires Note; Pitched and melodic Multisample instruments transpose relative to phrase root, but kit instruments require neutral root-note triggers. Only parent release/cut effects are allowed",trigger_address,&["pattern_id","track_id","row","column","trigger"]),
        operation("upsert_marker_set","Create or replace accepted manual source-frame boundaries, separate from detector suggestions. Existing lock flags are preserved; removing a locked boundary or rebinding its source is rejected. Exactly 0 and source.frames endpoints, strictly increasing 2..257 entries. source_bpm is required nullable: explicit BPM 20..400 or null when unknown; no tempo inference",json!({"id":id,"name":text,"sample_id":id,"source_bpm":{"type":["number","null"],"minimum":20,"maximum":400},"markers":{"type":"array","minItems":2,"maxItems":257,"items":{"type":"integer","minimum":0,"maximum":u64::MAX},"uniqueItems":true}}),&["id","name","sample_id","source_bpm","markers"]),
        operation("remove_marker_set","Remove an unlocked accepted marker set without deleting source audio or previously compiled ordinary patterns/regions; explicit unlock required before deleting locked sets",json!({"id":id}),&["id"]),
        operation("set_marker_lock","Explicitly lock/unlock one existing internal accepted boundary; source endpoints are always fixed and reject lock toggles",json!({"marker_set_id":id,"frame":{"type":"integer","minimum":1,"maximum":u64::MAX},"locked":{"type":"boolean"}}),&["marker_set_id","frame","locked"]),
        operation("add_marker","Insert an exact internal source-frame boundary; no snap or implicit lock; duplicates/out-of-bounds/capacity errors reject atomically",json!({"marker_set_id":id,"frame":{"type":"integer","minimum":1,"maximum":u64::MAX}}),&["marker_set_id","frame"]),
        operation("move_marker","Move one unlocked internal boundary to an exact free source frame. Locked endpoints/boundaries never move implicitly; explicit unlock/move/relock can share one batch",json!({"marker_set_id":id,"from_frame":{"type":"integer","minimum":1,"maximum":u64::MAX},"to_frame":{"type":"integer","minimum":1,"maximum":u64::MAX}}),&["marker_set_id","from_frame","to_frame"]),
        operation("remove_marker","Remove one unlocked internal boundary, never a fixed endpoint. Locks must be explicitly cleared before deletion",json!({"marker_set_id":id,"frame":{"type":"integer","minimum":1,"maximum":u64::MAX}}),&["marker_set_id","frame"]),
        operation("remove_sample","Remove an imported source only after every referencing region is removed; files are never deleted",json!({"id":id}),&["id"]),
        operation("upsert_region","Create/edit a non-destructive source view. Preserve existing amp envelope and loop; shrinking/rebinding must leave stored loop valid. New regions default to legacy one-shot playback. start_frame inclusive, end_frame exclusive in original source frames; root_note is native pitch independently of BPM",json!({"id":id,"name":text,"sample_id":id,"start_frame":{"type":"integer","minimum":0,"maximum":u64::MAX},"end_frame":{"type":"integer","minimum":1,"maximum":u64::MAX},"root_note":{"type":"integer","minimum":0,"maximum":127}}),&["id","name","sample_id","start_frame","end_frame","root_note"]),
        operation("set_region_playback","Replace both optional playback controls for an existing region, affecting all mapped instruments and kit pads. Both fields are required nullable; null explicitly clears. Shared metadata validation only; evaluation/export reject loop + offset/Reverse on used voices before audio decode/output. Output-clock ADSR and source-frame loop are independent; no file rewriting or live audition",json!({"region_id":id,"amp_envelope":{"oneOf":[{"type":"null"},object(json!({"attack_ms":{"type":"integer","minimum":0,"maximum":10000},"decay_ms":{"type":"integer","minimum":0,"maximum":10000},"sustain_percent":{"type":"integer","minimum":0,"maximum":100},"release_ms":{"type":"integer","minimum":0,"maximum":500,"description":"Ordinary release duration in output-clock milliseconds; zero stops immediately and may click. Choke/replacement/mute instead use 50 ms linear release"}}),&["attack_ms","decay_ms","sustain_percent","release_ms"])]},"forward_loop":{"oneOf":[{"type":"null"},object(json!({"start_frame":{"type":"integer","minimum":0,"maximum":u64::MAX,"description":"Inclusive original source frame, inside region; prefix plays once"},"end_frame":{"type":"integer","minimum":2,"maximum":u64::MAX,"description":"Exclusive original source frame, inside region; end-start >=2. Continues during release; no crossfade or post-loop tail"}}),&["start_frame","end_frame"])]}}),&["region_id","amp_envelope","forward_loop"]),
        operation("set_region_role","Explicit user-authored role/articulation bound to current source/bounds metadata. Required null clears role only, preserving features. Ghost is contextual, never inferred from level; this operation does not compute or verify audio",json!({"region_id":id,"label":{"oneOf":[{"type":"null"},role_label_schema()]}}),&["region_id","label"]),
        operation("clear_region_features","Remove measured feature snapshot only, preserving reviewed role and its historical binding. No source or playback change",json!({"region_id":id}),&["region_id"]),
        operation("remove_region","Remove only an unmapped region; clear instrument mappings first. Source file and other regions remain unchanged",json!({"id":id}),&["id"]),
        operation("assign_region","Map an existing instrument to a region, explicitly replacing legacy/kit source and selecting Pitched. Required null clears only the region mapping and preserves any kit",json!({"instrument_id":id,"region_id":{"type":["integer","null"],"minimum":1,"maximum":u64::MAX}}),&["instrument_id","region_id"]),
    ];
    let mut schema = object(
        json!({"expected_revision":revision_schema(),"operations":{"type":"array","minItems":1,"maxItems":MAX_OPERATIONS,"items":{"oneOf":operations}}}),
        &["expected_revision", "operations"],
    );
    let fx = object(
        json!({
            "retrigger":{"oneOf":[{"type":"null"},object(json!({"repeats":{"type":"integer","minimum":1,"maximum":16},"spacing":{"type":"number","exclusiveMinimum":0,"exclusiveMaximum":1,"description":"Row-fraction spacing; delayed final repeat must remain before row end"}}),&["repeats","spacing"])]},
            "release":{"type":["number","null"],"minimum":0,"exclusiveMaximum":1,"description":"Absolute fraction of this row, at/after delayed note and retriggers; initiates legacy de-click or opted-in region release_ms. A zero-ms envelope release stops abruptly"},
            "glide":{"oneOf":[{"type":"null"},object(json!({"target":{"type":"integer","minimum":0,"maximum":127},"semitones_per_second":{"type":"number","minimum":0.01,"maximum":120}}),&["target","semitones_per_second"])]},
            "cut":{"type":["number","null"],"minimum":0,"exclusiveMaximum":1,"description":"Abrupt active-voice stop at absolute row fraction, at/after delayed note/retriggers. May click; cannot combine release or OFF"},
            "offset":{"type":["integer","null"],"minimum":0,"maximum":u32::MAX,"description":"Zero-based source sample frame, not output frames. Must fit actual decoded sample/region; forbidden on forward-loop voices"},
            "direction":{"type":["string","null"],"enum":["Forward","Reverse",null],"description":"Changes source playback direction. Reverse is forbidden on forward-loop voices; otherwise a reverse new note starts at last real source frame unless offset is supplied. Effect-only requires active voice"}
        }),
        &[],
    );
    let cell = object(
        json!({
            "note":{"oneOf":[{"type":"string","enum":["Empty","Off"]},object(json!({"Note":{"type":"integer","minimum":0,"maximum":127}}),&["Note"])],"description":"Use {\"Note\":60} for pitched MIDI middle C or a kit pad selector; Empty leaves note unspecified. Off releases pitched/Gated voices; OneShot ignores ordinary OFF while its column pointer clears"},
            "instrument":{"type":["integer","null"],"minimum":1,"maximum":u64::MAX},
            "velocity":{"type":["number","null"],"minimum":0,"maximum":1},
            "pan":{"type":["number","null"],"minimum":-1,"maximum":1},
            "delay":{"type":["number","null"],"minimum":0,"exclusiveMaximum":1},
            "effects":{"$ref":"#/$defs/effects"},
        "phrase":{"oneOf":[{"type":"null"},{"$ref":"#/$defs/phrase_trigger"}],"description":"Optional instrument-bound phrase trigger; Pitched and melodic Multisample instruments transpose relative to phrase root_note. Kit phrases require neutral trigger note equal to phrase root_note and never transpose pad selectors. Parent effects permit only release/cut. No nested phrase triggers"}
        }),
        &["note"],
    );
    let mut phrase_cell = cell.clone();
    phrase_cell["properties"]["phrase"] = json!({"type":"null","description":"Nested phrases are unsupported; omit this field or use null"});
    let phrase_trigger = object(
        json!({"phrase":id,"cycles":{"type":"integer","minimum":1,"maximum":64,"description":"Total finite phrase cycles, not additional repeats"}}),
        &["phrase", "cycles"],
    );
    schema["$defs"] =
        json!({"effects":fx,"cell":cell,"phrase_cell":phrase_cell,"phrase_trigger":phrase_trigger});
    schema
}
fn stable_id_schema() -> Value {
    json!({"type":"integer","minimum":1,"maximum":u64::MAX,"description":"Stable nonzero document ID, not an array index"})
}
fn analysis_schema() -> Value {
    object(
        json!({
            "sensitivity":{"type":["number","null"],"minimum":0,"maximum":1,"default":0.5,"description":"Higher values accept weaker rises; not a desired onset count"},
            "min_spacing_ms":{"type":["number","null"],"minimum":1,"maximum":1000,"default":25,"description":"Minimum suggestion spacing in milliseconds, rounded up to source frames; detector resolution roughly 2..6 ms"},
            "minimum_rms":{"type":["number","null"],"minimum":0,"maximum":1,"default":0.00001,"description":"Absolute RMS floor, without normalization"},
            "max_suggestions":{"type":["integer","null"],"minimum":1,"maximum":MAX_ANALYSIS_SUGGESTIONS,"default":4096,"description":"Exceeding the limit fails; results are never silently truncated"},
            "waveform_bins":{"type":["integer","null"],"minimum":0,"maximum":MAX_ANALYSIS_BINS,"default":1024,"description":"Overview bins, capped by real frame count; zero omits waveform"}
        }),
        &[],
    )
}
fn compile_schema() -> Value {
    let subdivision = json!({"type":"string","enum":["Sixteenth","ThirtySecond","SixtyFourth","OneTwentyEighth"],"description":"Whole-note subdivisions 1/16,1/32,1/64,1/128, not source frames"});
    let grid = object(json!({"division":subdivision}), &["division"]);
    object(
        json!({
            "marker_set":stable_id_schema(),
            "target_bpm":{"type":"number","minimum":20,"maximum":400,"description":"Explicit new global song BPM; existing LPB is preserved"},
            "timing":{"oneOf":[{"const":"SourceRhythm"},object(json!({"Grid":grid}), &["Grid"])],"description":"SourceRhythm maps source frame positions using declared source BPM; Grid places slices on chosen whole-note subdivision. Origin is source frame 0; no tempo inference"},
            "cycles":{"type":"integer","minimum":1,"maximum":16,"description":"Total finite repetitions"},
            "roll":{"oneOf":[{"type":"null"},object(json!({"division":subdivision,"hits":{"type":"integer","minimum":2,"maximum":16}}), &["division","hits"])],"description":"Optional finite roll; hits is total count including the original onset"},
            "tail":{"type":"string","enum":["Overlap","ReleasePrevious","CutPrevious"],"description":"Explicit voice-tail policy; cut is abrupt and can click, release uses existing fade"},
            "minimum_slice_frames":{"type":"integer","minimum":1,"maximum":u64::MAX,"description":"Reject slices shorter than this positive original-source-frame count"},
            "root_note":{"type":"integer","minimum":0,"maximum":127,"description":"Native-rate/root-pitch note for generated regions and events; no time stretch"},
            "pad_tails":{"type":"boolean","description":"Explicitly request compiler tail-aware pattern padding; inspect summary for exact bounds"},
            "name":{"type":"string","minLength":1,"maxLength":256,"description":"Nonblank, at most 256 UTF-8 bytes; backend also enforces byte length"}
        }),
        &[
            "marker_set",
            "target_bpm",
            "timing",
            "cycles",
            "tail",
            "minimum_slice_frames",
            "root_note",
            "pad_tails",
            "name",
        ],
    )
}
fn generation_schema() -> Value {
    let euclidean = object(
        json!({
            "steps":{"type":"integer","minimum":1,"maximum":128,"description":"Finite evenly spaced ticks per four-beat bar"},
            "pulses":{"type":"integer","minimum":1,"maximum":128,"description":"Euclidean pulses per bar; must not exceed steps"},
            "rotation":{"type":"integer","minimum":0,"maximum":127,"description":"Explicit rotation; must be less than steps"},
            "probability":{"type":"number","minimum":0,"maximum":1,"description":"Seeded pattrns gate probability per eligible pulse; checked as f64 before shared f32 conversion. All-closed result rejects; no automatic seed retry"}
        }),
        &["steps", "pulses", "rotation", "probability"],
    );
    let note = object(
        json!({
            "note":{"type":"integer","minimum":12,"maximum":127,"description":"Explicit audible MIDI pitch for Pitched/Multisample instruments, selector for kit. MIDI12..127 is the actual pinned pattrns mapping range; every choice must be valid/source-assigned, even if gated out"},
            "velocity":{"type":"number","minimum":0,"maximum":1,"description":"Explicit finite velocity, checked before f32 conversion; never inferred from prior column memory"}
        }),
        &["note", "velocity"],
    );
    object(
        json!({
            "version":{"type":"integer","const":crate::generation::VERSION,"description":"Required shared preset format version; unsupported versions reject, never silently upgrade"},
            "name":{"type":"string","minLength":1,"maxLength":256,"description":"Nonblank generated track/pattern name, at most256 UTF-8 bytes"},
            "bars":{"type":"integer","minimum":1,"maximum":16,"description":"Finite four-beat bars; existing Song BPM/LPB unchanged"},
            "instrument":stable_id_schema(),
            "seed":{"type":"integer","minimum":0,"maximum":u64::MAX,"description":"Required deterministic u64 seed; preserve JSON integer precision. No ambient RNG seed, default random value or hidden retry"},
            "preset":object(json!({"Euclidean":euclidean}), &["Euclidean"]),
            "sequence":{"type":"array","minItems":1,"maxItems":16,"items":note,"description":"Finite cyclic explicit note/velocity sequence advances on emitted hits only, not closed gates; seeded gate runs continuously across bars"}
        }),
        &[
            "version",
            "name",
            "bars",
            "instrument",
            "seed",
            "preset",
            "sequence",
        ],
    )
}
fn thin_config_schema() -> Value {
    object(
        json!({"version":{"type":"integer","const":crate::mutations::VERSION},"scope":transform_scope_schema(),"seed":{"type":"integer","minimum":0,"maximum":u64::MAX,"description":"Explicit deterministic pattrns seed; preserve exact JSON integer precision"},"removal_percent":{"type":"integer","minimum":0,"maximum":100,"description":"Integer percent of eligible stored cells proposed for removal, not a float probability or per-occurrence decision.0 is a true no-op but still requires a model-evaluable base"},"max_removed":{"type":"integer","minimum":0,"maximum":crate::mutations::MAX_RECORDS,"description":"Explicit maximum proposed removals; shared overflow policy fails rather than silently retuning the gate"},"protected_roles":{"type":"array","minItems":0,"maxItems":5,"uniqueItems":true,"items":{"type":"string","enum":["kick","snare","closed_hat","open_hat_ride","crash"]},"description":"Explicit unique concrete human-reviewed role protections; caller order preserved. Empty disables only these protections. Unknown/Mixed/unreviewed/stale always retained and cannot be opted out"}}),
        &[
            "version",
            "scope",
            "seed",
            "removal_percent",
            "max_removed",
            "protected_roles",
        ],
    )
}
fn role_label_schema() -> Value {
    object(
        json!({"role":{"type":"string","enum":["kick","snare","closed_hat","open_hat_ride","crash","mixed","unknown"]},"articulation":{"type":"string","enum":["unspecified","normal","ghost"],"description":"Explicit contextual user annotation; ghost is never inferred from absolute level"},"note":{"type":"string","maxLength":256,"description":"At most256 UTF-8 bytes (stricter than Unicode character count); blank allowed"}}),
        &["role", "articulation", "note"],
    )
}
fn feature_source_schema() -> Value {
    json!({"type":"object","oneOf":[object(json!({"kind":{"const":"verified_source"}}),&["kind"]),object(json!({"kind":{"const":"cached_onsets"},"cache_token":{"type":"string","minLength":1}}),&["kind","cache_token"])]})
}
fn feature_inspect_schema() -> Value {
    let common = json!({"expected_revision":revision_schema(),"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_FEATURE_PAGE,"default":MAX_FEATURE_PAGE}});
    let mut pending = common.clone();
    pending["kind"] = json!({"const":"pending"});
    pending["analysis_token"] = json!({"type":"string","minLength":1});
    let mut stored = common;
    stored["kind"] = json!({"const":"stored"});
    stored["sample_id"] = stable_id_schema();
    let mut properties = pending.clone();
    properties["sample_id"] = stable_id_schema();
    properties["kind"] = json!({"type":"string","enum":["pending","stored"]});
    json!({"type":"object","properties":properties,"additionalProperties":false,"oneOf":[object(pending,&["kind","expected_revision","analysis_token"]),object(stored,&["kind","expected_revision","sample_id"])]})
}
fn tempo_config_schema() -> Value {
    object(
        json!({"version":{"type":"integer","const":crate::tempo::VERSION},"min_bpm_tenths":{"type":"integer","minimum":200,"maximum":4000,"description":"Explicit lower bound in tenths BPM,200=20BPM; must not exceed upper bound"},"max_bpm_tenths":{"type":"integer","minimum":200,"maximum":4000,"description":"Explicit upper bound in tenths BPM,4000=400BPM"}}),
        &["version", "min_bpm_tenths", "max_bpm_tenths"],
    )
}
fn tempo_evidence_schema() -> Value {
    let markers = object(
        json!({"kind":{"const":"markers"},"marker_set_id":stable_id_schema(),"start_index":{"type":"integer","minimum":0,"maximum":255},"end_index":{"type":"integer","minimum":1,"maximum":256,"description":"Window endpoint index, excluded as a timing observation"},"include_start":{"type":"boolean","description":"Explicitly count or omit the starting marker as timing evidence"}}),
        &[
            "kind",
            "marker_set_id",
            "start_index",
            "end_index",
            "include_start",
        ],
    );
    let cached = object(
        json!({"kind":{"const":"cached_onsets"},"cache_token":{"type":"string","minLength":1},"start_frame":{"type":"integer","minimum":0,"maximum":u64::MAX},"end_frame":{"type":"integer","minimum":1,"maximum":u64::MAX,"description":"Exclusive native-source frame end, at most300seconds after start"},"basis":{"type":"string","enum":["peak","refined_attack"],"description":"Use trusted shared repick candidate peaks or refined attack suggestions; never treat them as interchangeable"}}),
        &["kind", "cache_token", "start_frame", "end_frame", "basis"],
    );
    json!({"type":"object","oneOf":[markers,cached]})
}
fn tempo_inspect_schema() -> Value {
    let common = json!({"expected_revision":revision_schema(),"analysis_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_TEMPO_PAGE,"default":MAX_TEMPO_PAGE}});
    let mut points = common.clone();
    points["kind"] = json!({"const":"points"});
    let mut pulses = common;
    pulses["kind"] = json!({"const":"pulses"});
    pulses["candidate_index"] = json!({"type":"integer","minimum":0,"maximum":crate::tempo::MAX_CANDIDATES-1,"description":"Explicit index in the retained analysis candidates, never an injected BPM"});
    let mut properties = pulses.clone();
    properties["kind"] = json!({"type":"string","enum":["points","pulses"]});
    json!({"type":"object","properties":properties,"additionalProperties":false,"oneOf":[object(points,&["expected_revision","analysis_token","kind"]),object(pulses,&["expected_revision","analysis_token","kind","candidate_index"])]})
}
fn track_mix_config_schema() -> Value {
    object(
        json!({
            "version":{"type":"integer","const":crate::track_mix::VERSION},
            "settings":{"type":"array","maxItems":crate::track_mix::MAX_TRACKS,
                "description":"Complete replacement by unique existing TrackIds, not a patch. Empty resets all tracks to neutral defaults. Duplicate IDs reject; no silent last-wins behavior. Values validate in f64 before f32 storage; no gain amplification.",
                "items":object(json!({
                    "track":stable_id_schema(),
                    "gain":{"type":"number","minimum":0,"maximum":1},
                    "balance":{"type":"number","minimum":-1,"maximum":1},
                    "mute":{"type":"boolean"},"solo":{"type":"boolean"}
                }), &["track","gain","balance","mute","solo"])}
        }),
        &["version", "settings"],
    )
}
fn cell_block_scope_schema() -> Value {
    object(
        json!({
            "pattern":stable_id_schema(),"track":stable_id_schema(),
            "row_start":{"type":"integer","minimum":0,"maximum":65535},
            "row_end":{"type":"integer","minimum":1,"maximum":65536},
            "column_start":{"type":"integer","minimum":0,"maximum":255},
            "column_end":{"type":"integer","minimum":1,"maximum":256}
        }),
        &[
            "pattern",
            "track",
            "row_start",
            "row_end",
            "column_start",
            "column_end",
        ],
    )
}
fn cell_block_destination_schema() -> Value {
    object(
        json!({
            "pattern":stable_id_schema(),"track":stable_id_schema(),
            "row_start":{"type":"integer","minimum":0,"maximum":65535},
            "column_start":{"type":"integer","minimum":0,"maximum":255}
        }),
        &["pattern", "track", "row_start", "column_start"],
    )
}
fn cell_block_operation_schema() -> Value {
    json!({"oneOf":[
        object(json!({"kind":{"const":"paste_replace"},"clipboard_token":{"type":"string","minLength":1},"destination":cell_block_destination_schema()}), &["kind","clipboard_token","destination"]),
        object(json!({"kind":{"const":"clear"},"scope":cell_block_scope_schema()}), &["kind","scope"])
    ]})
}
fn cell_block_inspect_schema() -> Value {
    let page = json!({"expected_revision":revision_schema(),
        "start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},
        "limit":{"type":"integer","minimum":1,"maximum":crate::cell_blocks::MAX_PAGE,"default":crate::cell_blocks::MAX_PAGE}});
    let mut variants = Vec::new();
    for kind in ["clipboard", "changes", "contexts", "aliases"] {
        let mut fields = page.clone();
        fields["kind"] = json!({"const":kind});
        let token = if kind == "clipboard" {
            "clipboard_token"
        } else {
            "preview_token"
        };
        fields[token] = json!({"type":"string","minLength":1});
        variants.push(object(fields, &["expected_revision", "kind", token]));
    }
    let mut fields = page;
    fields["kind"] = json!({"type":"string","enum":["clipboard","changes","contexts","aliases"]});
    fields["clipboard_token"] = json!({"type":"string","minLength":1});
    fields["preview_token"] = json!({"type":"string","minLength":1});
    json!({"type":"object","properties":fields,"additionalProperties":false,"oneOf":variants})
}
fn marker_bank_config_schema() -> Value {
    object(
        json!({
            "version":{"type":"integer","const":crate::marker_bank::VERSION},
            "marker_set":stable_id_schema(),
            "interval_indices":{"type":"array","minItems":1,"maxItems":crate::marker_bank::MAX_INTERVALS,"uniqueItems":true,
                "items":{"type":"integer","minimum":0,"maximum":255},
                "description":"Explicit strictly ascending unique zero-based interval indices in the persisted accepted marker set. Each i selects exactly [markers[i],markers[i+1]); sparse indices are allowed. Never sorted, merged or truncated."},
            "name":{"type":"string","minLength":1,"maxLength":256,"description":"Nonblank bank name, at most 256 UTF-8 bytes; used for the new kit and assigned instrument. No BPM/root/role inference."}
        }),
        &["version", "marker_set", "interval_indices", "name"],
    )
}
fn audition_target_schema() -> Value {
    let envelope = object(
        json!({
            "attack_ms":{"type":"integer","minimum":0,"maximum":10000},
            "decay_ms":{"type":"integer","minimum":0,"maximum":10000},
            "sustain_percent":{"type":"integer","minimum":0,"maximum":100},
            "release_ms":{"type":"integer","minimum":0,"maximum":500}
        }),
        &["attack_ms", "decay_ms", "sustain_percent", "release_ms"],
    );
    let looping = object(
        json!({
            "start_frame":{"type":"integer","minimum":0,"maximum":u64::MAX},
            "end_frame":{"type":"integer","minimum":2,"maximum":u64::MAX}
        }),
        &["start_frame", "end_frame"],
    );
    json!({"oneOf":[
        object(json!({
            "kind":{"const":"region_draft"},
            "sample_id":stable_id_schema(),
            "start_frame":{"type":"integer","minimum":0,"maximum":u64::MAX,"description":"Inclusive original source frame, not output time"},
            "end_frame":{"type":"integer","minimum":1,"maximum":u64::MAX,"description":"Exclusive original source frame within registered source bounds"},
            "root_note":{"type":"integer","minimum":0,"maximum":127,"description":"Native-pitch reference; the preview ON uses this same note, independent of Song BPM"},
            "amp_envelope":{"oneOf":[{"type":"null"},envelope],"description":"Required nullable output-clock ADSR; null is legacy de-click behavior, not an explicit ADSR"},
            "forward_loop":{"oneOf":[{"type":"null"},looping],"description":"Required nullable original-source-frame loop, at least two frames wholly within the region; no crossfade"}
        }), &["kind","sample_id","start_frame","end_frame","root_note","amp_envelope","forward_loop"]),
        object(json!({
            "kind":{"const":"pad"},
            "instrument_id":stable_id_schema(),
            "pad_id":stable_id_schema()
        }), &["kind","instrument_id","pad_id"])
    ]})
}
fn resample_config_schema() -> Value {
    object(
        json!({"version":{"type":"integer","const":crate::resample::VERSION},"occurrence":{"type":"integer","minimum":0,"maximum":u32::MAX,"description":"Explicit zero-based order occurrence; all tracks cold at frame0, not an arrangement crop"},
        "name":{"type":"string","minLength":1,"maxLength":256,"description":"Nonblank name, at most256 UTF-8 bytes, for new region/instrument"},"root_note":{"type":"integer","minimum":0,"maximum":127,"description":"Native replay MIDI root of the whole new region; rendering itself is not transposed"},
        "tail":{"type":"string","enum":["MusicalOnly","ReleaseTail"],"description":"MusicalOnly truncates at musical end and may click; ReleaseTail adds exactly500ms/24000frames"},
        "output":{"type":"string","minLength":1,"description":"New workspace-relative .wav under an existing directory, no traversal/symlinks/overwrite. Published file is durable independently of document Undo; WAV only, no sidecar"}}),
        &[
            "version",
            "occurrence",
            "name",
            "root_note",
            "tail",
            "output",
        ],
    )
}
fn keyzone_input_schema() -> Value {
    object(
        json!({"id":stable_id_schema(),"name":{"type":"string","minLength":1,"maxLength":128,"description":"Nonblank name, at most128 UTF-8 bytes"},
        "note_min":{"type":"integer","minimum":0,"maximum":127,"description":"Inclusive audible MIDI note range, not a kit selector"},"note_max":{"type":"integer","minimum":0,"maximum":127},
        "velocity_min":{"type":"integer","minimum":0,"maximum":127,"description":"Inclusive selection bin from pre-source incoming ON velocity; audio velocity is not quantized"},"velocity_max":{"type":"integer","minimum":0,"maximum":127},
        "region":stable_id_schema(),"root_note":{"type":"integer","minimum":0,"maximum":127,"description":"Explicit native reference overriding region root; audible note transposes relative to this root"}}),
        &[
            "id",
            "name",
            "note_min",
            "note_max",
            "velocity_min",
            "velocity_max",
            "region",
            "root_note",
        ],
    )
}
fn groove_capture_schema() -> Value {
    object(
        json!({
            "version":{"type":"integer","const":crate::groove::VERSION},
            "name":{"type":"string","minLength":1,"maxLength":256,"description":"Nonempty historical template name; shared UTF-8 byte limit applies"},
            "marker_set":stable_id_schema(),
            "start_marker":{"type":"integer","minimum":0,"maximum":256,"description":"Index of origin boundary in the accepted marker set; not a source frame"},
            "end_marker":{"type":"integer","minimum":1,"maximum":256,"description":"Exclusive marker-window boundary index, strictly after start; end marker is not an anchor"},
            "include_start_anchor":{"type":"boolean","description":"Explicitly include or omit the window-origin marker as a zero-phase anchor"},
            "division":{"type":"string","enum":["Sixteenth","ThirtySecond","SixtyFourth","OneTwentyEighth"],"description":"Declared whole-note subdivision; BPM comes from accepted MarkerSet.source_bpm, never inferred"},
            "cycle_steps":{"type":"integer","minimum":1,"maximum":1024,"description":"Explicit repeated grid period in subdivision steps; not inferred from audio or marker count"}
        }),
        &[
            "version",
            "name",
            "marker_set",
            "start_marker",
            "end_marker",
            "include_start_anchor",
            "division",
            "cycle_steps",
        ],
    )
}
fn groove_apply_schema() -> Value {
    object(
        json!({"groove":stable_id_schema(),"scope":transform_scope_schema(),"strength_percent":{"type":"integer","minimum":0,"maximum":100,"description":"Blend current note phase toward captured template phase, not add an offset:0 true bit-preserving no-op,100 exact rounded rational target. No velocity/accent changes"}}),
        &["groove", "scope", "strength_percent"],
    )
}
fn transform_scope_schema() -> Value {
    object(
        json!({
            "pattern":stable_id_schema(),"track":stable_id_schema(),
            "row_start":{"type":"integer","minimum":0,"maximum":u32::MAX,"description":"Inclusive source-pattern row; no inferred selection"},
            "row_end":{"type":"integer","minimum":1,"maximum":u32::MAX,"description":"Exclusive source-pattern row; destination timing moves must stay inside this range"},
            "columns":{"type":"array","minItems":1,"maxItems":256,"uniqueItems":true,"items":{"type":"integer","minimum":0,"maximum":255},"description":"Explicit sorted unique zero-based source note columns; backend validates actual track geometry"}
        }),
        &["pattern", "track", "row_start", "row_end", "columns"],
    )
}
fn transform_schema() -> Value {
    let scope = transform_scope_schema();
    let fields = object(
        json!({
            "notes":{"type":"boolean","description":"Transpose selected melodic notes (Pitched/Multisample); kit/ambiguous instrument contexts reject rather than changing pad selectors"},
            "glide_targets":{"type":"boolean","description":"Independently transpose explicit absolute MIDI glide targets"},
            "velocity":{"type":"boolean","description":"Scale only explicit velocity fields; unspecified remembered velocity is never materialized"},
            "timing":{"type":"boolean","description":"Move whole parent cells and translate row-local commands; no collision, overtaking or out-of-selection moves. Nonzero phrase-trigger timing is unsupported. Crossing mute boundaries may change audible event counts"}
        }),
        &["notes", "glide_targets", "velocity", "timing"],
    );
    let swing = object(
        json!({
            "division":{"type":"string","enum":["Sixteenth","ThirtySecond","SixtyFourth","OneTwentyEighth"],"description":"Pattern-row-zero anchored whole-note subdivisions; only odd subdivisions displaced"},
            "amount_percent":{"type":"integer","minimum":0,"maximum":50,"description":"Percentage of subdivision duration, rounded once to integer 48 kHz output frames"}
        }),
        &["division", "amount_percent"],
    );
    object(
        json!({"scope":scope,"fields":fields,
            "transpose_semitones":{"type":"integer","minimum":-48,"maximum":48,"description":"Applied independently to notes/glide targets selected by the field mask; no MIDI clamping"},
            "velocity_scale":{"type":"number","minimum":0,"maximum":4,"description":"Finite explicit-velocity multiplier; checked before f32 conversion, effective shared f32 value returned. Results outside 0..1 reject, never clamp"},
            "nudge_frames":{"type":"integer","minimum":-48000,"maximum":48000,"description":"Signed 48 kHz output/sample-clock frames, not native-source frames or milliseconds"},
            "swing":{"oneOf":[{"type":"null"},swing],"description":"Required nullable offline swing; null disables it. Not a persistent runtime/global groove"}
        }),
        &[
            "scope",
            "fields",
            "transpose_semitones",
            "velocity_scale",
            "nudge_frames",
            "swing",
        ],
    )
}
fn onset_picker_schema() -> Value {
    object(
        json!({
            "enable_phase":{"type":["boolean","null"],"description":"Explicit optional supporting phase evidence; OFF in application presets, never a standalone trigger, may add false positives"},
            "threshold":{"type":["number","null"],"minimum":0.1,"maximum":0.95,"description":"Lower threshold is more sensitive; heuristic, not probability"},
            "mad_multiplier":{"type":["number","null"],"minimum":0.5,"maximum":12,"description":"Local median-absolute-deviation multiplier; larger is more conservative"},
            "median_radius_ms":{"type":["number","null"],"minimum":20,"maximum":250,"description":"Local normalization radius in milliseconds; larger work may exceed bounded budget"},
            "min_gap_ms":{"type":["number","null"],"minimum":1,"maximum":1000},
            "dense_gap_ms":{"type":["number","null"],"minimum":1,"maximum":1000,"description":"Hard lower spacing; must not exceed min_gap_ms; near candidates need strong fast-attack evidence and novelty dip"},
            "dense_evidence":{"type":["number","null"],"minimum":0.5,"maximum":1},
            "refinement":{"oneOf":[{"type":"null"},object(json!({
                "enabled":{"type":["boolean","null"]},
                "backtrack_ms":{"type":["number","null"],"minimum":0,"maximum":100},
                "lookahead_ms":{"type":["number","null"],"minimum":0,"maximum":50},
                "attack_fraction":{"type":["number","null"],"minimum":0.001,"maximum":0.5,"description":"Relative local attack threshold; does not merge candidate peak and refined attack identity"}
            }),&[])]}
        }),
        &[],
    )
}
fn sample_peak_config_schema() -> Value {
    object(
        json!({"version":{"const":crate::meters::VERSION},"start_frame":{"type":"integer","minimum":0,"maximum":crate::meters::MAX_ANALYSIS_FRAMES,"multipleOf":crate::meters::WINDOW_FRAMES,"description":"Inclusive48kHz Song/Engine frame, not native asset frame; context still advances from0"},"end_frame":{"type":"integer","minimum":1,"maximum":crate::meters::MAX_ANALYSIS_FRAMES,"description":"Exclusive48kHz Song/Engine frame; must be2400-aligned OR exactly the natural musical+tail terminal. Greater than start and within Song; shared validation rejects without rounding"}}),
        &["version", "start_frame", "end_frame"],
    )
}
fn revision_schema() -> Value {
    json!({"type":"integer","minimum":0,"maximum":u64::MAX,"description":"Revision returned by project_read; mismatch rejects the request without mutation"})
}
fn path_schema() -> Value {
    json!({"type":"string","minLength":1,"description":"Path relative to explicit startup workspace; existing parent directories required; no absolute paths, traversal or symlinks"})
}
fn tool(
    name: &str,
    title: &str,
    description: &str,
    schema: Value,
    read_only: bool,
    destructive: bool,
) -> Value {
    json!({"name":name,"title":title,"description":description,"inputSchema":schema,
        "outputSchema":{"type":"object","required":["ok","revision","data"],"additionalProperties":false,"properties":{"ok":{"type":"boolean"},"revision":{"type":"integer","minimum":0},"data":{"type":"object"},"error":{"type":"object","required":["code","message","details"],"properties":{"code":{"type":"string"},"message":{"type":"string"},"details":{"type":"object"}},"additionalProperties":false}}},
        "annotations":{"readOnlyHint":read_only,"destructiveHint":destructive,"idempotentHint":read_only,"openWorldHint":false}})
}
/// Discoverable fixed tool inventory with strict JSON Schema 2020-12 input shapes.
pub fn tool_catalog() -> Vec<Value> {
    let empty = object(json!({}), &[]);
    let revision = object(
        json!({"expected_revision":revision_schema()}),
        &["expected_revision"],
    );
    let path = object(
        json!({"expected_revision":revision_schema(),"path":path_schema()}),
        &["expected_revision", "path"],
    );
    vec![
        tool(TOOL_NAMES[0],"Capabilities and boundaries","Read API/protocol versions, supported musical operations/effects, root policy, limits and explicit unavailable capabilities. Start here: this session does not control an open GUI or live audio.",empty.clone(),true,false),
        tool(TOOL_NAMES[1],"Read current song or section","Inspect the current headless song and its revision, stable IDs, sparse pattern cells, instrument-bound phrases, 16-pad drum kits, velocity zones and stable round-robin layer IDs, melodic multisample maps/keyzone IDs, imported source metadata, region boundaries/mappings and optional envelopes/loops, accepted marker sets, historical generation/groove provenance, persisted track gain/balance/mute/solo settings, order and undo state. Track-mix reads are metadata only, not compiled-routing or live-GUI status. Read does not load another file.",object(json!({"section":{"type":["string","null"],"enum":["summary","document","tracks","instruments","patterns","order","history","phrases","samples","regions","marker_sets","drum_kits","generation","grooves","multisamples","track_mix",null],"default":"summary"}}),&[]),true,false),
        tool(TOOL_NAMES[2],"New blank project","Replace this headless document with the same blank song as GUI New; clear history. Dirty changes require explicit discard_unsaved:true. Does not create a file.",object(json!({"expected_revision":revision_schema(),"discard_unsaved":{"type":"boolean","default":false}}),&["expected_revision"]),false,true),
        tool(TOOL_NAMES[3],"Open project safely","Open a .leviathan.json project under the startup root after validating every embedded sample reference before file access. Dirty changes require explicit discard_unsaved:true. Does not synchronize a GUI.",object(json!({"expected_revision":revision_schema(),"path":path_schema(),"discard_unsaved":{"type":"boolean","default":false}}),&["expected_revision","path"]),false,true),
        tool(TOOL_NAMES[4],"Save project with overwrite protection","Save via shared atomic project writer to .leviathan.json. Omit path only for an already named project. Existing files require a reviewed revision-bound token from prepare_overwrite; changed bytes cancel replacement.",object(json!({"expected_revision":revision_schema(),"path":{"oneOf":[{"type":"null"},path_schema()]},"overwrite_token":{"type":["string","null"]}}),&["expected_revision"]),false,true),
        tool(TOOL_NAMES[5],"Prepare explicit overwrite confirmation","Read an existing .leviathan.json file and return a single-use token tied to its exact bytes, destination and current revision. No file is changed. Use save with this token only after replacement is approved.",path.clone(),false,false),
        tool(TOOL_NAMES[6],"Apply atomic song edit batch","Apply 1..256 typed ordered song/phrase/kit/pad/region/marker/mapping edits as exactly one undo transaction. Import verified sample identities with sample_import first. Every operation must leave a valid Song; any failure rolls back the entire batch. Stable IDs are explicit except create_drum_kit_instrument uses the shared checked allocator; read allocated IDs afterward. No-op batches preserve revision/history.",edit_schema(),false,true),
        tool(TOOL_NAMES[7],"Undo one song transaction","Undo one edit batch/import/compiler commit using the shared Editor history; no-op if empty. File saves and created exports are not undone.",revision.clone(),false,true),
        tool(TOOL_NAMES[8],"Redo one song transaction","Redo one edit batch using the shared Editor history; no-op if empty.",revision.clone(),false,true),
        tool(TOOL_NAMES[9],"Evaluate exact song events","Compile the current snapshot with the same evaluator as export/transport. Returns inspectable event frames, note/effect commands, declared source provenance, persisted track-mix routing and timing; event compilation does not verify source hashes/decoding (source_identity_verified:false). Use inspect or export for verification. No audio device/output file.",revision,true,false),
        tool(TOOL_NAMES[10],"Export song WAV and event manifest","Render the current snapshot including persisted track gain/balance/mute/solo through the shared Engine to new .wav and companion .events.json files. Both destinations must be unused. Reports frames, peak and paths; does not imply speaker audibility or physical latency.",path.clone(),false,false),
        tool(TOOL_NAMES[11],"Generate deterministic demo sample","Create a new one-second 48 kHz stereo float32 WAV using the existing demo sample generator. Never overwrite; does not automatically assign it to an instrument.",path,false,false),
        tool(TOOL_NAMES[12],"List root musical artifacts","List immediate regular .wav, .flac and .leviathan.json filenames in the authorized root, excluding symlinks. Nonrecursive, bounded scan; not a generic filesystem browser.",empty,true,false),
        tool(TOOL_NAMES[13],"Import verified WAV/FLAC source","Decode one existing root-confined WAV/FLAC without resampling, normalization or modifying the file. Register SHA-256 and real native frames/channels/rate under a new immutable sample ID; one undo step, no automatic region or instrument assignment. Synchronous, maximum 64 MiB encoded and decoded; no async cancellation.",object(json!({"expected_revision":revision_schema(),"sample_id":stable_id_schema(),"path":path_schema()}),&["expected_revision","sample_id","path"]),false,false),
        tool(TOOL_NAMES[14],"Inspect and verify imported source","Re-read a registered source through bounded strict decode; reject changed encoded bytes or metadata. Return native metadata and independent bounded pages of regions, kit single/zone/alternative mappings (stable layer IDs and vector positions), and multisample map/keyzone references (stable IDs and exact ranges/root), not PCM. Follow each page next_start; defaults64/max128 records per axis and 256 KiB structured payload. Revision/source identity bind each page. Read-only synchronous strict decode, maximum64MiB encoded/decoded.",object(json!({"expected_revision":revision_schema(),"sample_id":stable_id_schema(),"regions_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"regions_limit":{"type":"integer","minimum":1,"maximum":MAX_SOURCE_INSPECTION_PAGE,"default":64},"mappings_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"mappings_limit":{"type":"integer","minimum":1,"maximum":MAX_SOURCE_INSPECTION_PAGE,"default":64},"keyzones_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"keyzones_limit":{"type":"integer","minimum":1,"maximum":MAX_SOURCE_INSPECTION_PAGE,"default":64}}),&["expected_revision","sample_id"]),true,false),
        tool(TOOL_NAMES[15],"Analyze transient boundary suggestions","Analyze a verified registered source using shared adaptive energy/difference detector. Return sorted original-source-frame suggestions, heuristic confidence and waveform overview. Does not edit accepted regions, BPM or source audio; not tempo/beat detection. Synchronous: at most 60 seconds, 64 MiB decoded, 4096 suggestions and 2048 overview bins; no async cancellation or partial/truncated analysis.",object(json!({"expected_revision":revision_schema(),"sample_id":stable_id_schema(),"config":analysis_schema()}),&["expected_revision","sample_id"]),true,false),
        tool(TOOL_NAMES[16],"Preview editable break compilation and diff","Verify source identity/metadata and run the shared pure break compiler against an accepted marker set. Return its bounded diff, generated IDs, config and review token; retain one private candidate, never mutate song/history/revision. A newer successful preview invalidates the older token. Explicit source/target BPM and tail policy; no inferred tempo/classification/stretch. Synchronous bounded source decode, at most 2048 starts; structured preview payload at most 256 KiB (MCP text mirroring adds bounded wire overhead).",object(json!({"expected_revision":revision_schema(),"config":compile_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[17],"Commit reviewed break preview atomically","Apply the current preview only after revision, exact base Song and source hash/path checks. Shared Editor commits target BPM, allocated regions/instruments, new track/pattern and appended order as one undo entry. No source-file changes. Any failed commit leaves document and pending preview unchanged; successful revision changes invalidate previews. Requires a reviewed token, not a client-supplied Song patch.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[18],"Review source marker cleanup proposal","Verify one registered source and suggest sorted full-source boundaries from explicit candidate frames using the shared cleanup helper. Reports each duplicate/near-end/too-close removal; no marker set or document changes, no automatic attack snapping. At most 4096 candidates, 257 resulting boundaries and 256 KiB structured payload; strict source decode at most 64 MiB. Accept only with an explicit upsert_marker_set edit.",object(json!({"expected_revision":revision_schema(),"sample_id":stable_id_schema(),"candidates":{"type":"array","maxItems":MAX_ANALYSIS_SUGGESTIONS,"items":{"type":"integer","minimum":0,"maximum":u64::MAX}},"minimum_slice_frames":{"type":"integer","minimum":1,"maximum":u64::MAX}}),&["expected_revision","sample_id","candidates","minimum_slice_frames"]),true,false),
        tool(TOOL_NAMES[19],"Prepare experimental v2 onset evidence cache","Explicitly opt into experimental v2 (phase-off default, optional explicit phase support) using Conservative/Balanced/GhostHunter; v1 sample_analyze remains unchanged/default. Shared verified native-PCM extraction returns result counts, heuristic-evidence summary, complete config/hash/algorithm binding and token; inspect server-owned peak/attack records using results pagination. Synchronous: <=15 seconds, 4096 proposals, 500M work units, 128 MiB estimated work, 96 MiB retained PCM/curve cache plus 8 MiB serialized analysis, paginated 256 KiB summaries/pages; over-budget fails, never truncates. Starting extraction retires the previous cache even on failure; no document/markers/revision change.",object(json!({"expected_revision":revision_schema(),"sample_id":stable_id_schema(),"engine":{"const":"experimental_v2"},"preset":{"type":"string","enum":["Conservative","Balanced","GhostHunter"]},"picker":onset_picker_schema()}),&["expected_revision","sample_id","engine","preset"]),false,false),
        tool(TOOL_NAMES[20],"Re-pick experimental cached onsets without FFT","Reauthorize and rehash the current source, then call the shared picker/refiner over the server-owned immutable cache; no FFT or decode. Omitted/null picker fields preserve current effective values; selecting preset resets to shared phase-off preset defaults before picker overrides. Successful re-pick returns a new full-config-bound cache token and invalidates the old token; failed re-pick preserves it. Same v2 budgets apply. No client Detection authority, marker mutation, classification or calibrated probability.",object(json!({"expected_revision":revision_schema(),"cache_token":{"type":"string","minLength":1},"preset":{"type":["string","null"],"enum":["Conservative","Balanced","GhostHunter",null]},"picker":onset_picker_schema()}),&["expected_revision","cache_token","picker"]),false,false),
        tool(TOOL_NAMES[21],"Inspect paginated experimental raw evidence","Reauthorize/rehash source before reading a bounded page of cached raw multiband evidence. start/next_start index curve bins; every point has its original source_frame. Raw values are not normalized confidence or accepted markers. Returns cache/source/algorithm/config binding, at most 256 bins / 256 KiB structured payload; no FFT/decode or document change. Any successful document revision invalidates caches.",object(json!({"expected_revision":revision_schema(),"cache_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_ONSET_PAGE,"default":128}}),&["expected_revision","cache_token"]),true,false),
        tool(TOOL_NAMES[22],"Inspect paginated experimental candidates or refined attacks","Read the actual shared service's cached candidate or suggestion records after current root/source SHA verification. Each page carries cache/source/algorithm/full-config binding; at most 64 records / 256 KiB structured payload. Candidate peak_frame and suggestion attack_frame remain separate; scores are heuristic, not probabilities. No automatic marker acceptance, FFT/decode or document changes.",object(json!({"expected_revision":revision_schema(),"cache_token":{"type":"string","minLength":1},"kind":{"type":"string","enum":["candidates","suggestions"]},"start":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_ONSET_RESULT_PAGE,"default":64}}),&["expected_revision","cache_token","kind"]),true,false),
        tool(TOOL_NAMES[23],"Inspect exact drum-step lanes and source cells","Project one explicit pattern/track/kit InstrumentId into a 16- or 32-step page, with pattern-row-zero origin and musical division. Returns exact resolved source/read-only status and row/column/cell snapshots, off-grid/multiple-hit/phrase/selection conflicts and a token bound to revision, exact Song and view. Read-only; no source decode, snapping, inference, column expansion or second sequencer. At most 65536 stored plus resolved-alias cells scanned,2048 source events/issues and256KiB structured payload. New successful inspection supersedes the previous token; any document revision invalidates it.",object(json!({"expected_revision":revision_schema(),"pattern_id":stable_id_schema(),"track_id":stable_id_schema(),"instrument_id":stable_id_schema(),"division":{"type":"string","enum":["Quarter","Eighth","Sixteenth","ThirtySecond","SixtyFourth","OneTwentyEighth"]},"steps":{"type":"integer","enum":[16,32]},"page":{"type":"integer","minimum":0,"maximum":u32::MAX,"default":0}}),&["expected_revision","pattern_id","track_id","instrument_id","division","steps"]),true,false),
        tool(TOOL_NAMES[24],"Inspect paginated order/track matrix","Read at most32 occurrences by32 display-order tracks through the shared alias resolver: exact target and resolved source IDs, stored-local versus resolved cell counts, direct aliases, complete bounded source chains and per-occurrence mute. No source IO or document mutation. Starts are zero-based indices, limits1..32, payload at most256KiB; empty end pages allowed. Existing project_read patterns exposes stored source cells.",object(json!({"expected_revision":revision_schema(),"occurrence_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"occurrence_limit":{"type":"integer","minimum":1,"maximum":32,"default":16},"track_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"track_limit":{"type":"integer","minimum":1,"maximum":32,"default":16}}),&["expected_revision"]),true,false),
        tool(TOOL_NAMES[25],"Preview scoped transpose, velocity and offline timing changes","Run the shared deterministic transform against explicit pattern/track, row range, columns and field mask; no broad inferred selection, seed, random humanization or runtime groove. Return effective options, summary/context counts and a review token; inspect exact changes before commit. Alias targets are read-only; out-of-bounds, collisions, unknown/kit transpose or unrepresentable all-use timing reject atomically. Root path preflight only, no decode/hash. Raw scheduled-frame proof does not promise audible event-count preservation across occurrence mute boundaries. At most4096 selected cells/affected uses,65536 event-use checks,8MiB candidate and256KiB summary. Failed preview preserves the previous token.",object(json!({"expected_revision":revision_schema(),"config":transform_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[26],"Inspect exact transform before/after diff page","Inspect server-owned shared CellChange records for the current review token. Explicit original/destination row, column and exact before/after cells preserve optional/zero/f64 values. At most64 records and256KiB per page, empty end allowed; no silent truncation. Source IDs and repeated/alias-use impact are in effective_config/preview summary. Reauthorize referenced paths; no audio-byte verification or document/history changes.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_TRANSFORM_PAGE,"default":MAX_TRANSFORM_PAGE}}),&["expected_revision","preview_token"]),true,false),
        tool(TOOL_NAMES[27],"Commit reviewed scoped transform in one undo step","Commit only the retained shared transform candidate after current token, MCP/Editor revision, exact full Song and path checks; never accept a caller-supplied candidate. Changed content is one undo entry. No-op consumes its token without advancing revision or clearing redo. Any failed commit preserves document and token; changed revisions invalidate all pending reviews. No source-file changes or live GUI/device control.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[28],"Preview seeded versioned pattrns Euclidean pattern","Run the actual pinned pattrns shared preset with explicit version, seed, finite bars, existing instrument and typed Euclidean/sequence parameters. No code/Lua/scripts, hidden seed or automatic retry. Return effective config, checked fresh track/pattern IDs, summary/provenance and token without editing Song/history. All sequence choices must be mapped/source-assigned; all-closed, bounds/evaluator/ID failures reject atomically. At most2048 raw ticks,65536 each prospective ordered blocks/resolved row visits/cell visits (including empty/default/muted entries),8MiB compact candidate,256KiB summary; shared bounded pretty-project8MiB preflight also rejects unsaveable plans (conservative current paths). Reauthorize paths, no decode/hash claim; inspect exact generated cells before commit.",object(json!({"expected_revision":revision_schema(),"config":generation_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[29],"Inspect exact generated ordinary cells and provenance","Read retained shared generated-cell pages with exact tick, synthetic virtual_time, absolute48kHz Song frame, row/column/cell, effective seed/config/version and provenance. Virtual ticks are not seconds/output frames. At most64 cells/256KiB, explicit total/next offset and empty end page; never silently truncate. Current token/fullbase/revision/root guards; no document mutation or current audio-byte verification.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_GENERATION_PAGE,"default":MAX_GENERATION_PAGE}}),&["expected_revision","preview_token"]),true,false),
        tool(TOOL_NAMES[30],"Commit reviewed generated pattern as one undo","Apply only the private shared seeded plan after token, both revisions, exact full base Song and root checks. Allocate new track/pattern and append one unmuted occurrence, preserving old cells/tempo/LPB; retain historical generation provenance. No caller candidate injection, overwrites, file writes or runtime generator. Any failure preserves plan/document; changed revision consumes and invalidates pending reviews. Generated cells remain normally editable and save/reopen as ordinary Song content.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[31],"Preview historical groove capture from accepted markers","Capture an explicit accepted-marker window using its stored source BPM, declared grid/cycle and include-start policy. Require a fresh nonzero groove_id; never overwrite a template or accept raw anchors. Shared pure bounded preview, no source decode/hash or inference; source SHA is historical declared identity. One active capture/apply plan; failed preview preserves prior plan. Inspect recipe records before commit.",object(json!({"expected_revision":revision_schema(),"groove_id":stable_id_schema(),"config":groove_capture_schema()}),&["expected_revision","groove_id","config"]),false,false),
        tool(TOOL_NAMES[32],"Commit reviewed groove capture as one undo","Commit only a current capture-kind token after both revisions, full base Song and root checks. Shared Editor registers the immutable historical template at the reviewed fresh ID, one undo entry. No file changes or audio verification; conflicting/stale/wrong-kind tokens preserve the pending plan.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[33],"Inspect persisted groove recipe and historical provenance","Read one immutable historical template with bounded anchor pages. Raw source-frame deltas are exact authority; offset_beats is approximate display. Source/marker IDs and SHA need not remain live and are not reverified; no source-file access. Default/max64 anchors,256KiB, explicit total/next offset and empty end page. Revision guard keeps independent requests on the same document.",object(json!({"expected_revision":revision_schema(),"groove_id":stable_id_schema(),"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_GROOVE_PAGE,"default":MAX_GROOVE_PAGE}}),&["expected_revision","groove_id"]),true,false),
        tool(TOOL_NAMES[34],"Preview scoped timing transfer from a captured groove","Apply the shared historical template to explicit ordinary-note selection, pitched or kit selectors unchanged. Origin is scope.row_start, nearest rational grid tick half-up, declared cycles repeat and sparse slots stay unchanged. Strength0 is bit-noop;100 blends to exact rounded target phase, not additive timing. No accents/velocity, inference, audio decode or runtime global groove. Alias targets, collisions/overtaking, changed phrase triggers, out-of-bounds or changed mute eligibility reject. Review exact parent-cell diff and shared warnings before commit; one active capture/apply plan. This is not full renderability or audible-event preservation: interleaving/RR/choke/phrase children can change; evaluation/export checks sources and expanded budgets.",object(json!({"expected_revision":revision_schema(),"config":groove_apply_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[35],"Inspect exact pending groove recipe or timing diff","Read the retained plan identified by token. Capture-kind records are raw anchors plus approximate beat offsets; apply-kind records are exact shared CellChange before/after values. At most64 records/256KiB, explicit kind/total/next offset and empty end page; no silent truncation. Full-base/both-revision/root guards, no audio hash verification or document/history changes.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_GROOVE_PAGE,"default":MAX_GROOVE_PAGE}}),&["expected_revision","preview_token"]),true,false),
        tool(TOOL_NAMES[36],"Commit reviewed groove timing transfer as one undo","Commit only a current apply-kind token after both revisions, exact full base Song and root checks. One shared Editor transaction of ordinary editable cells; no client candidate, source writes or runtime recipe. No-op consumes the token without revision/history/redo changes. Failed commit leaves plan and document unchanged; changed revisions invalidate every pending review.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[37],"Inspect shared melodic multisample map and key zones","Read exact stable map/zone IDs, inclusive note and velocity rectangles, region IDs and explicit native roots, plus independently paginated assigned instruments. Map is mutable and shared: edits affect all users and existing notes on future render. Gaps are valid drafts, not guaranteed coverage; actual uncovered ON rejects in evaluation/export. No source IO, root-pitch inference, audio hash verification or document/history mutation. Default/max64 records per axis,256KiB payload, exact totals/next offsets and empty end pages.",object(json!({"expected_revision":revision_schema(),"map_id":stable_id_schema(),"zones_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"zones_limit":{"type":"integer","minimum":1,"maximum":MAX_KEYZONE_PAGE,"default":64},"instruments_start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"instruments_limit":{"type":"integer","minimum":1,"maximum":MAX_KEYZONE_PAGE,"default":64}}),&["expected_revision","map_id"]),true,false),
        tool(TOOL_NAMES[38],"Prepare isolated occurrence resampling to a new asset","Review a shared pure plan for one explicit cold occurrence/all tracks, retaining aliases/mutes but not incoming arrangement state; phrase instances and RR counters restart. Actually used legacy paths reject: import/register regions first. Strict explicit tail/root/output/name/version, fresh checked sample/region/instrument IDs and bounded work/save preflight. MCP path authorization only, no source hashing/render/file creation here. Fixed pre-master PCM×2 (not normalization); inspect scope/warnings before render. Successful new prepare supersedes the private workflow but never deletes any prior published WAV.",object(json!({"expected_revision":revision_schema(),"config":resample_config_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[39],"Render prepared occurrence as a durable new WAV","Unix-only guarded publication in v1 (other platforms fail closed). Side-effectful bounded synchronous shared Engine render, verify/import owned sibling temporary and atomic no-clobber publication of WAV only. Reauthorize sources/output; used registered identities verified. Float32 asset=Engine PCM×2 under fixed master0.5, may exceed1; not normalization. No document/history change. Same workflow_token becomes Printed for recoverable inspection/registration; repeated render refuses. Published file survives failed/stale registration, supersession, Undo and process close; no in-process cancellation or latency SLA.",object(json!({"expected_revision":revision_schema(),"workflow_token":{"type":"string","minLength":1}}),&["expected_revision","workflow_token"]),false,false),
        tool(TOOL_NAMES[40],"Inspect prepared or printed resample receipt and source page","Read private workflow scope/config/summary/IDs and paginated exact selected source identities; Printed adds output hash/metadata/peaks/energy and historical render receipt. At most64 source records/256KiB per page, explicit totals/next offset and empty end allowed. Metadata-only, no source/output IO or fresh identity verification. No retained event-snapshot claim. Latest expected_revision and matching token required; stale Printed receipts remain readable with registration_current=false, but render/register retain strict full-base/both-revision guards. Published artifacts remain independently durable.",object(json!({"expected_revision":revision_schema(),"workflow_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_RESAMPLE_PAGE,"default":MAX_RESAMPLE_PAGE}}),&["expected_revision","workflow_token"]),true,false),
        tool(TOOL_NAMES[41],"Register printed WAV as new sample, region and instrument","Reauthorize the published path/parent and verify the prospective whole candidate through shared storage preflight: all source paths, registered identities including output SHA,64MiB/file,256MiB registered aggregate and8MiB pretty-project bound. Then commit through Editor, guarded by token, both revisions and exact full base. One Undo adds fresh immutable sample identity, whole region and Pitched instrument, without altering old notes/order. No caller asset metadata or candidate accepted. Failed registration preserves receipt/file; success consumes workflow. Undo removes document references only; Redo does not rewrite/reverify the durable WAV.",object(json!({"expected_revision":revision_schema(),"workflow_token":{"type":"string","minLength":1}}),&["expected_revision","workflow_token"]),false,false),
        tool(TOOL_NAMES[42],"Analyze reviewable tempo and pulse-phase hypotheses","Use an explicit accepted-marker window or current server-owned experimental v2 cache with explicit peak/refined-attack basis; never client evidence. Shared deterministic bounded timing analysis, no decode/new FFT or document mutation. Explicit BPM range in tenths, all returned alternatives/component scores/alias ambiguity visible; scores are heuristic NOT probabilities, insufficient evidence has no fallback BPM. Marker identities are stored/unverified; cached mode reauthorizes/hash-checks source and binds current cache/config epoch. One retained analysis; failed Analyze preserves prior review. No downbeat/meter/role/loop-length or stretch claim.",object(json!({"expected_revision":revision_schema(),"evidence":tempo_evidence_schema(),"config":tempo_config_schema()}),&["expected_revision","evidence","config"]),false,false),
        tool(TOOL_NAMES[43],"Inspect exact tempo evidence points or candidate pulse grid","Read the retained shared analysis and bounded exact source-frame point or pulse pages. Choose kind=points, or kind=pulses with explicit candidate_index;64 records/256KiB max, totals/next offset and empty end allowed, no silent truncation. Exact rational phase is window-relative and advisory, not downbeat. Full-base/both-revision/cache-epoch guards; cached evidence freshly reauthorizes/hash-checks the source, markers remain metadata-only. No document changes or independent scoring.",tempo_inspect_schema(),true,false),
        tool(TOOL_NAMES[44],"Preview explicit source-BPM choice for a marker set","Choose one retained candidate_index and an explicit same-source target_marker_set_id. Shared pure Use-BPM candidate changes only source_bpm, preserving marker frames/locks/name/sample, Song tempo, audio and baked cells. Phase is NOT applied; compiler origin stays frame0. Review before commit; no raw BPM/candidate injection. Both revisions/full base and cached evidence token/config epoch remain authoritative; failed preview keeps the previous Use review.",object(json!({"expected_revision":revision_schema(),"analysis_token":{"type":"string","minLength":1},"target_marker_set_id":stable_id_schema(),"candidate_index":{"type":"integer","minimum":0,"maximum":crate::tempo::MAX_CANDIDATES-1}}),&["expected_revision","analysis_token","target_marker_set_id","candidate_index"]),false,false),
        tool(TOOL_NAMES[45],"Commit reviewed source BPM only as one undo","Commit only the current private Use-BPM preview after full-base, MCP/Editor revision, root and exact cached-evidence epoch checks. One shared Editor transaction changes the chosen marker set source_bpm; no phase, source-frame, Song-clock or audio changes. No-op consumes only the Use review without changing revision/dirty/history/redo. Response is preflighted before mutation and returns bounded history indicators only, explicitly omitting title/path/document counts. Stale/invalid requests preserve document; no automatic BPM assignment or calibrated tempo claim.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[46],"Analyze native slice acoustic descriptors and heuristic suggestions","Explicit sorted unique regions of one registered source; strict verified decode or current authorized/hash-verified opaque onset PCM. Both compute NEW feature FFTs over full bounds, bounded synchronous work. No automatic label, BPM, playback or document changes. Suggestions expose named handcrafted rules, NOT probabilities; Ghost is manual context. One retained compact result independent of later onset picker changes; failed Analyze retains old review.",object(json!({"expected_revision":revision_schema(),"request":object(json!({"version":{"type":"integer","const":crate::slice_features::VERSION},"sample":stable_id_schema(),"regions":{"type":"array","minItems":1,"maxItems":crate::slice_features::MAX_REGIONS,"uniqueItems":true,"items":stable_id_schema(),"description":"Explicit strictly ascending RegionIds, all from this sample; never sorted/deduplicated silently"}}),&["version","sample","regions"]),"source":feature_source_schema()}),&["expected_revision","request","source"]),false,false),
        tool(TOOL_NAMES[47],"Inspect measured slice preview or historical catalog pages","Pending pages require current full-base/both-revision token but only display computed historical numbers, without fresh file IO/hash. Stored pages enumerate current regions of explicit sample, including uncatalogued entries; metadata-only with NO source-file IO, so missing/tampered media does not hide history. Current-match is model source/bounds equality, NOT fresh bytes/computation/attestation. Native frame units and effective spectral layout included.16records/256KiB max; exact totals/next offsets, empty end pages, no silent truncation.",feature_inspect_schema(),true,false),
        tool(TOOL_NAMES[48],"Review storing descriptors and explicit slice role choices","Create opaque shared Store preview from retained measured result after fresh authorized source hash. All measured descriptors stored; only explicit unique selected-region roles replace reviewed labels. Required roles:[] means descriptors-only, preserving old labels and their historical binding. No arbitrary metric/candidate injection; Ghost is explicit context. Failed review preserves previous review.",object(json!({"expected_revision":revision_schema(),"analysis_token":{"type":"string","minLength":1},"roles":{"type":"array","maxItems":crate::slice_features::MAX_REGIONS,"items":object(json!({"region":stable_id_schema(),"label":role_label_schema()}),&["region","label"])}}),&["expected_revision","analysis_token","roles"]),false,false),
        tool(TOOL_NAMES[49],"Store reviewed slice catalog metadata as one undo","Fresh source authorization/hash plus exact full-base and both revisions guard shared Editor commit. Stores measured descriptors and only explicitly chosen roles; no audio/BPM/playback changes. Pre-sized bounded history-only response, no arbitrary title/path echo. No-op consumes Store review without history/revision/redo change. Stored values remain historical metadata, not a signed attestation or fresh recomputation.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[50],"Preview deterministic reviewed-role THIN cell deletion","Explicit version/scope/seed/integer removal percent/max removals/protected roles. Real pinned pattrns gate once per eligible stored cell; no per-occurrence/live random state. Shared eligibility protects uncertain/stale/unreviewed/Mixed/Unknown and any protected use; alias targets reject. Whole model-evaluable prerequisite even0%; retained-ON/owned-control candidate proof rejects semantic drift. No source hash/decode, but normal MCP all-path policy applies. Review bounded exact decisions/uses/contexts/changes and reversible record before one-Undo commit; no arbitrary code/candidate.",object(json!({"expected_revision":revision_schema(),"config":thin_config_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[51],"Inspect exact THIN decisions, resolved uses, contexts or deletions","Choose one independently paginated record kind from the current opaque plan: decisions with original cells and retention/gate reasons, resolved unsuppressed ON uses with reviewed source roles, ordered affected contexts, or exact reversible before→None changes.64records/256KiB max; independent totals/next offsets and empty end pages, no hidden truncation. Both revisions/full-base/path guards; source identity is declared model metadata, not freshly verified bytes. No document mutation or duplicate evaluation/randomness.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1},"kind":{"type":"string","enum":["decisions","uses","contexts","changes"]},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_THIN_PAGE,"default":MAX_THIN_PAGE}}),&["expected_revision","preview_token","kind"]),true,false),
        tool(TOOL_NAMES[52],"Commit reviewed THIN deletions as one undo","Only current private token, exact full base and both revisions authorize shared Editor commit. Removes whole ordinary cells; no source/role/BPM rewrite or persistent live recipe. Pre-sized bounded history/record response before mutation. No-op consumes review without revision/history/redo change; stale/error leaves current review/document untouched. Standard path authorization remains, no fresh source hash or PCM-preservation claim.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[53],"Prepare region or pad audition metadata (no audio)","Resolve a shared isolated one-ON snapshot: fixed2s musical +0.5s tail,120000 frames at48kHz. Explicit uncommitted region bounds/root/ADSR/loop at native pitch, or committed instrument/pad at unity input velocity1.0/bin127 with actual mapping/gain/pan/tune/root/chosen zone/first RR. Fresh context each request; no pattern/cursor/history/dirty/revision change. Path authorization includes all retained selected-pad alternatives (stricter than native chosen-source loading); declared SHA is NOT freshly verified, no decode, audio rendering, file creation, device or GUI control. Complete summary <=256KiB, never silently truncated. review_ref binds exact base/request/revisions/shared snapshot for provenance only, not a retained/consumable capability. No audition-render/export tool or MIDI.",object(json!({"expected_revision":revision_schema(),"target":audition_target_schema()}),&["expected_revision","target"]),true,false),
        tool(TOOL_NAMES[54],"Preview accepted markers as a new pad bank","Pure IO-free shared preview from one persisted accepted marker set and 1..16 explicit strictly ascending unique adjacent-interval indices; sparse selection allowed. No source BPM needed, no sorting/truncation/inference. Review every fresh region/kit/instrument ID and safe pad default in a complete <=256KiB summary. One private exact-base/request/both-revision-bound plan; failed preview preserves it. No source path/hash/decode check, document/file/audio/transport change or arbitrary caller candidate. Existing source identity is declared metadata, not newly verified.",object(json!({"expected_revision":revision_schema(),"config":marker_bank_config_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[55],"Commit reviewed marker pad bank in one undo","Commit only the retained shared plan after token, both revisions and exact serialized full-base guards; reauthorize the selected source's lexical path and freshly verify its SHA under the existing 64MiB encoded limit, then recheck before the shared Editor transaction. No all-source file proof or decode. Fresh regions, one kit/pad bank and newly assigned instrument only: existing patterns/order/clock/markers/source metadata/audio and live transport unchanged. Preflight bounded history-only receipt; no output files/MIDI/ML. Failure preserves review/document, success consumes the token and is one Undo.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[56],"Copy exact tracker rectangle to private clipboard","Freeze an explicit one-pattern/one-TrackId half-open rectangle through the shared Editor-owned capture. Read-only alias sources resolve to exact cells; None holes and Some(default) cells remain distinct, all scalar/effect/phrase/signed-zero bits preserved. <=4096 positions including holes and1MiB serialized clipboard. No caller cells, OS clipboard, path/file/audio IO or Song/history change; allowed without Edit Mode. Successful Copy supersedes the private clipboard and invalidates its rectangle preview; failure preserves both. Clipboard survives edits/Save/Undo/Redo, but successful New/Open revokes its opaque Editor owner.",object(json!({"expected_revision":revision_schema(),"scope":cell_block_scope_schema()}),&["expected_revision","scope"]),false,false),
        tool(TOOL_NAMES[57],"Preview whole-cell Paste replace or Clear","Shared pure opaque preview, no path/file/hash/decode or document mutation. Paste uses only the current server-owned clipboard, explicit upper-left destination and SAME logical TrackId (different pattern allowed); frozen holes remove destinations, whole cells replace exactly. Clear removes stored cells only in the explicit rectangle. Alias destinations reject; no implicit Make unique, growth/cropping/merge, ID remapping or inherited-memory repair. Current Editor owner, both revisions and exact full base bind the plan. Bounded projected model work,8MiB candidate,2MiB diff; inspect paged changes/aliases/occurrences before Commit. Failed preview preserves prior review; no Edit Mode requirement for metadata.",object(json!({"expected_revision":revision_schema(),"operation":cell_block_operation_schema()}),&["expected_revision","operation"]),false,false),
        tool(TOOL_NAMES[58],"Inspect frozen rectangle cells or reviewed changes and uses","Read one explicit bounded page: clipboard row-major positions (including null holes and stored default cells), exact reviewed before/after changes, affected aliases, or affected order contexts including mute flags. Clipboard inspection requires its current-document token but not its old copy revision; preview pages require current owner/revisions/exact base. Strict token kind,1..64 records/256KiB, exact totals/next_start and empty end pages; no silent truncation. No source/path IO, file validation, document changes or audio-equivalence claim.",cell_block_inspect_schema(),true,false),
        tool(TOOL_NAMES[59],"Commit reviewed rectangle edit in one undo","Accept only the current server-owned Editor-bound plan; no caller cells/candidate. Require owner, both revisions, exact full base and Edit Mode, then commit shared whole-cell replacement/Clear. Changed content is one Undo; true no-op consumes the review while preserving revision/dirty/history/redo. Frozen same-document clipboard survives. Actual Editor revision delta reports signed-zero changes correctly. Pre-sized256KiB history-only receipt, no arbitrary title/path echo, file/path/hash/decode/audio work or resource remapping.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[60],"Preview persisted track gain, balance, mute and solo","Review a complete strict <=64-entry setting set through the shared pure track-balance compiler. Explicit Available/Unavailable/Unsupported routing status, effective stereo coefficients and shared-legacy ownership constraints; unavailable draft routing is not playback/live readiness. Opaque Editor-owner/both-revision/exact-base plan, one retained review, <=256KiB complete summary. No path/file/hash/decode/audio or GUI/live mailbox access. Required explicit version and gain/balance/mute/solo; unknown/duplicate tracks reject, empty resets neutral defaults. Read Mode allowed; failed review preserves prior plan.",object(json!({"expected_revision":revision_schema(),"config":track_mix_config_schema()}),&["expected_revision","config"]),false,false),
        tool(TOOL_NAMES[61],"Commit reviewed track balance to this headless Song","Apply only the current private complete-set plan through protected Editor owner/revision/exact-base guards. Changed settings are one Undo; genuine no-op preserves redo, dirty and revisions, but consumes the review. No caller candidate, partial patch, GUI synchronization or live audio/device command. Saved/current settings affect later starts and offline export under ordinary source/routing policy. No path/file/hash/decode/render work; complete bounded history-only receipt is prepared before mutation.",object(json!({"expected_revision":revision_schema(),"preview_token":{"type":"string","minLength":1}}),&["expected_revision","preview_token"]),false,true),
        tool(TOOL_NAMES[62],"Analyze bounded offline Song sample-peak windows","Render the same full-context Engine from frame0 in memory only, through at most 120,000 frames at 48 kHz. Explicit aligned interval selects2400-frame sample-peak windows, plus only an exact natural-end partial. Reports post-balance/post-master track sums and actual final master PCM; not RMS/true peak/loudness/live GUI/device/heard audio. Shared strict work/source preflight: actually used routes must be registered regions, lexical root/no-symlink authorization precedes fresh verified decode, unused source files are not opened or byte-verified; full model and bounded metadata validation still applies. No files, PCM payload, history or device output. One complete result replaces only on success, guarded by owner/both revisions/exact Song; <=1MiB report plus<=8MiB base/bounded metadata, <=256KiB response. Returns shared summary and first<=4 windows; follow inspect pages. Synchronous; no cancellation or timing SLA.",object(json!({"expected_revision":revision_schema(),"config":sample_peak_config_schema()}),&["expected_revision","config"]),true,false),
        tool(TOOL_NAMES[63],"Inspect retained offline Song sample peaks","Read a bounded page from the current private completed offline report; no path/hash/decode/render now and no fresh-source-byte claim. Exact Editor owner, both revisions and full Song including current mix must still match. start is zero-based among selected windows; limit1..4, default4, exact total/end page and next_start, no truncation. Report indexes/48kHz Song coordinates retain original full-context positions; track_peaks slots match returned layout, null means unavailable not silence. No GUI/device telemetry or heard-output claim. Failed requests preserve current report.",object(json!({"expected_revision":revision_schema(),"analysis_token":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0,"maximum":u64::MAX,"default":0},"limit":{"type":"integer","minimum":1,"maximum":MAX_SAMPLE_PEAK_PAGE,"default":MAX_SAMPLE_PEAK_PAGE}}),&["expected_revision","analysis_token"]),true,false),
    ]
}

#[cfg(test)]
mod marker_bank_adapter_tests {
    use super::*;

    fn seeded_server() -> Server {
        let root = std::env::temp_dir();
        let mut server = Server::new(&root).unwrap();
        server.editor.song.samples.insert(
            SampleId(7),
            SampleAsset {
                path: root.join(format!(
                    "marker-bank-metadata-only-{}.wav",
                    std::process::id()
                )),
                sha256: "0".repeat(64),
                sample_rate: 48000,
                channels: 1,
                frames: 128,
            },
        );
        server.editor.song.marker_sets.insert(
            MarkerSetId(31),
            MarkerSet {
                name: "Accepted".into(),
                sample: SampleId(7),
                source_bpm: None,
                markers: vec![0, 64, 128],
                locked_frames: vec![],
            },
        );
        server
            .editor
            .song
            .patterns
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
                    pan: Some(0.0),
                    ..Cell::default()
                },
            );
        server
    }
    fn request(marker_set: u64) -> Value {
        json!({"expected_revision":0,"config":{"version":1,"marker_set":marker_set,"interval_indices":[0],"name":"Bank"}})
    }

    #[test]
    fn marker_bank_adapter_exact_signed_zero_guard_and_token_overflow_preserve_review() {
        let mut server = seeded_server();
        let before_revision = server.editor.revision();
        let prepared = server
            .execute("leviathan_marker_bank_preview", request(31))
            .unwrap();
        let token = prepared["preview_token"].as_str().unwrap();
        server.token_serial = u64::MAX;
        assert_eq!(
            server
                .execute("leviathan_marker_bank_preview", request(31))
                .unwrap_err()
                .code,
            "token_exhausted"
        );
        assert!(server.checked_marker_bank(token).is_ok());
        server
            .editor
            .song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .pan = Some(-0.0);
        assert_eq!(
            &server.editor.song,
            server.marker_bank.as_ref().unwrap().preview.base(),
            "PartialEq alone misses this change"
        );
        let failed = server
            .execute(
                "leviathan_marker_bank_commit",
                json!({"expected_revision":0,"preview_token":token}),
            )
            .unwrap_err();
        assert_eq!(
            failed.code, "marker_bank_preview_stale",
            "reject before any source hash/path IO"
        );
        assert_eq!(server.revision, 0);
        assert_eq!(server.editor.revision(), before_revision);
        assert!(!server.editor.can_undo());
        assert_eq!(server.marker_bank.as_ref().unwrap().token, token);
    }

    #[test]
    fn marker_bank_adapter_response_limit_rejects_whole_header_without_replacing_prior_plan() {
        let mut server = seeded_server();
        // Path metadata can be adversarial in an unchecked in-memory draft.
        // Pure preparation must bound its response before retaining a new plan.
        server.editor.song.samples.insert(
            SampleId(8),
            SampleAsset {
                path: PathBuf::from(format!("/{}", "x".repeat(MAX_PREVIEW_BYTES))),
                sha256: "0".repeat(64),
                sample_rate: 48000,
                channels: 1,
                frames: 128,
            },
        );
        server.editor.song.marker_sets.insert(
            MarkerSetId(32),
            MarkerSet {
                name: "Other accepted".into(),
                sample: SampleId(8),
                source_bpm: None,
                markers: vec![0, 64, 128],
                locked_frames: vec![],
            },
        );
        let prepared = server
            .execute("leviathan_marker_bank_preview", request(31))
            .unwrap();
        let token = prepared["preview_token"].as_str().unwrap();
        let serial = server.token_serial;
        let before = serde_json::to_vec(&server.editor.song).unwrap();
        let failed = server
            .execute("leviathan_marker_bank_preview", request(32))
            .unwrap_err();
        assert_eq!(failed.code, "marker_bank_response_limit");
        assert_eq!(server.token_serial, serial);
        assert!(server.checked_marker_bank(token).is_ok());
        assert_eq!(serde_json::to_vec(&server.editor.song).unwrap(), before);
        assert!(!server.editor.can_undo());
    }
}

#[cfg(test)]
mod cell_block_adapter_tests {
    use super::*;
    fn server() -> Server {
        let mut server = Server::new(&std::env::temp_dir()).unwrap();
        server
            .editor
            .song
            .patterns
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
                    pan: Some(-0.0),
                    ..Cell::default()
                },
            );
        server
    }
    fn copy_request() -> Value {
        json!({"expected_revision":0,"scope":{"pattern":1,"track":1,"row_start":0,"row_end":1,"column_start":0,"column_end":1}})
    }
    fn clear_request(revision: u64) -> Value {
        json!({"expected_revision":revision,"operation":{"kind":"clear","scope":{"pattern":1,"track":1,"row_start":0,"row_end":1,"column_start":0,"column_end":1}}})
    }
    #[test]
    fn cell_block_adapter_read_mode_allows_copy_review_inspect_but_commit_requires_edit_mode() {
        let mut server = server();
        server.editor.edit_mode = false;
        let copied = server
            .execute("leviathan_cell_block_copy", copy_request())
            .unwrap();
        let preview = server
            .execute("leviathan_cell_block_preview", clear_request(0))
            .unwrap();
        let token = preview["preview_token"].as_str().unwrap();
        assert!(server
            .execute(
                "leviathan_cell_block_inspect",
                json!({"expected_revision":0,"kind":"changes","preview_token":token})
            )
            .is_ok());
        let before = serde_json::to_vec(&server.editor.song).unwrap();
        let denied = server
            .execute(
                "leviathan_cell_block_commit",
                json!({"expected_revision":0,"preview_token":token}),
            )
            .unwrap_err();
        assert_eq!(denied.code, "validation_error");
        assert_eq!(serde_json::to_vec(&server.editor.song).unwrap(), before);
        assert_eq!(server.revision, 0);
        assert!(server.checked_cell_block(token).is_ok());
        server.editor.edit_mode = true;
        assert!(server
            .execute(
                "leviathan_cell_block_commit",
                json!({"expected_revision":0,"preview_token":token})
            )
            .unwrap()["changed"]
            .as_bool()
            .unwrap());
        assert_eq!(server.revision, 1);
        let clipboard_token = copied["clipboard_token"].as_str().unwrap();
        assert!(server.checked_cell_clipboard(clipboard_token).is_ok());
        let preview = server
            .execute("leviathan_cell_block_preview", clear_request(1))
            .unwrap();
        // Simulate same-content replacement outside the MCP dispatcher: opaque
        // Editor ownership still rejects both stale references independently.
        server
            .editor
            .replace_document(server.editor.song.clone())
            .unwrap();
        assert_eq!(
            server
                .checked_cell_clipboard(clipboard_token)
                .err()
                .unwrap()
                .code,
            "cell_block_clipboard_owner_changed"
        );
        assert_eq!(
            server
                .checked_cell_block(preview["preview_token"].as_str().unwrap())
                .err()
                .unwrap()
                .code,
            "cell_block_preview_stale"
        );
    }
    #[test]
    fn cell_block_adapter_token_overflow_and_unchecked_signed_zero_preserve_private_handles() {
        let mut server = server();
        let copied = server
            .execute("leviathan_cell_block_copy", copy_request())
            .unwrap();
        let preview = server
            .execute("leviathan_cell_block_preview", clear_request(0))
            .unwrap();
        let clipboard_token = copied["clipboard_token"].as_str().unwrap();
        let preview_token = preview["preview_token"].as_str().unwrap();
        server.token_serial = u64::MAX;
        for (name, request) in [
            ("leviathan_cell_block_copy", copy_request()),
            ("leviathan_cell_block_preview", clear_request(0)),
        ] {
            assert_eq!(
                server.execute(name, request).unwrap_err().code,
                "token_exhausted"
            );
            assert!(server.checked_cell_clipboard(clipboard_token).is_ok());
            assert!(server.checked_cell_block(preview_token).is_ok());
        }
        server
            .editor
            .song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .pan = Some(0.0);
        assert!(
            server.checked_cell_clipboard(clipboard_token).is_ok(),
            "clipboard is frozen, owner-only authority after acceptance"
        );
        assert_eq!(
            server.checked_cell_block(preview_token).err().unwrap().code,
            "cell_block_preview_stale"
        );
        assert_eq!(server.revision, 0);
        assert!(!server.editor.can_undo());
        assert_eq!(server.cell_block.as_ref().unwrap().token, preview_token);
    }
}

#[cfg(test)]
mod track_mix_adapter_tests {
    use super::*;
    fn request(balance: f64) -> Value {
        json!({"expected_revision":0,"config":{"version":1,"settings":[{"track":1,"gain":0.5,"balance":balance,"mute":false,"solo":false}]}})
    }
    #[test]
    fn track_mix_adapter_read_mode_review_commit_protection_and_equal_document_owner_guard() {
        let mut server = Server::new(&std::env::temp_dir()).unwrap();
        server.editor.edit_mode = false;
        let prepared = server
            .execute("leviathan_track_mix_preview", request(0.0))
            .unwrap();
        let token = prepared["preview_token"].as_str().unwrap();
        assert!(server
            .execute("leviathan_project_read", json!({"section":"track_mix"}))
            .is_ok());
        let before = serde_json::to_vec(&server.editor.song).unwrap();
        let denied = server
            .execute(
                "leviathan_track_mix_commit",
                json!({"expected_revision":0,"preview_token":token}),
            )
            .unwrap_err();
        assert_eq!(denied.code, "validation_error");
        assert_eq!(serde_json::to_vec(&server.editor.song).unwrap(), before);
        assert!(server.checked_track_mix(token).is_ok());
        assert!(!server.editor.can_undo());
        server.editor.edit_mode = true;
        server
            .editor
            .replace_document(server.editor.song.clone())
            .unwrap();
        assert_eq!(
            server.checked_track_mix(token).err().unwrap().code,
            "track_mix_preview_stale"
        );
        assert_eq!(server.revision, 0, "owner guard independent of MCP advance");
        let prepared = server
            .execute("leviathan_track_mix_preview", request(0.0))
            .unwrap();
        // Document replacement deliberately restores Read Mode; preparing in it
        // remains valid, but the following explicit authoring action needs Edit.
        server.editor.edit_mode = true;
        assert_eq!(
            server
                .execute(
                    "leviathan_track_mix_commit",
                    json!({"expected_revision":0,"preview_token":prepared["preview_token"]})
                )
                .unwrap()["changed"],
            true
        );
        assert_eq!(server.revision, 1);
    }
    #[test]
    fn track_mix_adapter_token_overflow_exact_unchecked_base_and_large_title_receipts_are_bounded()
    {
        let mut server = Server::new(&std::env::temp_dir()).unwrap();
        server.editor.song.title = "large title".repeat(60_000);
        let prepared = server
            .execute("leviathan_track_mix_preview", request(-0.0))
            .unwrap();
        let token = prepared["preview_token"].as_str().unwrap();
        assert!(serde_json::to_vec(&prepared).unwrap().len() <= MAX_PREVIEW_BYTES);
        server.token_serial = u64::MAX;
        assert_eq!(
            server
                .execute("leviathan_track_mix_preview", request(0.0))
                .unwrap_err()
                .code,
            "token_exhausted"
        );
        assert!(server.checked_track_mix(token).is_ok());
        let receipt = server
            .execute(
                "leviathan_track_mix_commit",
                json!({"expected_revision":0,"preview_token":token}),
            )
            .unwrap();
        assert!(serde_json::to_vec(&receipt).unwrap().len() <= MAX_PREVIEW_BYTES);
        assert_eq!(receipt["session_scope"], "history_only");
        assert!(receipt["session"].get("title").is_none());
        server.token_serial = 10;
        let mut next = request(0.0);
        next["expected_revision"] = json!(1);
        let prepared = server.execute("leviathan_track_mix_preview", next).unwrap();
        server
            .editor
            .song
            .track_mix
            .get_mut(&TrackId(1))
            .unwrap()
            .balance = 0.0;
        assert_eq!(
            server
                .checked_track_mix(prepared["preview_token"].as_str().unwrap())
                .err()
                .unwrap()
                .code,
            "track_mix_preview_stale"
        );
        assert_eq!(server.revision, 1);
        assert!(server.editor.song.track_mix[&TrackId(1)]
            .balance
            .is_sign_positive());
    }
}

#[cfg(test)]
mod sample_peak_adapter_tests {
    use super::*;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn setup() -> (Fixture, Server) {
        let root = std::env::temp_dir().join(format!(
            "leviathan-private-peaks-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Fixture(root.clone());
        let mut server = Server::new(&root).unwrap();
        server
            .execute(
                "leviathan_sample_generate",
                json!({"expected_revision":0,"path":"owned.wav"}),
            )
            .unwrap();
        server
            .execute(
                "leviathan_sample_import",
                json!({"expected_revision":0,"sample_id":1,"path":"owned.wav"}),
            )
            .unwrap();
        server.execute("leviathan_edit_batch",json!({"expected_revision":1,"operations":[
            {"op":"upsert_region","id":1,"name":"Owned","sample_id":1,"start_frame":0,"end_frame":48000,"root_note":60},
            {"op":"assign_region","instrument_id":1,"region_id":1},
            {"op":"upsert_pattern","id":1,"name":"Peaks","rows":4},
            {"op":"set_order","pattern_ids":[1]},
            {"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.25}},
            {"op":"set_track_mix","config":{"version":1,"settings":[{"track":1,"gain":0.5,"balance":-0.0,"mute":false,"solo":false}]}}
        ]})).unwrap();
        (fixture, server)
    }
    fn request() -> Value {
        json!({"expected_revision":2,"config":{"version":1,"start_frame":0,"end_frame":2400}})
    }
    #[cfg(windows)]
    #[test]
    fn workspace_contains_long_verbatim_child_beneath_short_dos_root() {
        let root = std::env::temp_dir().join(format!(
            "leviathan-long-path-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Fixture(root.clone());
        let mut long_parent = root.clone();
        for index in 0..6 {
            long_parent.push(format!("segment-{index}-{}", "x".repeat(43)));
        }
        fs::create_dir_all(&long_parent).unwrap();
        let sample = long_parent.join("sample.wav");
        fs::write(&sample, b"fixture").unwrap();
        assert!(sample.as_os_str().len() >= 260);

        let workspace = Workspace::new(&root).unwrap();
        let canonical_sample = sample.canonicalize().unwrap();
        assert!(
            workspace.check(&canonical_sample, true).is_ok(),
            "a long verbatim child path should remain beneath the short DOS root"
        );

        let long_name = format!("{}.wav", "n".repeat(240));
        let long_sample = root.join(long_name);
        fs::write(&long_sample, b"fixture").unwrap();
        assert!(long_sample.as_os_str().len() >= 260);
        let canonical_long_sample = long_sample.canonicalize().unwrap();
        assert!(
            workspace.check(&canonical_long_sample, true).is_ok(),
            "a long filename under a short parent should remain beneath the root"
        );
        drop(fixture);
    }

    #[cfg(windows)]
    #[test]
    fn workspace_matches_unc_and_verbatim_unc_prefixes_by_server_and_share() {
        let root = PathBuf::from(r"\\server\share\root");
        let child = PathBuf::from(r"\\?\UNC\server\share\root\child.wav");
        assert_eq!(
            relative_under_root(&root, &child),
            Some(PathBuf::from("child.wav"))
        );
        let other_share = PathBuf::from(r"\\?\UNC\server\other\root\child.wav");
        assert!(relative_under_root(&root, &other_share).is_none());
        let device_path = PathBuf::from(r"\\.\C:\root\child.wav");
        assert!(relative_under_root(&PathBuf::from(r"C:\root"), &device_path).is_none());
    }

    #[test]
    fn sample_peak_adapter_read_mode_owner_exact_signed_zero_and_token_exhaustion() {
        let (_fixture, mut server) = setup();
        server.editor.edit_mode = false;
        let song = serde_json::to_vec(&server.editor.song).unwrap();
        let revision = server.editor.revision();
        let prepared = server
            .execute("leviathan_sample_peaks_analyze", request())
            .unwrap();
        let token = prepared["analysis_token"].as_str().unwrap();
        assert_eq!(serde_json::to_vec(&server.editor.song).unwrap(), song);
        assert_eq!(server.editor.revision(), revision);
        assert_eq!(server.revision, 2);
        server.token_serial = u64::MAX;
        assert_eq!(
            server
                .execute("leviathan_sample_peaks_analyze", request())
                .unwrap_err()
                .code,
            "token_exhausted"
        );
        assert!(server.checked_sample_peaks(token).is_ok());
        server
            .editor
            .song
            .track_mix
            .get_mut(&TrackId(1))
            .unwrap()
            .balance = 0.0;
        assert_eq!(
            server.checked_sample_peaks(token).err().unwrap().code,
            "sample_peaks_stale"
        );
        assert_eq!(server.editor.revision(), revision);
        assert_eq!(server.sample_peaks.as_ref().unwrap().token, token);
        server
            .editor
            .song
            .track_mix
            .get_mut(&TrackId(1))
            .unwrap()
            .balance = -0.0;
        assert!(server.checked_sample_peaks(token).is_ok());
        server
            .editor
            .replace_document(server.editor.song.clone())
            .unwrap();
        assert_eq!(
            server.checked_sample_peaks(token).err().unwrap().code,
            "sample_peaks_stale"
        );
        assert_eq!(
            server.revision, 2,
            "Editor owner guard is independent of MCP advance"
        );
    }
    #[test]
    fn sample_peak_adapter_pure_decoded_budget_path_scope_and_response_bounds_preserve_report() {
        let (fixture, mut server) = setup();
        server.editor.song.title = "bounded large title".repeat(20000);
        let prepared = server
            .execute("leviathan_sample_peaks_analyze", request())
            .unwrap();
        let token = prepared["analysis_token"].as_str().unwrap();
        assert!(serde_json::to_vec(&prepared).unwrap().len() <= MAX_PREVIEW_BYTES);
        let path = fixture.0.join("owned.wav");
        let bytes = fs::read(&path).unwrap();
        let source = server.editor.song.samples[&SampleId(1)].clone();
        server
            .editor
            .song
            .samples
            .get_mut(&SampleId(1))
            .unwrap()
            .frames = crate::sample_asset::MAX_SAMPLE_VALUES as u64;
        fs::remove_file(&path).unwrap();
        let denied = server
            .execute("leviathan_sample_peaks_analyze", request())
            .unwrap_err();
        assert!(
            denied.message.contains("decoded"),
            "Pure budget must reject before missing path: {denied:?}"
        );
        assert_eq!(server.sample_peaks.as_ref().unwrap().token, token);
        fs::write(&path, bytes).unwrap();
        server
            .editor
            .song
            .samples
            .insert(SampleId(1), source.clone());
        assert!(server.checked_sample_peaks(token).is_ok());
        server
            .editor
            .song
            .samples
            .get_mut(&SampleId(1))
            .unwrap()
            .path = fixture.0.parent().unwrap().join("outside-authority.wav");
        assert_eq!(
            server
                .execute("leviathan_sample_peaks_analyze", request())
                .unwrap_err()
                .code,
            "path_denied"
        );
        assert_eq!(server.sample_peaks.as_ref().unwrap().token, token);
        server.editor.song.samples.insert(SampleId(1), source);
        assert!(server.checked_sample_peaks(token).is_ok());
        assert_eq!(
            bounded_sample_peaks(json!({"notice":"x".repeat(MAX_PREVIEW_BYTES)}))
                .unwrap_err()
                .code,
            "sample_peaks_response_limit"
        );
        assert!(server.checked_sample_peaks(token).is_ok());
        let pending = server.sample_peaks.as_ref().unwrap();
        assert!(sample_peak_page(pending, usize::MAX, 4).is_err());
        assert!(sample_peak_page(pending, 0, 5).is_err());
        assert_eq!(
            sample_peak_page(pending, pending.analysis.reports.len(), 1).unwrap()["windows"],
            json!([])
        );
    }
}
