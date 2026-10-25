use aqua_pattern_core::{
    editor::{effects_label, parse_effects, Editor, Field},
    project::{self, SaveMode},
    song::*,
    song_render::{self, Command, Engine, Release},
    transport::{Producer, Shared},
    RATE,
};
use std::{
    path::{Path, PathBuf},
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
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-m6-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("sample.wav");
        let samples: Vec<[f32; 2]> = (0..RATE * 3)
            .map(|i| {
                let value =
                    (i as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() as f32 * 0.2;
                [value, value * 0.7]
            })
            .collect();
        write_sample(&path, &samples);
        let mut song = example_song();
        song.order.truncate(1);
        for pattern in song.patterns.values_mut() {
            pattern.rows = 16;
            pattern.tracks.clear();
        }
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some(path.clone());
        }
        put(&mut song, 0, 0, note(60));
        Self { dir, song }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn render(&self, name: &str) -> (Vec<f32>, song_render::Manifest) {
        let path = self.path(name);
        let (_, manifest) = song_render::export(&self.song, None, &path).unwrap();
        (read_sample(&path), manifest)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn write_sample(path: &Path, frames: &[[f32; 2]]) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in frames {
        for &v in frame {
            writer.write_sample(v).unwrap();
        }
    }
    writer.finalize().unwrap();
}
fn read_sample(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect()
}
fn note(pitch: u8) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.5),
        ..Default::default()
    }
}
fn put(song: &mut Song, row: u32, column: u16, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(column, cell);
}
fn cell(song: &mut Song, row: u32) -> &mut Cell {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&row)
        .unwrap()
        .get_mut(&0)
        .unwrap()
}
#[test]
fn effect_model_legacy_loading_validation_and_project_roundtrip() {
    let mut f = Fixture::new();
    let legacy = serde_json::to_value(&f.song).unwrap();
    assert!(legacy.pointer("/patterns/1/tracks/1/0/0/effects").is_none());
    assert_eq!(serde_json::from_value::<Song>(legacy).unwrap(), f.song);
    cell(&mut f.song, 0).effects =
        parse_effects("retrigger=4 cut=0.7 offset=0.25 reverse").unwrap();
    project::save(&f.song, &f.path("fx.json"), SaveMode::CreateNew).unwrap();
    let (reopened, _) = project::load(&f.path("fx.json")).unwrap();
    assert_eq!(reopened, f.song);
    let (_, before) = f.render("first.wav");
    let (_, after) = song_render::export(&reopened, None, &f.path("reopened.wav")).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        read_sample(&f.path("first.wav")),
        read_sample(&f.path("reopened.wav"))
    );
    assert_eq!(before.schema, 2);
    let mut invalid = serde_json::to_value(&f.song).unwrap();
    invalid["patterns"]["1"]["tracks"]["1"]["0"]["0"]["effects"]["typo"] = true.into();
    assert!(serde_json::from_value::<Song>(invalid).is_err());
    for effects in [
        StepEffects {
            retrigger: Some(1),
            ..Default::default()
        },
        StepEffects {
            retrigger: Some(17),
            ..Default::default()
        },
        StepEffects {
            cut: Some(f64::NAN),
            ..Default::default()
        },
        StepEffects {
            cut: Some(1.1),
            ..Default::default()
        },
        StepEffects {
            offset: Some(1.0),
            ..Default::default()
        },
        StepEffects {
            offset: Some(-0.1),
            ..Default::default()
        },
        StepEffects {
            glide: Some(0.0),
            ..Default::default()
        },
        StepEffects {
            glide: Some(f32::INFINITY),
            ..Default::default()
        },
        parse_effects("glide=12 reverse").unwrap(),
        parse_effects("glide=12 retrigger=4").unwrap(),
    ] {
        cell(&mut f.song, 0).effects = effects;
        assert!(f.song.validate().is_err());
    }
    // Validation also covers unused pattern content.
    cell(&mut f.song, 0).effects = StepEffects::default();
    f.song.patterns.get_mut(&PatternId(2)).unwrap().tracks =
        f.song.patterns[&PatternId(1)].tracks.clone();
    f.song
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .effects
        .cut = Some(-1.0);
    assert!(f.song.validate().is_err());
}
#[test]
fn effect_editing_is_guarded_atomic_and_undoable() {
    let mut editor = Editor {
        field: Field::Effects,
        ..Default::default()
    };
    assert!(editor.enter_value("reverse").is_err());
    editor.edit_mode = true;
    let original = editor.song.clone();
    editor
        .enter_value("retrigger=4 cut=0.5 offset=0.25 reverse")
        .unwrap();
    let edited = editor.song.clone();
    let effects = editor.cell().effects;
    assert_eq!(parse_effects(&effects_label(&effects)).unwrap(), effects);
    for bad in [
        "oops=1",
        "retrigger=4 retrigger=5",
        "reverse=false",
        "offset=NaN",
        "cut=2",
        "glide=12 reverse",
    ] {
        assert!(editor.enter_value(bad).is_err(), "{bad}");
        assert_eq!(editor.song, edited);
    }
    editor.undo();
    assert_eq!(editor.song, original);
    editor.redo();
    assert_eq!(editor.song, edited);
    editor.clear(false).unwrap();
    assert!(editor.cell().effects.is_empty());
    editor.undo();
    assert_eq!(editor.song, edited);
    editor.enter_value("").unwrap();
    assert!(editor.cell().effects.is_empty());
    editor.enter_value("cut=0.5").unwrap();
    editor.field = Field::Note;
    editor.clear(false).unwrap(); // Cut is meaningful on an empty cell.
    assert_eq!(editor.cell().note, Note::Empty);
    assert_eq!(editor.cell().effects.cut, Some(0.5));
}
#[test]
fn retriggers_and_cut_use_absolute_fractional_timing_after_delay() {
    let mut f = Fixture::new();
    f.song.bpm = 137.3;
    f.song.lines_per_beat = 7;
    cell(&mut f.song, 0).delay = Some(0.2);
    cell(&mut f.song, 0).effects = parse_effects("retrigger=4 cut=0.7").unwrap();
    let (_, m) = f.render("timing.wav");
    let row: f64 = 48000.0 * 60.0 / (137.3 * 7.0);
    let attacks: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| matches!(e.command, Command::On { .. }).then_some(e.frame))
        .collect();
    assert_eq!(attacks, [0.2, 0.4, 0.6].map(|v| (v * row).round() as u64));
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == (0.76 * row).round() as u64 && matches!(e.command, Command::Cut)));
    assert_eq!(
        m.events
            .iter()
            .filter(|e| matches!(
                e.command,
                Command::Off {
                    reason: Release::Retrigger,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(m.events.windows(2).all(|e| e[0].frame <= e[1].frame));
}
#[test]
fn cut_silences_all_retrigger_release_tails_at_the_exact_frame() {
    let mut f = Fixture::new();
    cell(&mut f.song, 0).effects = parse_effects("retrigger=4 cut=0.6").unwrap();
    let (audio, _) = f.render("cut.wav");
    assert!(audio[..3600 * 2].iter().any(|v| v.abs() > 0.01));
    assert!(audio[3600 * 2..].iter().all(|v| *v == 0.0));
    cell(&mut f.song, 0).effects = parse_effects("cut=0").unwrap();
    let (audio, _) = f.render("zero.wav");
    assert!(audio.iter().all(|v| *v == 0.0));
    cell(&mut f.song, 0).delay = Some(0.999999);
    let (audio, _) = f.render("late-zero.wav");
    assert!(audio.iter().all(|v| *v == 0.0));
}
#[test]
fn cut_after_off_clears_the_release_without_touching_other_columns() {
    let mut f = Fixture::new();
    put(&mut f.song, 0, 1, note(67));
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Off,
            delay: Some(0.9),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        0,
        Cell {
            effects: parse_effects("cut=0").unwrap(),
            ..Default::default()
        },
    );
    let (mixed, m) = f.render("mixed.wav");
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == 12000 && matches!(e.command, Command::Cut)));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .retain(|_, columns| {
            columns.remove(&0);
            !columns.is_empty()
        });
    // Keep the isolated renderer in the reference without changing source/audio.
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .effects
        .offset = Some(0.0);
    let (other, _) = f.render("other.wav");
    assert_eq!(&mixed[12000 * 2..], &other[12000 * 2..]);
    assert!(other[12000 * 2..14000 * 2].iter().any(|v| v.abs() > 0.01));
}
#[test]
fn offset_and_reverse_match_independently_prepared_stereo_sources() {
    let mut f = Fixture::new();
    let frames: Vec<[f32; 2]> = (0..48000)
        .map(|i| {
            [
                ((i as f32 / 137.0).sin() * 0.1) + i as f32 / 480000.0,
                ((i as f32 / 83.0).cos() * 0.08) - i as f32 / 960000.0,
            ]
        })
        .collect();
    write_sample(&f.path("asymmetric.wav"), &frames);
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.path("asymmetric.wav"));
    for (index, reverse) in [false, true].into_iter().enumerate() {
        cell(&mut f.song, 0).effects = StepEffects {
            offset: Some(0.25),
            reverse,
            ..Default::default()
        };
        let (actual, _) = f.render(&format!("actual-{index}.wav"));
        // Oracle is a separately written WAV of the expected original source frames.
        let expected: Vec<_> = if reverse {
            frames[..36000].iter().rev().copied().collect()
        } else {
            frames[12000..].to_vec()
        };
        write_sample(&f.path("prepared.wav"), &expected);
        f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.path("prepared.wav"));
        cell(&mut f.song, 0).effects = StepEffects {
            offset: Some(0.0),
            ..Default::default()
        };
        let (reference, _) = f.render(&format!("expected-{index}.wav"));
        assert_eq!(actual, reference);
        f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
            Some(f.path("asymmetric.wav"));
    }
}
fn crossings(audio: &[f32], start: usize, end: usize) -> usize {
    (start + 1..end)
        .filter(|i| audio[(i - 1) * 2] <= 0.0 && audio[i * 2] > 0.0)
        .count()
}
#[test]
fn glide_reuses_the_voice_and_moves_pitch_gradually_to_target() {
    let mut f = Fixture::new();
    let mut target = note(72);
    target.instrument = None;
    target.effects.glide = Some(24.0);
    put(&mut f.song, 2, 0, target);
    let (audio, m) = f.render("glide.wav");
    assert_eq!(
        m.events
            .iter()
            .filter(|e| matches!(e.command, Command::On { .. }))
            .count(),
        1
    );
    assert!(m.events.iter().any(|e| e.frame == 12000
        && matches!(
            e.command,
            Command::Speed {
                voice: 1,
                pitch: 72,
                rate: 24.0
            }
        )));
    let initial = crossings(&audio, 2400, 7200);
    let early = crossings(&audio, 13200, 18000);
    let late = crossings(&audio, 38400, 43200);
    assert!((43..=45).contains(&initial), "{initial}");
    assert!(
        early > initial && early < 70,
        "early={early} initial={initial}"
    );
    assert!((86..=90).contains(&late), "late={late}");
    // A glide without a prior voice starts its pitched note normally.
    cell(&mut f.song, 0).effects.glide = Some(12.0);
    f.render("first-glide.wav");
    cell(&mut f.song, 2).instrument = Some(InstrumentId(2));
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("sounding instrument"));
}
#[test]
fn row_end_cut_precedes_the_next_occurrence_attack_and_effects_do_not_latch() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1;
    f.song.order.push(Occurrence::new(PatternId(2)));
    f.song.patterns.get_mut(&PatternId(2)).unwrap().rows = 2;
    f.song.patterns.get_mut(&PatternId(2)).unwrap().tracks =
        f.song.patterns[&PatternId(1)].tracks.clone();
    cell(&mut f.song, 0).effects = parse_effects("reverse offset=0.25 cut=1").unwrap();
    let (audio, m) = f.render("boundary.wav");
    let at: Vec<_> = m
        .events
        .iter()
        .filter(|e| e.frame == 6000)
        .map(|e| &e.command)
        .collect();
    assert!(matches!(at[0], Command::Cut));
    assert!(matches!(
        at[1],
        Command::On {
            offset: 0.0,
            reverse: false,
            ..
        }
    ));
    assert!(audio[6200 * 2..7000 * 2].iter().any(|v| v.abs() > 0.01));
}
#[test]
fn invalid_combinations_and_subframe_retriggers_fail_before_writing_outputs() {
    let mut f = Fixture::new();
    for note in [Note::Empty, Note::Off] {
        cell(&mut f.song, 0).note = note;
        cell(&mut f.song, 0).effects = parse_effects("reverse").unwrap();
        assert!(song_render::export(&f.song, None, &f.path("invalid.wav")).is_err());
        assert!(!f.path("invalid.wav").exists());
        assert!(!f.path("invalid.events.json").exists());
    }
    cell(&mut f.song, 0).note = Note::Note(60);
    cell(&mut f.song, 0).delay = Some(0.99999);
    cell(&mut f.song, 0).effects = parse_effects("retrigger=16").unwrap();
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("spacing"));
}
#[test]
fn expanded_trigger_budget_is_enforced() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 129;
    for row in 0..129 {
        let mut c = note(60);
        c.effects.retrigger = Some(16);
        put(&mut f.song, row, 0, c);
    }
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("2048 note triggers"));
}
#[test]
fn streamed_effects_match_offline_pcm_and_loop_seek_stop_remain_defined() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 8;
    cell(&mut f.song, 0).effects =
        parse_effects("retrigger=4 cut=0.8 offset=0.25 reverse").unwrap();
    let mut n = note(60);
    n.effects.offset = Some(0.5);
    put(&mut f.song, 2, 0, n);
    let mut n = note(67);
    n.instrument = None;
    n.effects.glide = Some(48.0);
    put(&mut f.song, 3, 0, n);
    put(
        &mut f.song,
        5,
        0,
        Cell {
            effects: parse_effects("cut=0.3").unwrap(),
            ..Default::default()
        },
    );
    let (expected, m) = f.render("offline.wav");
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut got = Vec::new();
    for _ in 0..2000 {
        for _ in 0..8 {
            producer.pump().unwrap();
        }
        let mut out = [0.0; 512];
        shared.fill(&mut out);
        got.extend_from_slice(&out);
        if shared.ended.load(Ordering::Relaxed) {
            break;
        }
    }
    assert!(shared.ended.load(Ordering::Relaxed));
    assert_eq!(&got[..expected.len()], expected);
    let shared = Arc::new(Shared::default());
    shared.looping.store(true, Ordering::SeqCst);
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut loops = Vec::new();
    while loops.len() < (m.musical_frames * 4) as usize {
        for _ in 0..8 {
            producer.pump().unwrap();
        }
        let mut out = [0.0; 512];
        shared.fill(&mut out);
        loops.extend_from_slice(&out);
    }
    let length = m.musical_frames as usize * 2;
    assert_eq!(&loops[..length], &loops[length..length * 2]);
    // Seek into the glide: pre-seek voices are not reconstructed.
    shared.seek(20000);
    for _ in 0..8 {
        producer.pump().unwrap();
    }
    let mut out = [1.0; 512];
    shared.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
    shared.stop();
    shared.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
    let mut engine = Engine::new(&f.song, None).unwrap();
    // A pending retrigger is a real future onset after a cold seek.
    engine.cold_seek(2000).unwrap();
    let mut buffer = [0.0; 1024];
    while engine.position < 3000 {
        let count = engine.block(&mut buffer, 3000).unwrap();
        assert!(buffer[..count * 2].iter().all(|v| *v == 0.0));
    }
    engine.block(&mut buffer, m.musical_frames).unwrap();
    assert!(buffer.iter().any(|v| v.abs() > 0.001));
}

#[test]
fn excessive_sample_variants_are_rejected_before_output_reservation() {
    let mut f = Fixture::new();
    write_sample(&f.path("short.wav"), &vec![[0.1, -0.05]; 1024]);
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.path("short.wav"));
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 257;
    for row in 0..257 {
        let mut n = note(60);
        n.effects.offset = Some(row as f64 / 512.0);
        put(&mut f.song, row, 0, n);
    }
    let error = song_render::export(&f.song, None, &f.path("variants.wav")).unwrap_err();
    assert!(error.contains("256 source variants"), "{error}");
    assert!(!f.path("variants.wav").exists());
    assert!(!f.path("variants.events.json").exists());
}

#[test]
fn simultaneous_voice_limit_cleans_partial_export_outputs() {
    let mut f = Fixture::new();
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for column in 0..256 {
        let mut n = note(60);
        n.velocity = Some(0.0001);
        n.effects.offset = Some(0.0);
        put(&mut f.song, 0, column, n);
    }
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(2))
        .or_default()
        .entry(0)
        .or_default()
        .insert(0, note(60));
    let error = song_render::export(&f.song, None, &f.path("voices.wav")).unwrap_err();
    assert!(error.contains("256 simultaneous voices"), "{error}");
    assert!(!f.path("voices.wav").exists());
    assert!(!f.path("voices.events.json").exists());
}
