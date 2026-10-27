//! Worker-side isolated voices for step effects; the device callback never enters here.
use crate::{
    song::{InstrumentId, TrackId},
    song_render::{send_with_root, Command, Event, Manifest},
    RATE,
};
use phonic::{generators::Sampler, GeneratorPlaybackOptions, Source, SourceTime};
use std::{collections::BTreeMap, sync::Arc};

type Key = (InstrumentId, u64, bool);
fn key(instrument: InstrumentId, offset: f64, reverse: bool) -> Key {
    (
        instrument,
        if offset == 0.0 { 0 } else { offset.to_bits() },
        reverse,
    )
}
struct Voice {
    sampler: Sampler,
    track: TrackId,
    column: u16,
    root: u8,
}
pub(crate) struct StepAudio {
    buffers: BTreeMap<Key, Arc<phonic::AudioFileBuffer>>,
    roots: BTreeMap<InstrumentId, u8>,
    voices: BTreeMap<usize, Voice>,
}
impl StepAudio {
    pub fn new(
        manifest: &Manifest,
        source: &BTreeMap<InstrumentId, Arc<phonic::AudioFileBuffer>>,
    ) -> Result<Self, String> {
        let mut buffers = BTreeMap::new();
        let mut bytes = 0usize;
        for event in &manifest.events {
            let Command::On {
                offset, reverse, ..
            } = event.command
            else {
                continue;
            };
            let key = key(event.instrument, offset, reverse);
            if buffers.contains_key(&key) {
                continue;
            }
            if buffers.len() >= 256 {
                return Err("Step effects exceed 256 source variants".into());
            }
            let original = &source[&event.instrument];
            let frames = manifest
                .regions
                .get(&event.instrument)
                .map_or_else(
                    || original.frame_count().saturating_sub(1).max(1),
                    |region| region.end_frame.saturating_sub(region.start_frame) as usize,
                )
                .min(original.frame_count().saturating_sub(1).max(1));
            let buffer = if offset == 0.0 && !reverse {
                original.clone()
            } else {
                // AudioFileBuffer::from_file appends one silent interpolation frame.
                // Do not reverse that sentinel into the beginning of a sample.
                let skip = ((offset * frames as f64).floor() as usize).min(frames - 1);
                let channels = original.channel_count();
                let length = (frames - skip + 1)
                    .checked_mul(channels)
                    .ok_or("Step sample size overflow")?;
                bytes = bytes
                    .checked_add(length.checked_mul(4).ok_or("Step sample size overflow")?)
                    .ok_or("Step sample size overflow")?;
                if bytes > 128 * 1024 * 1024 {
                    return Err("Step sample variants exceed 128 MiB".into());
                }
                let mut data = Vec::with_capacity(length);
                // Engine buffers for mapped instruments are already cropped to
                // their region, so offset and reverse stay region-local here.
                let region_start = 0usize;
                if reverse {
                    for frame in (0..frames - skip).rev() {
                        data.extend_from_slice(
                            &original.buffer()[(region_start + frame) * channels
                                ..(region_start + frame + 1) * channels],
                        );
                    }
                } else {
                    data.extend_from_slice(
                        &original.buffer()
                            [(region_start + skip) * channels..(region_start + frames) * channels],
                    );
                }
                data.resize(length, 0.0);
                Arc::new(
                    phonic::AudioFileBuffer::new(data, channels, original.sample_rate(), None)
                        .map_err(|e| e.to_string())?,
                )
            };
            buffers.insert(key, buffer);
        }
        Ok(Self {
            buffers,
            roots: manifest
                .regions
                .iter()
                .map(|(id, region)| (*id, region.root_note))
                .collect(),
            voices: BTreeMap::new(),
        })
    }
    pub fn reset(&mut self) {
        self.voices.clear();
    }
    pub fn apply(&mut self, event: &Event) -> Result<(), String> {
        match event.command {
            Command::On {
                voice,
                offset,
                reverse,
                ..
            } => {
                if self.voices.len() >= 256 {
                    return Err(
                        "Step playback exceeds 256 simultaneous voices including release tails"
                            .into(),
                    );
                }
                let source = phonic::sources::PreloadedFileSource::from_shared_buffer(
                    self.buffers[&key(event.instrument, offset, reverse)].clone(),
                    "leviathan-step-sample",
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
                let root = self.roots.get(&event.instrument).copied().unwrap_or(60);
                send_with_root(&sampler, &event.command, root)?;
                self.voices.insert(
                    voice,
                    Voice {
                        sampler,
                        track: event.track,
                        column: event.column,
                        root,
                    },
                );
            }
            Command::Cut => self
                .voices
                .retain(|_, voice| voice.track != event.track || voice.column != event.column),
            Command::Off { voice, .. }
            | Command::Volume { voice, .. }
            | Command::Pan { voice, .. }
            | Command::Speed { voice, .. } => {
                // Cold seek intentionally skips voices that started before its target.
                if let Some(active) = self.voices.get(&voice) {
                    send_with_root(&active.sampler, &event.command, active.root)?;
                }
            }
        }
        Ok(())
    }
    pub fn mix(
        &mut self,
        data: &mut [f32],
        scratch: &mut [f32],
        time: &SourceTime,
        gain: f32,
    ) -> Result<(), String> {
        let mut invalid = false;
        self.voices.retain(|_, voice| {
            scratch.fill(0.0);
            let written = voice.sampler.write(scratch, time);
            if written > scratch.len() {
                invalid = true;
                return false;
            }
            for (dest, value) in data.iter_mut().zip(scratch.iter()) {
                *dest += value * gain;
            }
            written != 0
        });
        if invalid {
            return Err("Invalid step sampler frame count".into());
        }
        Ok(())
    }
}
