//! Resolve kit selectors after the shared row/phrase scheduler. No separate clock.
//! Voice IDs and column ownership remain those of the original event authority.
use super::{Command, Event, Manifest, Release, RenderLayerSelection, RenderPad, RenderRegion};
use crate::song::*;
use std::{collections::BTreeMap, path::Path};

const MAX_EVENTS: usize = 65536;
fn push(events: &mut Vec<Event>, event: Event) -> Result<(), String> {
    if events.len() >= MAX_EVENTS {
        return Err("Song exceeds 65536 events including drum releases".into());
    }
    events.push(event);
    Ok(())
}
pub(super) fn resolve(
    song: &Song,
    base: Option<&Path>,
    manifest: &mut Manifest,
    check_paths: bool,
) -> Result<(), String> {
    let mut counters: BTreeMap<(InstrumentId, PadId, u8), u32> = BTreeMap::new();
    let mute_schedule = super::arrangement::MuteSchedule::new(song, crate::RATE);
    // Resolve per-voice source metadata, never alias multiple pad sources under
    // the kit's InstrumentId. Unused kit definitions leave the old path untouched.
    for event in &manifest.events {
        let Command::On {
            voice,
            pitch,
            volume,
            ..
        } = event.command
        else {
            continue;
        };
        let InstrumentKind::DrumKit { kit } = song.instruments[&event.instrument].kind else {
            continue;
        };
        let (pad_id, pad) = song.drum_kits[&kit]
            .pads
            .iter()
            .find(|(_, pad)| pad.note == pitch)
            .ok_or_else(|| {
                format!(
                    "Kit instrument {} has no pad for selector {pitch}",
                    event.instrument.0
                )
            })?;
        let (region_id, layer_selection) = if pad.velocity_zones.is_empty() {
            (
                pad.region
                    .ok_or("Triggered drum pad has no region assigned")?,
                None,
            )
        } else {
            // Do not consume a choice or run pad policies for a cancelled spill
            // rounded onto a mute boundary. Normal root/phrase compilation also
            // filters these before latching column state.
            if mute_schedule.contains(event.track, event.frame) {
                continue;
            }
            if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
                return Err(
                    "Velocity-zone selection requires finite pre-pad ON velocity in 0..1".into(),
                );
            }
            // f32->f64 is exact, as is multiplication by integer127. Rounding
            // nonnegative ties upward only selects a bin; gain stays unchanged.
            let velocity_bin = (f64::from(volume) * 127.0).round() as u8;
            let zone = pad
                .velocity_zones
                .iter()
                .find(|z| z.min_velocity <= velocity_bin && velocity_bin <= z.max_velocity)
                .ok_or("No velocity zone covers this trigger")?;
            let ordinal = counters
                .entry((event.instrument, *pad_id, zone.min_velocity))
                .or_default();
            let index = *ordinal as usize % zone.alternatives.len();
            let layer = zone.alternatives[index];
            let selection = RenderLayerSelection {
                layer: layer.id,
                input_velocity: volume,
                velocity_bin,
                min_velocity: zone.min_velocity,
                max_velocity: zone.max_velocity,
                round_robin_index: index as u8,
                round_robin_ordinal: *ordinal,
            };
            *ordinal = ordinal
                .checked_add(1)
                .ok_or("Round-robin ordinal overflow")?;
            (layer.region, Some(selection))
        };
        let region = &song.regions[&region_id];
        let sample = &song.samples[&region.sample];
        let path = if !check_paths {
            sample.path.clone()
        } else {
            let path = if sample.path.is_absolute() {
                sample.path.clone()
            } else {
                base.ok_or("Relative drum source requires a saved project directory")?
                    .join(&sample.path)
            }
            .canonicalize()
            .map_err(|e| format!("Drum pad source: {e}"))?;
            if !path.is_file() {
                return Err("Drum pad source must be a regular file".into());
            }
            path
        };
        manifest.pad_voices.insert(
            voice,
            RenderPad {
                instrument: event.instrument,
                kit,
                pad: *pad_id,
                slot: pad.slot,
                note: pad.note,
                gain: pad.gain,
                pan: pad.pan,
                mode: pad.mode,
                new_note: pad.new_note,
                choke_group: pad.choke_group,
                tune_semitones: pad.tune_semitones,
                path,
                region: RenderRegion {
                    region: region_id,
                    sample: region.sample,
                    sha256: sample.sha256.clone(),
                    sample_rate: sample.sample_rate,
                    channels: sample.channels,
                    frames: sample.frames,
                    start_frame: region.start_frame,
                    end_frame: region.end_frame,
                    root_note: pad.root_note,
                    amp_envelope: region.amp_envelope,
                    forward_loop: region.forward_loop,
                },
                layer_selection,
            },
        );
    }
    if manifest.pad_voices.is_empty() {
        return Ok(());
    }
    let mut events = Vec::with_capacity(manifest.events.len());
    // Already released tails are not re-released. Natural exhaustion needs no
    // DSP polling: an extra release on an exhausted logical voice is harmless.
    let mut held: BTreeMap<usize, Event> = BTreeMap::new();
    for mut event in std::mem::take(&mut manifest.events) {
        let voice = event.command.voice();
        let Some(pad) = manifest.pad_voices.get(&voice) else {
            push(&mut events, event)?;
            continue;
        };
        match &mut event.command {
            Command::On {
                pitch, volume, pan, ..
            } => {
                let stops: Vec<_> = held
                    .iter()
                    .filter_map(|(&old, origin)| {
                        let previous = &manifest.pad_voices[&old];
                        if previous.instrument != pad.instrument {
                            return None;
                        }
                        let reason = if pad.choke_group.is_some()
                            && pad.choke_group == previous.choke_group
                        {
                            Release::Choke
                        } else if pad.pad == previous.pad
                            && pad.new_note == PadNewNote::ReleasePrevious
                        {
                            Release::PadReplaced
                        } else {
                            return None;
                        };
                        Some((old, origin.clone(), reason))
                    })
                    .collect();
                for (old, mut stop, reason) in stops {
                    held.remove(&old);
                    stop.frame = event.frame;
                    stop.position = event.position;
                    if let Some(phrase) = &mut stop.phrase {
                        phrase.cycle = None;
                        phrase.row = None;
                    }
                    stop.command = Command::Off { voice: old, reason };
                    push(&mut events, stop)?;
                }
                *pitch = (i16::from(pad.region.root_note) + i16::from(pad.tune_semitones)) as u8;
                *volume *= pad.gain;
                *pan = (*pan + pad.pan).clamp(-1.0, 1.0);
                held.insert(voice, event.clone());
            }
            Command::Off { reason, .. } => {
                let owner_stop = event
                    .phrase
                    .is_some_and(|p| p.cycle.is_none() && p.row.is_none());
                let replacement = matches!(reason, Release::Replaced | Release::Retrigger);
                if (replacement && !owner_stop)
                    || (pad.mode == PadMode::OneShot
                        && (replacement
                            || matches!(reason, Release::ExplicitOff | Release::PhraseEnd)))
                {
                    continue;
                }
                if held.remove(&voice).is_none() {
                    continue;
                }
            }
            Command::Cut { .. } => {
                held.remove(&voice);
            }
            Command::Volume { value, .. } => *value *= pad.gain,
            Command::Pan { value, .. } => *value = (*value + pad.pan).clamp(-1.0, 1.0),
            _ => (),
        }
        push(&mut events, event)?;
    }
    // One-shot voices may no longer be the column's active pointer after an OFF
    // or another pad. Explicitly release all remaining kit-owned voices at end.
    for (voice, mut event) in held {
        event.frame = manifest.musical_frames;
        event.position = None;
        if let Some(phrase) = &mut event.phrase {
            phrase.cycle = None;
            phrase.row = None;
        }
        event.command = Command::Off {
            voice,
            reason: Release::SongEnd,
        };
        push(&mut events, event)?;
    }
    manifest.events = events;
    manifest.schema = 6;
    manifest.evaluator = "leviathan-song-v6".into();
    Ok(())
}
