use aqua_pattern_core::{
    editor::Editor,
    generation::{self, Recipe, WriteMode},
    project::{self, SaveMode},
    song::*,
    song_render::{self, Command},
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
            "leviathan-m10-{}-{}",
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
    fn alias(&mut self) {
        self.song
            .patterns
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
fn note_at(cells: &TrackPattern, row: u32, column: u16) -> Note {
    cells
        .get(&row)
        .and_then(|c| c.get(&column))
        .map_or(Note::Empty, |c| c.note)
}
#[test]
fn euclidean_grid_sequence_and_off_have_independent_expected_rows() {
    let recipe = Recipe::default();
    let output = generation::generate(&recipe).unwrap();
    assert_eq!(
        output.cells.keys().copied().collect::<Vec<_>>(),
        vec![0, 3, 6, 8, 11, 14]
    );
    assert_eq!(
        [0, 3, 6, 8, 11, 14].map(|r| note_at(&output.cells, r, 0)),
        [
            Note::Note(60),
            Note::Note(63),
            Note::Note(67),
            Note::Off,
            Note::Note(60),
            Note::Note(63)
        ]
    );
    assert_eq!(output.notes, 5);
    assert_eq!(output.offs, 1);
    let cell = &output.cells[&0][&0];
    assert_eq!(cell.instrument, Some(InstrumentId(1)));
    assert_eq!(cell.velocity, Some(0.4));
    assert_eq!(cell.pan, Some(0.0));
    assert_eq!(output.cells[&8][&0].instrument, None);
}
#[test]
fn seed_repeats_exactly_and_distinct_seeds_change_probability_decisions() {
    let recipe = Recipe {
        rows: 128,
        hits: 8,
        probability: 0.5,
        seed: 9876,
        ..Default::default()
    };
    let a = generation::generate(&recipe).unwrap();
    assert_eq!(a, generation::generate(&recipe).unwrap());
    assert!(a.notes > 0 && a.notes < 128);
    let b = generation::generate(&Recipe {
        seed: 9877,
        ..recipe.clone()
    })
    .unwrap();
    assert_ne!(a.cells, b.cells);
    let silent = generation::generate(&Recipe {
        probability: 0.0,
        ..recipe.clone()
    })
    .unwrap();
    assert!(silent.cells.is_empty());
    let no_hits = generation::generate(&Recipe { hits: 0, ..recipe }).unwrap();
    assert!(no_hits.cells.is_empty());
}
#[test]
fn positive_and_negative_rotation_and_step_rows_map_to_exact_rows() {
    let recipe = Recipe {
        rows: 16,
        step_rows: 2,
        rotation: 3,
        ..Default::default()
    };
    let output = generation::generate(&recipe).unwrap();
    assert_eq!(
        output.cells.keys().copied().collect::<Vec<_>>(),
        vec![0, 6, 10]
    );
    let recipe = Recipe {
        rotation: -3,
        ..recipe
    };
    let output = generation::generate(&recipe).unwrap();
    assert_eq!(
        output.cells.keys().copied().collect::<Vec<_>>(),
        vec![2, 6, 12]
    );
}
#[test]
fn chords_shrink_with_explicit_offs_while_rest_tokens_sustain() {
    let recipe = Recipe {
        rows: 4,
        cycle_steps: 4,
        hits: 4,
        sequence: generation::parse_sequence("60,64 67 . off").unwrap(),
        ..Default::default()
    };
    let output = generation::generate(&recipe).unwrap();
    assert_eq!(output.columns, 2);
    assert_eq!(note_at(&output.cells, 0, 1), Note::Note(64));
    assert_eq!(note_at(&output.cells, 1, 1), Note::Off);
    assert!(!output.cells.contains_key(&2));
    assert_eq!(note_at(&output.cells, 3, 0), Note::Off);
    assert_eq!(
        generation::parse_sequence(&generation::sequence_text(&recipe.sequence)).unwrap(),
        recipe.sequence
    );
}
#[test]
fn invalid_recipes_reject_before_generation_including_revision_and_resource_bounds() {
    let mut recipes = Vec::new();
    for rows in [0, 1025] {
        recipes.push(Recipe {
            rows,
            ..Default::default()
        });
    }
    recipes.extend([
        Recipe {
            schema: 2,
            ..Default::default()
        },
        Recipe {
            revision: "other".into(),
            ..Default::default()
        },
        Recipe {
            cycle_steps: 0,
            ..Default::default()
        },
        Recipe {
            hits: 9,
            ..Default::default()
        },
        Recipe {
            step_rows: 0,
            ..Default::default()
        },
        Recipe {
            rotation: i16::MIN,
            ..Default::default()
        },
        Recipe {
            probability: f32::NAN,
            ..Default::default()
        },
        Recipe {
            velocity: 1.1,
            ..Default::default()
        },
        Recipe {
            pan: f32::INFINITY,
            ..Default::default()
        },
        Recipe {
            instrument: InstrumentId(0),
            ..Default::default()
        },
        Recipe {
            sequence: vec![],
            ..Default::default()
        },
        Recipe {
            sequence: vec![vec![Note::Note(11)]],
            ..Default::default()
        },
        Recipe {
            sequence: vec![vec![Note::Note(128)]],
            ..Default::default()
        },
        Recipe {
            sequence: vec![vec![Note::Off; 9]],
            ..Default::default()
        },
        Recipe {
            sequence: vec![vec![Note::Off]; 65],
            ..Default::default()
        },
    ]);
    for recipe in recipes {
        assert!(generation::generate(&recipe).is_err());
    }
    let dense = Recipe {
        rows: 1024,
        hits: 8,
        sequence: vec![vec![Note::Note(60); 8]],
        ..Default::default()
    };
    assert!(generation::generate(&dense).unwrap_err().contains("2048"));
    for text in ["", "11", "128", "bad", "60,,64", "60,"] {
        assert!(generation::parse_sequence(text).is_err());
    }
    assert!(generation::parse_sequence(&"60 ".repeat(1000)).is_err());
}
#[test]
fn preview_is_pure_and_commit_cells_and_recipe_is_one_saved_state_undo_step() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.song.clone();
    let recipe = Recipe {
        rows: 4,
        hits: 8,
        ..Default::default()
    };
    let preview = e
        .preview_generation("Motif", &recipe, WriteMode::Replace)
        .unwrap();
    assert_eq!(e.song, original);
    assert!(preview.recipe_changed);
    assert!(preview.summary.changed_cells > 0);
    e.apply_generation(preview).unwrap();
    assert_eq!(e.song.generation_recipes["Motif"], recipe);
    let changed = e.song.clone();
    e.mark_saved();
    e.undo();
    assert_eq!(e.song, original);
    assert!(e.dirty);
    e.redo();
    assert_eq!(e.song, changed);
    assert!(!e.dirty);
    let p = e
        .preview_generation("Motif", &recipe, WriteMode::Replace)
        .unwrap();
    assert!(!p.recipe_changed);
    assert_eq!(p.summary.changed_cells, 0);
    e.apply_generation(p).unwrap();
    e.undo();
    assert_eq!(e.song, original);
}
#[test]
fn merge_refuses_collision_replace_clears_only_selected_rectangle() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.song.clone();
    let recipe = Recipe {
        rows: 4,
        hits: 8,
        ..Default::default()
    };
    assert!(e
        .preview_generation("Motif", &recipe, WriteMode::Merge)
        .is_err());
    assert_eq!(e.song, original);
    e.column = 1;
    let p = e
        .preview_generation("Motif", &recipe, WriteMode::Merge)
        .unwrap();
    e.apply_generation(p).unwrap();
    assert_eq!(
        e.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0],
        original.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0]
    );
    let silent = Recipe { hits: 0, ..recipe };
    let p = e
        .preview_generation("Silent", &silent, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    assert_eq!(
        note_at(&e.song.patterns[&PatternId(1)].tracks[&TrackId(1)], 0, 1),
        Note::Empty
    );
    assert_eq!(
        note_at(&e.song.patterns[&PatternId(1)].tracks[&TrackId(1)], 0, 0),
        Note::Note(60)
    );
}
#[test]
fn cursor_offset_and_fit_errors_are_atomic() {
    let f = Fixture::new();
    let mut e = f.editor();
    e.row = 2;
    e.column = 1;
    let recipe = Recipe {
        rows: 2,
        hits: 8,
        ..Default::default()
    };
    let p = e
        .preview_generation("Offset", &recipe, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    let cells = &e.song.patterns[&PatternId(1)].tracks[&TrackId(1)];
    assert_eq!(note_at(cells, 2, 1), Note::Note(60));
    assert_eq!(note_at(cells, 3, 1), Note::Note(63));
    assert_eq!(note_at(cells, 1, 1), Note::Empty);
    let before = e.song.clone();
    for recipe in [
        Recipe {
            rows: 3,
            ..recipe.clone()
        },
        Recipe {
            sequence: vec![vec![Note::Note(60), Note::Note(64)]],
            ..recipe.clone()
        },
        Recipe {
            instrument: InstrumentId(999),
            ..recipe.clone()
        },
    ] {
        assert!(e
            .preview_generation("Invalid", &recipe, WriteMode::Replace)
            .is_err());
        assert_eq!(e.song, before);
    }
    e.row = u32::MAX;
    assert!(e
        .preview_generation("Invalid", &recipe, WriteMode::Replace)
        .is_err());
}
#[test]
fn alias_generation_detaches_preserving_source_and_reports_downstream_repeats() {
    let mut f = Fixture::new();
    f.alias();
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .aliases
        .insert(
            TrackId(3),
            BlockRef {
                pattern: PatternId(2),
                track: TrackId(2),
            },
        );
    let mut e = f.editor();
    e.select(1, TrackId(2)).unwrap();
    let source = e.song.patterns[&PatternId(1)].tracks[&TrackId(1)].clone();
    let recipe = Recipe {
        rows: 4,
        hits: 8,
        ..Default::default()
    };
    let p = e
        .preview_generation("Aliased", &recipe, WriteMode::Replace)
        .unwrap();
    assert_eq!(p.summary.detached_aliases, 1);
    assert_eq!(p.summary.outside_scope_positions, vec![0, 2]);
    e.apply_generation(p).unwrap();
    assert_eq!(e.song.patterns[&PatternId(1)].tracks[&TrackId(1)], source);
    assert!(!e.song.patterns[&PatternId(2)]
        .aliases
        .contains_key(&TrackId(2)));
    e.select(0, TrackId(1)).unwrap();
    let p = e
        .preview_generation("Shared", &recipe, WriteMode::Replace)
        .unwrap();
    assert!(p.summary.outside_scope_positions.contains(&2));
}
#[test]
fn stale_preview_and_edit_guard_cannot_mutate_song_or_save_recipe() {
    let f = Fixture::new();
    let mut e = f.editor();
    let recipe = Recipe {
        rows: 4,
        ..Default::default()
    };
    let p = e
        .preview_generation("Test", &recipe, WriteMode::Replace)
        .unwrap();
    e.edit_mode = false;
    assert!(e.apply_generation(p).is_err());
    assert_eq!(e.song, f.song);
    e.edit_mode = true;
    let p = e
        .preview_generation("Test", &recipe, WriteMode::Replace)
        .unwrap();
    e.mark_section("Change").unwrap();
    let before = e.song.clone();
    assert!(e.apply_generation(p).is_err());
    assert_eq!(e.song, before);
}
#[test]
fn recipe_limits_validation_removal_and_json_backwards_compatibility() {
    let f = Fixture::new();
    let mut e = f.editor();
    let old = serde_json::to_string(&e.song).unwrap();
    assert!(!old.contains("generation_recipes"));
    let recipe = Recipe {
        rows: 4,
        ..Default::default()
    };
    for i in 0..32 {
        e.song
            .generation_recipes
            .insert(format!("Recipe{i}"), recipe.clone());
    }
    e.song.validate().unwrap();
    assert!(e
        .preview_generation("Extra", &recipe, WriteMode::Replace)
        .is_err());
    assert!(e
        .preview_generation("", &recipe, WriteMode::Replace)
        .is_err());
    assert!(e
        .preview_generation(&"x".repeat(65), &recipe, WriteMode::Replace)
        .is_err());
    let cells = e.song.patterns.clone();
    e.remove_generation_recipe("Recipe0").unwrap();
    assert_eq!(e.song.patterns, cells);
    e.undo();
    assert_eq!(e.song.generation_recipes.len(), 32);
    e.song
        .generation_recipes
        .get_mut("Recipe0")
        .unwrap()
        .revision = "wrong".into();
    assert!(e.song.validate().is_err());
    let mut data = serde_json::to_value(Recipe::default()).unwrap();
    data["script"] = serde_json::json!("unused");
    assert!(serde_json::from_value::<Recipe>(data).is_err());
}
#[test]
fn generated_audio_equals_independently_authored_cells_and_is_tempo_independent() {
    let mut f = Fixture::new();
    let mut e = f.editor();
    let recipe = Recipe {
        rows: 4,
        cycle_steps: 4,
        hits: 4,
        sequence: generation::parse_sequence("60 off 67 off").unwrap(),
        ..Default::default()
    };
    let p = e
        .preview_generation("Four", &recipe, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    f.song = e.song;
    let (actual, m) = f.render("generated.wav");
    let onsets: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| matches!(e.command, Command::On { .. }).then_some(e.frame))
        .collect();
    assert_eq!(onsets, vec![0, 12000, 48000, 60000]);
    f.song.generation_recipes.clear();
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([
                (
                    0,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Note(60),
                            instrument: Some(InstrumentId(1)),
                            velocity: Some(0.4),
                            pan: Some(0.0),
                            ..Default::default()
                        },
                    )]),
                ),
                (
                    1,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Off,
                            ..Default::default()
                        },
                    )]),
                ),
                (
                    2,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Note(67),
                            instrument: Some(InstrumentId(1)),
                            velocity: Some(0.4),
                            pan: Some(0.0),
                            ..Default::default()
                        },
                    )]),
                ),
                (
                    3,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Off,
                            ..Default::default()
                        },
                    )]),
                ),
            ]),
        );
    let (expected, reference) = f.render("authored.wav");
    assert_eq!(actual, expected);
    assert_eq!(m, reference);
    f.song.bpm = 137.25;
    f.song.lines_per_beat = 7;
    let e = f.editor();
    let p = e
        .preview_generation("Four", &recipe, WriteMode::Replace)
        .unwrap();
    assert_eq!(p.summary.changed_cells, 0);
}
#[test]
fn saved_recipe_regenerates_after_edit_and_roundtrip_with_equal_streaming_audio() {
    let mut f = Fixture::new();
    let mut e = f.editor();
    let recipe = Recipe {
        rows: 4,
        hits: 8,
        probability: 0.75,
        seed: 42,
        ..Default::default()
    };
    let p = e
        .preview_generation("Seeded", &recipe, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    let generated = e.song.patterns.clone();
    let path = f.dir.join("song.json");
    project::save(&e.song, &path, SaveMode::CreateNew).unwrap();
    let (opened, _) = project::load(&path).unwrap();
    assert_eq!(opened, e.song);
    e.replace_document(opened.clone()).unwrap();
    e.edit_mode = true;
    e.set_cell(Cell {
        note: Note::Note(72),
        instrument: Some(InstrumentId(1)),
        ..Default::default()
    })
    .unwrap();
    let saved = e.song.generation_recipes["Seeded"].clone();
    let p = e
        .preview_generation("Seeded", &saved, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    assert_eq!(e.song.patterns, generated);
    f.song = opened;
    let (expected, _) = f.render("offline.wav");
    let state = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, state.clone()).unwrap();
    let mut got = Vec::new();
    for _ in 0..2000 {
        for _ in 0..8 {
            producer.pump().unwrap();
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
}

#[test]
fn exact_regeneration_of_an_alias_preserves_sharing_and_history() {
    let mut f = Fixture::new();
    let mut e = f.editor();
    let recipe = Recipe {
        rows: 4,
        hits: 8,
        ..Default::default()
    };
    let p = e
        .preview_generation("Exact", &recipe, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    f.song = e.song;
    f.alias();
    let mut e = f.editor();
    e.select(1, TrackId(2)).unwrap();
    let before = e.song.clone();
    let p = e
        .preview_generation("Exact", &recipe, WriteMode::Replace)
        .unwrap();
    assert_eq!(p.summary.changed_cells, 0);
    assert_eq!(p.summary.detached_aliases, 0);
    assert!(!p.recipe_changed);
    e.apply_generation(p).unwrap();
    assert_eq!(e.song, before);
    assert!(!e.can_undo());
}
#[test]
fn partial_alias_replacement_preserves_other_rows_columns_and_source_effects() {
    let mut f = Fixture::new();
    let outside = Cell {
        note: Note::Note(72),
        instrument: Some(InstrumentId(1)),
        effects: StepEffects {
            reverse: true,
            cut: Some(0.5),
            ..Default::default()
        },
        ..Default::default()
    };
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .insert(3, BTreeMap::from([(1, outside.clone())]));
    f.alias();
    let source = f.song.patterns[&PatternId(1)].tracks[&TrackId(1)].clone();
    let mut e = f.editor();
    e.select(1, TrackId(2)).unwrap();
    e.row = 1;
    let recipe = Recipe {
        rows: 2,
        hits: 8,
        ..Default::default()
    };
    let p = e
        .preview_generation("Partial", &recipe, WriteMode::Replace)
        .unwrap();
    e.apply_generation(p).unwrap();
    let block = &e.song.patterns[&PatternId(2)].tracks[&TrackId(2)];
    assert_eq!(block[&3][&1], outside);
    assert_eq!(block[&0][&0], source[&0][&0]);
    assert_eq!(e.song.patterns[&PatternId(1)].tracks[&TrackId(1)], source);
}
