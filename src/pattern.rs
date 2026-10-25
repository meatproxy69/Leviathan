//! pattrns runs on the control side; only owned, typed frame events cross to audio.
use crate::{Event, Manifest, Result, TimedEvent, RATE, TAIL};
use pattrns::prelude::*;
use std::collections::BTreeMap;
pub const REVISION: &str = "8505bfe09111d4123a202af5222466f55021db9f";

/// Explicit naming policy: pattrns C4 (48) means scientific C4 / MIDI 60 here.
/// Reject unrepresentable pitches instead of saturating or wrapping.
pub fn midi_note(note: Note) -> Result<u8> {
    if !note.is_note_on() {
        return Err("OFF/EMPTY is not a pitch".into());
    }
    (note as u8)
        .checked_add(12)
        .filter(|n| *n <= 127)
        .ok_or_else(|| "pattrns pitch outside supported MIDI range after +12 mapping".into())
}

pub fn time_base(bpm: f64, bars: u32) -> Result<BeatTimeBase> {
    if !bpm.is_finite() || !(20.0..=400.0).contains(&bpm) {
        return Err("BPM must be finite and in 20..=400".into());
    }
    if !(1..=256).contains(&bars) {
        return Err("bars must be in 1..=256".into());
    }
    Ok(BeatTimeBase {
        beats_per_min: bpm as f32,
        beats_per_bar: 4,
        samples_per_sec: RATE,
    })
}

/// Bridge one pattern's note columns to deterministic IDs. A new note releases the
/// previous note in its column; EMPTY/None sustain; OFF releases. Delay is measured
/// against the actual pattrns event duration, then flattened events are stably sorted.
/// Instrument routing and parameter automation must be explicit future extensions.
pub fn bridge_pattern(pattern: &mut dyn Pattern, end: u64) -> Result<Vec<TimedEvent>> {
    let mut raw = Vec::new();
    let mut count = 0;
    while let Some(item) = pattern.run_until_time(end) {
        count += 1;
        if count > 100_000 {
            return Err("pattern exceeds event budget".into());
        }
        if item.time >= end {
            break;
        }
        match item.event {
            None => (),
            Some(pattrns::Event::ParameterChangeEvent(_)) => {
                return Err("parameter events are unsupported by this bridge".into())
            }
            Some(pattrns::Event::NoteEvents(notes)) => {
                if notes.len() > 8 {
                    return Err("pattern exceeds eight note columns".into());
                }
                for (column, note) in notes.into_iter().enumerate() {
                    if let Some(note) = note {
                        if note.instrument.is_some() {
                            return Err("instrument routing requires a separate part".into());
                        }
                        if !note.delay.is_finite()
                            || !(0.0..=1.0).contains(&note.delay)
                            || !note.volume.is_finite()
                            || !(0.0..=1.0).contains(&note.volume)
                            || !note.panning.is_finite()
                            || !(-1.0..=1.0).contains(&note.panning)
                        {
                            return Err("invalid pattrns note attributes".into());
                        }
                        if note.note == Note::EMPTY {
                            continue;
                        }
                        let frame = item
                            .time
                            .checked_add((note.delay as f64 * item.duration as f64).floor() as u64)
                            .ok_or("event time overflow")?;
                        if frame < end {
                            raw.push((frame, column, note));
                        }
                    }
                }
            }
        }
    }
    raw.sort_by_key(|(frame, _, _)| *frame);
    let mut active = BTreeMap::new();
    let mut next_id = 1;
    let mut events = Vec::new();
    for (frame, column, note) in raw {
        if let Some(id) = active.remove(&column) {
            events.push(TimedEvent {
                frame,
                event: Event::NoteOff { id },
            });
        }
        if note.note.is_note_on() {
            let midi_note = midi_note(note.note)?;
            let id = next_id;
            next_id += 1;
            active.insert(column, id);
            events.push(TimedEvent {
                frame,
                event: Event::NoteOn {
                    id,
                    midi_note,
                    velocity: note.volume,
                    pan: note.panning,
                },
            });
        }
    }
    // Explicitly release remaining IDs before the end-of-transport panic message.
    for (_, id) in active {
        events.push(TimedEvent {
            frame: end,
            event: Event::NoteOff { id },
        });
    }
    events.push(TimedEvent {
        frame: end,
        event: Event::AllNotesOff,
    });
    Ok(events)
}

pub fn manifest_from_pattern(pattern: &mut dyn Pattern, bpm: f64, bars: u32) -> Result<Manifest> {
    let time = time_base(bpm, bars)?;
    if *pattern.time_base() != time {
        return Err("pattern time base does not match manifest".into());
    }
    let exact_end = time.samples_per_bar() * bars as f64;
    let end = exact_end.round() as u64;
    // pattrns truncates timestamps: floor the exclusive pattern horizon so a
    // next-bar event at floor(exact_end) cannot leak into a rounded-up file.
    let cutoff = exact_end.floor() as u64;
    let mut events = bridge_pattern(pattern, cutoff)?;
    for event in &mut events {
        if event.frame == cutoff {
            event.frame = end;
        }
    }
    Ok(Manifest {
        schema: 1,
        sequencer: format!("pattrns 0.9.3 @ {REVISION}"),
        sample_rate: RATE,
        bpm: time.beats_per_min as f64,
        bars,
        musical_frames: end,
        tail_frames: TAIL,
        events,
    })
}

/// Real pattrns emitter: notes alternate with OFF on thirty-second-note steps.
pub fn pattern_manifest(bpm: f64, bars: u32) -> Result<Manifest> {
    let time = time_base(bpm, bars)?;
    let mut sequence = Vec::new();
    for (index, note) in [Note::C4, Note::E4, Note::G4, Note::B4]
        .into_iter()
        .enumerate()
    {
        sequence.push(new_note((
            note,
            None,
            0.5,
            if index % 2 == 0 { -0.5 } else { 0.5 },
        )));
        sequence.push(new_note(Note::OFF));
    }
    let mut pattern = time
        .every_nth_sixteenth(0.5)
        .emit(new_note_sequence_emitter(sequence));
    manifest_from_pattern(&mut pattern, bpm, bars)
}
