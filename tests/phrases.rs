use aqua_pattern_core::{
    editor::{Editor, Field},
    project::{self, SaveMode},
    song::*,
    song_render::{self, Command, Engine, Release},
    transport::{Producer, Shared},
    RATE,
};
use std::{
    collections::BTreeMap,
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
            "leviathan-m7-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let sample = dir.join("tone.wav");
        let mut w = hound::WavWriter::create(
            &sample,
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..RATE * 3 {
            let v = (i as f32 * std::f32::consts::TAU * 220.0 / RATE as f32).sin() * 0.15;
            w.write_sample(v).unwrap();
            w.write_sample(v * 0.8).unwrap();
        }
        w.finalize().unwrap();
        let mut song = example_song();
        song.order.truncate(1);
        for p in song.patterns.values_mut() {
            p.rows = 8;
            p.tracks.clear();
        }
        for i in song.instruments.values_mut() {
            i.sample = Some(sample.clone());
        }
        let mut phrase = Phrase {
            rows: 4,
            lines_per_beat: 8,
            ..Default::default()
        };
        phrase.cells.insert(
            0,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(60),
                    velocity: Some(0.4),
                    pan: Some(0.3),
                    ..Default::default()
                },
            )]),
        );
        phrase.cells.insert(
            1,
            BTreeMap::from([(
                0,
                Cell {
                    velocity: Some(0.2),
                    ..Default::default()
                },
            )]),
        );
        phrase.cells.insert(
            2,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(64),
                    ..Default::default()
                },
            )]),
        );
        phrase.cells.insert(
            3,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Off,
                    ..Default::default()
                },
            )]),
        );
        song.instruments
            .get_mut(&InstrumentId(1))
            .unwrap()
            .phrases
            .insert(PhraseId(1), phrase);
        put(
            &mut song,
            0,
            0,
            Cell {
                note: Note::Note(72),
                instrument: Some(InstrumentId(1)),
                phrase: Some(PhraseId(1)),
                velocity: Some(0.5),
                pan: Some(-0.2),
                ..Default::default()
            },
        );
        Self { dir, song }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn render(&self, name: &str) -> (Vec<f32>, song_render::Manifest) {
        let p = self.path(name);
        let (_, m) = song_render::export(&self.song, None, &p).unwrap();
        (audio(&p), m)
    }
    fn phrase(&mut self) -> &mut Phrase {
        self.song
            .instruments
            .get_mut(&InstrumentId(1))
            .unwrap()
            .phrases
            .get_mut(&PhraseId(1))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn audio(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect()
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
fn trigger(song: &mut Song) -> &mut Cell {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
}
fn onsets(m: &song_render::Manifest) -> Vec<(u64, u8)> {
    m.events
        .iter()
        .filter_map(|e| {
            if let Command::On { pitch, .. } = e.command {
                Some((e.frame, pitch))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn phrase_matches_an_independently_authored_flat_song_with_transposition_and_attributes() {
    let mut f = Fixture::new();
    let (actual, m) = f.render("phrase.wav");
    assert_eq!(m.schema, 3);
    assert_eq!(onsets(&m), vec![(0, 72), (6000, 76)]);
    assert!(m.events.iter().any(|e| e.frame == 3000
        && matches!(e.command,Command::Volume{value,..} if (value-0.1).abs()<1e-6)));
    assert!(m.events.iter().all(|e| e
        .phrase
        .as_ref()
        .is_some_and(|p| p.instrument == InstrumentId(1)
            && p.phrase == PhraseId(1)
            && p.trigger_column == 0)));
    f.song.lines_per_beat = 8;
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 16;
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    put(
        &mut f.song,
        0,
        0,
        Cell {
            note: Note::Note(72),
            instrument: Some(InstrumentId(1)),
            velocity: Some(0.2),
            pan: Some(0.3 - 0.2),
            effects: StepEffects {
                offset: Some(0.0),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        1,
        0,
        Cell {
            velocity: Some(0.1),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        0,
        Cell {
            note: Note::Note(76),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        3,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let (expected, _) = f.render("flat.wav");
    assert_eq!(actual, expected);
}
#[test]
fn natural_phrase_end_releases_one_shot_voices_before_song_end() {
    let mut f = Fixture::new();
    f.phrase().cells.retain(|row, _| *row == 0);
    let (sound, m) = f.render("end.wav");
    assert_eq!(onsets(&m), vec![(0, 72)]);
    assert!(m.events.iter().any(|e| e.frame == 12000
        && matches!(
            e.command,
            Command::Off {
                reason: Release::PhraseEnd,
                ..
            }
        )));
    assert!(sound[24000 * 2..].iter().all(|v| v.abs() < 1e-7));
    // Natural end uses the sampler's ordinary release, including DSP buffering.
    // Verify against an independently authored explicit OFF at the same frame.
    let c = trigger(&mut f.song);
    c.phrase = None;
    c.velocity = Some(0.2);
    c.pan = Some(0.3 - 0.2);
    c.effects.offset = Some(0.0);
    put(
        &mut f.song,
        2,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let (reference, _) = f.render("explicit-off.wav");
    assert_eq!(sound, reference);
}
#[test]
fn parent_off_and_replacement_cancel_future_phrase_events() {
    let mut f = Fixture::new();
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let (_, m) = f.render("off.wav");
    assert_eq!(onsets(&m), vec![(0, 72)]);
    assert!(m.events.iter().any(|e| e.frame == 6000
        && matches!(
            e.command,
            Command::Off {
                reason: Release::ExplicitOff,
                ..
            }
        )));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .remove(&1);
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    let (_, m) = f.render("sample.wav");
    assert_eq!(onsets(&m), vec![(0, 72), (6000, 60)]);
    assert!(m.events.iter().any(|e| e.frame == 6000
        && matches!(
            e.command,
            Command::Off {
                reason: Release::Replaced,
                ..
            }
        )));
}
#[test]
fn retriggered_phrases_restart_and_never_leak_the_previous_schedule() {
    let mut f = Fixture::new();
    f.phrase().lines_per_beat = 4;
    for row in 0..4 {
        f.phrase().cells.insert(
            row,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(60 + row as u8),
                    ..Default::default()
                },
            )]),
        );
    }
    put(
        &mut f.song,
        2,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            phrase: Some(PhraseId(1)),
            ..Default::default()
        },
    );
    let (_, m) = f.render("restart.wav");
    assert_eq!(
        onsets(&m),
        vec![
            (0, 72),
            (6000, 73),
            (12000, 60),
            (18000, 61),
            (24000, 62),
            (30000, 63)
        ]
    );
    let ids: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On { voice, .. } = e.command {
                Some(voice)
            } else {
                None
            }
        })
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), unique.len());
}
#[test]
fn outer_cut_silences_phrase_polyphony_and_tails_without_cutting_other_root_columns() {
    let mut f = Fixture::new();
    f.phrase().note_columns = 2;
    f.phrase().cells.get_mut(&0).unwrap().insert(
        1,
        Cell {
            note: Note::Note(67),
            ..Default::default()
        },
    );
    trigger(&mut f.song).effects.cut = Some(0.5);
    let (sound, m) = f.render("cut.wav");
    assert_eq!(onsets(&m), vec![(0, 72), (0, 79)]);
    assert!(sound[3000 * 2..].iter().all(|v| *v == 0.0));
    put(
        &mut f.song,
        0,
        1,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            phrase: Some(PhraseId(1)),
            ..Default::default()
        },
    );
    let (mixed, _) = f.render("other-column.wav");
    assert!(mixed[4000 * 2..5000 * 2].iter().any(|v| v.abs() > 0.01));
    trigger(&mut f.song).effects.cut = Some(0.0);
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .remove(&1);
    let (sound, _) = f.render("zero-cut.wav");
    assert!(sound.iter().all(|v| *v == 0.0));
}
#[test]
fn delayed_phrase_timing_uses_absolute_rounding_at_fractional_tempo() {
    let mut f = Fixture::new();
    f.song.bpm = 137.3;
    f.song.lines_per_beat = 7;
    trigger(&mut f.song).delay = Some(0.37);
    f.phrase().lines_per_beat = 11;
    let (_, m) = f.render("fractional.wav");
    let origin: f64 = 0.37 * 48000.0 * 60.0 / (137.3 * 7.0);
    let row: f64 = 48000.0 * 60.0 / (137.3 * 11.0);
    assert_eq!(
        onsets(&m),
        vec![
            (origin.round() as u64, 72),
            ((origin + 2.0 * row).round() as u64, 76)
        ]
    );
}
#[test]
fn phrase_can_cross_pattern_boundaries_and_is_truncated_at_song_end() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1;
    f.song.order.push(Occurrence::new(PatternId(2)));
    f.song.patterns.get_mut(&PatternId(2)).unwrap().rows = 1;
    f.phrase().lines_per_beat = 4;
    let (_, m) = f.render("boundary.wav");
    assert_eq!(onsets(&m), vec![(0, 72)]);
    assert!(!m
        .events
        .iter()
        .any(|e| e.frame == 6000 && matches!(e.command, Command::Off { .. })));
    assert!(m.events.iter().any(|e| e.frame == 12000
        && matches!(
            e.command,
            Command::Off {
                reason: Release::SongEnd,
                ..
            }
        )));
    assert!(m.events.iter().all(|e| e.frame <= 12000));
}
#[test]
fn phrase_effects_are_shared_with_the_step_evaluator() {
    let mut f = Fixture::new();
    let c = f.phrase().cells.get_mut(&0).unwrap().get_mut(&0).unwrap();
    c.effects = StepEffects {
        retrigger: Some(4),
        cut: Some(0.7),
        reverse: true,
        offset: Some(0.25),
        ..Default::default()
    };
    let (_, m) = f.render("effects.wav");
    assert_eq!(&onsets(&m)[..3], &[(0, 72), (750, 72), (1500, 72)]);
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == 2100 && matches!(e.command, Command::Cut)));
    let c = f.phrase().cells.get_mut(&2).unwrap().get_mut(&0).unwrap();
    c.effects.glide = Some(24.0);
    // Cut removed the prior voice, so glide starts normally (the same cold-voice policy).
    f.render("glide-fallback.wav");
    f.phrase()
        .cells
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .effects = StepEffects::default();
    let (_, m) = f.render("glide.wav");
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::Speed { pitch: 76, .. })));
}
#[test]
fn phrases_are_owned_by_instruments_and_omitted_selection_is_resolved() {
    let mut f = Fixture::new();
    let mut second = f.phrase().clone();
    second.cells.get_mut(&0).unwrap().get_mut(&0).unwrap().note = Note::Note(65);
    f.song
        .instruments
        .get_mut(&InstrumentId(2))
        .unwrap()
        .phrases
        .insert(PhraseId(1), second);
    put(
        &mut f.song,
        4,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            phrase: Some(PhraseId(1)),
            ..Default::default()
        },
    );
    let (_, m) = f.render("owners.wav");
    assert!(m.events.iter().any(|e| e.frame == 24000
        && e.instrument == InstrumentId(2)
        && matches!(e.command, Command::On { pitch: 65, .. })));
    let mut start = trigger(&mut f.song).clone();
    start.phrase = None;
    start.note = Note::Empty;
    put(&mut f.song, 0, 0, start);
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Note(60),
            phrase: Some(PhraseId(1)),
            ..Default::default()
        },
    );
    f.render("inherited.wav");
    f.song
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .phrases
        .clear();
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("has no phrase"));
}
#[test]
fn phrase_model_rejects_recursion_bad_bounds_and_unsupported_trigger_effects() {
    let mut f = Fixture::new();
    for invalid in [
        Phrase {
            rows: 0,
            ..Default::default()
        },
        Phrase {
            note_columns: 17,
            ..Default::default()
        },
        Phrase {
            root_note: 128,
            ..Default::default()
        },
        Phrase {
            lines_per_beat: 0,
            ..Default::default()
        },
    ] {
        *f.phrase() = invalid;
        assert!(f.song.validate().is_err());
    }
    *f.phrase() = Phrase::default();
    for cell in [
        Cell {
            note: Note::Note(60),
            phrase: Some(PhraseId(1)),
            ..Default::default()
        },
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
        Cell {
            velocity: Some(f32::NAN),
            ..Default::default()
        },
    ] {
        f.phrase().cells = BTreeMap::from([(0, BTreeMap::from([(0, cell)]))]);
        assert!(f.song.validate().is_err());
    }
    f.phrase().cells.clear();
    trigger(&mut f.song).effects.reverse = true;
    assert!(f.song.validate().is_err());
    trigger(&mut f.song).effects = StepEffects::default();
    trigger(&mut f.song).note = Note::Off;
    assert!(f.song.validate().is_err());
}
#[test]
fn phrase_transposition_out_of_range_fails_without_outputs() {
    let mut f = Fixture::new();
    trigger(&mut f.song).note = Note::Note(127);
    let error = song_render::export(&f.song, None, &f.path("bad.wav")).unwrap_err();
    assert!(error.contains("transposes outside"));
    assert!(!f.path("bad.wav").exists());
    assert!(!f.path("bad.events.json").exists());
}
#[test]
fn phrase_editor_history_reference_guards_and_project_roundtrip() {
    let f = Fixture::new();
    let mut editor = Editor::default();
    editor.replace_document(f.song.clone()).unwrap();
    assert!(editor
        .add_phrase(InstrumentId(1), Phrase::default())
        .is_err());
    editor.edit_mode = true;
    let before = editor.song.clone();
    let id = editor
        .add_phrase(InstrumentId(1), Phrase::default())
        .unwrap();
    assert_eq!(id, PhraseId(2));
    editor.undo();
    assert_eq!(editor.song, before);
    editor.redo();
    editor
        .set_phrase_cell(
            InstrumentId(1),
            id,
            5,
            0,
            Cell {
                note: Note::Note(64),
                ..Default::default()
            },
        )
        .unwrap();
    let edited = editor.song.clone();
    let mut shrink = editor.song.instruments[&InstrumentId(1)].phrases[&id].clone();
    shrink.rows = 4;
    assert!(editor.update_phrase(InstrumentId(1), id, shrink).is_err());
    assert_eq!(editor.song, edited);
    assert!(editor.delete_phrase(InstrumentId(1), PhraseId(1)).is_err());
    editor.delete_phrase(InstrumentId(1), id).unwrap();
    editor.undo();
    assert_eq!(editor.song, edited);
    editor.field = Field::Phrase;
    editor.enter_value("2").unwrap();
    assert_eq!(editor.cell().phrase, Some(id));
    editor.clear(false).unwrap();
    assert_eq!(editor.cell().phrase, None);
    editor.undo();
    project::save(&editor.song, &f.path("song.json"), SaveMode::CreateNew).unwrap();
    let (opened, _) = project::load(&f.path("song.json")).unwrap();
    assert_eq!(opened, editor.song);
    let (_, a) = song_render::export(&opened, None, &f.path("opened.wav")).unwrap();
    let (_, b) = song_render::export(&editor.song, None, &f.path("original.wav")).unwrap();
    assert_eq!(a, b);
    assert_eq!(audio(&f.path("opened.wav")), audio(&f.path("original.wav")));
}
#[test]
fn old_projects_default_to_empty_phrase_pools_and_no_trigger_selector() {
    let mut song = example_song();
    for i in song.instruments.values_mut() {
        i.phrases.clear();
    }
    let value = serde_json::to_value(&song).unwrap();
    assert!(value.pointer("/instruments/1/phrases").is_none());
    let restored: Song = serde_json::from_value(value).unwrap();
    assert_eq!(restored, song);
}
#[test]
fn phrase_streaming_matches_offline_and_seek_loop_stop_do_not_recreate_earlier_notes() {
    let f = Fixture::new();
    let (expected, m) = f.render("offline.wav");
    let state = Arc::new(Shared::default());
    let mut p = Producer::new(&f.song, None, state.clone()).unwrap();
    let mut got = Vec::new();
    for _ in 0..1000 {
        for _ in 0..8 {
            p.pump().unwrap();
        }
        let mut out = [0.0; 512];
        state.fill(&mut out);
        got.extend_from_slice(&out);
        if state.ended.load(Ordering::Relaxed) {
            break;
        }
    }
    assert!(state.ended.load(Ordering::Relaxed));
    assert_eq!(&got[..expected.len()], expected);
    let state = Arc::new(Shared::default());
    state.looping.store(true, Ordering::SeqCst);
    let mut p = Producer::new(&f.song, None, state.clone()).unwrap();
    let mut loops = Vec::new();
    while loops.len() < (m.musical_frames * 4) as usize {
        for _ in 0..8 {
            p.pump().unwrap();
        }
        let mut out = [0.0; 512];
        state.fill(&mut out);
        loops.extend_from_slice(&out);
    }
    let length = m.musical_frames as usize * 2;
    assert_eq!(&loops[..length], &loops[length..length * 2]);
    state.seek(7000);
    for _ in 0..8 {
        p.pump().unwrap();
    }
    let mut out = [1.0; 512];
    state.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
    state.stop();
    state.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(5000).unwrap();
    let mut buf = [0.0; 1024];
    while engine.position < 6000 {
        let n = engine.block(&mut buf, 6000).unwrap();
        assert!(buf[..n * 2].iter().all(|v| *v == 0.0));
    }
    engine.block(&mut buf, m.musical_frames).unwrap();
    assert!(buf.iter().any(|v| v.abs() > 0.001));
}
#[test]
fn phrase_launch_and_expanded_note_limits_are_bounded() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 257;
    for row in 1..257 {
        put(
            &mut f.song,
            row,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                phrase: Some(PhraseId(1)),
                ..Default::default()
            },
        );
    }
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("256 phrase launches"));
    let mut f = Fixture::new();
    f.phrase().rows = 256;
    f.phrase().note_columns = 16;
    f.phrase().cells.clear();
    for row in 0..256 {
        for column in 0..16 {
            f.phrase().cells.entry(row).or_default().insert(
                column,
                Cell {
                    note: Note::Note(60),
                    ..Default::default()
                },
            );
        }
    }
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("2048 note triggers"));
}

#[test]
fn phrase_row_end_cut_kills_tails_and_precedes_a_new_launch() {
    let mut f = Fixture::new();
    f.phrase().rows = 1;
    f.phrase().lines_per_beat = 4;
    let c = f.phrase().cells.get_mut(&0).unwrap().get_mut(&0).unwrap();
    c.effects.cut = Some(1.0);
    f.phrase().cells.retain(|row, _| *row == 0);
    let (sound, _) = f.render("natural-cut.wav");
    assert!(sound[6000 * 2..].iter().all(|v| *v == 0.0));
    trigger(&mut f.song).effects.cut = Some(1.0);
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            phrase: Some(PhraseId(1)),
            ..Default::default()
        },
    );
    let (sound, m) = f.render("cut-restart.wav");
    assert_eq!(onsets(&m), vec![(0, 72), (6000, 60)]);
    let at: Vec<_> = m
        .events
        .iter()
        .filter(|e| e.frame == 6000)
        .map(|e| &e.command)
        .collect();
    let last_on = at
        .iter()
        .rposition(|c| matches!(c, Command::On { .. }))
        .unwrap();
    assert!(at[..last_on].iter().any(|c| matches!(c, Command::Cut)));
    assert!(sound[6200 * 2..7000 * 2].iter().any(|v| v.abs() > 0.001));
}
#[test]
fn final_frame_delay_is_clamped_and_cut_after_an_empty_phrase_still_kills_old_tails() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1;
    trigger(&mut f.song).delay = Some(0.999999);
    let (_, m) = f.render("last.wav");
    assert_eq!(onsets(&m), vec![(5999, 72)]);
    let mut f = Fixture::new();
    f.song.instruments.get_mut(&InstrumentId(2)).unwrap().sample = None;
    f.song
        .instruments
        .get_mut(&InstrumentId(2))
        .unwrap()
        .phrases
        .insert(PhraseId(1), Phrase::default());
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            phrase: Some(PhraseId(1)),
            effects: StepEffects {
                cut: Some(0.1),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let (sound, _) = f.render("empty-cut.wav");
    assert!(sound[6600 * 2..].iter().all(|v| *v == 0.0));
}

#[test]
fn note_budget_is_global_across_individually_valid_phrase_launches() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 129;
    f.phrase().rows = 16;
    f.phrase().lines_per_beat = 256;
    f.phrase().cells.clear();
    for row in 0..16 {
        f.phrase().cells.insert(
            row,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(60),
                    ..Default::default()
                },
            )]),
        );
    }
    for row in 0..129 {
        put(
            &mut f.song,
            row,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                phrase: Some(PhraseId(1)),
                ..Default::default()
            },
        );
    }
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("2048 expanded note triggers"));
}
#[test]
fn phrase_pool_limit_and_unknown_json_fields_do_not_mutate_the_document() {
    let f = Fixture::new();
    let mut editor = Editor::default();
    editor.replace_document(f.song.clone()).unwrap();
    editor.edit_mode = true;
    for _ in 1..64 {
        editor
            .add_phrase(InstrumentId(1), Phrase::default())
            .unwrap();
    }
    let before = editor.song.clone();
    assert!(editor
        .add_phrase(InstrumentId(1), Phrase::default())
        .is_err());
    assert_eq!(editor.song, before);
    let mut value = serde_json::to_value(&f.song).unwrap();
    value["instruments"]["1"]["phrases"]["1"]["unknown"] = true.into();
    assert!(serde_json::from_value::<Song>(value).is_err());
}
