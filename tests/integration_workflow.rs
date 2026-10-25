use aqua_pattern_core::{
    editor::{Editor, Field},
    generation::{Recipe, WriteMode},
    session::{Action, Session},
    song::*,
    song_render::{self, Command},
    transform::{Scope, Transform},
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
    song: Song,
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-integration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("tone.wav");
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..RATE * 3 {
            let v = (i as f32 * std::f32::consts::TAU * 220.0 / RATE as f32).sin() * 0.12;
            w.write_sample(v).unwrap();
            w.write_sample(v * 0.8).unwrap();
        }
        w.finalize().unwrap();
        let mut song = example_song();
        for p in song.patterns.values_mut() {
            p.rows = 4;
            p.tracks.clear();
        }
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some(path.clone());
        }
        song.patterns.get_mut(&PatternId(1)).unwrap().tracks.insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        ..Default::default()
                    },
                )]),
            )]),
        );
        Self { song, dir }
    }
    fn render(&self, name: &str) -> (Vec<f32>, song_render::Manifest) {
        let path = self.dir.join(name);
        let (_, m) = song_render::export(&self.song, None, &path).unwrap();
        let samples = hound::WavReader::open(path)
            .unwrap()
            .into_samples::<f32>()
            .map(Result::unwrap)
            .collect();
        (samples, m)
    }
    fn editor(&self) -> Editor {
        let mut e = Editor::default();
        e.replace_document(self.song.clone()).unwrap();
        e.edit_mode = true;
        e
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
#[test]
fn all_milestones_share_one_document_through_history_project_render_and_transport() {
    let mut f = Fixture::new();
    f.song.patterns.insert(
        PatternId(99),
        Pattern {
            name: "Short coda".into(),
            rows: 2,
            tracks: f.song.patterns[&PatternId(1)].tracks.clone(),
            aliases: BTreeMap::new(),
        },
    );
    // Model and central editing: variable lengths, distinct columns and typed FX.
    let mut e = f.editor();
    e.edit_step = 0;
    e.field = Field::Effects;
    e.enter_value("retrigger=2 offset=0.1 reverse cut=0.8")
        .unwrap();
    let mut phrase = Phrase {
        rows: 8,
        lines_per_beat: 8,
        note_columns: 2,
        ..Default::default()
    };
    phrase.cells.insert(
        0,
        BTreeMap::from([
            (
                0,
                Cell {
                    note: Note::Note(60),
                    velocity: Some(0.4),
                    ..Default::default()
                },
            ),
            (
                1,
                Cell {
                    note: Note::Note(67),
                    velocity: Some(0.2),
                    ..Default::default()
                },
            ),
        ]),
    );
    phrase.cells.insert(
        2,
        BTreeMap::from([(
            0,
            Cell {
                note: Note::Note(64),
                effects: StepEffects {
                    retrigger: Some(2),
                    cut: Some(0.6),
                    ..Default::default()
                },
                ..Default::default()
            },
        )]),
    );
    let phr = e.add_phrase(InstrumentId(2), phrase).unwrap();
    e.select(0, TrackId(2)).unwrap();
    e.set_cell(Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(2)),
        phrase: Some(phr),
        velocity: Some(0.5),
        ..Default::default()
    })
    .unwrap();
    e.mark_section("Verse").unwrap();
    // An alias routes phrase playback to another track; its mute is occurrence-local.
    e.select(1, TrackId(3)).unwrap();
    e.alias_block(BlockRef {
        pattern: PatternId(1),
        track: TrackId(2),
    })
    .unwrap();
    e.mute_track(true).unwrap();
    e.mark_section("Muted bridge").unwrap();
    e.select(2, TrackId(1)).unwrap();
    e.mark_section("Return").unwrap();
    e.append_section(true).unwrap();
    let copy = e.pattern_id();
    assert_ne!(copy, PatternId(1));
    // Generation + groove + transpose + further manual edits use the same history.
    e.select(3, TrackId(4)).unwrap();
    let recipe = Recipe {
        rows: 4,
        cycle_steps: 4,
        hits: 4,
        sequence: vec![
            vec![Note::Note(60), Note::Note(67)],
            vec![Note::Off, Note::Off],
        ],
        instrument: InstrumentId(4),
        velocity: 0.2,
        ..Default::default()
    };
    let p = e
        .preview_generation("Return chords", &recipe, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Groove(vec![0.0, 0.25]))
        .unwrap();
    e.apply_transform(p).unwrap();
    let p = e
        .preview_transform(Scope::Column, 0..4, &Transform::Transpose(2))
        .unwrap();
    e.apply_transform(p).unwrap();
    let before_manual = e.song.clone();
    e.field = Field::Velocity;
    e.enter_value("0.15").unwrap();
    e.undo();
    assert_eq!(e.song, before_manual);
    e.redo();
    assert_eq!(e.song.generation_recipes["Return chords"], recipe);
    assert!(e.song.patterns[&copy].aliases.is_empty());
    assert!(!e.song.patterns[&PatternId(1)]
        .tracks
        .contains_key(&TrackId(4))); // independent copy
    e.append(false).unwrap();
    e.assign_pattern(PatternId(99)).unwrap();
    assert_eq!(e.pattern().rows, 2);
    let snapshot = e.song.clone();
    let mut session = Session::default();
    let path = f.dir.join("integrated.json");
    session.save_request(&mut e, path.clone(), false).unwrap();
    assert!(!e.dirty);
    e.select(0, TrackId(1)).unwrap();
    e.set_cell(Cell::default()).unwrap();
    session.request(&mut e, Action::Open(path.clone())).unwrap();
    assert!(session.pending.is_some());
    session.cancel();
    assert!(e.dirty);
    session.request(&mut e, Action::Open(path)).unwrap();
    session.discard(&mut e).unwrap();
    assert_eq!(e.song, snapshot);
    assert!(!e.dirty);
    f.song = e.song.clone();
    let (pcm, manifest) = f.render("integrated.wav");
    assert_eq!(manifest.schema, 4);
    assert!(manifest.events.iter().any(|event| event.phrase.is_some()));
    assert!(manifest
        .events
        .iter()
        .any(|event| matches!(event.command, Command::On { reverse: true, .. })));
    assert!(manifest
        .events
        .iter()
        .any(|event| event.position == Some((3, 0))
            && event.track == TrackId(4)
            && matches!(event.command, Command::On { pitch: 62, .. })));
    assert!(!manifest
        .events
        .iter()
        .any(|event| event.position.is_some_and(|p| p.0 == 1)
            && event.track == TrackId(3)
            && matches!(event.command, Command::On { .. })));
    // Streaming remains a start snapshot while editor mutations continue.
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&e.song, None, shared.clone()).unwrap();
    e.edit_mode = true;
    e.select(0, TrackId(1)).unwrap();
    e.unique_block(true).unwrap();
    let mut streamed = Vec::new();
    for _ in 0..2000 {
        for _ in 0..8 {
            producer.pump().unwrap();
        }
        let mut out = [0.0; 512];
        shared.fill(&mut out);
        streamed.extend_from_slice(&out);
        if shared.ended.load(Ordering::Relaxed) {
            break;
        }
    }
    assert!(shared.ended.load(Ordering::Relaxed));
    assert_eq!(&streamed[..pcm.len()], pcm);
    // Loop/cold seek/Stop operate on that same combined document.
    let shared = Arc::new(Shared::default());
    shared.looping.store(true, Ordering::SeqCst);
    let mut producer = Producer::new(&snapshot, None, shared.clone()).unwrap();
    let length = manifest.musical_frames as usize * 2;
    let mut looping = Vec::new();
    while looping.len() < length * 2 {
        for _ in 0..8 {
            producer.pump().unwrap();
        }
        let mut out = [0.0; 512];
        shared.fill(&mut out);
        looping.extend_from_slice(&out);
    }
    assert_eq!(&looping[..length], &looping[length..length * 2]);
    shared.seek(6000);
    for _ in 0..8 {
        producer.pump().unwrap();
    }
    let mut out = [0.0; 512];
    shared.fill(&mut out);
    assert!(!shared.ended.load(Ordering::Relaxed));
    assert!((6000..6512).contains(&shared.position.load(Ordering::Relaxed)));
    shared.stop();
    out.fill(1.0);
    shared.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
}
#[test]
fn clone_can_allocate_when_imported_pattern_uses_the_maximum_id() {
    let f = Fixture::new();
    let mut e = f.editor();
    let p = e.song.patterns[&PatternId(1)].clone();
    e.song.patterns.insert(PatternId(u64::MAX), p);
    e.append(true).unwrap();
    assert_eq!(e.pattern_id(), PatternId(3));
    e.song.validate().unwrap();
}
#[test]
fn oversized_resolved_arrangement_rejects_before_allocating_alias_variants() {
    let mut song = example_song();
    for p in song.patterns.values_mut() {
        p.rows = 4096;
        p.tracks.clear();
    }
    let block = (0..4096)
        .map(|r| {
            (
                r,
                BTreeMap::from([(
                    0,
                    Cell {
                        velocity: Some(0.5),
                        ..Default::default()
                    },
                )]),
            )
        })
        .collect();
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .insert(TrackId(1), block);
    song.patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .aliases
        .insert(
            TrackId(2),
            BlockRef {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
    song.order = vec![Occurrence::new(PatternId(2)); 9];
    song.validate().unwrap();
    let error = song_render::evaluate(&song, None).unwrap_err();
    assert!(error.contains("32768 populated cells after arrangement"));
}
