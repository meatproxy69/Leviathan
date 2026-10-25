//! Deterministic, stateful document evaluator and synchronous multi-instrument renderer.
use crate::{song::*, RenderStats, RATE, TAIL};
use phonic::{
    generators::Sampler, Generator, GeneratorPlaybackEvent as P, GeneratorPlaybackMessage,
    GeneratorPlaybackOptions, Source, SourceTime,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::BufWriter,
    path::{Path, PathBuf},
    time::Instant,
};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Release {
    ExplicitOff,
    Replaced,
    Retrigger,
    SongEnd,
    PhraseEnd,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    On {
        voice: usize,
        pitch: u8,
        volume: f32,
        pan: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        offset: f64,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse: bool,
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
    Cut,
    Speed {
        voice: usize,
        pitch: u8,
        rate: f32,
    },
}
fn is_zero(value: &f64) -> bool {
    *value == 0.0
}
#[derive(Clone, Copy)]
enum Action<'a> {
    Cell(&'a Cell, bool),
    Cut,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub frame: u64,
    pub track: TrackId,
    /// Phrase subcolumns use 256 + trigger_column * 16 + local_column.
    pub column: u16,
    pub instrument: InstrumentId,
    /// Originating song cell; phrase events point to the trigger. None for ordinary end releases.
    pub position: Option<(usize, u32)>,
    pub command: Command,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase: Option<PhraseOrigin>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseOrigin {
    pub instrument: InstrumentId,
    pub phrase: PhraseId,
    pub trigger_column: u16,
    pub row: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<crate::arrangement::ArrangementInfo>,
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
}
struct Column {
    instrument: Option<InstrumentId>,
    volume: f32,
    pan: f32,
    active: Option<(InstrumentId, usize)>,
    last_instrument: Option<InstrumentId>,
}
impl Default for Column {
    fn default() -> Self {
        Self {
            instrument: None,
            volume: 1.0,
            pan: 0.0,
            active: None,
            last_instrument: None,
        }
    }
}
pub fn evaluate(song: &Song, project_dir: Option<&Path>) -> Result<Manifest, String> {
    song.validate()?;
    let arranged = song.patterns.values().any(|p| !p.aliases.is_empty())
        || song
            .order
            .iter()
            .any(|o| !o.muted_tracks.is_empty() || o.section.is_some());
    if arranged {
        let (content, mut info) = crate::arrangement::materialize(song)?;
        let allow_empty = song.order.iter().any(|o| !o.muted_tracks.is_empty());
        let mut manifest = evaluate_content(&content, project_dir, allow_empty)?;
        info.isolated_voices = manifest.schema >= 2;
        manifest.schema = 4;
        manifest.evaluator = "leviathan-song-v4".into();
        manifest.arrangement = Some(info);
        Ok(manifest)
    } else {
        evaluate_content(song, project_dir, false)
    }
}
fn evaluate_content(
    song: &Song,
    project_dir: Option<&Path>,
    allow_empty: bool,
) -> Result<Manifest, String> {
    if song
        .order
        .iter()
        .filter_map(|o| song.patterns.get(&o.pattern))
        .any(|p| {
            p.tracks.values().any(|rows| {
                rows.values()
                    .any(|columns| columns.values().any(|cell| cell.phrase.is_some()))
            })
        })
    {
        crate::phrases::evaluate(song, project_dir, allow_empty)
    } else {
        evaluate_flat(song, project_dir, allow_empty, 0.0)
    }
}
pub(crate) fn evaluate_flat(
    song: &Song,
    project_dir: Option<&Path>,
    allow_empty: bool,
    origin: f64,
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
    let end = (origin + rows as f64 * per_row).round() as u64;
    if rows as f64 * per_row < 1.0 || rows as f64 * per_row > RATE as f64 * 600.0 {
        return Err("Song duration must be positive and at most ten minutes".into());
    }
    let mut raw = Vec::new();
    let mut effected = false;
    let mut populated = 0usize;
    let mut offset = 0u64;
    for (occurrence, order) in song.order.iter().enumerate() {
        let pattern = &song.patterns[&order.pattern];
        for (track_index, track) in song.track_order.iter().enumerate() {
            if let Some(track_rows) = pattern.tracks.get(track) {
                for (row, columns) in track_rows {
                    for (column, cell) in columns {
                        if *cell == Cell::default() {
                            continue;
                        }
                        populated += 1;
                        if populated > 32768 {
                            return Err("Song exceeds 32768 populated cells".into());
                        }
                        let absolute_row = offset + *row as u64;
                        let frame = (origin
                            + (absolute_row as f64 + cell.delay.unwrap_or(0.0)) * per_row)
                            .round() as u64;
                        effected |= !cell.effects.is_empty();
                        let start = frame.min(end - 1);
                        raw.push((
                            start,
                            absolute_row,
                            track_index,
                            *column,
                            occurrence,
                            *row,
                            *track,
                            0u8,
                            Action::Cell(cell, false),
                        ));
                        let row_end = (origin + (absolute_row + 1) as f64 * per_row).round() as u64;
                        let delay = cell.delay.unwrap_or(0.0);
                        let cut = cell.effects.cut.map(|fraction| {
                            if fraction == 0.0 {
                                start
                            } else {
                                ((origin
                                    + (absolute_row as f64 + delay + (1.0 - delay) * fraction)
                                        * per_row)
                                    .round() as u64)
                                    .max(start)
                            }
                        });
                        let mut previous = start;
                        if let Some(count) = cell.effects.retrigger {
                            for repeat in 1..count {
                                let at = (origin
                                    + (absolute_row as f64
                                        + delay
                                        + (1.0 - delay) * repeat as f64 / count as f64)
                                        * per_row)
                                    .round() as u64;
                                if cut.is_some_and(|cut| at >= cut) {
                                    break;
                                }
                                if at <= previous || at >= row_end || at >= end {
                                    return Err("Retrigger spacing is below one rendered frame; reduce delay/count".into());
                                }
                                previous = at;
                                raw.push((
                                    at,
                                    absolute_row,
                                    track_index,
                                    *column,
                                    occurrence,
                                    *row,
                                    *track,
                                    repeat,
                                    Action::Cell(cell, true),
                                ));
                            }
                        }
                        if let Some(at) = cut {
                            raw.push((
                                at.min(end),
                                absolute_row,
                                track_index,
                                *column,
                                occurrence,
                                *row,
                                *track,
                                16,
                                Action::Cut,
                            ));
                        }
                        if raw.len() > 65536 {
                            return Err("Song exceeds 65536 scheduled cell/effect actions".into());
                        }
                    }
                }
            }
        }
        offset += pattern.rows as u64;
    }
    // Ties keep earlier rows/columns first, with a cut after its own cell attack.
    raw.sort_by_key(|e| (e.0, e.1, e.2, e.3, e.7));
    let mut states: BTreeMap<(TrackId, u16), Column> = BTreeMap::new();
    let mut events = Vec::new();
    let mut next_voice = 1;
    for (frame, _, _, column, occurrence, row, track, _, action) in raw {
        let state = states.entry((track, column)).or_default();
        let cell = match action {
            Action::Cell(cell, _) => cell,
            Action::Cut => {
                state.active = None;
                if let Some(instrument) = state.last_instrument {
                    events.push(Event {
                        frame,
                        track,
                        column,
                        instrument,
                        position: Some((occurrence, row)),
                        command: Command::Cut,
                        phrase: None,
                    });
                }
                continue;
            }
        };
        if let Some(instrument) = cell.instrument {
            state.instrument = Some(instrument);
        }
        if let Some(value) = cell.velocity {
            state.volume = value;
        }
        if let Some(value) = cell.pan {
            state.pan = value;
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
        match cell.note {
            Note::Note(pitch) => {
                let instrument = state.instrument.ok_or_else(|| format!("Occurrence {occurrence}, track {}, row {row}, column {column}: note has no selected instrument", track.0))?;
                if let (Some(rate), Some((active_instrument, voice))) =
                    (cell.effects.glide, state.active)
                {
                    if active_instrument != instrument {
                        return Err("Glide cannot change the sounding instrument; start an ordinary note first".into());
                    }
                    if let Some(value) = cell.velocity {
                        emit(instrument, Command::Volume { voice, value });
                    }
                    if let Some(value) = cell.pan {
                        emit(instrument, Command::Pan { voice, value });
                    }
                    emit(instrument, Command::Speed { voice, pitch, rate });
                    continue;
                }
                if let Some((instrument, voice)) = state.active.take() {
                    emit(
                        instrument,
                        Command::Off {
                            voice,
                            reason: if matches!(action, Action::Cell(_, true)) {
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
                        offset: cell.effects.offset.unwrap_or(0.0),
                        reverse: cell.effects.reverse,
                    },
                );
                state.active = Some((instrument, next_voice));
                state.last_instrument = Some(instrument);
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
    }
    if next_voice == 1 && !allow_empty {
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
    let mut assets = BTreeMap::new();
    for event in &events {
        if assets.contains_key(&event.instrument) {
            continue;
        }
        let source = song.instruments[&event.instrument]
            .sample
            .as_ref()
            .ok_or_else(|| format!("Instrument {} has no sample assigned", event.instrument.0))?;
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
    Ok(Manifest {
        arrangement: None,
        schema: if effected { 2 } else { 1 },
        evaluator: if effected {
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
    })
}
pub(crate) fn send(sampler: &Sampler, command: &Command) -> Result<(), String> {
    let event = match *command {
        Command::On {
            voice,
            pitch,
            volume,
            pan,
            ..
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
        Command::Speed { voice, pitch, rate } => P::SetSpeed {
            note_id: voice,
            speed: phonic::utils::speed_from_note(pitch),
            glide: Some(rate),
        },
        Command::Cut => return Err("Hard cut requires the isolated-voice renderer".into()),
    };
    sampler
        .playback_message_queue()
        .push(GeneratorPlaybackMessage::Trigger { event })
        .map_err(|_| "Song sampler event queue full".into())
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

/// Shared incremental DSP engine: used by both offline export and streaming worker.
/// No device/thread/UI state; buffers decode once and are shared across cold resets.
pub struct Engine {
    pub manifest: Manifest,
    pub position: u64,
    next: usize,
    buffers: BTreeMap<InstrumentId, std::sync::Arc<phonic::AudioFileBuffer>>,
    samplers: BTreeMap<InstrumentId, Sampler>,
    steps: Option<crate::step_audio::StepAudio>,
    scratch: [f32; 1024],
    origin: Instant,
}
impl Engine {
    pub fn new(song: &Song, base: Option<&Path>) -> Result<Self, String> {
        let manifest = evaluate(song, base)?;
        let mut buffers = BTreeMap::new();
        for (id, path) in &manifest.assets {
            let buffer = phonic::AudioFileBuffer::from_file(path)
                .map_err(|e| format!("Instrument {}: {e}", id.0))?;
            if buffer.frame_count() == 0 {
                return Err(format!("Instrument {} sample is empty", id.0));
            }
            buffers.insert(*id, std::sync::Arc::new(buffer));
        }
        let steps = if manifest
            .arrangement
            .as_ref()
            .map_or(manifest.schema >= 2, |a| a.isolated_voices)
        {
            Some(crate::step_audio::StepAudio::new(&manifest, &buffers)?)
        } else {
            None
        };
        let mut engine = Self {
            manifest,
            position: 0,
            next: 0,
            buffers,
            samplers: BTreeMap::new(),
            steps,
            scratch: [0.0; 1024],
            origin: Instant::now(),
        };
        engine.cold_seek(0)?;
        Ok(engine)
    }
    /// Drop every old voice. No pre-roll: notes begun before this frame stay silent.
    /// Later events retain evaluator-resolved instrument/attribute memory.
    pub fn cold_seek(&mut self, frame: u64) -> Result<(), String> {
        if frame >= self.manifest.musical_frames {
            return Err("Seek must be inside the musical region".into());
        }
        self.samplers.clear();
        if let Some(steps) = &mut self.steps {
            steps.reset();
        }
        for (id, buffer) in self.buffers.iter().filter(|_| self.steps.is_none()) {
            let voices = self
                .manifest
                .events
                .iter()
                .filter(|e| e.instrument == *id && matches!(e.command, Command::On { .. }))
                .count();
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
        self.position = frame;
        self.next = self.manifest.events.partition_point(|e| e.frame < frame);
        Ok(())
    }
    pub fn block(&mut self, output: &mut [f32], end: u64) -> Result<usize, String> {
        if self.position >= end {
            return Ok(0);
        }
        while self.next < self.manifest.events.len()
            && self.manifest.events[self.next].frame == self.position
        {
            let event = &self.manifest.events[self.next];
            if let Some(steps) = &mut self.steps {
                steps.apply(event)?;
            } else {
                send(&self.samplers[&event.instrument], &event.command)?;
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
        if let Some(steps) = &mut self.steps {
            steps.mix(
                data,
                &mut self.scratch[..count * 2],
                &SourceTime {
                    pos_in_frames: self.position,
                    pos_instant: self.origin,
                },
                self.manifest.master_gain,
            )?;
        }
        for sampler in self.samplers.values_mut() {
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
            for (dest, value) in data.iter_mut().zip(source.iter()) {
                *dest += value * self.manifest.master_gain;
            }
        }
        if data.iter().any(|v| !v.is_finite() || v.abs() > 1.0) {
            return Err("Song audio is non-finite or clipping; reduce note velocities".into());
        }
        self.position += count as u64;
        Ok(count)
    }
}
