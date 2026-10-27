//! Per-voice controls. Cut-only legacy voices retain phonic DSP. Region and
//! offset/direction voices use bounded linear interpolation, NOT phonic quality
//! parity. Region envelopes/forward loops are opt-in independent voice state.
//! All construction and DSP run on the rendering worker, never the callback.
use super::{send, Command, Release, RenderRegion};
use crate::{
    song::{AmpEnvelope, Direction, ForwardLoop},
    RATE,
};
use phonic::{
    generators::Sampler,
    utils::smoothing::{
        apply_smoothed_gain, apply_smoothed_panning, ExponentialSmoothedValue, SmoothedValue,
    },
    AudioFileBuffer, GeneratorPlaybackOptions, Source, SourceTime,
};
use std::{path::Path, sync::Arc};

pub(super) const MAX_SAMPLE_VALUES: usize = 16 * 1024 * 1024; // 64 MiB decoded
pub(super) const MAX_SOURCE_RATE: u32 = 384_000;

pub(super) enum ControlledVoice {
    Phonic {
        sampler: Box<Sampler>,
        active: bool,
        silent_start: bool,
    },
    Buffer(BufferVoice),
}
impl ControlledVoice {
    pub(super) fn new(
        buffer: Arc<AudioFileBuffer>,
        path: &Path,
        linear: bool,
        region: Option<&RenderRegion>,
        immediate_start: bool,
    ) -> Result<Self, String> {
        if linear {
            let mut voice = BufferVoice::new(buffer, region)?;
            voice.immediate_start = immediate_start;
            Ok(Self::Buffer(voice))
        } else {
            let source = phonic::sources::PreloadedFileSource::from_shared_buffer(
                buffer,
                &path.to_string_lossy(),
                phonic::FilePlaybackOptions::default(),
                RATE,
            )
            .map_err(|e| e.to_string())?;
            let sampler = Sampler::from_file_source(
                source,
                GeneratorPlaybackOptions::default()
                    .voices(1)
                    .playback_pos_emit_disabled(),
                2,
                RATE,
            )
            .map_err(|e| e.to_string())?;
            Ok(Self::Phonic {
                sampler: Box::new(sampler),
                active: false,
                silent_start: false,
            })
        }
    }
    pub(super) fn command(&mut self, command: &Command, time: &SourceTime) -> Result<(), String> {
        match self {
            Self::Phonic {
                sampler,
                active,
                silent_start,
            } => {
                if matches!(command, Command::Cut { .. }) {
                    *active = false;
                    return Ok(());
                }
                if let Command::On { volume, .. } = *command {
                    *active = true;
                    *silent_start = volume == 0.0;
                }
                if matches!(command, Command::Volume { value, .. } if *value > 0.0) {
                    *silent_start = false;
                }
                if *active {
                    send(sampler, command, time)?;
                }
                Ok(())
            }
            Self::Buffer(voice) => voice.command(command),
        }
    }
    pub(super) fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        match self {
            Self::Phonic {
                sampler,
                active: true,
                silent_start,
            } => {
                let written = sampler.write(output, time);
                if *silent_start {
                    output[..written].fill(0.0);
                }
                written
            }
            Self::Phonic { active: false, .. } => 0,
            Self::Buffer(voice) => voice.write(output),
        }
    }
}

/// Integer output-frame counters make ADSR independent of render partitions,
/// source rate, pitch, BPM and future gain controls. No heap storage or event fanout.
struct Envelope {
    settings: AmpEnvelope,
    held_frames: u64,
    release: Option<(f64, u32, u32)>, // initial gain, elapsed, total
}
impl Envelope {
    fn new(settings: AmpEnvelope) -> Self {
        Self {
            settings,
            held_frames: 0,
            release: None,
        }
    }
    fn gain(&self) -> f64 {
        if let Some((gain, elapsed, total)) = self.release {
            return if elapsed >= total {
                0.0
            } else {
                gain * (1.0 - f64::from(elapsed) / f64::from(total))
            };
        }
        // The engine is explicitly fixed at 48 kHz. These are not source frames.
        let attack = u64::from(self.settings.attack_ms) * 48;
        let decay = u64::from(self.settings.decay_ms) * 48;
        let sustain = f64::from(self.settings.sustain_percent) / 100.0;
        if self.held_frames < attack {
            self.held_frames as f64 / attack as f64
        } else if self.held_frames - attack < decay {
            1.0 - (1.0 - sustain) * (self.held_frames - attack) as f64 / decay as f64
        } else {
            sustain
        }
    }
    fn release(&mut self, reason: &Release) {
        if self.release.is_none() {
            let frames = if matches!(
                reason,
                Release::Choke | Release::PadReplaced | Release::TrackMute
            ) {
                2400 // Explicit opt-in safety policy: linear 50 ms, not hard cut.
            } else {
                u32::from(self.settings.release_ms) * 48
            };
            self.release = Some((self.gain(), 0, frames));
        }
    }
    fn advance(&mut self) {
        if let Some((_, elapsed, _)) = &mut self.release {
            *elapsed += 1;
        } else {
            self.held_frames += 1;
        }
    }
    fn finished(&self) -> bool {
        self.release
            .is_some_and(|(_, elapsed, total)| elapsed >= total)
    }
}

pub(super) struct BufferVoice {
    buffer: Arc<AudioFileBuffer>,
    real_frames: usize,
    start_frame: usize,
    root_note: u8,
    position: f64,
    /// Region and native-rate-converted controlled voices derive phase from an
    /// integer output-frame count, avoiding cumulative rate-ratio boundary error.
    stable_phase: bool,
    phase_anchor: f64,
    phase_frames: i64,
    direction: Direction,
    active: bool,
    speed: f64,
    target_speed: f64,
    glide_rate: f32,
    frames_until_speed_update: u32,
    volume: ExponentialSmoothedValue,
    pan: ExponentialSmoothedValue,
    release_gain: Option<f32>,
    immediate_start: bool,
    envelope: Option<Envelope>,
    forward_loop: Option<ForwardLoop>,
}
impl BufferVoice {
    fn new(buffer: Arc<AudioFileBuffer>, region: Option<&RenderRegion>) -> Result<Self, String> {
        // The pinned decoder adds exactly one zero guard frame. It is NOT a
        // source frame and must never become the beginning of reverse playback.
        let real_frames = buffer.frame_count().saturating_sub(1);
        if real_frames == 0
            || !(1..=2).contains(&buffer.channel_count())
            || !(1..=MAX_SOURCE_RATE).contains(&buffer.sample_rate())
            || buffer.buffer().len() > MAX_SAMPLE_VALUES
        {
            return Err("Controlled sample requires nonempty mono/stereo, sample rate 1..=384000, and at most 64 MiB decoded PCM".into());
        }
        let start_frame = region.map_or(0, |region| region.start_frame as usize);
        let end_frame = region.map_or(real_frames, |region| region.end_frame as usize);
        if start_frame >= end_frame || end_frame > real_frames {
            return Err("Source region is outside real source frames".into());
        }
        let stable_phase = region.is_some() || buffer.sample_rate() != RATE;
        Ok(Self {
            buffer,
            real_frames: end_frame,
            start_frame,
            root_note: region.map_or(60, |region| region.root_note),
            position: 0.0,
            stable_phase,
            phase_anchor: 0.0,
            phase_frames: 0,
            direction: Direction::Forward,
            active: false,
            speed: 1.0,
            target_speed: 1.0,
            glide_rate: 0.0,
            frames_until_speed_update: 0,
            volume: ExponentialSmoothedValue::new(1.0, RATE),
            pan: ExponentialSmoothedValue::new(0.0, RATE),
            release_gain: None,
            immediate_start: false,
            envelope: region.and_then(|r| r.amp_envelope).map(Envelope::new),
            forward_loop: region.and_then(|r| r.forward_loop),
        })
    }
    fn command(&mut self, command: &Command) -> Result<(), String> {
        match *command {
            Command::On {
                pitch, volume, pan, ..
            } => {
                self.active = true;
                self.position = self.start_frame as f64;
                self.reset_phase();
                self.direction = Direction::Forward;
                self.speed = if self.root_note == 60 {
                    phonic::utils::speed_from_note(pitch)
                } else {
                    2.0_f64.powf((f64::from(pitch) - f64::from(self.root_note)) / 12.0)
                };
                self.target_speed = self.speed;
                self.glide_rate = 0.0;
                self.frames_until_speed_update = 0;
                self.volume.set_target(volume);
                if volume == 0.0 || self.immediate_start {
                    self.volume.init(volume);
                }
                self.pan.set_target(pan);
                if self.immediate_start {
                    self.pan.init(pan);
                }
                self.release_gain = None;
                if let Some(envelope) = &mut self.envelope {
                    *envelope = Envelope::new(envelope.settings);
                }
            }
            Command::Cut { .. } => self.active = false,
            _ if !self.active => (), // Logical notes may outlive natural sample exhaustion.
            Command::Off { ref reason, .. } => {
                if let Some(envelope) = &mut self.envelope {
                    envelope.release(reason);
                    if envelope.finished() {
                        self.active = false;
                    }
                } else {
                    self.release_gain.get_or_insert(1.0);
                }
            }
            Command::Volume { value, .. } => self.volume.set_target(value),
            Command::Pan { value, .. } => self.pan.set_target(value),
            Command::Glide {
                target,
                semitones_per_second,
                ..
            } => {
                self.target_speed =
                    2.0_f64.powf((f64::from(target) - f64::from(self.root_note)) / 12.0);
                self.glide_rate = semitones_per_second;
                self.frames_until_speed_update = 0;
            }
            Command::SampleStart {
                offset, direction, ..
            } => {
                self.direction = direction;
                self.position = offset.map_or_else(
                    || {
                        if direction == Direction::Reverse {
                            (self.real_frames - 1) as f64
                        } else {
                            self.start_frame as f64
                        }
                    },
                    |frame| frame as f64,
                );
                self.check_position()?;
                self.reset_phase();
            }
            Command::Offset { frame, .. } => {
                self.position = frame as f64;
                self.check_position()?;
                self.reset_phase();
            }
            Command::Direction { direction, .. } => self.direction = direction,
        }
        Ok(())
    }
    fn check_position(&self) -> Result<(), String> {
        if self.position < self.start_frame as f64 || self.position >= self.real_frames as f64 {
            Err("Sample offset is outside decoded source frames".into())
        } else {
            Ok(())
        }
    }
    fn reset_phase(&mut self) {
        self.phase_anchor = self.position;
        self.phase_frames = 0;
    }
    fn wrap_loop(&mut self) {
        if let Some(loop_) = self.forward_loop {
            if self.position >= loop_.end_frame as f64 {
                // A high playback ratio may cross thousands of loop lengths in
                // one output frame. Modulo is O(1), not a loop/event generator.
                self.position = loop_.start_frame as f64
                    + (self.position - loop_.start_frame as f64)
                        .rem_euclid((loop_.end_frame - loop_.start_frame) as f64);
            }
        }
    }
    fn update_speed(&mut self) {
        if self.frames_until_speed_update != 0 {
            return;
        }
        let previous = self.speed;
        let difference = self.target_speed - self.speed;
        if self.glide_rate > 0.0 && difference.abs() > 0.0001 {
            let semitones = (12.0 * (self.target_speed / self.speed).log2()).abs();
            let seconds = semitones as f32 / self.glide_rate;
            let step = difference / f64::from(seconds * RATE as f32) * 64.0;
            if step.abs() >= difference.abs() {
                self.speed = self.target_speed;
            } else {
                self.speed += step;
            }
        } else {
            self.speed = self.target_speed;
        }
        if self.stable_phase && self.speed != previous {
            self.reset_phase();
        }
        self.frames_until_speed_update = 64;
    }
    fn write(&mut self, output: &mut [f32]) -> usize {
        if !self.active {
            return 0;
        }
        let channels = self.buffer.channel_count();
        let mut written = 0;
        let release_inertia = 1.0 - (-4.605f32 / (RATE as f32 * 0.05)).exp();
        for frame in output.as_chunks_mut::<2>().0 {
            if self.position < self.start_frame as f64 || self.position >= self.real_frames as f64 {
                self.active = false;
                break;
            }
            self.update_speed();
            let lower = self.position.floor() as usize;
            let fraction = (self.position - lower as f64) as f32;
            for (channel, value) in frame.iter_mut().enumerate() {
                let source_channel = if channels == 1 { 0 } else { channel };
                let a = self.buffer.buffer()[lower * channels + source_channel];
                let b = if let Some(loop_) = self
                    .forward_loop
                    .filter(|l| lower as u64 + 1 == l.end_frame)
                {
                    self.buffer.buffer()[loop_.start_frame as usize * channels + source_channel]
                } else if lower + 1 < self.real_frames {
                    self.buffer.buffer()[(lower + 1) * channels + source_channel]
                } else {
                    0.0
                };
                *value = a + (b - a) * fraction;
            }
            if let Some(envelope) = &mut self.envelope {
                let gain = envelope.gain() as f32;
                for sample in frame.iter_mut() {
                    *sample *= gain;
                }
                envelope.advance();
                if envelope.finished() {
                    self.active = false;
                }
            }
            if let Some(gain) = &mut self.release_gain {
                *gain += -*gain * release_inertia;
                for sample in frame {
                    *sample *= *gain;
                }
                if *gain < 0.0001 {
                    self.active = false;
                }
            }
            written += 2;
            let direction = if self.direction == Direction::Forward {
                1.0
            } else {
                -1.0
            };
            if self.stable_phase {
                // Keep signed progress across direction flips. Multiplication
                // before division preserves exact native-rate boundaries such
                // as 294 source frames / 44100 Hz == 320 output frames / 48000.
                self.phase_frames += if self.direction == Direction::Forward {
                    1
                } else {
                    -1
                };
                self.position = (self.phase_anchor * f64::from(RATE)
                    + self.phase_frames as f64 * self.speed * f64::from(self.buffer.sample_rate()))
                    / f64::from(RATE);
            } else {
                // Existing 48k no-region controlled voices retain exact PCM.
                self.position +=
                    direction * self.speed * f64::from(self.buffer.sample_rate()) / f64::from(RATE);
            }
            self.frames_until_speed_update -= 1;
            self.wrap_loop();
            // Exhaustion belongs to the final consumed source frame, not the
            // next write call. A control exactly at a block/row boundary must
            // not revive a sample merely because that next call has not run.
            if self.position < self.start_frame as f64 || self.position >= self.real_frames as f64 {
                self.active = false;
            }
            if !self.active {
                break;
            }
        }
        apply_smoothed_gain(&mut output[..written], &mut self.volume);
        apply_smoothed_panning(&mut output[..written], 2, &mut self.pan);
        written
    }
}
