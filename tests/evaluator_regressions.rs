use aqua_pattern_core::{
    song::*,
    song_render::{self, Command, Engine, Event, Release},
    RATE,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    dir: PathBuf,
    song: Song,
}
impl Fixture {
    fn new(rows: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-evaluator-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let sample = dir.join("quiet.wav");
        let mut writer = hound::WavWriter::create(
            &sample,
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..RATE * 2 {
            writer.write_sample(0.001f32).unwrap();
        }
        writer.finalize().unwrap();
        let mut song = example_song();
        song.order = vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }];
        song.patterns.clear();
        song.patterns.insert(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "A".into(),
                rows,
                tracks: BTreeMap::new(),
            },
        );
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some(sample.clone());
        }
        Self { dir, song }
    }
    fn put(&mut self, pattern: u64, track: u64, row: u32, column: u16, cell: Cell) {
        self.song
            .patterns
            .get_mut(&PatternId(pattern))
            .unwrap()
            .tracks
            .entry(TrackId(track))
            .or_default()
            .entry(row)
            .or_default()
            .insert(column, cell);
    }
    fn evaluate(&self) -> song_render::Manifest {
        song_render::evaluate(&self.song, None).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn note(pitch: u8, instrument: u64) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(instrument)),
        ..Cell::default()
    }
}
fn render(engine: &mut Engine, frames_per_block: usize) -> Vec<f32> {
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut output = Vec::new();
    let mut block = vec![0.0; frames_per_block * 2];
    while engine.position < end {
        let count = engine.block(&mut block, end).unwrap();
        assert!(count > 0 && count <= frames_per_block.min(512));
        output.extend_from_slice(&block[..count * 2]);
    }
    output
}
// Frame, track, column, instrument, voice, pitch, volume, pan.
type OnSummary = (u64, u64, u16, u64, usize, u8, f32, f32);
fn ons(events: &[Event]) -> Vec<OnSummary> {
    events
        .iter()
        .filter_map(|event| match event.command {
            Command::On {
                voice,
                pitch,
                volume,
                pan,
            } => Some((
                event.frame,
                event.track.0,
                event.column,
                event.instrument.0,
                voice,
                pitch,
                volume,
                pan,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn dense_same_frame_chords_and_controls_do_not_overflow_sampler_queue() {
    let mut f = Fixture::new(3);
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for column in 0..256 {
        f.put(1, 1, 0, column, note(60, 1));
        f.put(
            1,
            1,
            1,
            column,
            Cell {
                velocity: Some(0.5),
                pan: Some(0.25),
                ..Cell::default()
            },
        );
        f.put(
            1,
            1,
            2,
            column,
            Cell {
                note: Note::Off,
                ..Cell::default()
            },
        );
    }
    let mut engine = Engine::new(&f.song, None).unwrap();
    let expected_frames = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let output = render(&mut engine, 512);
    assert_eq!(output.len(), expected_frames as usize * 2);
    assert!(output
        .iter()
        .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
    assert!(output.iter().any(|sample| *sample > 0.01));
    assert!(output[output.len() - 512..]
        .iter()
        .all(|sample| *sample == 0.0));
    engine.cold_seek(0).unwrap();
    assert_eq!(output, render(&mut engine, 512));
}

#[test]
fn selection_and_controls_remain_per_column_and_address_the_old_active_instrument() {
    let mut f = Fixture::new(5);
    f.put(
        1,
        1,
        0,
        0,
        Cell {
            velocity: Some(0.2),
            pan: Some(-0.5),
            ..note(60, 1)
        },
    );
    f.put(
        1,
        1,
        0,
        1,
        Cell {
            velocity: Some(0.3),
            pan: Some(0.5),
            ..note(72, 2)
        },
    );
    f.put(
        1,
        2,
        0,
        0,
        Cell {
            velocity: Some(0.4),
            ..note(48, 1)
        },
    );
    f.put(
        1,
        1,
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(3)),
            velocity: Some(0.0),
            pan: Some(1.0),
            ..Cell::default()
        },
    );
    f.put(
        1,
        1,
        2,
        0,
        Cell {
            note: Note::Note(64),
            ..Cell::default()
        },
    );
    f.put(
        1,
        1,
        3,
        0,
        Cell {
            note: Note::Off,
            instrument: Some(InstrumentId(2)),
            velocity: Some(0.6),
            pan: Some(-0.25),
            ..Cell::default()
        },
    );
    f.put(
        1,
        1,
        4,
        0,
        Cell {
            note: Note::Note(65),
            ..Cell::default()
        },
    );
    let m = f.evaluate();
    assert_eq!(
        ons(&m.events),
        [
            (0, 1, 0, 1, 1, 60, 0.2, -0.5),
            (0, 1, 1, 2, 2, 72, 0.3, 0.5),
            (0, 2, 0, 1, 3, 48, 0.4, 0.0),
            (12000, 1, 0, 3, 4, 64, 0.0, 1.0),
            (24000, 1, 0, 2, 5, 65, 0.6, -0.25),
        ]
    );
    let controls: Vec<_> = m
        .events
        .iter()
        .filter(|event| event.frame == 6000)
        .collect();
    assert_eq!(controls.len(), 2);
    assert!(controls
        .iter()
        .all(|event| event.instrument == InstrumentId(1)
            && event.track == TrackId(1)
            && event.column == 0));
    assert_eq!(
        controls[0].command,
        Command::Volume {
            voice: 1,
            value: 0.0
        }
    );
    assert_eq!(
        controls[1].command,
        Command::Pan {
            voice: 1,
            value: 1.0
        }
    );
    let off_row: Vec<_> = m
        .events
        .iter()
        .filter(|event| event.frame == 18000)
        .collect();
    assert_eq!(off_row.len(), 3);
    assert!(off_row
        .iter()
        .all(|event| event.instrument == InstrumentId(3)));
    assert_eq!(
        off_row[0].command,
        Command::Volume {
            voice: 4,
            value: 0.6
        }
    );
    assert_eq!(
        off_row[1].command,
        Command::Pan {
            voice: 4,
            value: -0.25
        }
    );
    assert_eq!(
        off_row[2].command,
        Command::Off {
            voice: 4,
            reason: Release::ExplicitOff
        }
    );
    let ending_voices: Vec<_> = m
        .events
        .iter()
        .filter_map(|event| match event.command {
            Command::Off {
                voice,
                reason: Release::SongEnd,
            } => Some(voice),
            _ => None,
        })
        .collect();
    assert_eq!(ending_voices, [5, 2, 3]);
}

#[test]
fn neither_another_track_nor_another_column_can_supply_a_missing_selection() {
    for (track, column) in [(1, 1), (2, 0)] {
        let mut f = Fixture::new(2);
        f.put(1, 1, 0, 0, note(60, 1));
        f.put(
            1,
            track,
            1,
            column,
            Cell {
                note: Note::Note(64),
                ..Cell::default()
            },
        );
        let error = song_render::evaluate(&f.song, None).unwrap_err();
        assert!(error.contains("no selected instrument"), "{error}");
        assert!(
            error.contains(&format!("track {track}, row 1, column {column}")),
            "{error}"
        );
    }
}

#[test]
fn instrument_only_selection_does_not_reassign_or_require_assets_for_the_active_voice() {
    let mut f = Fixture::new(3);
    f.put(1, 1, 0, 0, note(60, 1));
    f.put(
        1,
        1,
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..Cell::default()
        },
    );
    f.put(
        1,
        1,
        2,
        0,
        Cell {
            velocity: Some(0.25),
            pan: Some(-1.0),
            ..Cell::default()
        },
    );
    f.song.instruments.get_mut(&InstrumentId(2)).unwrap().sample = None;
    let m = f.evaluate();
    assert_eq!(
        m.assets.keys().copied().collect::<Vec<_>>(),
        [InstrumentId(1)]
    );
    assert!(m
        .events
        .iter()
        .all(|event| event.instrument == InstrumentId(1)));
    assert!(!m.events.iter().any(|event| event.frame == 6000));
    assert_eq!(
        m.events.last().unwrap().command,
        Command::Off {
            voice: 1,
            reason: Release::SongEnd
        }
    );
}

#[test]
fn repeated_and_cloned_occurrences_keep_column_memory_without_sharing_content() {
    let mut f = Fixture::new(2);
    f.put(
        1,
        1,
        0,
        0,
        Cell {
            velocity: Some(0.2),
            ..note(60, 1)
        },
    );
    f.song.clone_pattern(PatternId(1), PatternId(2)).unwrap();
    f.put(
        2,
        1,
        0,
        0,
        Cell {
            note: Note::Note(67),
            ..Cell::default()
        },
    );
    f.put(
        2,
        1,
        1,
        0,
        Cell {
            velocity: Some(0.7),
            pan: Some(-0.5),
            ..Cell::default()
        },
    );
    f.song.append_repeat(PatternId(2)).unwrap();
    f.song.append_repeat(PatternId(1)).unwrap();
    let m = f.evaluate();
    assert_eq!(
        ons(&m.events),
        [
            (0, 1, 0, 1, 1, 60, 0.2, 0.0),
            (12000, 1, 0, 1, 2, 67, 0.2, 0.0),
            (24000, 1, 0, 1, 3, 60, 0.2, -0.5),
        ]
    );
    let off: Vec<_> = m
        .events
        .iter()
        .filter_map(|event| match &event.command {
            Command::Off { voice, reason } => Some((event.frame, *voice, reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        off,
        [
            (12000, 1, Release::Replaced),
            (24000, 2, Release::Replaced),
            (36000, 3, Release::SongEnd)
        ]
    );
    f.put(1, 1, 0, 0, note(62, 1));
    let updated = f.evaluate();
    assert_eq!(
        ons(&updated.events)
            .iter()
            .map(|on| on.5)
            .collect::<Vec<_>>(),
        [62, 67, 62]
    );
}

#[test]
fn rounded_late_off_precedes_the_next_occurrence_on_and_last_on_precedes_song_end() {
    let mut f = Fixture::new(2);
    f.song.bpm = 123.45;
    f.song.lines_per_beat = 7;
    f.put(1, 1, 0, 0, note(60, 1));
    f.put(
        1,
        1,
        1,
        0,
        Cell {
            note: Note::Off,
            delay: Some(0.999999999999),
            ..Cell::default()
        },
    );
    f.song.clone_pattern(PatternId(1), PatternId(2)).unwrap();
    f.put(2, 1, 0, 0, note(64, 1));
    f.put(
        2,
        1,
        1,
        0,
        Cell {
            delay: Some(0.999999999999),
            ..note(67, 1)
        },
    );
    f.song.append_repeat(PatternId(2)).unwrap();
    let m = f.evaluate();
    let per_row = RATE as f64 * 60.0 / (123.45 * 7.0);
    let boundary = (2.0 * per_row).round() as u64;
    let at_boundary: Vec<_> = m
        .events
        .iter()
        .filter(|event| event.frame == boundary)
        .collect();
    assert_eq!(at_boundary.len(), 2);
    assert_eq!(at_boundary[0].position, Some((0, 1)));
    assert_eq!(
        at_boundary[0].command,
        Command::Off {
            voice: 1,
            reason: Release::ExplicitOff
        }
    );
    assert_eq!(at_boundary[1].position, Some((1, 0)));
    assert!(matches!(
        at_boundary[1].command,
        Command::On { voice: 2, .. }
    ));
    assert_eq!(m.musical_frames, (4.0 * per_row).round() as u64);
    let last_on = m
        .events
        .iter()
        .find(|event| matches!(event.command, Command::On { voice: 3, .. }))
        .unwrap();
    assert_eq!(last_on.frame, m.musical_frames - 1);
    assert_eq!(m.events.last().unwrap().frame, m.musical_frames);
    assert!(m
        .events
        .windows(2)
        .all(|pair| pair[0].frame <= pair[1].frame));
    assert_eq!(m, f.evaluate());
}

#[test]
fn same_frame_ties_follow_displayed_track_order_then_column_order() {
    let mut f = Fixture::new(2);
    f.song.track_order.reverse();
    for track in 1..=4 {
        for column in 0..2 {
            f.put(1, track, 0, column, note(60 + column as u8, track));
            f.put(1, track, 1, column, note(64 + column as u8, track));
        }
    }
    let m = f.evaluate();
    let first: Vec<_> = m
        .events
        .iter()
        .filter(|event| event.frame == 0)
        .map(|event| (event.track.0, event.column))
        .collect();
    assert_eq!(
        first,
        [
            (4, 0),
            (4, 1),
            (3, 0),
            (3, 1),
            (2, 0),
            (2, 1),
            (1, 0),
            (1, 1)
        ]
    );
    let replacements: Vec<_> = m
        .events
        .iter()
        .filter(|event| event.frame == 6000)
        .collect();
    assert_eq!(replacements.len(), 16);
    for (events, expected) in replacements.as_chunks::<2>().0.iter().zip(first) {
        assert_eq!((events[0].track.0, events[0].column), expected);
        assert_eq!((events[1].track.0, events[1].column), expected);
        assert!(matches!(
            events[0].command,
            Command::Off {
                reason: Release::Replaced,
                ..
            }
        ));
        assert!(matches!(events[1].command, Command::On { .. }));
    }
}

#[test]
fn fractional_row_positions_do_not_accumulate_per_row_rounding_error() {
    let mut f = Fixture::new(4096);
    f.song.bpm = 399.99;
    f.song.lines_per_beat = 256;
    f.put(1, 1, 0, 0, note(60, 1));
    f.put(
        1,
        1,
        4095,
        0,
        Cell {
            note: Note::Off,
            delay: Some(0.75),
            ..Cell::default()
        },
    );
    let m = f.evaluate();
    let per_row = RATE as f64 * 60.0 / (399.99 * 256.0);
    assert_eq!(m.musical_frames, (4096.0 * per_row).round() as u64);
    assert_ne!(m.musical_frames, 4096 * per_row.round() as u64);
    let off = m.events.last().unwrap();
    assert_eq!(off.frame, (4095.75 * per_row).round() as u64);
    assert_eq!(
        off.command,
        Command::Off {
            voice: 1,
            reason: Release::ExplicitOff
        }
    );
}

#[test]
fn maximum_trigger_count_handles_dense_replacements_and_song_end() {
    let mut f = Fixture::new(2);
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    for track in f.song.tracks.values_mut() {
        track.note_columns = 256;
    }
    for track in 1..=4 {
        for column in 0..256 {
            f.put(
                1,
                track,
                0,
                column,
                Cell {
                    velocity: Some(0.1),
                    ..note(60, 1)
                },
            );
            f.put(
                1,
                track,
                1,
                column,
                Cell {
                    note: Note::Note(64),
                    ..Cell::default()
                },
            );
        }
    }
    let mut engine = Engine::new(&f.song, None).unwrap();
    assert_eq!(ons(&engine.manifest.events).len(), 2048);
    assert_eq!(
        engine
            .manifest
            .events
            .iter()
            .filter(|event| matches!(
                event.command,
                Command::Off {
                    reason: Release::Replaced,
                    ..
                }
            ))
            .count(),
        1024
    );
    assert_eq!(
        engine
            .manifest
            .events
            .iter()
            .filter(|event| matches!(
                event.command,
                Command::Off {
                    reason: Release::SongEnd,
                    ..
                }
            ))
            .count(),
        1024
    );
    let output = render(&mut engine, 512);
    assert!(output
        .iter()
        .all(|sample| sample.is_finite() && sample.abs() < 1.0));
    assert!(output.iter().any(|sample| *sample > 0.01));
    assert!(output[output.len() - 512..]
        .iter()
        .all(|sample| *sample == 0.0));
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 3;
    f.put(1, 1, 2, 0, note(67, 1));
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("2048 note triggers"));
}

#[test]
fn dense_queue_drains_do_not_advance_audio_relative_to_small_independent_batches() {
    let mut f = Fixture::new(4);
    // A rapidly changing waveform makes even one accidentally consumed frame
    // distinguishable from harmless floating-point regrouping error.
    let mut writer = hound::WavWriter::create(
        f.song.instruments[&InstrumentId(1)]
            .sample
            .as_ref()
            .unwrap(),
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..RATE {
        let sample = if frame % 4 < 2 { 0.001f32 } else { -0.001 };
        writer.write_sample(sample).unwrap();
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    f.song.bpm = 321.23;
    f.song.lines_per_beat = 128;
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for column in 0..256 {
        f.put(
            1,
            1,
            0,
            column,
            Cell {
                velocity: Some(0.25),
                ..note(60, 1)
            },
        );
        f.put(
            1,
            1,
            1,
            column,
            Cell {
                velocity: Some(0.7),
                pan: Some(-0.2),
                ..Cell::default()
            },
        );
        f.put(
            1,
            1,
            2,
            column,
            Cell {
                note: Note::Note(64),
                ..Cell::default()
            },
        );
        f.put(
            1,
            1,
            3,
            column,
            Cell {
                note: Note::Off,
                velocity: Some(0.15),
                pan: Some(0.4),
                ..Cell::default()
            },
        );
    }
    let dense = render(&mut Engine::new(&f.song, None).unwrap(), 512);
    // Four columns per otherwise identical instrument means at most twelve
    // messages per sampler/frame, below even phonic's minimum queue capacity.
    // This is a no-drain reference, not merely a second run of the dense path.
    let instrument = f.song.instruments[&InstrumentId(1)].clone();
    for id in 1..=64 {
        f.song
            .instruments
            .insert(InstrumentId(id), instrument.clone());
    }
    for columns in f
        .song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .values_mut()
    {
        for (column, cell) in columns {
            if cell.instrument.is_some() {
                cell.instrument = Some(InstrumentId(u64::from(*column / 4) + 1));
            }
        }
    }
    let split = render(&mut Engine::new(&f.song, None).unwrap(), 512);
    assert_eq!(dense.len(), split.len());
    let max_error = dense
        .iter()
        .zip(&split)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let one_frame_shift_error = dense
        .iter()
        .skip(2)
        .zip(&split)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(one_frame_shift_error > 0.01);
    // Up to 512 overlapping voices are regrouped into 64 f32 sums. Permit
    // accumulated rounding, far below the verified one-frame shift error.
    assert!(max_error < 0.000008, "dense/split PCM error {max_error}");
}
