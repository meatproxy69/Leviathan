use aqua_pattern_core::{
    song::*,
    song_render::{self, Command, Engine, Manifest, Release},
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

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    song: Song,
}
impl Fixture {
    fn new(rows: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-effects-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut song = example_song();
        song.patterns.clear();
        song.patterns.insert(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "FX".into(),
                rows,
                tracks: BTreeMap::new(),
            },
        );
        song.order = vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }];
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some(dir.join("sample.wav"));
        }
        let fixture = Self { dir, song };
        fixture.sample(|frame| {
            (std::f32::consts::TAU * 220.0 * frame as f32 / RATE as f32).sin() * 0.1
        });
        fixture
    }
    fn sample(&self, value: impl Fn(u32) -> f32) {
        let mut writer = hound::WavWriter::create(
            self.dir.join("sample.wav"),
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..RATE * 2 {
            for _ in 0..2 {
                writer.write_sample(value(frame)).unwrap();
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
    fn evaluate(&self) -> Manifest {
        song_render::evaluate(&self.song, None).unwrap()
    }
    fn render(&self) -> Vec<f32> {
        render(&mut Engine::new(&self.song, None).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn note() -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.5),
        ..Cell::default()
    }
}
fn render(engine: &mut Engine) -> Vec<f32> {
    let mut samples = Vec::new();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut block = [0.0; 1024];
    while engine.position < end {
        let count = engine.block(&mut block, end).unwrap();
        samples.extend_from_slice(&block[..count * 2]);
    }
    samples
}
fn energy(data: &[f32], start: usize, end: usize) -> f32 {
    data[start * 2..end * 2].iter().map(|v| v.abs()).sum()
}
fn positive_crossings(data: &[f32], start: usize, end: usize) -> usize {
    (start + 1..end)
        .filter(|frame| data[(frame - 1) * 2] <= 0.0 && data[frame * 2] > 0.0)
        .count()
}

#[test]
fn retrigger_expands_delayed_absolute_times_and_restarts_real_sample_audio() {
    let mut f = Fixture::new(2);
    f.sample(|frame| if frame < 400 { 0.1 } else { 0.0 });
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.1),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 3,
                    spacing: 0.2,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    let m = f.evaluate();
    assert_eq!(m.schema, 2);
    assert_eq!(m.evaluator, "leviathan-song-v2");
    let starts: Vec<_> = m
        .events
        .iter()
        .filter_map(|event| matches!(event.command, Command::On { .. }).then_some(event.frame))
        .collect();
    assert_eq!(starts, [600, 1800, 3000, 4200]);
    assert_eq!(
        m.events
            .iter()
            .filter(|event| matches!(
                event.command,
                Command::Off {
                    reason: Release::Retrigger,
                    ..
                }
            ))
            .count(),
        3
    );
    let pcm = f.render();
    for start in starts {
        assert!(energy(&pcm, start as usize + 32, start as usize + 256) > 0.1);
        assert_eq!(
            energy(&pcm, start as usize + 500, start as usize + 900),
            0.0
        );
    }
    assert_eq!(energy(&pcm, 0, 600), 0.0);
    let pattern = f.song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .effects = StepEffects::default();
    let normal = f.render();
    assert_eq!(&pcm[..1800 * 2], &normal[..1800 * 2]);
    assert_eq!(energy(&normal, 1800, 2200), 0.0);
}

#[test]
fn timed_release_is_real_declick_note_off_not_a_hard_cut() {
    let mut f = Fixture::new(3);
    f.sample(|_| 0.1);
    f.put(0, 0, note());
    let normal = f.render();
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                release: Some(0.5),
                ..Default::default()
            },
            ..note()
        },
    );
    let m = f.evaluate();
    let release = m
        .events
        .iter()
        .find(|event| {
            matches!(
                event.command,
                Command::Off {
                    reason: Release::TimedRelease,
                    ..
                }
            )
        })
        .unwrap();
    assert_eq!(release.frame, 3000);
    assert_eq!(release.position, Some((0, 0)));
    assert!(!m.events.iter().any(|event| matches!(
        event.command,
        Command::Off {
            reason: Release::SongEnd,
            ..
        }
    )));
    let released = f.render();
    assert_eq!(&released[..6000], &normal[..6000]);
    assert!(
        energy(&released, 3000, 4000) > 1.0,
        "release must preserve documented de-click tail"
    );
    // phonic defines 50 ms as 99% exponential completion, not a hard
    // duration. Completion is detected after roughly 100 ms at a DSP boundary.
    let ratio_at_50ms = released[5400 * 2] / normal[5400 * 2];
    assert!((0.005..0.02).contains(&ratio_at_50ms));
    assert_eq!(energy(&released, 9000, 12000), 0.0);
    assert!(energy(&normal, 6000, 9000) > 1.0);
}

#[test]
fn glide_changes_actual_pitch_at_requested_rate_without_changing_future_notes() {
    for (target, crossings) in [(72, 44), (48, 11)] {
        let mut f = Fixture::new(8);
        f.put(0, 0, note());
        f.put(5, 0, note());
        let normal = f.render();
        f.put(
            1,
            0,
            Cell {
                effects: StepEffects {
                    glide: Some(Glide {
                        target,
                        semitones_per_second: 120.0,
                    }),
                    ..Default::default()
                },
                ..Cell::default()
            },
        );
        let m = f.evaluate();
        assert!(m.events.iter().any(|event| event.frame == 6000
            && event.command
                == Command::Glide {
                    voice: 1,
                    target,
                    semitones_per_second: 120.0
                }));
        let glided = f.render();
        assert_eq!(&normal[..12000], &glided[..12000]);
        assert!((positive_crossings(&glided, 19200, 24000) as i32 - crossings).abs() <= 2);
        assert!((positive_crossings(&glided, 38400, 43200) as i32 - 22).abs() <= 2);
        assert_ne!(&normal[12000..24000], &glided[12000..24000]);
        f.put(
            1,
            0,
            Cell {
                effects: StepEffects {
                    glide: Some(Glide {
                        target,
                        semitones_per_second: 1.0,
                    }),
                    ..Default::default()
                },
                ..Cell::default()
            },
        );
        let slow = f.render();
        assert!((positive_crossings(&slow, 19200, 24000) as i32 - 22).abs() <= 2);
    }
}

#[test]
fn delayed_effect_ties_finish_prior_occurrence_before_the_next_note() {
    let mut f = Fixture::new(1);
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.999),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.0001,
                }),
                release: Some(0.9995),
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    f.song.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Next".into(),
            rows: 1,
            tracks: BTreeMap::from([(
                TrackId(1),
                BTreeMap::from([(0, BTreeMap::from([(0, note())]))]),
            )]),
        },
    );
    f.song.order.push(Occurrence {
        muted_tracks: Default::default(),
        section: None,
        pattern: PatternId(2),
    });
    let m = f.evaluate();
    let boundary: Vec<_> = m.events.iter().filter(|event| event.frame == 28).collect();
    assert_eq!(boundary.len(), 10);
    assert!(matches!(boundary[0].command, Command::On { voice: 1, .. }));
    assert!(matches!(
        boundary[1].command,
        Command::Glide { voice: 1, .. }
    ));
    assert_eq!(
        boundary[2].command,
        Command::Off {
            voice: 1,
            reason: Release::Retrigger
        }
    );
    assert!(matches!(boundary[3].command, Command::On { voice: 2, .. }));
    assert!(matches!(
        boundary[4].command,
        Command::Glide { voice: 2, .. }
    ));
    assert_eq!(
        boundary[5].command,
        Command::Off {
            voice: 2,
            reason: Release::Retrigger
        }
    );
    assert!(matches!(boundary[6].command, Command::On { voice: 3, .. }));
    assert!(matches!(
        boundary[7].command,
        Command::Glide { voice: 3, .. }
    ));
    assert_eq!(
        boundary[8].command,
        Command::Off {
            voice: 3,
            reason: Release::TimedRelease
        }
    );
    assert!(matches!(boundary[9].command, Command::On { voice: 4, .. }));
    assert!(boundary[..9]
        .iter()
        .all(|event| event.position == Some((0, 0))));
    assert_eq!(boundary[9].position, Some((1, 0)));
    assert_eq!(m, f.evaluate());
    assert!(f.render().iter().all(|value| value.is_finite()));
}

#[test]
fn final_fractional_effects_clamp_before_end_and_decimal_release_is_valid() {
    let mut f = Fixture::new(1);
    f.song.bpm = 123.45;
    f.song.lines_per_beat = 7;
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
                release: Some(0.3),
                ..Default::default()
            },
            ..note()
        },
    );
    f.song.validate().unwrap();
    let m = f.evaluate();
    let last = m.events.last().unwrap();
    assert_eq!(
        last.frame,
        (0.3 * RATE as f64 * 60.0 / (123.45 * 7.0)).round() as u64
    );
    assert!(matches!(
        last.command,
        Command::Off {
            reason: Release::TimedRelease,
            ..
        }
    ));
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.999999),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.0000001,
                }),
                release: Some(0.9999999),
                ..Default::default()
            },
            ..note()
        },
    );
    let m = f.evaluate();
    assert!(m
        .events
        .iter()
        .all(|event| event.frame == m.musical_frames - 1));
    assert!(f.render().iter().all(|value| value.is_finite()));
}

#[test]
fn glide_targets_active_instrument_not_new_selection_or_other_columns() {
    let mut f = Fixture::new(4);
    f.put(0, 0, note());
    f.put(
        0,
        1,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..note()
        },
    );
    f.put(
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(3)),
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..Cell::default()
        },
    );
    f.put(
        2,
        0,
        Cell {
            note: Note::Note(67),
            ..Cell::default()
        },
    );
    let m = f.evaluate();
    let glides: Vec<_> = m
        .events
        .iter()
        .filter(|event| matches!(event.command, Command::Glide { .. }))
        .collect();
    assert_eq!(glides.len(), 1);
    assert_eq!(glides[0].instrument, InstrumentId(1));
    assert_eq!(glides[0].column, 0);
    assert!(m.events.iter().any(|event| event.frame == 12000
        && event.instrument == InstrumentId(3)
        && matches!(event.command, Command::On { pitch: 67, .. })));
    let mut empty = Fixture::new(2);
    empty.put(0, 0, note());
    empty.put(
        0,
        1,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..Cell::default()
        },
    );
    assert!(song_render::evaluate(&empty.song, None)
        .unwrap_err()
        .contains("glide requires an active voice"));
}

#[test]
fn release_without_active_voice_is_noop_and_effect_memory_does_not_repeat_implicitly() {
    let mut f = Fixture::new(4);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                release: Some(0.5),
                ..Default::default()
            },
            ..Cell::default()
        },
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.5,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    f.put(
        2,
        0,
        Cell {
            note: Note::Note(64),
            ..Cell::default()
        },
    );
    let m = f.evaluate();
    assert!(!m.events.iter().any(|event| event.frame < 6000));
    let starts: Vec<_> = m
        .events
        .iter()
        .filter_map(|event| matches!(event.command, Command::On { .. }).then_some(event.frame))
        .collect();
    assert_eq!(starts, [6000, 9000, 12000]);
}

#[test]
fn cold_seek_into_pending_retrigger_starts_only_the_later_resolved_note() {
    let mut f = Fixture::new(2);
    f.sample(|_| 0.1);
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.1),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.5,
                }),
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(1000).unwrap();
    let pcm = render(&mut engine);
    assert_eq!(energy(&pcm, 0, 2600), 0.0);
    assert!(energy(&pcm, 2600, 3000) > 0.1);
}

#[test]
fn effects_stream_and_export_use_identical_pcm_and_manifest() {
    let mut f = Fixture::new(4);
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.1),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.2,
                }),
                release: Some(0.8),
                glide: Some(Glide {
                    target: 67,
                    semitones_per_second: 24.0,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    let path = f.dir.join("effects.wav");
    let (stats, manifest) = song_render::export(&f.song, None, &path).unwrap();
    let expected: Vec<f32> = hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    assert_eq!(producer.engine.manifest, manifest);
    let mut streamed = Vec::new();
    for _ in 0..1000 {
        producer.pump_until_buffered().unwrap();
        let mut block = [0.0; 512];
        shared.fill(&mut block);
        streamed.extend_from_slice(&block);
        if shared.has_ended() {
            break;
        }
    }
    assert!(shared.has_ended());
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    assert_eq!(&streamed[..stats.frames as usize * 2], expected);
    assert!(streamed[stats.frames as usize * 2..]
        .iter()
        .all(|value| *value == 0.0));
}

#[test]
fn expanded_retriggers_are_bounded_and_rejections_do_not_create_outputs() {
    let mut f = Fixture::new(1);
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for column in 0..128 {
        f.put(
            0,
            column,
            Cell {
                effects: StepEffects {
                    retrigger: Some(Retrigger {
                        repeats: 15,
                        spacing: 0.01,
                    }),
                    ..Default::default()
                },
                ..note()
            },
        );
    }
    let m = f.evaluate();
    assert_eq!(
        m.events
            .iter()
            .filter(|event| matches!(event.command, Command::On { .. }))
            .count(),
        2048
    );
    f.put(0, 128, note());
    let output = f.dir.join("too-many.wav");
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("2048 note triggers"));
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
}

#[test]
fn invalid_effect_parameters_and_unsupported_fields_are_rejected() {
    let mut f = Fixture::new(1);
    let invalid = [
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 0,
                spacing: 0.1,
            }),
            ..Default::default()
        },
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 17,
                spacing: 0.01,
            }),
            ..Default::default()
        },
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.0,
            }),
            ..Default::default()
        },
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: f64::NAN,
            }),
            ..Default::default()
        },
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 2,
                spacing: 0.5,
            }),
            ..Default::default()
        },
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.5,
            }),
            release: Some(0.4),
            ..Default::default()
        },
        StepEffects {
            release: Some(f64::INFINITY),
            ..Default::default()
        },
        StepEffects {
            release: Some(-0.1),
            ..Default::default()
        },
        StepEffects {
            release: Some(1.0),
            ..Default::default()
        },
        StepEffects {
            glide: Some(Glide {
                target: 128,
                semitones_per_second: 12.0,
            }),
            ..Default::default()
        },
        StepEffects {
            glide: Some(Glide {
                target: 60,
                semitones_per_second: 0.0,
            }),
            ..Default::default()
        },
        StepEffects {
            glide: Some(Glide {
                target: 60,
                semitones_per_second: 121.0,
            }),
            ..Default::default()
        },
        StepEffects {
            glide: Some(Glide {
                target: 60,
                semitones_per_second: f32::NAN,
            }),
            ..Default::default()
        },
    ];
    for effects in invalid {
        f.put(0, 0, Cell { effects, ..note() });
        assert!(f.song.validate().is_err());
    }
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.6),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.4,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    assert!(f.song.validate().is_err());
    for effects in [
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.1,
            }),
            ..Default::default()
        },
        StepEffects {
            release: Some(0.1),
            ..Default::default()
        },
        StepEffects {
            glide: Some(Glide {
                target: 60,
                semitones_per_second: 1.0,
            }),
            ..Default::default()
        },
    ] {
        f.put(
            0,
            0,
            Cell {
                note: Note::Off,
                effects,
                ..Cell::default()
            },
        );
        assert!(f.song.validate().is_err());
    }
    for unsupported in ["offset_percent", "reverse", "filter"] {
        let json = format!(r#"{{"retrigger":null,"release":null,"glide":null,"{unsupported}":0}}"#);
        assert!(serde_json::from_str::<StepEffects>(&json).is_err());
    }
}

#[test]
fn decimal_equal_release_never_rounds_before_the_final_retrigger() {
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
                release: Some(0.3),
                ..Default::default()
            },
            ..note()
        },
    );
    let m = f.evaluate();
    let last_on = m
        .events
        .iter()
        .rfind(|event| matches!(event.command, Command::On { .. }))
        .unwrap();
    let released = m.events.last().unwrap();
    assert_eq!(last_on.frame, 721);
    assert_eq!(released.frame, last_on.frame);
    assert_eq!(
        released.command,
        Command::Off {
            voice: 3,
            reason: Release::TimedRelease
        }
    );
    assert!(!m.events.iter().any(|event| matches!(
        event.command,
        Command::Off {
            reason: Release::SongEnd,
            ..
        }
    )));
}

#[test]
fn overlapping_retriggers_that_clip_clean_only_new_export_outputs() {
    let mut f = Fixture::new(2);
    f.sample(|_| 0.9);
    f.put(
        0,
        0,
        Cell {
            velocity: Some(1.0),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 16,
                    spacing: 0.005,
                }),
                ..Default::default()
            },
            ..note()
        },
    );
    f.song.validate().unwrap();
    assert_eq!(
        f.evaluate()
            .events
            .iter()
            .filter(|event| matches!(event.command, Command::On { .. }))
            .count(),
        17
    );
    let path = f.dir.join("clipped.wav");
    assert!(song_render::export(&f.song, None, &path)
        .unwrap_err()
        .contains("clipping"));
    assert!(!path.exists());
    assert!(!path.with_extension("events.json").exists());
    assert!(f.dir.join("sample.wav").is_file());
}

#[test]
fn extreme_supported_glide_targets_and_rates_render_finite_audio() {
    for target in [0, 127] {
        for rate in [0.01, 120.0] {
            let mut f = Fixture::new(8);
            f.put(
                0,
                0,
                Cell {
                    effects: StepEffects {
                        glide: Some(Glide {
                            target,
                            semitones_per_second: rate,
                        }),
                        ..Default::default()
                    },
                    ..note()
                },
            );
            let pcm = f.render();
            assert!(
                pcm.iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0),
                "target {target}, rate {rate}"
            );
            assert!(energy(&pcm, 0, 4800) > 0.01);
        }
    }
}

#[test]
fn glide_musical_pcm_is_independent_of_small_processing_partitions() {
    let mut f = Fixture::new(4);
    f.put(0, 0, note());
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 24.0,
                }),
                ..Default::default()
            },
            ..Cell::default()
        },
    );
    let baseline = f.render();
    for frames_per_block in [1, 37] {
        let mut engine = Engine::new(&f.song, None).unwrap();
        let end = engine.manifest.musical_frames;
        let mut pcm = Vec::new();
        let mut block = vec![0.0; frames_per_block * 2];
        while engine.position < end {
            let count = engine.block(&mut block, end).unwrap();
            pcm.extend_from_slice(&block[..count * 2]);
        }
        assert_eq!(pcm.len(), end as usize * 2);
        assert!(
            pcm.iter().zip(&baseline).all(|(a, b)| a == b),
            "glide depended on {frames_per_block}-frame partition"
        );
    }
}
