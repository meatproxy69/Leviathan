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
            "leviathan-phrases-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut song = example_song();
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "Song".into(),
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
        f.sample(|i| (std::f32::consts::TAU * 220.0 * i as f32 / RATE as f32).sin() * 0.1);
        f
    }
    fn sample(&self, value: impl Fn(u32) -> f32) {
        let mut w = hound::WavWriter::create(
            self.dir.join("sample.wav"),
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..RATE * 2 {
            for _ in 0..2 {
                w.write_sample(value(i)).unwrap();
            }
        }
        w.finalize().unwrap();
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
    fn phrase(&mut self, rows: u32, lpb: u16) {
        self.song.phrases.insert(
            PhraseId(1),
            Phrase {
                name: "Reusable".into(),
                instrument: InstrumentId(2),
                rows,
                lines_per_beat: lpb,
                note_columns: 2,
                root_note: 60,
                cells: BTreeMap::new(),
            },
        );
    }
    fn pput(&mut self, row: u32, column: u16, cell: Cell) {
        self.song
            .phrases
            .get_mut(&PhraseId(1))
            .unwrap()
            .cells
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
fn note(pitch: u8) -> Cell {
    Cell {
        note: Note::Note(pitch),
        ..Default::default()
    }
}
fn trigger(pitch: u8, cycles: u16) -> Cell {
    Cell {
        phrase: Some(PhraseTrigger {
            phrase: PhraseId(1),
            cycles,
        }),
        ..note(pitch)
    }
}
fn render(engine: &mut Engine) -> Vec<f32> {
    let mut pcm = Vec::new();
    let mut block = [0.0; 1024];
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    while engine.position < end {
        let count = engine.block(&mut block, end).unwrap();
        pcm.extend_from_slice(&block[..count * 2]);
    }
    pcm
}
fn starts(m: &Manifest) -> Vec<(u64, u8, f32, f32, Option<usize>)> {
    m.events
        .iter()
        .filter_map(|e| match e.command {
            Command::On {
                pitch, volume, pan, ..
            } => Some((e.frame, pitch, volume, pan, e.phrase.map(|p| p.instance))),
            _ => None,
        })
        .collect()
}
fn crossings(pcm: &[f32], start: usize, end: usize) -> usize {
    (start + 1..end)
        .filter(|i| pcm[(i - 1) * 2] <= 0.0 && pcm[i * 2] > 0.0)
        .count()
}

#[test]
fn phrase_clock_is_independent_of_trigger_pitch_and_transposes_glide_targets() {
    let mut f = Fixture::new(8);
    f.phrase(4, 8);
    f.pput(0, 0, note(60));
    f.pput(
        1,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 67,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.pput(2, 0, note(64));
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.25),
            ..trigger(72, 1)
        },
    );
    let transposed = f.evaluate();
    assert_eq!(transposed.schema, 4);
    assert_eq!(
        starts(&transposed),
        [
            (7500, 72, 1.0, 0.0, Some(1)),
            (13500, 76, 1.0, 0.0, Some(1))
        ]
    );
    assert!(transposed
        .events
        .iter()
        .any(|e| e.frame == 10500 && matches!(e.command, Command::Glide { target: 79, .. })));
    assert!(transposed.events.iter().any(|e| e.frame == 19500
        && matches!(
            e.command,
            Command::Off {
                reason: Release::PhraseEnd,
                ..
            }
        )));
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.25),
            ..trigger(60, 1)
        },
    );
    let original = f.evaluate();
    assert_eq!(
        original.events.iter().map(|e| e.frame).collect::<Vec<_>>(),
        transposed
            .events
            .iter()
            .map(|e| e.frame)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        starts(&original).iter().map(|e| e.1).collect::<Vec<_>>(),
        [60, 64]
    );
}

#[test]
fn phrase_transposition_changes_real_pitch_not_phrase_duration() {
    let mut f = Fixture::new(8);
    f.phrase(16, 8);
    f.pput(0, 0, note(60));
    f.put(0, 0, trigger(60, 1));
    let base = f.render();
    f.put(0, 0, trigger(72, 1));
    let octave = f.render();
    assert_eq!(base.len(), octave.len());
    assert!((crossings(&base, 4800, 9600) as i32 - 22).abs() <= 1);
    assert!((crossings(&octave, 4800, 9600) as i32 - 44).abs() <= 1);
}

#[test]
fn fractional_phrase_times_round_the_combined_absolute_origin_once() {
    let mut f = Fixture::new(8);
    f.song.bpm = 137.0;
    f.song.lines_per_beat = 7;
    f.phrase(3, 11);
    f.pput(0, 0, note(60));
    f.pput(1, 0, note(64));
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.1),
            ..trigger(60, 1)
        },
    );
    let m = f.evaluate();
    let beat = RATE as f64 * 60.0 / 137.0;
    let expected = ((1.1 / 7.0 + 1.0 / 11.0) * beat).round() as u64;
    assert_eq!(expected, 5215);
    assert_ne!(
        expected,
        (1.1 / 7.0 * beat).round() as u64 + (1.0 / 11.0 * beat).round() as u64
    );
    assert_eq!(starts(&m)[1].0, expected);
}

#[test]
fn simultaneous_instances_and_repeated_cycles_keep_independent_uncompounded_memory() {
    let mut f = Fixture::new(8);
    f.phrase(2, 8);
    f.pput(
        0,
        0,
        Cell {
            velocity: Some(0.5),
            pan: Some(0.1),
            ..note(60)
        },
    );
    f.pput(
        0,
        1,
        Cell {
            velocity: Some(0.25),
            pan: Some(-0.2),
            ..note(67)
        },
    );
    f.put(
        0,
        0,
        Cell {
            velocity: Some(0.2),
            pan: Some(0.2),
            ..trigger(60, 3)
        },
    );
    f.put(
        0,
        1,
        Cell {
            velocity: Some(0.8),
            pan: Some(-0.4),
            ..trigger(72, 3)
        },
    );
    f.put(
        1,
        0,
        Cell {
            velocity: Some(0.9),
            pan: Some(0.8),
            ..Default::default()
        },
    );
    f.put(4, 0, note(64));
    let m = f.evaluate();
    for event in &m.events {
        if let (
            Command::On {
                pitch, volume, pan, ..
            },
            Some(p),
        ) = (&event.command, event.phrase)
        {
            let local = if p.column == 0 { 0.5 } else { 0.25 };
            let gain = if p.instance == 1 { 0.2 } else { 0.8 };
            assert_eq!(*volume, local * gain);
            assert_eq!(
                *pitch,
                if p.column == 0 { 60 } else { 67 } + if p.instance == 1 { 0 } else { 12 }
            );
            let local_pan = if p.column == 0 { 0.1 } else { -0.2 };
            let parent_pan = if p.instance == 1 { 0.2 } else { -0.4 };
            assert_eq!(*pan, local_pan + parent_pan);
        }
    }
    let ordinary = m
        .events
        .iter()
        .find(|e| e.frame == 24000 && matches!(e.command, Command::On { .. }))
        .unwrap();
    assert_eq!(ordinary.instrument, InstrumentId(2));
    assert_eq!(ordinary.phrase, None);
    assert!(matches!(
        ordinary.command,
        Command::On {
            volume: 0.9,
            pan: 0.8,
            ..
        }
    ));
    assert_eq!(
        m.events
            .iter()
            .filter(|e| e.phrase.is_some() && matches!(e.command, Command::On { .. }))
            .count(),
        12
    );
}

#[test]
fn parent_off_and_replacement_cancel_same_frame_pending_children_only_for_that_instance() {
    let mut f = Fixture::new(8);
    f.phrase(2, 8);
    f.pput(0, 0, note(60));
    f.put(0, 0, trigger(60, 4));
    f.put(0, 1, trigger(72, 4));
    f.put(
        1,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let m = f.evaluate();
    let first: Vec<_> = m
        .events
        .iter()
        .filter(|e| {
            e.phrase.is_some_and(|p| p.instance == 1) && matches!(e.command, Command::On { .. })
        })
        .collect();
    assert_eq!(first.len(), 1);
    assert!(m.events.iter().any(|e| e.frame == 6000
        && e.phrase.is_some_and(|p| p.instance == 1)
        && matches!(
            e.command,
            Command::Off {
                reason: Release::ExplicitOff,
                ..
            }
        )));
    assert_eq!(
        m.events
            .iter()
            .filter(|e| e.phrase.is_some_and(|p| p.instance == 2)
                && matches!(e.command, Command::On { .. }))
            .count(),
        4
    );
    f.put(1, 0, note(64));
    let replaced = f.evaluate();
    assert!(replaced.events.iter().any(|e| e.frame == 6000
        && e.phrase.is_none()
        && e.instrument == InstrumentId(2)
        && matches!(e.command, Command::On { pitch: 64, .. })));
    assert_eq!(
        replaced
            .events
            .iter()
            .filter(|e| e.phrase.is_some_and(|p| p.instance == 1)
                && matches!(e.command, Command::On { .. }))
            .count(),
        1
    );
    f.put(1, 0, trigger(67, 1));
    let retriggered = f.evaluate();
    assert!(retriggered.events.iter().any(|e| e.frame == 6000
        && e.phrase.is_some_and(|p| p.instance == 3)
        && matches!(e.command, Command::On { pitch: 67, .. })));
}

#[test]
fn parent_hard_cut_kills_all_owned_phrase_tails_but_not_other_song_voices() {
    let mut f = Fixture::new(8);
    f.sample(|_| 0.1);
    f.phrase(2, 8);
    f.pput(
        0,
        0,
        Cell {
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.5,
                }),
                ..Default::default()
            },
            ..note(60)
        },
    );
    f.pput(0, 1, note(67));
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.75),
                ..Default::default()
            },
            ..trigger(60, 4)
        },
    );
    let solo_phrase = f.render();
    assert!(solo_phrase[..9000].iter().any(|v| *v > 0.0));
    assert!(solo_phrase[9000..].iter().all(|v| *v == 0.0));
    let m = f.evaluate();
    assert_eq!(
        m.events
            .iter()
            .filter(|e| e.frame == 4500 && matches!(e.command, Command::Cut { .. }))
            .count(),
        3
    );
    f.put(
        0,
        1,
        Cell {
            instrument: Some(InstrumentId(1)),
            ..note(60)
        },
    );
    let together = f.render();
    for sample in &together[9000..48000 * 2] {
        assert_eq!(*sample, 0.05);
    }
}

#[test]
fn phrase_owner_overrides_omitted_parent_selection_and_parent_voice_controls_reject() {
    let mut f = Fixture::new(8);
    f.phrase(8, 4);
    f.pput(0, 0, note(60));
    f.put(
        0,
        0,
        Cell {
            instrument: Some(InstrumentId(1)),
            ..note(48)
        },
    );
    f.put(1, 0, trigger(60, 1));
    f.put(2, 0, note(64));
    let m = f.evaluate();
    let ons: Vec<_> = m
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .map(|e| (e.frame, e.instrument))
        .collect();
    assert_eq!(
        ons,
        [
            (0, InstrumentId(1)),
            (6000, InstrumentId(2)),
            (12000, InstrumentId(2))
        ]
    );
    for effects in [
        StepEffects {
            glide: Some(Glide {
                target: 72,
                semitones_per_second: 12.0,
            }),
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
            2,
            0,
            Cell {
                effects,
                ..Default::default()
            },
        );
        assert!(song_render::evaluate(&f.song, None)
            .unwrap_err()
            .contains("cannot address a running phrase instance"));
    }
}

#[test]
fn phrase_truncation_drops_future_rows_and_cycles_instead_of_creating_end_bursts() {
    let mut f = Fixture::new(1);
    f.phrase(4, 4);
    for row in 0..4 {
        f.pput(row, 0, note(60 + row as u8));
    }
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.9),
            ..trigger(60, 64)
        },
    );
    let m = f.evaluate();
    assert_eq!(starts(&m), [(5400, 60, 1.0, 0.0, Some(1))]);
    assert_eq!(m.events.last().unwrap().frame, 6000);
    assert!(matches!(
        m.events.last().unwrap().command,
        Command::Off {
            reason: Release::SongEnd,
            ..
        }
    ));
    f.put(
        0,
        0,
        Cell {
            delay: Some(0.999999999999),
            ..trigger(60, 64)
        },
    );
    let m = f.evaluate();
    assert_eq!(starts(&m).len(), 1);
    assert_eq!(starts(&m)[0].0, 5999);
    assert_eq!(m.events.last().unwrap().frame, 6000);
}

#[test]
fn internal_sample_controls_retrigger_glide_and_release_use_same_engine() {
    let mut f = Fixture::new(8);
    f.phrase(4, 8);
    f.pput(
        0,
        0,
        Cell {
            velocity: Some(0.3),
            effects: StepEffects {
                offset: Some(1000),
                direction: Some(Direction::Reverse),
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                }),
                cut: Some(0.75),
                ..Default::default()
            },
            ..note(60)
        },
    );
    f.pput(
        1,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 67,
                    semitones_per_second: 24.0,
                }),
                release: Some(0.8),
                ..Default::default()
            },
            ..note(64)
        },
    );
    f.put(0, 0, trigger(60, 2));
    let m = f.evaluate();
    assert!(m.events.iter().any(|e| matches!(
        e.command,
        Command::SampleStart {
            offset: Some(1000),
            direction: Direction::Reverse,
            ..
        }
    )));
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::Cut { .. })));
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::Glide { target: 67, .. })));
    assert!(f.render().iter().all(|v| v.is_finite() && v.abs() <= 1.0));
}

#[test]
fn unused_phrase_banks_preserve_no_trigger_manifest_and_pcm_exactly() {
    let mut f = Fixture::new(4);
    f.put(
        0,
        0,
        Cell {
            instrument: Some(InstrumentId(1)),
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                cut: Some(0.75),
                ..Default::default()
            },
            ..note(60)
        },
    );
    let before = f.evaluate();
    let audio = f.render();
    f.phrase(4, 7);
    f.pput(0, 0, note(67));
    assert_eq!(f.evaluate(), before);
    assert_eq!(f.render(), audio);
    assert_eq!(before.schema, 3);
    f.song.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Unused".into(),
            rows: 4,
            tracks: BTreeMap::from([(
                TrackId(1),
                BTreeMap::from([(0, BTreeMap::from([(0, trigger(60, 1))]))]),
            )]),
        },
    );
    assert_eq!(f.evaluate(), before);
}

#[test]
fn phrases_roundtrip_and_stream_match_offline_and_cold_seek_keeps_only_future_starts() {
    let mut f = Fixture::new(4);
    f.phrase(2, 8);
    f.pput(0, 0, note(60));
    f.put(0, 0, trigger(60, 3));
    let path = f.dir.join("phrases.json");
    aqua_pattern_core::project::save(
        &f.song,
        &path,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    let (restored, _) = aqua_pattern_core::project::load(&path).unwrap();
    assert_eq!(restored, f.song);
    let wav = f.dir.join("phrases.wav");
    let (stats, manifest) = song_render::export(&restored, None, &wav).unwrap();
    let expected: Vec<f32> = hound::WavReader::open(wav)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&restored, None, shared.clone()).unwrap();
    assert_eq!(producer.engine.manifest, manifest);
    let mut actual = Vec::new();
    for _ in 0..1000 {
        producer.pump_until_buffered().unwrap();
        let mut data = [0.0; 512];
        shared.fill(&mut data);
        actual.extend(data);
        if shared.has_ended() {
            break;
        }
    }
    assert!(shared.has_ended());
    assert_eq!(&actual[..stats.frames as usize * 2], expected);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(3000).unwrap();
    let cold = render(&mut engine);
    assert!(cold[..6000].iter().all(|v| *v == 0.0));
    assert!(cold[6000..12000].iter().any(|v| *v != 0.0));
}

#[test]
fn phrase_validation_rejects_nested_unsupported_transposed_and_dangling_content() {
    let mut f = Fixture::new(4);
    f.phrase(4, 8);
    f.pput(0, 0, note(120));
    f.put(0, 0, trigger(72, 1));
    assert!(f.song.validate().unwrap_err().contains("transposition"));
    f.pput(
        0,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 120,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..note(60)
        },
    );
    assert!(f.song.validate().unwrap_err().contains("transposition"));
    f.put(0, 0, trigger(60, 1));
    f.pput(0, 0, trigger(60, 1));
    assert!(f.song.validate().unwrap_err().contains("nested"));
    f.pput(
        0,
        0,
        Cell {
            instrument: Some(InstrumentId(1)),
            ..note(60)
        },
    );
    assert!(f.song.validate().is_err());
    f.pput(0, 0, note(60));
    for cycles in [0, 65] {
        f.put(0, 0, trigger(60, cycles));
        assert!(f.song.validate().is_err());
    }
    for effects in [
        StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.2,
            }),
            ..Default::default()
        },
        StepEffects {
            offset: Some(0),
            ..Default::default()
        },
        StepEffects {
            direction: Some(Direction::Forward),
            ..Default::default()
        },
        StepEffects {
            glide: Some(Glide {
                target: 72,
                semitones_per_second: 12.0,
            }),
            ..Default::default()
        },
    ] {
        f.put(
            0,
            0,
            Cell {
                effects,
                ..trigger(60, 1)
            },
        );
        assert!(f.song.validate().is_err());
    }
    f.put(0, 0, trigger(60, 1));
    let before = f.song.clone();
    assert!(f.song.remove_phrase(PhraseId(1)).is_err());
    assert_eq!(f.song, before);
    assert!(f.song.set_phrase_cell(PhraseId(1), 4, 0, note(60)).is_err());
    assert_eq!(f.song, before);
}

#[test]
fn phrase_start_instance_and_expansion_budgets_are_enforced() {
    let mut f = Fixture::new(257);
    f.phrase(1, 256);
    f.pput(0, 0, note(60));
    for row in 0..257 {
        f.put(row, 0, trigger(60, 1));
    }
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("256 phrase trigger"));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 128;
    f.phrase(128, 256);
    f.song.phrases.get_mut(&PhraseId(1)).unwrap().note_columns = 16;
    for row in 0..128 {
        for column in 0..16 {
            f.pput(row, column, note(60));
        }
    }
    f.put(0, 0, trigger(60, 1));
    assert_eq!(starts(&f.evaluate()).len(), 2048);
    f.put(0, 0, trigger(60, 2));
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("2048 note triggers"));
    f.phrase(1024, 256);
    f.song.phrases.get_mut(&PhraseId(1)).unwrap().note_columns = 16;
    for row in 0..1024 {
        for column in 0..16 {
            f.pput(
                row,
                column,
                Cell {
                    velocity: Some(0.5),
                    ..Default::default()
                },
            );
        }
    }
    f.pput(0, 0, note(60));
    f.put(0, 0, trigger(60, 64));
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("65536 expanded"));
}

#[test]
fn phrase_local_parameter_memory_is_not_compounded_with_trigger_gain() {
    let mut f = Fixture::new(8);
    f.phrase(2, 8);
    f.pput(0, 0, note(60));
    f.pput(
        1,
        0,
        Cell {
            velocity: Some(0.5),
            pan: Some(0.2),
            ..Default::default()
        },
    );
    f.put(
        0,
        0,
        Cell {
            velocity: Some(0.5),
            pan: Some(0.1),
            ..trigger(60, 3)
        },
    );
    let on = starts(&f.evaluate());
    assert_eq!(
        on.iter().map(|e| e.2).collect::<Vec<_>>(),
        [0.5, 0.25, 0.25]
    );
    assert_eq!(on.iter().map(|e| e.3).collect::<Vec<_>>(), [0.1, 0.3, 0.3]);
}

#[test]
fn owner_stop_at_trigger_time_cancels_children_and_preserves_no_actual_notes_error() {
    for cut in [false, true] {
        let mut f = Fixture::new(4);
        f.phrase(2, 8);
        f.pput(0, 0, note(60));
        f.put(
            0,
            0,
            Cell {
                effects: if cut {
                    StepEffects {
                        cut: Some(0.0),
                        ..Default::default()
                    }
                } else {
                    StepEffects {
                        release: Some(0.0),
                        ..Default::default()
                    }
                },
                ..trigger(60, 1)
            },
        );
        let output = f.dir.join("canceled.wav");
        assert!(song_render::export(&f.song, None, &output)
            .unwrap_err()
            .contains("contains no note triggers"));
        assert!(!output.exists());
        assert!(!output.with_extension("events.json").exists());
        f.put(
            0,
            1,
            Cell {
                instrument: Some(InstrumentId(1)),
                ..note(60)
            },
        );
        let m = f.evaluate();
        assert!(m.events.iter().all(|event| event.phrase.is_none()));
        assert_eq!(starts(&m).len(), 1);
    }
}

#[test]
fn whole_song_loops_reset_phrase_instances_and_stop_silences_queued_pcm() {
    let mut f = Fixture::new(4);
    f.phrase(2, 8);
    f.pput(0, 0, note(60));
    f.pput(1, 0, note(67));
    f.put(0, 0, trigger(60, 2));
    let baseline = f.render();
    let musical_frames = f.evaluate().musical_frames as usize;
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    shared.set_loop(true, musical_frames as u64);
    let mut audio = Vec::new();
    while audio.len() < musical_frames * 4 {
        producer.pump_until_buffered().unwrap();
        let mut block = [0.0; 512];
        shared.fill(&mut block);
        audio.extend(block);
    }
    assert_eq!(
        &audio[..musical_frames * 2],
        &baseline[..musical_frames * 2]
    );
    assert_eq!(
        &audio[musical_frames * 2..musical_frames * 4],
        &baseline[..musical_frames * 2]
    );
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut silence = [1.0; 512];
    shared.fill(&mut silence);
    assert!(silence.iter().all(|sample| *sample == 0.0));
}

#[test]
fn trigger_owned_hard_cut_stops_release_tails_after_the_phrase_clock_ends() {
    let mut f = Fixture::new(1);
    f.song.lines_per_beat = 1;
    f.song.bpm = 120.0;
    f.sample(|_| 0.1);
    f.phrase(1, 256);
    f.pput(0, 0, note(60));
    let mut cell = trigger(60, 1);
    cell.effects.cut = Some(0.005);
    f.put(0, 0, cell.clone());
    let m = f.evaluate();
    assert!(m.events.iter().any(|e| e.frame == 94
        && matches!(
            e.command,
            Command::Off {
                reason: Release::PhraseEnd,
                voice: 1
            }
        )));
    assert!(m.events.iter().any(|e| e.frame == 120
        && e.phrase.is_some_and(|p| p.instance == 1)
        && matches!(e.command, Command::Cut { voice: 1 })));
    let pcm = f.render();
    assert!(pcm[94 * 2..120 * 2].iter().any(|x| *x > 0.01));
    assert!(pcm[120 * 2..].iter().all(|x| *x == 0.0));

    // A late release does not restart a release envelope or become a cut.
    cell.effects.cut = None;
    cell.effects.release = Some(0.005);
    f.put(0, 0, cell);
    let m = f.evaluate();
    assert_eq!(
        m.events
            .iter()
            .filter(|e| matches!(e.command, Command::Off { .. }))
            .count(),
        1
    );
    assert!(!m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::Cut { .. })));
    assert!(f.render()[120 * 2] > 0.01);
}
