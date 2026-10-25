use aqua_pattern_core::{
    editor::Editor,
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
            "leviathan-m8-{}-{}",
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
#[test]
fn alias_audio_equals_independently_materialized_content_and_routes_to_destination() {
    let mut f = Fixture::new();
    f.alias();
    let (actual, m) = f.render("alias.wav");
    assert_eq!(m.schema, 4);
    assert!(!m.arrangement.as_ref().unwrap().isolated_voices);
    assert!(m
        .events
        .iter()
        .any(|e| e.track == TrackId(2) && matches!(e.command, Command::On { .. })));
    let source = f.song.patterns[&PatternId(1)].tracks[&TrackId(1)].clone();
    let p = f.song.patterns.get_mut(&PatternId(2)).unwrap();
    p.aliases.clear();
    p.tracks.insert(TrackId(2), source);
    let (expected, base) = f.render("local.wav");
    assert_eq!(actual, expected);
    assert_eq!(m.events, base.events);
    assert_eq!(base.schema, 1);
}
#[test]
fn alias_chain_validates_unused_blocks_cycles_dangling_and_shape_mismatches() {
    let mut f = Fixture::new();
    f.alias();
    f.song
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .aliases
        .insert(
            TrackId(3),
            BlockRef {
                pattern: PatternId(2),
                track: TrackId(2),
            },
        );
    assert_eq!(
        f.song.block_source(PatternId(2), TrackId(3)).unwrap(),
        BlockRef {
            pattern: PatternId(1),
            track: TrackId(1)
        }
    );
    let valid = f.song.clone();
    for source in [
        BlockRef {
            pattern: PatternId(2),
            track: TrackId(3),
        },
        BlockRef {
            pattern: PatternId(999),
            track: TrackId(1),
        },
        BlockRef {
            pattern: PatternId(1),
            track: TrackId(999),
        },
    ] {
        f.song = valid.clone();
        f.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .remove(&TrackId(1));
        f.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .aliases
            .insert(TrackId(1), source);
        assert!(f.song.validate().is_err());
    }
    f.song = valid.clone();
    f.song.patterns.get_mut(&PatternId(2)).unwrap().rows = 8;
    assert!(f.song.validate().is_err());
    f.song = valid.clone();
    f.song.tracks.get_mut(&TrackId(2)).unwrap().note_columns = 1;
    assert!(f.song.validate().is_err());
    f.song = valid;
    f.song
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .insert(TrackId(2), BTreeMap::new());
    assert!(f.song.validate().is_err());
}
#[test]
fn editing_alias_updates_source_repeats_and_undo_then_unique_detaches() {
    let mut f = Fixture::new();
    f.alias();
    let mut e = f.editor();
    e.select(1, TrackId(2)).unwrap();
    let original = e.song.clone();
    let mut cell = e.cell();
    cell.velocity = Some(0.3);
    e.set_cell(cell.clone()).unwrap();
    e.select(0, TrackId(1)).unwrap();
    assert_eq!(e.cell(), cell);
    e.undo();
    assert_eq!(e.song, original);
    e.redo();
    e.select(1, TrackId(2)).unwrap();
    e.unique_block(false).unwrap();
    let detached = e.cell();
    let mut changed = detached.clone();
    changed.note = Note::Note(72);
    e.set_cell(changed).unwrap();
    e.select(0, TrackId(1)).unwrap();
    assert_eq!(e.cell(), detached);
    e.select(1, TrackId(2)).unwrap();
    e.unique_block(true).unwrap();
    assert_eq!(e.cell(), Cell::default());
    assert!(!e.song.patterns[&PatternId(1)].tracks.is_empty());
}
#[test]
fn independent_pattern_clone_materializes_chained_aliases() {
    let mut f = Fixture::new();
    f.alias();
    f.song.clone_pattern(PatternId(2), PatternId(3)).unwrap();
    assert!(f.song.patterns[&PatternId(3)].aliases.is_empty());
    let copied = f.song.patterns[&PatternId(3)].tracks[&TrackId(2)].clone();
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    assert_eq!(f.song.patterns[&PatternId(3)].tracks[&TrackId(2)], copied);
}
#[test]
fn section_repeat_shares_but_variation_copies_each_pattern_once_and_preserves_mutes() {
    let mut f = Fixture::new();
    f.alias();
    let mut e = f.editor();
    e.mark_section("Verse").unwrap();
    e.select(1, TrackId(2)).unwrap();
    e.mute_track(true).unwrap();
    e.append_section(false).unwrap();
    assert_eq!(e.song.order.len(), 6);
    assert_eq!(e.song.order[3].pattern, PatternId(1));
    assert!(e.song.order[4].muted_tracks.contains(&TrackId(2)));
    assert_ne!(
        e.song.order[0].section.as_ref().unwrap().id,
        e.song.order[3].section.as_ref().unwrap().id
    );
    e.select(0, TrackId(1)).unwrap();
    e.append_section(true).unwrap();
    assert_eq!(e.song.order.len(), 9);
    let copy = e.song.order[6].pattern;
    assert_ne!(copy, PatternId(1));
    assert_eq!(e.song.order[8].pattern, copy);
    assert!(e.song.patterns[&e.song.order[7].pattern].aliases.is_empty());
    e.set_cell(Cell::default()).unwrap();
    assert!(!e.song.patterns[&PatternId(1)].tracks.is_empty());
    e.undo();
    e.undo();
    assert_eq!(e.song.order.len(), 6);
    e.redo();
    assert_eq!(e.song.order.len(), 9);
}
#[test]
fn section_ranges_follow_markers_and_invalid_edits_are_atomic() {
    let f = Fixture::new();
    let mut e = f.editor();
    assert_eq!(e.song.section_range(1).unwrap(), 0..3);
    e.select(2, TrackId(1)).unwrap();
    e.mark_section("Chorus").unwrap();
    assert_eq!(e.song.section_range(1).unwrap(), 0..2);
    assert_eq!(e.song.section_range(2).unwrap(), 2..3);
    let before = e.song.clone();
    assert!(e.mark_section(&"x".repeat(65)).is_err());
    assert_eq!(e.song, before);
    e.move_occurrence(-1).unwrap();
    assert_eq!(e.song.section_range(2).unwrap(), 1..3);
    e.mark_section("").unwrap();
    assert_eq!(e.song.section_range(2).unwrap(), 0..3);
    e.edit_mode = false;
    assert!(e.mute_track(true).is_err());
    assert!(e.unique_block(true).is_err());
    assert!(e.append_section(true).is_err());
    assert!(e
        .alias_block(BlockRef {
            pattern: PatternId(2),
            track: TrackId(1)
        })
        .is_err());
}
#[test]
fn section_and_mute_references_and_copy_limit_are_validated() {
    let mut f = Fixture::new();
    f.song.order[0].section = Some(Section {
        id: SectionId(1),
        name: "Verse".into(),
    });
    f.song.order[1].section = f.song.order[0].section.clone();
    assert!(f.song.validate().is_err());
    f.song.order[1].section = None;
    f.song.order[1].muted_tracks.insert(TrackId(99));
    assert!(f.song.validate().is_err());
    f.song.order[1].muted_tracks.clear();
    f.song.order.resize(4096, Occurrence::new(PatternId(1)));
    let mut e = f.editor();
    let before = e.song.clone();
    assert!(e.append_section(false).is_err());
    assert!(e.append(false).is_err());
    assert_eq!(e.song, before);
}
#[test]
fn muted_occurrence_cuts_sustained_audio_and_unmute_requires_a_new_note() {
    let mut f = Fixture::new();
    f.song.order[1].muted_tracks.insert(TrackId(1));
    let (sound, m) = f.render("mute.wav");
    assert!(sound[..48000].iter().any(|v| *v != 0.0));
    assert!(sound[48000..96000].iter().all(|v| *v == 0.0));
    assert!(sound[96000..144000].iter().any(|v| *v != 0.0));
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == 24000 && e.track == TrackId(1) && e.command == Command::Cut));
    f.song.order[2] = Occurrence::new(PatternId(2));
    let (sound, _) = f.render("no-resume.wav");
    assert!(sound[48000..].iter().all(|v| *v == 0.0));
}
#[test]
fn mute_removes_phrase_future_events_and_polyphonic_release_tails_at_boundary() {
    let mut f = Fixture::new();
    let mut phrase = Phrase {
        rows: 16,
        note_columns: 2,
        ..Default::default()
    };
    for row in [0, 6, 10] {
        phrase.cells.insert(
            row,
            BTreeMap::from([
                (
                    0,
                    Cell {
                        note: Note::Note(60),
                        ..Default::default()
                    },
                ),
                (
                    1,
                    Cell {
                        note: Note::Note(67),
                        ..Default::default()
                    },
                ),
            ]),
        );
    }
    f.song
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .phrases
        .insert(PhraseId(1), phrase);
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .phrase = Some(PhraseId(1));
    f.song.order[1].muted_tracks.insert(TrackId(1));
    let (sound, m) = f.render("phrase-mute.wav");
    assert!(sound[48000..96000].iter().all(|v| *v == 0.0));
    assert!(!m
        .events
        .iter()
        .any(|e| (24000..48000).contains(&e.frame) && matches!(e.command, Command::On { .. })));
}
#[test]
fn all_muted_song_exports_silence_without_loading_unassigned_samples() {
    let mut f = Fixture::new();
    for o in &mut f.song.order {
        o.muted_tracks.extend(f.song.track_order.iter().copied());
    }
    for i in f.song.instruments.values_mut() {
        i.sample = None;
    }
    let (sound, m) = f.render("silent.wav");
    assert!(sound.iter().all(|v| *v == 0.0));
    assert!(m.assets.is_empty());
}
#[test]
fn section_only_keeps_audio_and_original_dsp_path_and_legacy_json_has_no_new_fields() {
    let mut f = Fixture::new();
    let old = serde_json::to_string(&f.song).unwrap();
    assert!(!old.contains("aliases"));
    assert!(!old.contains("section"));
    assert!(!old.contains("muted_tracks"));
    let (original, m) = f.render("original.wav");
    assert_eq!(m.schema, 1);
    assert!(m.arrangement.is_none());
    f.song.order[0].section = Some(Section {
        id: SectionId(1),
        name: "Verse".into(),
    });
    let (sound, m) = f.render("section.wav");
    assert_eq!(sound, original);
    assert!(!m.arrangement.unwrap().isolated_voices);
}
#[test]
fn arrangement_project_roundtrip_and_streaming_match_offline() {
    let mut f = Fixture::new();
    f.alias();
    f.song.order[1].muted_tracks.insert(TrackId(1));
    f.song.order[0].section = Some(Section {
        id: SectionId(1),
        name: "Verse".into(),
    });
    let path = f.dir.join("song.json");
    project::save(&f.song, &path, SaveMode::CreateNew).unwrap();
    let (opened, _) = project::load(&path).unwrap();
    assert_eq!(opened, f.song);
    let (expected, _) = f.render("offline.wav");
    let state = Arc::new(Shared::default());
    let mut p = Producer::new(&opened, None, state.clone()).unwrap();
    let mut got = Vec::new();
    for _ in 0..2000 {
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
    state.stop();
    let mut out = [1.0; 512];
    state.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
}

#[test]
fn alias_phrase_and_effects_match_local_rendering_with_isolated_voices() {
    let mut f = Fixture::new();
    let mut phrase = Phrase {
        rows: 4,
        ..Default::default()
    };
    phrase.cells.insert(
        0,
        BTreeMap::from([(
            0,
            Cell {
                note: Note::Note(60),
                effects: StepEffects {
                    retrigger: Some(3),
                    cut: Some(0.7),
                    reverse: true,
                    offset: Some(0.1),
                    ..Default::default()
                },
                ..Default::default()
            },
        )]),
    );
    f.song
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .phrases
        .insert(PhraseId(1), phrase);
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .phrase = Some(PhraseId(1));
    f.alias();
    let (actual, m) = f.render("phrase-alias.wav");
    assert!(m.arrangement.as_ref().unwrap().isolated_voices);
    let source = f.song.patterns[&PatternId(1)].tracks[&TrackId(1)].clone();
    let p = f.song.patterns.get_mut(&PatternId(2)).unwrap();
    p.aliases.clear();
    p.tracks.insert(TrackId(2), source);
    let (expected, base) = f.render("phrase-local.wav");
    assert_eq!(actual, expected);
    assert_eq!(m.events, base.events);
}
#[test]
fn repeated_mute_variants_have_correct_positions_and_snapshot_is_immutable() {
    let mut f = Fixture::new();
    f.song.order = vec![Occurrence::new(PatternId(1)); 4];
    f.song.order[1].muted_tracks.insert(TrackId(1));
    f.song.order[3].muted_tracks.insert(TrackId(1));
    let mut engine = song_render::Engine::new(&f.song, None).unwrap();
    let m = engine.manifest.clone();
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == 24000 && e.command == Command::Cut));
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == 72000 && e.command == Command::Cut));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    let mut out = [0.0; 1024];
    engine.block(&mut out, 512).unwrap();
    assert!(out.iter().any(|v| *v != 0.0));
    assert_eq!(engine.manifest, m);
}
#[test]
fn muted_cells_do_not_change_inherited_instrument_or_velocity() {
    let mut f = Fixture::new();
    f.song
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        instrument: Some(InstrumentId(2)),
                        velocity: Some(0.1),
                        ..Default::default()
                    },
                )]),
            )]),
        );
    f.song.order.push(Occurrence::new(PatternId(1)));
    f.song.order[1].muted_tracks.insert(TrackId(1));
    let mut p = f.song.patterns[&PatternId(1)].clone();
    let c = p
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap();
    c.instrument = None;
    c.velocity = None;
    f.song.patterns.insert(PatternId(3), p);
    f.song.order[2].pattern = PatternId(3);
    let m = song_render::evaluate(&f.song, None).unwrap();
    let event = m
        .events
        .iter()
        .find(|e| e.position == Some((2, 0)) && matches!(e.command, Command::On { .. }))
        .unwrap();
    assert_eq!(event.instrument, InstrumentId(1));
    assert!(matches!(event.command, Command::On { volume: 1.0, .. }));
}
