use aqua_pattern_core::{
    editor::Editor,
    project::{self, SaveMode},
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
            "leviathan-m9-{}-{}",
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
fn cell(song: &Song, p: u64, t: u64, r: u32, c: u16) -> Cell {
    song.block(PatternId(p), TrackId(t))
        .unwrap()
        .and_then(|b| b.get(&r))
        .and_then(|v| v.get(&c))
        .cloned()
        .unwrap_or_default()
}
fn put(e: &mut Editor, r: u32, c: u16, value: Cell) {
    e.row = r;
    e.column = c;
    e.set_cell(value).unwrap();
}
#[test]
fn preview_is_pure_apply_is_one_undo_step_and_redo_restores_saved_state() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.song.clone();
    let p = e
        .preview_transform(Scope::Song, 0..0, &Transform::Transpose(7))
        .unwrap();
    assert_eq!(e.song, original);
    assert_eq!(p.summary.changed_cells, 1);
    assert_eq!(p.summary.affected_positions, vec![0, 2]);
    e.apply_transform(p).unwrap();
    let changed = e.song.clone();
    assert_ne!(original, changed);
    assert!(e.dirty);
    e.mark_saved();
    e.undo();
    assert_eq!(e.song, original);
    assert!(e.dirty);
    e.redo();
    assert_eq!(e.song, changed);
    assert!(!e.dirty);
}
#[test]
fn column_scope_only_changes_selected_rows_and_column() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.cell();
    for r in [1, 2, 3] {
        for c in [0, 1] {
            put(&mut e, r, c, original.clone());
        }
    }
    e.column = 1;
    let p = e
        .preview_transform(Scope::Column, 1..3, &Transform::Transpose(-12))
        .unwrap();
    assert_eq!(p.summary.changed_cells, 2);
    assert_eq!(p.summary.outside_scope_positions, vec![2]);
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 1, 1).note, Note::Note(48));
    assert_eq!(cell(&e.song, 1, 1, 2, 1).note, Note::Note(48));
    assert_eq!(cell(&e.song, 1, 1, 3, 1), original);
    assert_eq!(cell(&e.song, 1, 1, 1, 0), original);
    assert_eq!(cell(&e.song, 1, 1, 0, 0), original);
}
#[test]
fn track_and_pattern_scopes_respect_other_tracks_and_rows() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.cell();
    e.track = TrackId(2);
    put(&mut e, 0, 1, original.clone());
    e.track = TrackId(1);
    let p = e
        .preview_transform(Scope::Track, 0..1, &Transform::Transpose(5))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 2, 0, 1), original);
    let p = e
        .preview_transform(Scope::Pattern, 0..4, &Transform::Transpose(2))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 0, 0).note, Note::Note(67));
    assert_eq!(cell(&e.song, 1, 2, 0, 1).note, Note::Note(62));
}
#[test]
fn section_uses_marker_range_and_song_deduplicates_repeated_patterns() {
    let f = Fixture::new();
    let mut e = f.editor();
    e.mark_section("A").unwrap();
    e.select(1, TrackId(1)).unwrap();
    e.mark_section("B").unwrap();
    let original = e.song.clone();
    let p = e
        .preview_transform(Scope::Section, 99..100, &Transform::Transpose(3))
        .unwrap();
    assert_eq!(p.summary.affected_positions, vec![0, 2]);
    assert_eq!(p.summary.outside_scope_positions, vec![0]);
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 0, 0).note, Note::Note(63));
    e.replace_document(original).unwrap();
    e.edit_mode = true;
    let p = e
        .preview_transform(Scope::Song, 99..100, &Transform::Transpose(12))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 0, 0).note, Note::Note(72));
}
#[test]
fn alias_destination_detaches_and_does_not_transpose_source_or_double_apply_in_song() {
    let mut f = Fixture::new();
    f.alias();
    let mut e = f.editor();
    e.select(1, TrackId(2)).unwrap();
    let original = cell(&e.song, 1, 1, 0, 0);
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Transpose(7))
        .unwrap();
    assert_eq!(p.summary.detached_aliases, 1);
    assert_eq!(p.summary.outside_scope_positions, Vec::<usize>::new());
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 0, 0), original);
    assert_eq!(cell(&e.song, 2, 2, 0, 0).note, Note::Note(67));
    e.undo();
    let p = e
        .preview_transform(Scope::Song, 0..4, &Transform::Transpose(12))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 0, 0).note, Note::Note(72));
    assert_eq!(cell(&e.song, 2, 2, 0, 0).note, Note::Note(72));
}
#[test]
fn preview_reports_downstream_aliases_including_detached_intermediate_blocks() {
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
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Transpose(2))
        .unwrap();
    assert_eq!(p.summary.affected_positions, vec![0, 1, 2]);
    assert_eq!(p.summary.outside_scope_positions, vec![0, 2]);
}
#[test]
fn stale_preview_and_edit_mode_guard_leave_song_unchanged() {
    let f = Fixture::new();
    let mut e = f.editor();
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Transpose(2))
        .unwrap();
    e.edit_mode = false;
    assert!(e.apply_transform(p).is_err());
    assert_eq!(e.song, f.song);
    e.edit_mode = true;
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Transpose(2))
        .unwrap();
    e.mark_section("Changed").unwrap();
    let before = e.song.clone();
    assert!(e.apply_transform(p).is_err());
    assert_eq!(e.song, before);
}
#[test]
fn numeric_overflow_and_invalid_grooves_ranges_fail_atomically() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.song.clone();
    for op in [
        Transform::Transpose(100),
        Transform::Transpose(i16::MAX),
        Transform::ScaleVelocity(f32::NAN),
        Transform::OffsetPan(f32::INFINITY),
        Transform::Groove(vec![]),
        Transform::Groove(vec![0.0; 17]),
        Transform::Groove(vec![-0.1]),
        Transform::Groove(vec![1.0]),
        Transform::Groove(vec![f64::NAN]),
    ] {
        assert!(e.preview_transform(Scope::Song, 0..4, &op).is_err());
        assert_eq!(e.song, original);
    }
    for rows in [2..2, std::ops::Range { start: 3, end: 2 }, 0..5] {
        assert!(e
            .preview_transform(Scope::Track, rows, &Transform::Transpose(1))
            .is_err());
    }
    let mut c = e.cell();
    c.velocity = Some(0.8);
    c.pan = Some(0.8);
    e.set_cell(c).unwrap();
    let before = e.song.clone();
    assert!(e
        .preview_transform(Scope::Song, 0..4, &Transform::ScaleVelocity(2.0))
        .is_err());
    assert!(e
        .preview_transform(Scope::Song, 0..4, &Transform::OffsetPan(0.3))
        .is_err());
    assert_eq!(e.song, before);
}
#[test]
fn velocity_pan_change_only_explicit_values_and_leave_notes_fx_phrases_untouched() {
    let f = Fixture::new();
    let mut e = f.editor();
    let mut c = e.cell();
    c.velocity = Some(0.6);
    c.pan = Some(-0.2);
    c.effects.reverse = true;
    e.set_cell(c).unwrap();
    let p = e
        .preview_transform(Scope::Song, 0..4, &Transform::ScaleVelocity(0.5))
        .unwrap();
    e.apply_transform(p).unwrap();
    let p = e
        .preview_transform(Scope::Song, 0..4, &Transform::OffsetPan(0.4))
        .unwrap();
    e.apply_transform(p).unwrap();
    let actual = e.cell();
    assert_eq!(actual.velocity, Some(0.3));
    assert_eq!(actual.pan, Some(0.2));
    assert!(actual.effects.reverse);
    assert_eq!(actual.note, Note::Note(60));
    put(
        &mut e,
        1,
        1,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let p = e
        .preview_transform(Scope::Song, 0..4, &Transform::ScaleVelocity(0.0))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 1, 1).velocity, None);
}
#[test]
fn groove_uses_absolute_pattern_phase_replaces_delay_and_straight_clears_it() {
    let f = Fixture::new();
    let mut e = f.editor();
    let original = e.cell();
    for r in [1, 2, 3] {
        let mut c = original.clone();
        c.delay = Some(0.1);
        put(&mut e, r, 0, c);
    }
    let p = e
        .preview_transform(
            Scope::Column,
            1..4,
            &Transform::Groove(vec![0.0, 0.25, 0.1, 0.4]),
        )
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 1, 0).delay, Some(0.25));
    assert_eq!(cell(&e.song, 1, 1, 2, 0).delay, Some(0.1));
    assert_eq!(cell(&e.song, 1, 1, 3, 0).delay, Some(0.4));
    assert_eq!(cell(&e.song, 1, 1, 0, 0).delay, None);
    let p = e
        .preview_transform(Scope::Column, 0..4, &Transform::Groove(vec![0.0]))
        .unwrap();
    e.apply_transform(p).unwrap();
    for r in 0..4 {
        assert_eq!(cell(&e.song, 1, 1, r, 0).delay, None);
    }
}
#[test]
fn reverse_rows_handles_holes_off_and_parameter_cells_and_is_an_involution() {
    let f = Fixture::new();
    let mut e = f.editor();
    put(
        &mut e,
        1,
        0,
        Cell {
            velocity: Some(0.2),
            ..Default::default()
        },
    );
    put(
        &mut e,
        2,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    put(
        &mut e,
        0,
        1,
        Cell {
            note: Note::Note(67),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    let before = e.song.clone();
    e.column = 0;
    let p = e
        .preview_transform(Scope::Column, 0..4, &Transform::ReverseRows)
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(cell(&e.song, 1, 1, 0, 0), Cell::default());
    assert_eq!(cell(&e.song, 1, 1, 1, 0).note, Note::Off);
    assert_eq!(cell(&e.song, 1, 1, 2, 0).velocity, Some(0.2));
    assert_eq!(cell(&e.song, 1, 1, 3, 0).note, Note::Note(60));
    assert_eq!(cell(&e.song, 1, 1, 0, 1), cell(&before, 1, 1, 0, 1));
    let p = e
        .preview_transform(Scope::Column, 0..4, &Transform::ReverseRows)
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(e.song, before);
}
#[test]
fn no_op_preserves_aliases_history_and_unordered_patterns() {
    let mut f = Fixture::new();
    f.alias();
    let mut e = f.editor();
    e.select(1, TrackId(2)).unwrap();
    let before = e.song.clone();
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Transpose(0))
        .unwrap();
    assert_eq!(p.summary.changed_cells, 0);
    assert_eq!(p.summary.detached_aliases, 0);
    e.apply_transform(p).unwrap();
    assert_eq!(e.song, before);
    assert!(!e.can_undo());
    e.song.order.retain(|o| o.pattern == PatternId(2));
    let unused = e.song.patterns[&PatternId(1)].clone();
    e.select(0, TrackId(2)).unwrap();
    let p = e
        .preview_transform(Scope::Song, 0..4, &Transform::Transpose(2))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(e.song.patterns[&PatternId(1)], unused);
}
#[test]
fn committed_groove_has_expected_frames_and_matches_independently_authored_audio() {
    let mut f = Fixture::new();
    let mut e = f.editor();
    let c = e.cell();
    for r in 1..4 {
        put(&mut e, r, 0, c.clone());
    }
    e.row = 0;
    let straight = e.song.clone();
    let p = e
        .preview_transform(Scope::Track, 0..4, &Transform::Groove(vec![0.0, 0.25]))
        .unwrap();
    e.apply_transform(p).unwrap();
    f.song = e.song.clone();
    let (actual, m) = f.render("groove.wav");
    let on: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if matches!(e.command, Command::On { .. }) {
                Some(e.frame)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(on, vec![0, 7500, 12000, 19500, 48000, 55500, 60000, 67500]);
    f.song = straight;
    for row in [1, 3] {
        f.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&row)
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .delay = Some(0.25);
    }
    let (expected, reference) = f.render("authored.wav");
    assert_eq!(actual, expected);
    assert_eq!(m, reference);
}
#[test]
fn committed_transforms_roundtrip_and_stream_the_same_pcm() {
    let mut f = Fixture::new();
    let mut e = f.editor();
    let c = e.cell();
    put(&mut e, 1, 0, c);
    let p = e
        .preview_transform(Scope::Song, 0..0, &Transform::Groove(vec![0.0, 0.2]))
        .unwrap();
    e.apply_transform(p).unwrap();
    f.song = e.song;
    let path = f.dir.join("song.json");
    project::save(&f.song, &path, SaveMode::CreateNew).unwrap();
    let (opened, _) = project::load(&path).unwrap();
    assert_eq!(opened, f.song);
    let (expected, _) = f.render("offline.wav");
    let state = Arc::new(Shared::default());
    let mut producer = Producer::new(&opened, None, state.clone()).unwrap();
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
fn phrase_triggers_transform_without_rewriting_instrument_phrases_or_effects() {
    let mut f = Fixture::new();
    let mut phrase = Phrase {
        rows: 2,
        ..Default::default()
    };
    phrase.cells.insert(
        0,
        BTreeMap::from([(
            0,
            Cell {
                note: Note::Note(60),
                effects: StepEffects {
                    reverse: true,
                    offset: Some(0.2),
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
    let mut e = f.editor();
    let mut c = e.cell();
    c.phrase = Some(PhraseId(1));
    c.effects.cut = Some(0.75);
    e.set_cell(c).unwrap();
    let instruments = e.song.instruments.clone();
    let p = e
        .preview_transform(Scope::Song, 0..0, &Transform::Transpose(4))
        .unwrap();
    e.apply_transform(p).unwrap();
    let p = e
        .preview_transform(Scope::Song, 0..0, &Transform::Groove(vec![0.125]))
        .unwrap();
    e.apply_transform(p).unwrap();
    assert_eq!(e.song.instruments, instruments);
    assert_eq!(e.cell().effects.cut, Some(0.75));
    f.song = e.song;
    let (actual, m) = f.render("phrase-groove.wav");
    let event = m
        .events
        .iter()
        .find(|e| matches!(e.command, Command::On { .. }))
        .unwrap();
    assert_eq!(event.frame, 750);
    assert!(matches!(
        event.command,
        Command::On {
            pitch: 64,
            reverse: true,
            ..
        }
    ));
    let c = f
        .song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap();
    c.note = Note::Note(64);
    c.delay = Some(0.125);
    let (expected, reference) = f.render("authored-phrase.wav");
    assert_eq!(actual, expected);
    assert_eq!(m, reference);
}
#[test]
fn empty_and_invalid_selection_reject_and_preview_examples_are_bounded() {
    let f = Fixture::new();
    let mut e = f.editor();
    let c = e.cell();
    for row in 1..4 {
        for col in 0..2 {
            put(&mut e, row, col, c.clone());
        }
    }
    e.track = TrackId(2);
    for row in 0..4 {
        for col in 0..2 {
            put(&mut e, row, col, c.clone());
        }
    }
    let p = e
        .preview_transform(Scope::Pattern, 0..4, &Transform::Transpose(1))
        .unwrap();
    assert!(p.summary.changed_cells > 12);
    assert_eq!(p.summary.examples.len(), 12);
    assert!(p.summary.examples.iter().all(|c| c.before != c.after));
    e.occurrence = 999;
    assert!(e
        .preview_transform(Scope::Song, 0..0, &Transform::Transpose(1))
        .is_err());
    e.occurrence = 0;
    e.column = 999;
    assert!(e
        .preview_transform(Scope::Song, 0..0, &Transform::Transpose(1))
        .is_err());
    let mut empty = example_song();
    empty.order.clear();
    assert!(aqua_pattern_core::transform::preview(
        &empty,
        &aqua_pattern_core::transform::Selection {
            occurrence: 0,
            track: TrackId(1),
            column: 0,
            scope: Scope::Song,
            rows: 0..0
        },
        &Transform::Transpose(1)
    )
    .is_err());
}
