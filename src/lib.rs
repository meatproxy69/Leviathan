//! Sample-accurate pattrns -> phonic bridge and synchronous offline renderer.
pub mod arrangement;
pub mod editor;
pub mod generation;
pub mod pattern;
#[cfg(feature = "desktop")]
mod phrase_ui;
mod phrases;
pub mod project;
pub mod session;
pub mod song;
pub mod song_render;
mod step_audio;
pub mod transform;
pub mod transport;
pub use pattern::pattern_manifest;
use phonic::{
    generators::Sampler, Generator, GeneratorPlaybackEvent as PhonicEvent,
    GeneratorPlaybackMessage, GeneratorPlaybackOptions, Source, SourceTime,
};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::BufWriter, path::Path, time::Instant};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const RATE: u32 = 48_000;
pub const TAIL: u64 = 24_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    NoteOn {
        id: usize,
        midi_note: u8,
        velocity: f32,
        pan: f32,
    },
    NoteOff {
        id: usize,
    },
    AllNotesOff,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedEvent {
    pub frame: u64,
    pub event: Event,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub sequencer: String,
    pub sample_rate: u32,
    pub bpm: f64,
    pub bars: u32,
    pub musical_frames: u64,
    pub tail_frames: u64,
    pub events: Vec<TimedEvent>,
}

pub fn dispatch(generator: &impl Generator, event: &Event) -> Result<()> {
    let event = match *event {
        Event::NoteOn {
            id,
            midi_note,
            velocity,
            pan,
        } => {
            if midi_note > 127
                || !velocity.is_finite()
                || !(0.0..=1.0).contains(&velocity)
                || !pan.is_finite()
                || !(-1.0..=1.0).contains(&pan)
            {
                return Err("invalid note event".into());
            }
            PhonicEvent::NoteOn {
                note_id: id,
                note: midi_note,
                volume: Some(velocity),
                panning: Some(pan),
                context: None,
            }
        }
        Event::NoteOff { id } => PhonicEvent::NoteOff { note_id: id },
        Event::AllNotesOff => PhonicEvent::AllNotesOff,
    };
    generator
        .playback_message_queue()
        .push(GeneratorPlaybackMessage::Trigger { event })
        .map_err(|_| "phonic event queue is full")?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct RenderStats {
    pub frames: u64,
    pub peak: f32,
    pub energy: f64,
}

/// Synchronous adapter: split at every event boundary; finalize explicitly and propagate I/O errors.
/// No Player/WavOutput threads, wallclock scheduling, or upstream stop deadlock.
pub fn render(
    sample: &Path,
    output: &Path,
    manifest: &Manifest,
    block_frames: usize,
) -> Result<RenderStats> {
    if block_frames == 0 || block_frames > 8192 {
        return Err("block size must be 1..=8192".into());
    }
    if manifest.sample_rate != RATE || manifest.events.windows(2).any(|w| w[0].frame > w[1].frame) {
        return Err("invalid manifest timing".into());
    }
    let total = manifest
        .musical_frames
        .checked_add(manifest.tail_frames)
        .ok_or("duration overflow")?;
    if manifest.events.iter().any(|e| e.frame > total) {
        return Err("event after render end".into());
    }
    let mut sampler = Sampler::from_file(
        sample,
        GeneratorPlaybackOptions::default()
            .voices(8)
            .playback_pos_emit_disabled(),
        2,
        RATE,
    )?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut writer = hound::WavWriter::new(
        BufWriter::new(file),
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut position = 0;
    let mut next = 0;
    let mut buffer = vec![0.0; block_frames * 2];
    let mut stats = RenderStats {
        frames: total,
        peak: 0.0,
        energy: 0.0,
    };
    let origin = Instant::now();
    while position < total {
        while next < manifest.events.len() && manifest.events[next].frame == position {
            dispatch(&sampler, &manifest.events[next].event)?;
            next += 1;
        }
        let boundary = manifest
            .events
            .get(next)
            .map_or(total, |e| e.frame)
            .min(total);
        let count = (boundary - position).min(block_frames as u64) as usize;
        let data = &mut buffer[..count * 2];
        data.fill(0.0);
        let written = sampler.write(
            data,
            &SourceTime {
                pos_in_frames: position,
                pos_instant: origin,
            },
        );
        if written > data.len() {
            return Err("source returned invalid sample count".into());
        }
        for &value in data.iter() {
            if !value.is_finite() || value.abs() > 1.0 {
                return Err("non-finite or clipping audio sample".into());
            }
            stats.peak = stats.peak.max(value.abs());
            stats.energy += (value as f64).powi(2);
            writer.write_sample(value)?;
        }
        position += count as u64;
    }
    dispatch(&sampler, &Event::AllNotesOff)?;
    // Drain shutdown command before dropping the generator; this writes no extra file frames.
    sampler.write(
        &mut buffer[..2],
        &SourceTime {
            pos_in_frames: total,
            pos_instant: origin,
        },
    );
    writer.finalize()?;
    Ok(stats)
}

pub fn generate_sample(path: &Path) -> Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = hound::WavWriter::new(
        BufWriter::new(file),
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for frame in 0..RATE {
        let t = frame as f64 / RATE as f64;
        let envelope = (t / 0.005).min(1.0) * ((1.0 - t) / 0.02).min(1.0);
        for frequency in [261.625565, 329.627557] {
            writer.write_sample(
                (0.25 * envelope * (std::f64::consts::TAU * frequency * t).sin()) as f32,
            )?;
        }
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(feature = "desktop")]
pub mod desktop;
pub mod jobs;

#[cfg(feature = "desktop")]
pub mod tracker_ui;

#[cfg(feature = "desktop")]
pub mod arrangement_ui;

#[cfg(feature = "desktop")]
pub mod transform_ui;

#[cfg(feature = "desktop")]
pub mod generation_ui;
