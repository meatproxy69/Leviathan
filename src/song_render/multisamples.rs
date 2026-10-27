//! Resolve one melodic source per actual ON after the common row/phrase clock.
//! Source choice is immutable for that voice; later gain/glide never remaps it.
use super::{Command, Manifest, RenderMultisample, RenderRegion};
use crate::song::*;
use std::path::Path;
pub(super) fn resolve(
    song: &Song,
    base: Option<&Path>,
    manifest: &mut Manifest,
    check_paths: bool,
) -> Result<(), String> {
    let mutes = super::arrangement::MuteSchedule::new(song, crate::RATE);
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
        let InstrumentKind::Multisample { map } = song.instruments[&event.instrument].kind else {
            continue;
        };
        if mutes.contains(event.track, event.frame) {
            continue;
        }
        let (zone_id, zone, bin) = song
            .select_keyzone(map, pitch, volume)
            .map_err(|e| format!("Instrument {} voice {voice}: {e}", event.instrument.0))?;
        let region = &song.regions[&zone.region];
        let sample = &song.samples[&region.sample];
        let path = if !check_paths {
            sample.path.clone()
        } else {
            let path = if sample.path.is_absolute() {
                sample.path.clone()
            } else {
                base.ok_or("Relative multisample source requires a saved project directory")?
                    .join(&sample.path)
            }
            .canonicalize()
            .map_err(|e| format!("Multisample source: {e}"))?;
            if !path.is_file() {
                return Err("Multisample source must be a regular file".into());
            }
            path
        };
        manifest.multisample_voices.insert(
            voice,
            RenderMultisample {
                instrument: event.instrument,
                map,
                zone: zone_id,
                note: pitch,
                input_velocity: volume,
                velocity_bin: bin,
                note_min: zone.note_min,
                note_max: zone.note_max,
                velocity_min: zone.velocity_min,
                velocity_max: zone.velocity_max,
                path,
                region: RenderRegion {
                    region: zone.region,
                    sample: region.sample,
                    sha256: sample.sha256.clone(),
                    sample_rate: sample.sample_rate,
                    channels: sample.channels,
                    frames: sample.frames,
                    start_frame: region.start_frame,
                    end_frame: region.end_frame,
                    root_note: zone.root_note,
                    amp_envelope: region.amp_envelope,
                    forward_loop: region.forward_loop,
                },
            },
        );
    }
    Ok(())
}
