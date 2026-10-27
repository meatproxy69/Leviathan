use aqua_pattern_core::{
    song::*,
    song_render::{self, Command, Engine},
    transport::{Producer, Shared},
    RATE,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

thread_local! { static CALLBACK_ALLOCS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }
struct CountingAllocator;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
fn count_alloc() {
    let _ = CALLBACK_ALLOCS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_alloc();
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_alloc();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        count_alloc();
        unsafe { std::alloc::System.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(pointer, layout) }
    }
}
fn callback(shared: &Shared, output: &mut [f32]) {
    CALLBACK_ALLOCS.with(|count| count.set(Some(0)));
    shared.fill(output);
    let allocations = CALLBACK_ALLOCS.with(|count| count.replace(None).unwrap());
    assert_eq!(
        allocations, 0,
        "advanced effects allocated in audio callback"
    );
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    song: Song,
}
impl Fixture {
    fn new(rows: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-sample-control-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut song = example_song();
        song.bpm = 400.0;
        song.lines_per_beat = 120; // Exactly 60 output frames per row.
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "Controls".into(),
                rows,
                tracks: BTreeMap::new(),
            },
        )]);
        song.order = vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }];
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some(dir.join("sample.wav"));
        }
        let f = Self { dir, song };
        f.sample(RATE, 2, 512, source);
        f
    }
    fn sample(&self, rate: u32, channels: u16, frames: u32, value: impl Fn(u32, u16) -> f32) {
        let mut writer = hound::WavWriter::create(
            self.dir.join("sample.wav"),
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..frames {
            for channel in 0..channels {
                writer.write_sample(value(frame, channel)).unwrap();
            }
        }
        writer.finalize().unwrap();
    }
    fn put(&mut self, row: u32, column: u16, cell: Cell) {
        self.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(row)
            .or_default()
            .insert(column, cell);
    }
    fn render(&self) -> Vec<f32> {
        render(&mut Engine::new(&self.song, None).unwrap(), 512)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn source(frame: u32, channel: u16) -> f32 {
    if channel == 0 {
        0.01 + frame as f32 * 0.0001
    } else {
        -0.02 - frame as f32 * 0.0002
    }
}
fn note() -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity: Some(1.0),
        ..Default::default()
    }
}
fn render(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut result = Vec::new();
    let mut block = vec![0.0; frames * 2];
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    while engine.position < end {
        let count = engine.block(&mut block, end).unwrap();
        result.extend_from_slice(&block[..count * 2]);
    }
    result
}
fn assert_frame(pcm: &[f32], frame: usize, expected: [f32; 2]) {
    for channel in 0..2 {
        assert!(
            (pcm[frame * 2 + channel] - expected[channel]).abs() < 0.0000001,
            "frame {frame}, channel {channel}: {} != {}",
            pcm[frame * 2 + channel],
            expected[channel]
        );
    }
}
fn interpolated(position: f64, channel: u16, frames: u32) -> f32 {
    let lower = position.floor() as u32;
    let fraction = (position - f64::from(lower)) as f32;
    let a = source(lower, channel);
    let b = if lower + 1 < frames {
        source(lower + 1, channel)
    } else {
        0.0
    };
    (a + (b - a) * fraction) * 0.5
}

#[test]
fn offset_and_reverse_use_real_source_frames_without_guard_or_stereo_swap() {
    for channels in [1, 2] {
        for (offset, direction, positions) in [
            (Some(3), Direction::Forward, vec![3, 4, 5, 6, 7]),
            (None, Direction::Reverse, vec![7, 6, 5, 4, 3, 2, 1, 0]),
            (Some(3), Direction::Reverse, vec![3, 2, 1, 0]),
        ] {
            let mut f = Fixture::new(2);
            f.sample(RATE, channels, 8, source);
            f.put(
                0,
                0,
                Cell {
                    effects: StepEffects {
                        offset,
                        direction: Some(direction),
                        ..Default::default()
                    },
                    ..note()
                },
            );
            let manifest = song_render::evaluate(&f.song, None).unwrap();
            assert_eq!(manifest.schema, 3);
            let pcm = f.render();
            for (output, input) in positions.iter().enumerate() {
                assert_frame(
                    &pcm,
                    output,
                    [
                        source(*input, 0) * 0.5,
                        source(*input, if channels == 1 { 0 } else { 1 }) * 0.5,
                    ],
                );
            }
            assert!(pcm[positions.len() * 2..]
                .iter()
                .all(|sample| *sample == 0.0));
        }
    }
}

#[test]
fn source_rate_and_pitch_use_fractional_linear_interpolation() {
    for rate in [24000, 44100, 96000] {
        for pitch in [48, 60, 72] {
            let mut f = Fixture::new(4);
            f.sample(rate, 2, 512, source);
            f.put(
                0,
                0,
                Cell {
                    note: Note::Note(pitch),
                    effects: StepEffects {
                        offset: Some(3),
                        ..Default::default()
                    },
                    ..note()
                },
            );
            let pcm = f.render();
            let step = phonic::utils::speed_from_note(pitch) * f64::from(rate) / f64::from(RATE);
            let mut position = 3.0;
            for frame in 0..100 {
                assert_frame(
                    &pcm,
                    frame,
                    [
                        interpolated(position, 0, 512),
                        interpolated(position, 1, 512),
                    ],
                );
                position += step;
            }
        }
    }
}

#[test]
fn direction_switches_preserve_next_unread_fractional_cursor_and_do_not_latch() {
    let mut f = Fixture::new(4);
    f.sample(24000, 2, 512, source);
    f.put(0, 0, note()); // A later direction command selects this whole voice's adapter.
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(
        2,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Forward),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(
        3,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(5),
                ..Default::default()
            },
            ..note()
        },
    );
    let pcm = f.render();
    for frame in 0..180 {
        let position = if frame < 60 {
            frame as f64 * 0.5
        } else if frame < 120 {
            30.0 - (frame - 60) as f64 * 0.5
        } else {
            (frame - 120) as f64 * 0.5
        };
        assert_frame(
            &pcm,
            frame,
            [
                interpolated(position, 0, 512),
                interpolated(position, 1, 512),
            ],
        );
    }
    // New note defaults forward; the older release tail still exists, so inspect
    // the manifest rather than pretending the column contains only the new voice.
    let m = song_render::evaluate(&f.song, None).unwrap();
    assert!(m.events.iter().any(|event| event.frame == 180
        && event.command
            == Command::SampleStart {
                voice: 2,
                offset: Some(5),
                direction: Direction::Forward
            }));
}

#[test]
fn offset_on_active_voice_seeks_without_retrigger_and_direction_is_independent() {
    let mut f = Fixture::new(3);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..note()
        },
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(10),
                direction: Some(Direction::Forward),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let pcm = f.render();
    for frame in 0..60 {
        assert_frame(
            &pcm,
            frame,
            [
                source(100 - frame as u32, 0) * 0.5,
                source(100 - frame as u32, 1) * 0.5,
            ],
        );
    }
    for frame in 60..120 {
        assert_frame(
            &pcm,
            frame,
            [
                source(10 + frame as u32 - 60, 0) * 0.5,
                source(10 + frame as u32 - 60, 1) * 0.5,
            ],
        );
    }
    let m = song_render::evaluate(&f.song, None).unwrap();
    assert_eq!(
        m.events
            .iter()
            .filter(|event| matches!(event.command, Command::On { .. }))
            .count(),
        1
    );
    let changes: Vec<_> = m
        .events
        .iter()
        .filter(|event| event.frame == 60)
        .map(|event| &event.command)
        .collect();
    assert_eq!(
        changes,
        [
            &Command::Offset {
                voice: 1,
                frame: 10
            },
            &Command::Direction {
                voice: 1,
                direction: Direction::Forward
            }
        ]
    );
}

#[test]
fn retrigger_restarts_its_original_offset_and_direction() {
    let mut f = Fixture::new(2);
    f.sample(RATE, 2, 8, source);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(5),
                direction: Some(Direction::Reverse),
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.25,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    let pcm = f.render();
    for start in [0, 15, 30] {
        for index in 0..6 {
            assert_frame(
                &pcm,
                start + index,
                [
                    source(5 - index as u32, 0) * 0.5,
                    source(5 - index as u32, 1) * 0.5,
                ],
            );
        }
        assert!(pcm[(start + 6) * 2..(start + 15) * 2]
            .iter()
            .all(|sample| *sample == 0.0));
    }
}

#[test]
fn hard_cut_is_exact_at_the_scheduled_frame_and_isolates_other_voices() {
    let mut f = Fixture::new(3);
    f.sample(RATE, 2, 1000, |_, _| 0.1);
    f.put(0, 0, note());
    let normal = f.render();
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.5),
                ..Default::default()
            },
            ..note()
        },
    );
    let cut = f.render();
    assert_eq!(&cut[..60], &normal[..60]);
    assert!(cut[60..].iter().all(|sample| *sample == 0.0));
    f.put(0, 1, note());
    let together = f.render();
    for frame in 0..30 {
        assert_frame(&together, frame, [0.1, 0.1]);
    }
    for frame in 30..180 {
        assert_frame(&together, frame, [0.05, 0.05]);
    }
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.0),
                ..Default::default()
            },
            ..note()
        },
    );
    let zero = f.render();
    for frame in 0..180 {
        assert_frame(&zero, frame, [0.05, 0.05]);
    }
}

#[test]
fn cut_does_not_silence_an_older_retrigger_release_tail() {
    let mut f = Fixture::new(3);
    f.sample(RATE, 2, 1000, |_, _| 0.1);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                release: Some(0.25),
                ..Default::default()
            },
            ..note()
        },
    );
    let old_tail = f.render();
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                }),
                cut: Some(0.5),
                ..Default::default()
            },
            ..note()
        },
    );
    let retrigger_cut = f.render();
    assert!(retrigger_cut[60] > 0.0);
    assert_eq!(&retrigger_cut[60..], &old_tail[60..]);
}

#[test]
fn controlled_audio_and_release_are_identical_across_processing_partitions() {
    let mut f = Fixture::new(4);
    f.sample(44100, 2, 512, source);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                glide: Some(Glide {
                    target: 67,
                    semitones_per_second: 24.0,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(
        2,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Forward),
                release: Some(0.5),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let expected = f.render();
    for frames in [1, 7, 37] {
        let pcm = render(&mut Engine::new(&f.song, None).unwrap(), frames);
        assert_eq!(pcm, expected);
    }
}

#[test]
fn cold_seek_cannot_resurrect_skipped_voices_but_later_retrigger_starts() {
    let mut f = Fixture::new(3);
    f.put(0, 0, note());
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(30).unwrap();
    assert!(render(&mut engine, 512).iter().all(|sample| *sample == 0.0));
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.75,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(30).unwrap();
    let pcm = render(&mut engine, 512);
    assert!(pcm[..30].iter().all(|sample| *sample == 0.0));
    assert_frame(&pcm, 15, [source(100, 0) * 0.5, source(100, 1) * 0.5]);
}

#[test]
fn advanced_stream_loops_reset_and_stop_without_callback_allocation() {
    let mut f = Fixture::new(3);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                cut: Some(0.75),
                ..Default::default()
            },
            ..note()
        },
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(5),
                ..Default::default()
            },
            ..note()
        },
    );
    let baseline = f.render();
    let musical_frames = 180usize;
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    shared.set_loop(true, musical_frames as u64);
    let mut pcm = Vec::new();
    for _ in 0..4 {
        producer.pump_until_buffered().unwrap();
        let mut data = [0.0; 256];
        callback(&shared, &mut data);
        pcm.extend_from_slice(&data);
    }
    for repeat in 0..2 {
        assert_eq!(
            &pcm[repeat * musical_frames * 2..(repeat + 1) * musical_frames * 2],
            &baseline[..musical_frames * 2]
        );
    }
    shared.seek(60);
    producer.pump_until_buffered().unwrap();
    let mut data = [0.0; 32];
    callback(&shared, &mut data);
    assert_frame(&data, 0, [source(5, 0) * 0.5, source(5, 1) * 0.5]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut stopped = [1.0; 256];
    callback(&shared, &mut stopped);
    assert!(stopped.iter().all(|sample| *sample == 0.0));
}

#[test]
fn invalid_control_assets_offsets_and_combinations_fail_before_export_reservation() {
    let mut f = Fixture::new(1);
    f.sample(RATE, 2, 8, source);
    for offset in [8, 9, u64::from(u32::MAX)] {
        f.put(
            0,
            0,
            Cell {
                effects: StepEffects {
                    offset: Some(offset),
                    ..Default::default()
                },
                ..note()
            },
        );
        let output = f.dir.join("invalid.wav");
        let error = song_render::export(&f.song, None, &output).unwrap_err();
        assert!(error.contains("outside 8 real source frames"), "{error}");
        assert!(!output.exists());
        assert!(!output.with_extension("events.json").exists());
    }
    for effects in [
        StepEffects {
            cut: Some(-0.1),
            ..Default::default()
        },
        StepEffects {
            cut: Some(1.0),
            ..Default::default()
        },
        StepEffects {
            cut: Some(f64::NAN),
            ..Default::default()
        },
        StepEffects {
            cut: Some(0.5),
            release: Some(0.25),
            ..Default::default()
        },
        StepEffects {
            cut: Some(0.1),
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.2,
            }),
            ..Default::default()
        },
        StepEffects {
            offset: Some(u64::from(u32::MAX) + 1),
            ..Default::default()
        },
    ] {
        f.put(0, 0, Cell { effects, ..note() });
        assert!(f.song.validate().is_err());
    }
    for effects in [
        StepEffects {
            cut: Some(0.5),
            ..Default::default()
        },
        StepEffects {
            offset: Some(0),
            ..Default::default()
        },
        StepEffects {
            direction: Some(Direction::Reverse),
            ..Default::default()
        },
    ] {
        f.put(
            0,
            0,
            Cell {
                note: Note::Off,
                effects,
                ..Default::default()
            },
        );
        assert!(f.song.validate().is_err());
    }
    for (rate, channels) in [(384001, 2), (48000, 3)] {
        f.sample(rate, channels, 8, source);
        f.put(
            0,
            0,
            Cell {
                effects: StepEffects {
                    direction: Some(Direction::Reverse),
                    ..Default::default()
                },
                ..note()
            },
        );
        assert!(Engine::new(&f.song, None)
            .err()
            .unwrap()
            .contains("controlled sample requires"));
    }
    f.sample(RATE, 2, 8, |_, _| f32::NAN);
    assert!(Engine::new(&f.song, None)
        .err()
        .unwrap()
        .contains("non-finite"));
}

#[test]
fn empty_and_exhausted_controls_do_not_invent_a_sounding_voice() {
    let mut f = Fixture::new(3);
    f.sample(RATE, 2, 8, source);
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(1),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("requires an active voice"));
    f.put(0, 0, note());
    let pcm = f.render();
    assert!(pcm[16..].iter().all(|sample| *sample == 0.0));
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.5),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(f.render()[120..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn zero_velocity_starts_are_silent_on_both_paths_and_later_volume_preserves_cursor() {
    for linear in [false, true] {
        let mut f = Fixture::new(3);
        f.sample(RATE, 2, 512, |frame, _| if frame < 30 { 0.8 } else { 0.1 });
        f.put(
            0,
            0,
            Cell {
                velocity: Some(0.0),
                effects: StepEffects {
                    offset: linear.then_some(0),
                    ..Default::default()
                },
                ..note()
            },
        );
        assert!(f.render().iter().all(|sample| *sample == 0.0));
        f.put(
            1,
            0,
            Cell {
                velocity: Some(1.0),
                ..Default::default()
            },
        );
        let pcm = f.render();
        assert!(pcm[..120].iter().all(|sample| *sample == 0.0));
        assert!(pcm[120..300].iter().any(|sample| *sample > 0.001));
        // Raising volume must not restart the loud first thirty source frames.
        assert!(pcm[120..360].iter().all(|sample| sample.abs() <= 0.050001));
        f.sample(RATE, 2, 8, |_, _| 0.8);
        assert!(
            f.render().iter().all(|sample| *sample == 0.0),
            "expired silent source was restarted"
        );
    }
}

#[test]
fn embedded_sample_loops_are_rejected_only_for_offset_direction_adapter() {
    let mut f = Fixture::new(3);
    f.sample(RATE, 2, 8, source);
    let path = f.dir.join("sample.wav");
    let mut bytes = std::fs::read(&path).unwrap();
    let mut body = vec![0u8; 60];
    body[28..32].copy_from_slice(&1u32.to_le_bytes());
    body[48..52].copy_from_slice(&7u32.to_le_bytes());
    bytes.extend_from_slice(b"smpl");
    bytes.extend_from_slice(&60u32.to_le_bytes());
    bytes.extend(body);
    let length = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    assert!(phonic::AudioFileInfo::from_file(&path)
        .unwrap()
        .loop_range()
        .is_some());
    f.put(0, 0, note());
    let normal = f.render();
    assert!(normal[120..240].iter().any(|sample| *sample != 0.0));
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.5),
                ..Default::default()
            },
            ..note()
        },
    );
    let cut = f.render();
    assert_eq!(&cut[..60], &normal[..60]);
    assert!(cut[60..].iter().all(|sample| *sample == 0.0));
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..note()
        },
    );
    let output = f.dir.join("unsupported-loop.wav");
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("does not support embedded sample loops"));
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
}

#[test]
fn source_exhaustion_at_exact_event_boundary_cannot_be_revived_by_offset() {
    let mut f = Fixture::new(3);
    f.sample(RATE, 2, 60, source);
    f.put(0, 0, note());
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(0),
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let baseline = f.render();
    assert!(baseline[120..].iter().all(|sample| *sample == 0.0));
    for block in [1, 13, 60] {
        assert_eq!(
            render(&mut Engine::new(&f.song, None).unwrap(), block),
            baseline
        );
    }
}

#[test]
fn controlled_export_roundtrip_and_streamed_pcm_are_identical() {
    let mut f = Fixture::new(4);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                }),
                cut: Some(0.75),
                ..Default::default()
            },
            ..note()
        },
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(5),
                glide: Some(Glide {
                    target: 67,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    let project = f.dir.join("controlled.json");
    aqua_pattern_core::project::save(
        &f.song,
        &project,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    let (restored, _) = aqua_pattern_core::project::load(&project).unwrap();
    assert_eq!(restored, f.song);
    let output = f.dir.join("controlled.wav");
    let (stats, manifest) = song_render::export(&restored, None, &output).unwrap();
    let expected: Vec<f32> = hound::WavReader::open(&output)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(expected, f.render());
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&restored, None, shared.clone()).unwrap();
    assert_eq!(producer.engine.manifest, manifest);
    let mut streamed = Vec::new();
    for _ in 0..1000 {
        producer.pump_until_buffered().unwrap();
        let mut data = [0.0; 512];
        callback(&shared, &mut data);
        streamed.extend(data);
        if shared.has_ended() {
            break;
        }
    }
    assert!(shared.has_ended());
    assert_eq!(&streamed[..stats.frames as usize * 2], expected);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
}

#[test]
fn controlled_asset_size_and_clipping_failures_preserve_outputs() {
    let mut f = Fixture::new(1);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(0),
                ..Default::default()
            },
            ..note()
        },
    );
    let sample = f.dir.join("sample.wav");
    let output = f.dir.join("bad.wav");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&sample)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("exceeds 64 MiB"));
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
    f.sample(RATE, 2, 8, |_, _| 3.0);
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("clipping"));
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
    assert!(sample.exists());
}

#[test]
fn hard_cut_decimal_tie_follows_the_last_retrigger_even_at_half_frame_boundary() {
    let mut f = Fixture::new(1);
    f.song.bpm = 399.72241498959056;
    f.song.lines_per_beat = 3;
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.1),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.1,
                }),
                cut: Some(0.3),
                ..Default::default()
            },
            ..note()
        },
    );
    let manifest = song_render::evaluate(&f.song, None).unwrap();
    assert_eq!(
        manifest.events.last().unwrap().command,
        Command::Cut { voice: 3 }
    );
    assert_eq!(manifest.events.last().unwrap().frame, 721);
    let last_on = manifest
        .events
        .iter()
        .rfind(|event| matches!(event.command, Command::On { .. }))
        .unwrap();
    assert_eq!(last_on.frame, 721);
    assert!(f.render().iter().all(|sample| sample.is_finite()));
}

#[test]
fn linear_adapter_handles_extreme_pitch_and_glide_ranges_without_invalid_pcm() {
    for pitch in [0, 127] {
        for target in [0, 127] {
            for rate in [0.01, 120.0] {
                let mut f = Fixture::new(4);
                f.put(
                    0,
                    0,
                    Cell {
                        note: Note::Note(pitch),
                        effects: StepEffects {
                            offset: Some(100),
                            direction: Some(Direction::Reverse),
                            glide: Some(Glide {
                                target,
                                semitones_per_second: rate,
                            }),
                            ..Default::default()
                        },
                        ..note()
                    },
                );
                assert!(f
                    .render()
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
            }
        }
    }
}
