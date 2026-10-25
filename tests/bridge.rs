use aqua_pattern_core::{pattern::*, Event, RATE};
use pattrns::prelude::{
    new_note, new_note_sequence_emitter, new_polyphonic_note_sequence_emitter, Note, NoteEvent,
    Pattern,
};
#[test]
fn pitch_mapping_and_range_are_explicit() {
    assert_eq!(Note::C4 as u8, 48);
    assert_eq!(midi_note(Note::C4).unwrap(), 60);
    assert_eq!(midi_note(Note::A4).unwrap(), 69);
    assert_eq!(midi_note(Note::C0).unwrap(), 12);
    for n in [Note::OFF, Note::EMPTY, Note::G10] {
        assert!(midi_note(n).is_err());
    }
}
#[test]
fn real_pattern_retrigger_rest_off_polyphony_and_reset() {
    let time = time_base(120.0, 1).unwrap();
    let mut p = time
        .every_nth_sixteenth(1.0)
        .emit(new_polyphonic_note_sequence_emitter(vec![
            vec![new_note(Note::C4), new_note(Note::E4)],
            vec![None, new_note(Note::OFF)],
            vec![new_note(Note::G4), new_note(Note::EMPTY)],
            vec![new_note(Note::OFF), None],
        ]));
    let events = bridge_pattern(&mut p, 24000).unwrap();
    assert_eq!(events.len(), 7);
    assert!(matches!(
        events[0].event,
        Event::NoteOn {
            id: 1,
            midi_note: 60,
            ..
        }
    ));
    assert!(matches!(
        events[1].event,
        Event::NoteOn {
            id: 2,
            midi_note: 64,
            ..
        }
    ));
    assert_eq!(events[2].frame, 6000);
    assert_eq!(events[2].event, Event::NoteOff { id: 2 });
    assert_eq!(events[3].frame, 12000);
    assert_eq!(events[3].event, Event::NoteOff { id: 1 });
    assert_eq!(events[4].frame, 12000);
    assert!(matches!(
        events[4].event,
        Event::NoteOn {
            id: 3,
            midi_note: 67,
            ..
        }
    ));
    assert_eq!(events[5].frame, 18000);
    assert_eq!(events[5].event, Event::NoteOff { id: 3 });
    assert_eq!(events[6].event, Event::AllNotesOff);
    p.reset();
    assert_eq!(events, bridge_pattern(&mut p, 24000).unwrap());
}
#[test]
fn real_delays_are_sorted_and_note_attributes_preserved() {
    let time = time_base(120.0, 1).unwrap();
    let mut p = time
        .every_nth_sixteenth(1.0)
        .emit(new_polyphonic_note_sequence_emitter(vec![
            vec![
                new_note((Note::C4, None, 0.3, -0.75, 0.5)),
                new_note((Note::E4, None, 0.4, 0.75, 0.0)),
            ],
            vec![new_note(Note::OFF), new_note(Note::OFF)],
        ]));
    let events = bridge_pattern(&mut p, 12000).unwrap();
    assert_eq!(events[0].frame, 0);
    assert_eq!(events[1].frame, 3000);
    assert_eq!(
        events[1].event,
        Event::NoteOn {
            id: 2,
            midi_note: 60,
            velocity: 0.3,
            pan: -0.75
        }
    );
    assert!(events.windows(2).all(|w| w[0].frame <= w[1].frame));
}
#[test]
fn fractional_timing_comes_from_pattrns_with_subframe_quantization() {
    let bpm = 137.125;
    let m = pattern_manifest(bpm, 16).unwrap();
    let ons: Vec<_> = m
        .events
        .iter()
        .filter(|e| matches!(e.event, Event::NoteOn { .. }))
        .collect();
    assert_eq!(ons.len(), 256);
    for (step, event) in ons.iter().enumerate() {
        let exact = step as f64 * RATE as f64 * 60.0 / m.bpm / 4.0;
        assert!((event.frame as f64 - exact).abs() < 1.01);
    }
    assert!(m.sequencer.contains(REVISION));
}
#[test]
fn invalid_bridge_values_and_time_base_are_rejected() {
    let time = time_base(120.0, 1).unwrap();
    for note in [
        NoteEvent {
            note: Note::C4,
            instrument: None,
            volume: f32::NAN,
            panning: 0.0,
            delay: 0.0,
        },
        NoteEvent {
            note: Note::C4,
            instrument: Some(1.into()),
            volume: 0.5,
            panning: 0.0,
            delay: 0.0,
        },
        NoteEvent {
            note: Note::C4,
            instrument: None,
            volume: 0.5,
            panning: 0.0,
            delay: 1.1,
        },
    ] {
        let mut p = time
            .every_nth_beat(1.0)
            .emit(new_note_sequence_emitter(vec![Some(note)]));
        assert!(bridge_pattern(&mut p, 24000).is_err());
    }
    let mut p = time
        .every_nth_beat(1.0)
        .emit(new_note_sequence_emitter(vec![new_note(Note::C4)]));
    assert!(manifest_from_pattern(&mut p, 130.0, 1).is_err());
}

#[test]
fn rounded_transport_end_does_not_admit_next_bar() {
    for bpm in [121.0, 123.0, 127.5, 137.0, 137.125, 199.75] {
        let m = pattern_manifest(bpm, 2).unwrap();
        assert_eq!(
            m.events
                .iter()
                .filter(|e| matches!(e.event, Event::NoteOn { .. }))
                .count(),
            32,
            "BPM {bpm}"
        );
        assert_eq!(m.events.last().unwrap().frame, m.musical_frames);
        assert_eq!(m.events.last().unwrap().event, Event::AllNotesOff);
    }
}
