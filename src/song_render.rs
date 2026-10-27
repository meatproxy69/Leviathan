//! Deterministic, stateful document evaluator and synchronous multi-instrument renderer.
mod arrangement;
mod controlled;
mod drums;
mod multisamples;
mod phrases;
use crate::{song::*, RenderStats, RATE, TAIL};
use controlled::ControlledVoice;
use phonic::{
    generators::Sampler, Generator, GeneratorPlaybackEvent as P, GeneratorPlaybackMessage,
    GeneratorPlaybackOptions, Source, SourceTime,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::BufWriter,
    path::{Path, PathBuf},
    time::Instant,
};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Release {
    ExplicitOff,
    Replaced,
    SongEnd,
    Retrigger,
    TimedRelease,
    PhraseEnd,
    Choke,
    PadReplaced,
    TrackMute,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    On {
        voice: usize,
        pitch: u8,
        volume: f32,
        pan: f32,
    },
    Off {
        voice: usize,
        reason: Release,
    },
    Volume {
        voice: usize,
        value: f32,
    },
    Pan {
        voice: usize,
        value: f32,
    },
    Glide {
        voice: usize,
        target: u8,
        semitones_per_second: f32,
    },
    /// Per-note initial cursor, applied after ON and before glide/audio.
    SampleStart {
        voice: usize,
        offset: Option<u64>,
        direction: Direction,
    },
    Offset {
        voice: usize,
        frame: u64,
    },
    Direction {
        voice: usize,
        direction: Direction,
    },
    Cut {
        voice: usize,
    },
}
impl Command {
    fn voice(&self) -> usize {
        match *self {
            Self::On { voice, .. }
            | Self::Off { voice, .. }
            | Self::Volume { voice, .. }
            | Self::Pan { voice, .. }
            | Self::Glide { voice, .. }
            | Self::SampleStart { voice, .. }
            | Self::Offset { voice, .. }
            | Self::Direction { voice, .. }
            | Self::Cut { voice } => voice,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub frame: u64,
    pub track: TrackId,
    pub column: u16,
    pub instrument: InstrumentId,
    /// None for final transport release; otherwise occurrence and row.
    pub position: Option<(usize, u32)>,
    pub command: Command,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase: Option<PhrasePosition>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhrasePosition {
    pub phrase: PhraseId,
    pub instance: usize,
    pub column: u16,
    /// None for owner-level OFF/cut/end, otherwise the internal cycle and row.
    pub cycle: Option<u16>,
    pub row: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub evaluator: String,
    pub sample_rate: u32,
    pub bpm: f64,
    pub lines_per_beat: u16,
    pub musical_frames: u64,
    pub tail_frames: u64,
    pub master_gain: f32,
    pub assets: BTreeMap<InstrumentId, PathBuf>,
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<InstrumentId, RenderRegion>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pad_voices: BTreeMap<usize, RenderPad>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub multisample_voices: BTreeMap<usize, RenderMultisample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<RenderArrangement>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_mix: BTreeMap<TrackId, crate::track_mix::TrackMix>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderArrangement {
    pub aliases: Vec<RenderAlias>,
    pub mutes: Vec<TrackMute>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderAlias {
    pub occurrence: usize,
    pub track: TrackId,
    pub chain: Vec<TrackAlias>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackMute {
    pub occurrence: usize,
    pub track: TrackId,
    pub start_frame: u64,
    pub end_frame: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderRegion {
    pub region: RegionId,
    pub sample: SampleId,
    pub sha256: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub root_note: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amp_envelope: Option<AmpEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_loop: Option<ForwardLoop>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderPad {
    pub instrument: InstrumentId,
    pub kit: DrumKitId,
    pub pad: PadId,
    pub slot: u8,
    pub note: u8,
    pub gain: f32,
    pub pan: f32,
    pub mode: PadMode,
    pub new_note: PadNewNote,
    pub choke_group: Option<u8>,
    pub tune_semitones: i8,
    pub path: PathBuf,
    pub region: RenderRegion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_selection: Option<RenderLayerSelection>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderLayerSelection {
    pub layer: PadLayerId,
    /// Actual pre-pad-gain ON value after column memory and phrase snapshot.
    pub input_velocity: f32,
    pub velocity_bin: u8,
    pub min_velocity: u8,
    pub max_velocity: u8,
    pub round_robin_index: u8,
    pub round_robin_ordinal: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderMultisample {
    pub instrument: InstrumentId,
    pub map: MultisampleId,
    pub zone: KeyZoneId,
    /// Actual sounding MIDI note after phrase transposition, not a pad selector.
    pub note: u8,
    pub input_velocity: f32,
    pub velocity_bin: u8,
    pub note_min: u8,
    pub note_max: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    pub path: PathBuf,
    /// Native root is the zone override; source identity/settings come from region.
    pub region: RenderRegion,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AssetKey {
    Instrument(InstrumentId),
    Pad(InstrumentId, PadId, Option<PadLayerId>),
    Multisample(InstrumentId, MultisampleId, KeyZoneId),
}
impl AssetKey {
    fn instrument(self) -> InstrumentId {
        match self {
            Self::Instrument(id) | Self::Pad(id, _, _) | Self::Multisample(id, _, _) => id,
        }
    }
}
struct RenderAsset {
    path: PathBuf,
    region: Option<RenderRegion>,
}
struct Column {
    instrument: Option<InstrumentId>,
    volume: f32,
    pan: f32,
    active: Option<(InstrumentId, usize)>,
}
impl Default for Column {
    fn default() -> Self {
        Self {
            instrument: None,
            volume: 1.0,
            pan: 0.0,
            active: None,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Cell,
    Retrigger,
    Release,
    Cut,
    Mute,
}
pub fn evaluate(song: &Song, project_dir: Option<&Path>) -> Result<Manifest, String> {
    evaluate_with_paths(song, project_dir, true)
}
/// Model-only scheduling for pure previews. Never canonicalizes or opens assets;
/// this manifest is not exposed as a playback-ready object.
pub(crate) fn evaluate_model(song: &Song) -> Result<Manifest, String> {
    evaluate_with_paths(song, None, false)
}
fn evaluate_with_paths(
    song: &Song,
    project_dir: Option<&Path>,
    check_paths: bool,
) -> Result<Manifest, String> {
    song.validate()?;
    let mut has_phrases = false;
    for order in &song.order {
        for &track in &song.track_order {
            if !order.muted_tracks.contains(&track)
                && song
                    .resolved_track(order.pattern, track)?
                    .is_some_and(|rows| {
                        rows.values()
                            .flat_map(|columns| columns.values())
                            .any(|cell| cell.phrase.is_some())
                    })
            {
                has_phrases = true;
            }
        }
    }
    let mut manifest = if has_phrases {
        phrases::evaluate(song, project_dir, check_paths)?
    } else {
        evaluate_plain(song, project_dir, check_paths)?
    };
    for instrument in manifest.assets.keys() {
        if let Some(id) = song.instruments[instrument].region {
            let region = &song.regions[&id];
            let sample = &song.samples[&region.sample];
            manifest.regions.insert(
                *instrument,
                RenderRegion {
                    region: id,
                    sample: region.sample,
                    sha256: sample.sha256.clone(),
                    sample_rate: sample.sample_rate,
                    channels: sample.channels,
                    frames: sample.frames,
                    start_frame: region.start_frame,
                    end_frame: region.end_frame,
                    root_note: region.root_note,
                    amp_envelope: region.amp_envelope,
                    forward_loop: region.forward_loop,
                },
            );
        }
    }
    if !manifest.regions.is_empty() {
        manifest.schema = 5;
        manifest.evaluator = "leviathan-song-v5".into();
    }
    drums::resolve(song, project_dir, &mut manifest, check_paths)?;
    multisamples::resolve(song, project_dir, &mut manifest, check_paths)?;
    arrangement::resolve(song, &mut manifest)?;
    // Resolve against the actual voice's source, not a later selected instrument.
    // Runs in pure previews too, before decoding or reserving output files.
    for event in &manifest.events {
        let region = manifest
            .pad_voices
            .get(&event.command.voice())
            .map(|pad| &pad.region)
            .or_else(|| {
                manifest
                    .multisample_voices
                    .get(&event.command.voice())
                    .map(|voice| &voice.region)
            })
            .or_else(|| manifest.regions.get(&event.instrument));
        if region.is_some_and(|r| r.forward_loop.is_some())
            && matches!(
                event.command,
                Command::Offset { .. }
                    | Command::Direction {
                        direction: Direction::Reverse,
                        ..
                    }
                    | Command::SampleStart {
                        offset: Some(_),
                        ..
                    }
                    | Command::SampleStart {
                        direction: Direction::Reverse,
                        ..
                    }
            )
        {
            return Err(format!(
                "Voice {} forward loop does not support sample offset or reverse direction",
                event.command.voice()
            ));
        }
    }
    if manifest
        .regions
        .values()
        .chain(manifest.pad_voices.values().map(|p| &p.region))
        .chain(manifest.multisample_voices.values().map(|m| &m.region))
        .any(|r| r.amp_envelope.is_some() || r.forward_loop.is_some())
    {
        manifest.schema = 8;
        manifest.evaluator = "leviathan-song-v8".into();
    }
    if manifest
        .pad_voices
        .values()
        .any(|p| p.layer_selection.is_some())
    {
        manifest.schema = 9;
        manifest.evaluator = "leviathan-song-v9".into();
    }
    if !manifest.multisample_voices.is_empty() {
        manifest.schema = 10;
        manifest.evaluator = "leviathan-song-v10".into();
    }
    if crate::track_mix::nonneutral(song, &manifest) {
        manifest.track_mix = song.track_mix.clone();
        manifest.schema = 11;
        manifest.evaluator = "leviathan-song-v11".into();
    }
    Ok(manifest)
}
fn evaluate_plain(
    song: &Song,
    project_dir: Option<&Path>,
    check_paths: bool,
) -> Result<Manifest, String> {
    song.validate()?;
    if song.order.is_empty() || song.tracks.is_empty() {
        return Err("Song needs tracks and at least one ordered pattern".into());
    }
    let rows: u64 = song
        .order
        .iter()
        .map(|o| song.patterns[&o.pattern].rows as u64)
        .sum();
    let per_row = RATE as f64 * 60.0 / (song.bpm * song.lines_per_beat as f64);
    let end = (rows as f64 * per_row).round() as u64;
    if end == 0 || end > RATE as u64 * 600 {
        return Err("Song duration must be positive and at most ten minutes".into());
    }
    let mute_schedule = arrangement::MuteSchedule::new(song, RATE);
    let mut raw = Vec::new();
    let mut populated = 0;
    let mut has_effects = false;
    let mut has_sample_control = false;
    let mut offset = 0u64;
    let mute_cell = Cell::default();
    for (occurrence, order) in song.order.iter().enumerate() {
        let pattern = &song.patterns[&order.pattern];
        for (track_index, track) in song.track_order.iter().enumerate() {
            if order.muted_tracks.contains(track) {
                raw.push((
                    (offset as f64 * per_row).round() as u64,
                    offset,
                    track_index,
                    0,
                    0,
                    occurrence,
                    0,
                    *track,
                    &mute_cell,
                    Step::Mute,
                ));
                if raw.len() > 65536 {
                    return Err("Song exceeds 65536 expanded step commands including mutes".into());
                }
                continue;
            }
            if let Some(track_rows) = song.resolved_track(order.pattern, *track)? {
                for (row, columns) in track_rows {
                    for (column, cell) in columns {
                        if *cell == Cell::default() {
                            continue;
                        }
                        let absolute_row = offset + *row as u64;
                        let mut push = |fraction: f64, suborder, step| {
                            let frame = (((absolute_row as f64 + fraction) * per_row).round()
                                as u64)
                                .min(end - 1);
                            if mute_schedule.contains(*track, frame) {
                                return;
                            }
                            raw.push((
                                frame.min(end - 1),
                                absolute_row,
                                track_index,
                                *column,
                                suborder,
                                occurrence,
                                *row,
                                *track,
                                cell,
                                step,
                            ));
                        };
                        let delay = cell.delay.unwrap_or(0.0);
                        push(delay, 0, Step::Cell);
                        if let Some(retrigger) = &cell.effects.retrigger {
                            for repeat in 1..=retrigger.repeats {
                                push(
                                    delay + f64::from(repeat) * retrigger.spacing,
                                    repeat,
                                    Step::Retrigger,
                                );
                            }
                        }
                        if let Some(release) = cell.effects.release {
                            // Validation tolerates decimal-equal f64 roundoff.
                            // Normalize it before frame rounding too: an ulp at
                            // a half-frame boundary must not release BEFORE the
                            // final retrigger and accidentally leave it running.
                            let last_start = cell
                                .effects
                                .retrigger
                                .as_ref()
                                .map_or(delay, |r| delay + f64::from(r.repeats) * r.spacing);
                            push(release.max(last_start), 17, Step::Release);
                        }
                        if let Some(cut) = cell.effects.cut {
                            let last_start = cell
                                .effects
                                .retrigger
                                .as_ref()
                                .map_or(delay, |r| delay + f64::from(r.repeats) * r.spacing);
                            push(cut.max(last_start), 17, Step::Cut);
                        }
                        has_effects |= !cell.effects.is_empty();
                        has_sample_control |= cell.effects.cut.is_some()
                            || cell.effects.offset.is_some()
                            || cell.effects.direction.is_some();
                        populated += 1;
                        if populated > 32768 {
                            return Err("Song exceeds 32768 populated cells".into());
                        }
                        if raw.len() > 65536 {
                            return Err("Song exceeds 65536 expanded step commands".into());
                        }
                    }
                }
            }
        }
        offset += pattern.rows as u64;
    }
    raw.sort_by_key(|e| (e.0, e.1, e.2, e.3, e.4));
    let mut states: BTreeMap<(TrackId, u16), Column> = BTreeMap::new();
    let mut events = Vec::new();
    let mut next_voice = 1;
    for (frame, _, _, column, _, occurrence, row, track, cell, step) in raw {
        if step == Step::Mute {
            for ((owner, _), state) in &mut states {
                if *owner == track {
                    state.active = None;
                }
            }
            continue;
        }
        let state = states.entry((track, column)).or_default();
        if step == Step::Cell {
            if let Some(instrument) = cell.instrument {
                state.instrument = Some(instrument);
            }
            if let Some(value) = cell.velocity {
                state.volume = value;
            }
            if let Some(value) = cell.pan {
                state.pan = value;
            }
        }
        let mut emit = |instrument, command| {
            events.push(Event {
                frame,
                track,
                column,
                instrument,
                position: Some((occurrence, row)),
                command,
                phrase: None,
            })
        };
        if step == Step::Release || step == Step::Cut {
            if let Some((instrument, voice)) = state.active.take() {
                emit(
                    instrument,
                    if step == Step::Cut {
                        Command::Cut { voice }
                    } else {
                        Command::Off {
                            voice,
                            reason: Release::TimedRelease,
                        }
                    },
                );
            }
            continue;
        }
        match cell.note {
            Note::Note(pitch) => {
                let instrument = state.instrument.ok_or_else(|| format!("Occurrence {occurrence}, track {}, row {row}, column {column}: note has no selected instrument", track.0))?;
                if let Some((instrument, voice)) = state.active.take() {
                    emit(
                        instrument,
                        Command::Off {
                            voice,
                            reason: if step == Step::Retrigger {
                                Release::Retrigger
                            } else {
                                Release::Replaced
                            },
                        },
                    );
                }
                if next_voice > 2048 {
                    return Err("Song exceeds 2048 note triggers".into());
                }
                emit(
                    instrument,
                    Command::On {
                        voice: next_voice,
                        pitch,
                        volume: state.volume,
                        pan: state.pan,
                    },
                );
                state.active = Some((instrument, next_voice));
                next_voice += 1;
            }
            Note::Off | Note::Empty => {
                if let Some((instrument, voice)) = state.active {
                    if let Some(value) = cell.velocity {
                        emit(instrument, Command::Volume { voice, value });
                    }
                    if let Some(value) = cell.pan {
                        emit(instrument, Command::Pan { voice, value });
                    }
                    if cell.note == Note::Off {
                        emit(
                            instrument,
                            Command::Off {
                                voice,
                                reason: Release::ExplicitOff,
                            },
                        );
                        state.active = None;
                    }
                }
            }
        }
        if cell.effects.offset.is_some() || cell.effects.direction.is_some() {
            let (instrument, voice) = state.active.ok_or_else(|| format!(
                "Occurrence {occurrence}, track {}, row {row}, column {column}: sample offset/direction requires an active voice", track.0
            ))?;
            if matches!(cell.note, Note::Note(_)) {
                emit(
                    instrument,
                    Command::SampleStart {
                        voice,
                        offset: cell.effects.offset,
                        direction: cell.effects.direction.unwrap_or(Direction::Forward),
                    },
                );
            } else {
                if let Some(frame) = cell.effects.offset {
                    emit(instrument, Command::Offset { voice, frame });
                }
                if let Some(direction) = cell.effects.direction {
                    emit(instrument, Command::Direction { voice, direction });
                }
            }
        }
        if let Some(glide) = &cell.effects.glide {
            let (instrument, voice) = state.active.ok_or_else(|| format!(
                "Occurrence {occurrence}, track {}, row {row}, column {column}: glide requires an active voice", track.0
            ))?;
            emit(
                instrument,
                Command::Glide {
                    voice,
                    target: glide.target,
                    semitones_per_second: glide.semitones_per_second,
                },
            );
        }
    }
    if next_voice == 1 && !arrangement::permits_silence(song)? {
        return Err("Song contains no note triggers".into());
    }
    for ((track, column), state) in states {
        if let Some((instrument, voice)) = state.active {
            events.push(Event {
                frame: end,
                track,
                column,
                instrument,
                position: None,
                command: Command::Off {
                    voice,
                    reason: Release::SongEnd,
                },
                phrase: None,
            });
        }
    }
    let assets = resolve_assets(song, &events, project_dir, check_paths)?;
    Ok(Manifest {
        schema: if has_sample_control {
            3
        } else if has_effects {
            2
        } else {
            1
        },
        evaluator: if has_sample_control {
            "leviathan-song-v3"
        } else if has_effects {
            "leviathan-song-v2"
        } else {
            "leviathan-song-v1"
        }
        .into(),
        sample_rate: RATE,
        bpm: song.bpm,
        lines_per_beat: song.lines_per_beat,
        musical_frames: end,
        tail_frames: TAIL,
        master_gain: 0.5,
        assets,
        events,
        regions: BTreeMap::new(),
        pad_voices: BTreeMap::new(),
        multisample_voices: BTreeMap::new(),
        arrangement: None,
        track_mix: BTreeMap::new(),
    })
}
fn resolve_assets(
    song: &Song,
    events: &[Event],
    project_dir: Option<&Path>,
    check_paths: bool,
) -> Result<BTreeMap<InstrumentId, PathBuf>, String> {
    let mut assets = BTreeMap::new();
    for event in events {
        if assets.contains_key(&event.instrument) {
            continue;
        }
        let instrument = &song.instruments[&event.instrument];
        if !instrument.kind.is_pitched() {
            continue;
        }
        let source = if let Some(region) = instrument.region {
            &song.samples[&song.regions[&region].sample].path
        } else {
            instrument.sample.as_ref().ok_or_else(|| {
                format!("Instrument {} has no sample assigned", event.instrument.0)
            })?
        };
        if !check_paths {
            assets.insert(event.instrument, source.clone());
            continue;
        }
        let path = if source.is_absolute() {
            source.clone()
        } else {
            project_dir
                .ok_or("Relative sample requires a saved project directory")?
                .join(source)
        };
        let path = path.canonicalize().map_err(|e| {
            format!(
                "Instrument {} missing sample {}: {e}",
                event.instrument.0,
                path.display()
            )
        })?;
        if !path.is_file() {
            return Err(format!(
                "Instrument {} sample is not a file",
                event.instrument.0
            ));
        }
        assets.insert(event.instrument, path);
    }
    Ok(assets)
}
fn send(sampler: &mut Sampler, command: &Command, time: &SourceTime) -> Result<(), String> {
    let event = match *command {
        Command::On {
            voice,
            pitch,
            volume,
            pan,
        } => P::NoteOn {
            note_id: voice,
            note: pitch,
            volume: Some(volume),
            panning: Some(pan),
            context: None,
        },
        Command::Off { voice, .. } => P::NoteOff { note_id: voice },
        Command::Volume { voice, value } => P::SetVolume {
            note_id: voice,
            volume: value,
        },
        Command::Pan { voice, value } => P::SetPanning {
            note_id: voice,
            panning: value,
        },
        Command::Glide {
            voice,
            target,
            semitones_per_second,
        } => P::SetSpeed {
            note_id: voice,
            speed: 2.0_f64.powf((f64::from(target) - 60.0) / 12.0),
            glide: Some(semitones_per_second),
        },
        Command::SampleStart { .. }
        | Command::Offset { .. }
        | Command::Direction { .. }
        | Command::Cut { .. } => {
            return Err("Sample-control command routed to an uncontrolled sampler".into())
        }
    };
    let queue = sampler.playback_message_queue();
    if let Err(message) = queue.push(GeneratorPlaybackMessage::Trigger { event }) {
        // phonic's fixed message capacity is independent of the voice count. A
        // valid wide chord (or its parameter/OFF rows) can exceed it. Its pinned
        // Sampler::write processes messages even for an empty output slice, so
        // drain at the SAME frame without advancing samples, fades or envelopes.
        // Only dense bursts take this path; ordinary rendering is unchanged.
        sampler.write(&mut [], time);
        queue
            .push(message)
            .map_err(|_| "Song sampler event queue remained full after draining".to_owned())?;
    }
    Ok(())
}
struct Outputs {
    paths: Vec<PathBuf>,
    complete: bool,
}
impl Drop for Outputs {
    fn drop(&mut self) {
        if !self.complete {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
        }
    }
}
/// Own the snapshot in the worker. Samples decode before outputs are reserved.
/// Each sampler has as many voices as its total note triggers: no voice stealing.
pub fn export(
    song: &Song,
    project_dir: Option<&Path>,
    output: &Path,
) -> Result<(RenderStats, Manifest), String> {
    if !output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
    {
        return Err("Song export filename must end in .wav".into());
    }
    let mut engine = Engine::new(song, project_dir)?;
    let manifest = engine.manifest.clone();
    let mut outputs = Outputs {
        paths: Vec::new(),
        complete: false,
    };
    let json = output.with_extension("events.json");
    let json_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&json)
        .map_err(|e| format!("Cannot reserve song manifest {}: {e}", json.display()))?;
    outputs.paths.push(json);
    let wav_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| format!("Cannot reserve song WAV {}: {e}", output.display()))?;
    outputs.paths.push(output.to_owned());
    let mut writer = hound::WavWriter::new(
        BufWriter::new(wav_file),
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .map_err(|e| e.to_string())?;
    let total = manifest.musical_frames + manifest.tail_frames;
    let mut stats = RenderStats {
        frames: total,
        peak: 0.0,
        energy: 0.0,
    };
    let mut mix = [0.0f32; 1024];
    while engine.position < total {
        let count = engine.block(&mut mix, total)?;
        for &value in &mix[..count * 2] {
            stats.peak = stats.peak.max(value.abs());
            stats.energy += (value as f64).powi(2);
            writer.write_sample(value).map_err(|e| e.to_string())?;
        }
    }
    writer.finalize().map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(json_file, &manifest).map_err(|e| e.to_string())?;
    outputs.complete = true;
    Ok((stats, manifest))
}

pub(crate) fn control_specs(manifest: &Manifest) -> BTreeMap<usize, (AssetKey, bool)> {
    let mut control_specs = BTreeMap::new();
    for event in &manifest.events {
        if let Some(voice) = manifest.multisample_voices.get(&event.command.voice()) {
            control_specs.insert(
                event.command.voice(),
                (
                    AssetKey::Multisample(voice.instrument, voice.map, voice.zone),
                    true,
                ),
            );
            continue;
        }
        if let Some(pad) = manifest.pad_voices.get(&event.command.voice()) {
            control_specs.insert(
                event.command.voice(),
                (
                    AssetKey::Pad(
                        pad.instrument,
                        pad.pad,
                        pad.layer_selection.as_ref().map(|l| l.layer),
                    ),
                    true,
                ),
            );
            continue;
        }
        let linear = if manifest.regions.contains_key(&event.instrument) {
            true
        } else {
            match event.command {
                Command::SampleStart { .. }
                | Command::Offset { .. }
                | Command::Direction { .. } => true,
                Command::Cut { .. } => false,
                // phonic ramps a new voice from gain 1 even for NoteOn volume 0.
                // Isolate only these voices so their source advances silently
                // until an explicit later positive Volume command.
                Command::On { volume: 0.0, .. } => false,
                _ => continue,
            }
        };
        let spec = control_specs
            .entry(event.command.voice())
            .or_insert((AssetKey::Instrument(event.instrument), false));
        spec.1 |= linear;
    }
    control_specs
}

/// Shared incremental DSP engine: used by both offline export and streaming worker.
/// No device/thread/UI state; buffers decode once and are shared across cold resets.
pub struct Engine {
    pub manifest: Manifest,
    pub position: u64,
    next: usize,
    buffers: BTreeMap<AssetKey, std::sync::Arc<phonic::AudioFileBuffer>>,
    asset_specs: BTreeMap<AssetKey, RenderAsset>,
    samplers: BTreeMap<InstrumentId, Sampler>,
    /// Voice ID -> instrument and whether source-position control is needed.
    control_specs: BTreeMap<usize, (AssetKey, bool)>,
    controlled: BTreeMap<usize, ControlledVoice>,
    scratch: [f32; 1024],
    origin: Instant,
    mix: Option<crate::track_mix::MixRuntime>,
    mix_unavailable: Option<String>,
    meters: Option<crate::meters::MeterTap>,
}
impl Engine {
    pub fn new(song: &Song, base: Option<&Path>) -> Result<Self, String> {
        let manifest = evaluate(song, base)?;
        let mut asset_specs: BTreeMap<_, _> = manifest
            .assets
            .iter()
            .map(|(&id, path)| {
                (
                    AssetKey::Instrument(id),
                    RenderAsset {
                        path: path.clone(),
                        region: manifest.regions.get(&id).cloned(),
                    },
                )
            })
            .collect();
        for pad in manifest.pad_voices.values() {
            asset_specs
                .entry(AssetKey::Pad(
                    pad.instrument,
                    pad.pad,
                    pad.layer_selection.as_ref().map(|l| l.layer),
                ))
                .or_insert_with(|| RenderAsset {
                    path: pad.path.clone(),
                    region: Some(pad.region.clone()),
                });
        }
        for voice in manifest.multisample_voices.values() {
            asset_specs
                .entry(AssetKey::Multisample(
                    voice.instrument,
                    voice.map,
                    voice.zone,
                ))
                .or_insert_with(|| RenderAsset {
                    path: voice.path.clone(),
                    region: Some(voice.region.clone()),
                });
        }
        let control_specs = control_specs(&manifest);
        let nonneutral = crate::track_mix::nonneutral(song, &manifest);
        let controlled_ids = control_specs.keys().copied().collect();
        let (mix, mix_unavailable) =
            match crate::track_mix::build_layout(song, &manifest, &controlled_ids) {
                Ok(layout) => {
                    let initial = layout.snapshot(&song.track_mix)?;
                    (
                        Some(crate::track_mix::MixRuntime::new(layout, initial)),
                        None,
                    )
                }
                Err(error) if nonneutral => return Err(error),
                Err(error) => (None, Some(error)),
            };
        let controlled_assets: BTreeSet<_> = control_specs
            .values()
            .filter_map(|(instrument, linear)| linear.then_some(*instrument))
            .collect();
        let mut buffers = BTreeMap::new();
        let mut controlled_values = 0usize;
        let mut source_cache: BTreeMap<String, std::sync::Arc<phonic::AudioFileBuffer>> =
            BTreeMap::new();
        let source_paths: BTreeSet<_> = asset_specs
            .values()
            .filter(|asset| asset.region.is_some())
            .map(|asset| &asset.path)
            .collect();
        let mut encoded_bytes = 0u64;
        // Bound all distinct registered-source IO before the first hash/decode.
        for path in source_paths {
            let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() > crate::sample_asset::MAX_ENCODED_BYTES as u64
            {
                return Err("Source must be a regular file at most 64 MiB encoded".into());
            }
            encoded_bytes = encoded_bytes
                .checked_add(metadata.len())
                .ok_or("Source encoded size overflow")?;
            if encoded_bytes > crate::sample_asset::MAX_TOTAL_ENCODED_BYTES {
                return Err("Used source files exceed 256 MiB aggregate encoded budget".into());
            }
        }
        let mut verified_paths: BTreeMap<PathBuf, (String, u32, u16, u64)> = BTreeMap::new();
        for (key, asset) in &asset_specs {
            let id = key.instrument();
            let path = &asset.path;
            if let Some(region) = &asset.region {
                let source = &song.samples[&region.sample];
                let identity = (
                    source.sha256.clone(),
                    source.sample_rate,
                    source.channels,
                    source.frames,
                );
                let buffer = if let Some(buffer) = source_cache.get(&source.sha256) {
                    // Even a duplicate identity at another path must still match
                    // its bytes and declared native metadata before reusing PCM.
                    if verified_paths.get(path) != Some(&identity) {
                        crate::sample_asset::verify_identity(source, path)?;
                    }
                    if buffer.sample_rate() != source.sample_rate
                        || buffer.channel_count() != usize::from(source.channels)
                        || buffer.frame_count() as u64 != source.frames + 1
                    {
                        return Err("Source metadata differs for a shared SHA-256 identity".into());
                    }
                    buffer.clone()
                } else {
                    let buffer = crate::sample_asset::load_verified(
                        source,
                        path,
                        controlled::MAX_SAMPLE_VALUES.saturating_sub(controlled_values),
                    )?;
                    controlled_values += buffer.buffer().len();
                    source_cache.insert(source.sha256.clone(), buffer.clone());
                    buffer
                };
                verified_paths.insert(path.clone(), identity);
                buffers.insert(*key, buffer);
                continue;
            }
            if controlled_assets.contains(key) {
                if fs::metadata(path).map_err(|e| e.to_string())?.len()
                    > (controlled::MAX_SAMPLE_VALUES * 4) as u64
                {
                    return Err(format!(
                        "Instrument {} controlled sample file exceeds 64 MiB",
                        id.0
                    ));
                }
                let info = phonic::AudioFileInfo::from_file(path)
                    .map_err(|e| format!("Instrument {}: {e}", id.0))?;
                if info.loop_range().is_some() {
                    return Err(format!("Instrument {} sample offset/direction does not support embedded sample loops", id.0));
                }
                if !(1..=2).contains(&info.channel_count())
                    || !(1..=controlled::MAX_SOURCE_RATE).contains(&info.sample_rate())
                    || info.frame_count() == 0
                    || info
                        .frame_count()
                        .saturating_add(1)
                        .saturating_mul(info.channel_count())
                        > controlled::MAX_SAMPLE_VALUES.saturating_sub(controlled_values)
                {
                    return Err(format!("Instrument {} controlled sample requires nonempty mono/stereo, rate 1..=384000, and total decoded PCM at most 64 MiB", id.0));
                }
            }
            let buffer = phonic::AudioFileBuffer::from_file(path)
                .map_err(|e| format!("Instrument {}: {e}", id.0))?;
            if buffer.frame_count() == 0 {
                return Err(format!("Instrument {} sample is empty", id.0));
            }
            if controlled_assets.contains(key) {
                if buffer.loop_range().is_some() {
                    return Err(format!("Instrument {} sample offset/direction does not support embedded sample loops", id.0));
                }
                controlled_values = controlled_values.saturating_add(buffer.buffer().len());
                if controlled_values > controlled::MAX_SAMPLE_VALUES
                    || buffer.buffer().iter().any(|sample| !sample.is_finite())
                {
                    return Err(format!("Instrument {} controlled sample exceeds 64 MiB total decoded PCM or contains non-finite samples", id.0));
                }
            }
            buffers.insert(*key, std::sync::Arc::new(buffer));
        }
        for event in &manifest.events {
            let offset = match event.command {
                Command::SampleStart { offset, .. } => offset,
                Command::Offset { frame, .. } => Some(frame),
                _ => None,
            };
            if let Some(offset) = offset {
                let key = control_specs
                    .get(&event.command.voice())
                    .map_or(AssetKey::Instrument(event.instrument), |spec| spec.0);
                let frames = buffers[&key].frame_count().saturating_sub(1);
                let bounds = asset_specs[&key]
                    .region
                    .as_ref()
                    .map_or((0, frames as u64), |region| {
                        (region.start_frame, region.end_frame)
                    });
                if offset < bounds.0 || offset >= bounds.1 {
                    if asset_specs[&key].region.is_none() {
                        return Err(format!(
                            "Instrument {} sample offset {offset} outside {frames} real source frames",
                            event.instrument.0
                        ));
                    }
                    return Err(format!(
                        "Instrument {} sample offset {offset} outside source-frame region [{}..{})",
                        event.instrument.0, bounds.0, bounds.1
                    ));
                }
            }
        }
        let mut engine = Self {
            manifest,
            position: 0,
            next: 0,
            buffers,
            asset_specs,
            samplers: BTreeMap::new(),
            control_specs,
            controlled: BTreeMap::new(),
            scratch: [0.0; 1024],
            origin: Instant::now(),
            mix,
            mix_unavailable,
            meters: None,
        };
        engine.cold_seek(0)?;
        Ok(engine)
    }
    /// Bounded live layout; absence never changes neutral legacy playback.
    pub fn mix_layout(&self) -> Option<&crate::track_mix::MixLayout> {
        self.mix.as_ref().map(|m| &m.layout)
    }
    pub fn mix_unavailable(&self) -> Option<&str> {
        self.mix_unavailable.as_deref()
    }
    /// Worker/offline only. Preallocates bounded observation storage; never
    /// changes voices, routing, coefficients, source IO or the audio arithmetic.
    pub fn enable_meters(&mut self) -> &crate::meters::MeterLayout {
        if self.meters.is_none() {
            self.meters = Some(crate::meters::MeterTap::new(
                self.mix.as_ref().map(|m| &m.layout),
                self.mix_unavailable.as_deref(),
            ));
        }
        &self.meters.as_ref().unwrap().layout
    }
    pub fn disable_meters(&mut self) {
        self.meters = None;
    }
    pub fn meter_layout(&self) -> Option<&crate::meters::MeterLayout> {
        self.meters.as_ref().map(|m| &m.layout)
    }
    pub fn meter_block(&self) -> Option<&crate::meters::MeterBlock> {
        self.meters.as_ref().and_then(|m| m.block())
    }
    /// Worker only. No source loading, schedule change, seek or queue flush.
    pub fn apply_mix_snapshot(
        &mut self,
        snapshot: &crate::track_mix::MixSnapshot,
    ) -> Result<(), String> {
        self.mix
            .as_mut()
            .ok_or("Live track mix is unavailable for this compiled snapshot")?
            .apply(snapshot)
    }
    /// Drop every old voice. No pre-roll: notes begun before this frame stay silent.
    /// Later events retain evaluator-resolved instrument/attribute memory.
    pub fn cold_seek(&mut self, frame: u64) -> Result<(), String> {
        if let Some(meters) = &mut self.meters {
            meters.invalidate();
        }
        if frame >= self.manifest.musical_frames {
            return Err("Seek must be inside the musical region".into());
        }
        self.samplers.clear();
        self.controlled.clear();
        for (key, buffer) in &self.buffers {
            let AssetKey::Instrument(id) = key else {
                continue;
            };
            let voices = self
                .manifest
                .events
                .iter()
                .filter(|e| {
                    e.instrument == *id
                        && matches!(e.command, Command::On { .. })
                        && !self.control_specs.contains_key(&e.command.voice())
                })
                .count();
            if voices == 0 {
                continue;
            }
            let source = phonic::sources::PreloadedFileSource::from_shared_buffer(
                buffer.clone(),
                &self.manifest.assets[id].to_string_lossy(),
                phonic::FilePlaybackOptions::default(),
                RATE,
            )
            .map_err(|e| e.to_string())?;
            let sampler = Sampler::from_file_source(
                source,
                GeneratorPlaybackOptions::default()
                    .voices(voices)
                    .playback_pos_emit_disabled(),
                2,
                RATE,
            )
            .map_err(|e| e.to_string())?;
            self.samplers.insert(*id, sampler);
        }
        for (voice, (key, linear)) in &self.control_specs {
            let asset = &self.asset_specs[key];
            self.controlled.insert(
                *voice,
                ControlledVoice::new(
                    self.buffers[key].clone(),
                    &asset.path,
                    *linear,
                    asset.region.as_ref(),
                    matches!(key, AssetKey::Pad(..)),
                )?,
            );
        }
        self.position = frame;
        self.next = self.manifest.events.partition_point(|e| e.frame < frame);
        Ok(())
    }
    pub fn block(&mut self, output: &mut [f32], end: u64) -> Result<usize, String> {
        if let Some(meters) = &mut self.meters {
            meters.invalidate();
        }
        if self.position >= end {
            return Ok(0);
        }
        while self.next < self.manifest.events.len()
            && self.manifest.events[self.next].frame == self.position
        {
            let event = &self.manifest.events[self.next];
            let time = SourceTime {
                pos_in_frames: self.position,
                pos_instant: self.origin,
            };
            if let Some(voice) = self.controlled.get_mut(&event.command.voice()) {
                voice.command(&event.command, &time)?;
            } else {
                send(
                    self.samplers.get_mut(&event.instrument).unwrap(),
                    &event.command,
                    &time,
                )?;
            }
            self.next += 1;
        }
        let boundary = self
            .manifest
            .events
            .get(self.next)
            .map_or(end, |e| e.frame)
            .min(end);
        let count = (boundary - self.position)
            .min(512)
            .min(output.len() as u64 / 2) as usize;
        if count == 0 {
            return Err("Non-monotonic song event schedule or empty buffer".into());
        }
        let data = &mut output[..count * 2];
        data.fill(0.0);
        if let Some(mix) = &mut self.mix {
            mix.prepare(count);
        }
        if let Some(meters) = &mut self.meters {
            meters.begin(self.position, count);
        }
        for (instrument, sampler) in &mut self.samplers {
            let source = &mut self.scratch[..count * 2];
            source.fill(0.0);
            let written = sampler.write(
                source,
                &SourceTime {
                    pos_in_frames: self.position,
                    pos_instant: self.origin,
                },
            );
            if written > source.len() {
                return Err("Invalid sampler frame count".into());
            }
            if let Some(mix) = &self.mix {
                mix.add(
                    mix.layout.legacy_tracks[instrument],
                    data,
                    source,
                    self.manifest.master_gain,
                );
                if let Some(meters) = &mut self.meters {
                    meters.observe(
                        mix.layout.legacy_tracks[instrument],
                        source,
                        self.manifest.master_gain,
                        mix,
                    );
                }
            } else {
                for (dest, value) in data.iter_mut().zip(source.iter()) {
                    *dest += value * self.manifest.master_gain;
                }
            }
        }
        for (voice_id, voice) in &mut self.controlled {
            let source = &mut self.scratch[..count * 2];
            source.fill(0.0);
            let written = voice.write(
                source,
                &SourceTime {
                    pos_in_frames: self.position,
                    pos_instant: self.origin,
                },
            );
            if written > source.len() {
                return Err("Invalid controlled voice sample count".into());
            }
            if let Some(mix) = &self.mix {
                mix.add(
                    mix.layout.controlled_tracks[voice_id],
                    data,
                    source,
                    self.manifest.master_gain,
                );
                if let Some(meters) = &mut self.meters {
                    meters.observe(
                        mix.layout.controlled_tracks[voice_id],
                        source,
                        self.manifest.master_gain,
                        mix,
                    );
                }
            } else {
                for (dest, value) in data.iter_mut().zip(source.iter()) {
                    *dest += value * self.manifest.master_gain;
                }
            }
        }
        if data.iter().any(|v| !v.is_finite() || v.abs() > 1.0) {
            return Err("Song audio is non-finite or clipping; reduce note velocities".into());
        }
        if let Some(meters) = &mut self.meters {
            meters.finish(data);
        }
        self.position += count as u64;
        Ok(count)
    }
}

#[cfg(test)]
mod region_cache_tests {
    use super::*;
    use crate::sample_asset::{metrics, reset_metrics};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "leviathan-cache-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn song(count: u16) -> Song {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slice-stereo-44100.flac");
        let asset = crate::sample_asset::import(&path).unwrap().asset;
        let mut song = example_song();
        song.samples.insert(SampleId(1), asset);
        song.regions.insert(
            RegionId(1),
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: "View".into(),
                sample: SampleId(1),
                start_frame: 10,
                end_frame: 30,
                root_note: 60,
            },
        );
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = count;
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "Shared".into(),
                rows: 1,
                tracks: BTreeMap::new(),
            },
        )]);
        song.order = vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }];
        for column in 0..count {
            let id = InstrumentId(u64::from(column) + 1);
            song.instruments.insert(
                id,
                Instrument {
                    kind: InstrumentKind::Pitched,
                    name: "Mapped".into(),
                    sample: None,
                    region: Some(RegionId(1)),
                },
            );
            song.patterns
                .get_mut(&PatternId(1))
                .unwrap()
                .tracks
                .entry(TrackId(1))
                .or_default()
                .entry(0)
                .or_default()
                .insert(
                    column,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(id),
                        ..Default::default()
                    },
                );
        }
        song
    }
    #[test]
    fn many_instruments_share_one_read_hash_decode_and_buffer_per_source_snapshot() {
        let mut song = song(256);
        song.samples
            .insert(SampleId(2), song.samples[&SampleId(1)].clone());
        song.regions.insert(
            RegionId(2),
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: "Other view".into(),
                sample: SampleId(2),
                start_frame: 100,
                end_frame: 130,
                root_note: 60,
            },
        );
        song.assign_region(InstrumentId(2), Some(RegionId(2)))
            .unwrap();
        reset_metrics();
        let engine = Engine::new(&song, None).unwrap();
        assert_eq!(metrics(), [1, 1, 1], "reads, hashes, decodes");
        assert_eq!(engine.controlled.len(), 256);
        assert!(engine.buffers.values().all(|buffer| std::sync::Arc::ptr_eq(
            buffer,
            &engine.buffers[&AssetKey::Instrument(InstrumentId(1))]
        )));
    }
    #[test]
    fn distinct_paths_are_verified_before_content_hash_buffer_reuse() {
        let mut song = song(2);
        let temp = Temp::new();
        let path = temp.0.join("alias.flac");
        fs::copy(&song.samples[&SampleId(1)].path, &path).unwrap();
        let mut asset = song.samples[&SampleId(1)].clone();
        asset.path = path.clone();
        song.samples.insert(SampleId(2), asset);
        song.regions.insert(
            RegionId(2),
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: "Alias".into(),
                sample: SampleId(2),
                start_frame: 10,
                end_frame: 30,
                root_note: 60,
            },
        );
        song.assign_region(InstrumentId(2), Some(RegionId(2)))
            .unwrap();
        reset_metrics();
        let engine = Engine::new(&song, None).unwrap();
        assert_eq!(metrics(), [2, 2, 1]);
        assert!(std::sync::Arc::ptr_eq(
            &engine.buffers[&AssetKey::Instrument(InstrumentId(1))],
            &engine.buffers[&AssetKey::Instrument(InstrumentId(2))]
        ));
        fs::write(path, b"changed bytes").unwrap();
        assert!(Engine::new(&song, None).err().unwrap().contains("SHA-256"));
    }
    #[test]
    fn aggregate_unique_encoded_sources_reject_before_any_read_hash_or_decode() {
        let mut song = song(5);
        let temp = Temp::new();
        let original = song.samples[&SampleId(1)].clone();
        for id in 1..=5 {
            let path = temp.0.join(format!("large-{id}.wav"));
            std::fs::File::create(&path)
                .unwrap()
                .set_len(crate::sample_asset::MAX_ENCODED_BYTES as u64)
                .unwrap();
            let mut asset = original.clone();
            asset.path = path;
            song.samples.insert(SampleId(id), asset);
            song.regions.insert(
                RegionId(id),
                SampleRegion {
                    amp_envelope: None,
                    forward_loop: None,
                    name: "Budget".into(),
                    sample: SampleId(id),
                    start_frame: 10,
                    end_frame: 30,
                    root_note: 60,
                },
            );
            song.assign_region(InstrumentId(id), Some(RegionId(id)))
                .unwrap();
        }
        reset_metrics();
        assert!(Engine::new(&song, None)
            .err()
            .unwrap()
            .contains("256 MiB aggregate"));
        assert_eq!(metrics(), [0, 0, 0]);
    }
    #[test]
    fn sixteen_kit_pads_share_verified_source_once_without_asset_aliasing() {
        let mut song = song(16);
        let pads = (0..16)
            .map(|slot| {
                (
                    PadId(u64::from(slot) + 1),
                    DrumPad {
                        velocity_zones: Vec::new(),
                        slot,
                        name: format!("Pad {slot}"),
                        note: 36 + slot,
                        region: Some(RegionId(1)),
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
            .collect();
        song.drum_kits.insert(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Shared source kit".into(),
                pads,
            },
        );
        for columns in song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .values_mut()
        {
            for (&column, cell) in columns {
                cell.note = Note::Note(36 + column as u8);
                cell.instrument = Some(InstrumentId(1));
            }
        }
        song.assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
        reset_metrics();
        let engine = Engine::new(&song, None).unwrap();
        assert_eq!(metrics(), [1, 1, 1]);
        assert_eq!(engine.buffers.len(), 16);
        let first = engine.buffers.values().next().unwrap();
        assert!(engine
            .buffers
            .values()
            .all(|buffer| std::sync::Arc::ptr_eq(buffer, first)));
        assert_eq!(engine.manifest.pad_voices.len(), 16);
    }
    #[test]
    fn round_robin_layers_share_one_verified_pcm_but_keep_distinct_asset_keys() {
        let mut song = song(16);
        for id in 2..=8 {
            let mut region = song.regions[&RegionId(1)].clone();
            region.start_frame += id * 20;
            region.end_frame += id * 20;
            song.regions.insert(RegionId(id), region);
        }
        song.drum_kits.insert(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Shared layered source".into(),
                pads: BTreeMap::from([(
                    PadId(1),
                    DrumPad {
                        slot: 0,
                        name: "RR".into(),
                        note: 60,
                        region: None,
                        velocity_zones: vec![VelocityZone {
                            min_velocity: 0,
                            max_velocity: 127,
                            alternatives: (1..=8)
                                .map(|id| PadLayer {
                                    id: PadLayerId(id),
                                    region: RegionId(id),
                                })
                                .collect(),
                        }],
                        root_note: 60,
                        tune_semitones: 0,
                        gain: 1.0,
                        pan: 0.0,
                        mode: PadMode::OneShot,
                        new_note: PadNewNote::Overlap,
                        choke_group: None,
                    },
                )]),
            },
        );
        for columns in song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .values_mut()
        {
            for cell in columns.values_mut() {
                cell.instrument = Some(InstrumentId(1));
            }
        }
        song.assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
        reset_metrics();
        let engine = Engine::new(&song, None).unwrap();
        assert_eq!(metrics(), [1, 1, 1], "one read/hash/decode, not eight");
        assert_eq!(engine.buffers.len(), 8);
        assert_eq!(engine.controlled.len(), 16);
        let first = engine.buffers.values().next().unwrap();
        assert!(engine
            .buffers
            .values()
            .all(|buffer| std::sync::Arc::ptr_eq(buffer, first)));
        for id in 1..=8 {
            let key = AssetKey::Pad(InstrumentId(1), PadId(1), Some(PadLayerId(id)));
            assert_eq!(
                engine.asset_specs[&key].region.as_ref().unwrap().region,
                RegionId(id)
            );
        }
    }
    #[test]
    fn melodic_zones_share_verified_pcm_but_keep_root_and_settings_per_asset() {
        let mut song = song(16);
        let mut zones = BTreeMap::new();
        for id in 1..=8 {
            let mut region = song.regions[&RegionId(1)].clone();
            region.amp_envelope = Some(AmpEnvelope {
                sustain_percent: (id * 10) as u8,
                ..Default::default()
            });
            region.forward_loop = (id % 2 == 0).then_some(ForwardLoop {
                start_frame: 12,
                end_frame: 20,
            });
            song.regions.insert(RegionId(id), region);
            let note = 59 + id as u8;
            zones.insert(
                KeyZoneId(id),
                KeyZone {
                    name: format!("Zone{id}"),
                    note_min: note,
                    note_max: note,
                    velocity_min: 0,
                    velocity_max: 127,
                    region: RegionId(id),
                    root_note: note,
                },
            );
        }
        song.upsert_multisample(
            MultisampleId(1),
            MultisampleInstrument {
                name: "Shared PCM".into(),
                zones,
            },
        )
        .unwrap();
        for columns in song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .values_mut()
        {
            for (&column, cell) in columns {
                cell.instrument = Some(InstrumentId(1));
                cell.note = Note::Note(60 + (column % 8) as u8);
            }
        }
        song.assign_multisample(InstrumentId(1), Some(MultisampleId(1)))
            .unwrap();
        reset_metrics();
        let engine = Engine::new(&song, None).unwrap();
        assert_eq!(metrics(), [1, 1, 1]);
        assert_eq!(engine.buffers.len(), 8);
        assert_eq!(engine.controlled.len(), 16);
        let first = engine.buffers.values().next().unwrap();
        assert!(engine
            .buffers
            .values()
            .all(|buffer| std::sync::Arc::ptr_eq(buffer, first)));
        for id in 1..=8 {
            let spec = &engine.asset_specs
                [&AssetKey::Multisample(InstrumentId(1), MultisampleId(1), KeyZoneId(id))];
            let region = spec.region.as_ref().unwrap();
            assert_eq!(region.root_note, 59 + id as u8);
            assert_eq!(
                region.amp_envelope.unwrap().sustain_percent,
                (id * 10) as u8
            );
            assert_eq!(region.forward_loop.is_some(), id % 2 == 0);
        }
    }
}
