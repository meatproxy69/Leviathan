//! Owner-bound whole-cell transactions and exact persisted history identity.
use aqua_pattern_core::{
    cell_blocks::{PasteDestination, RectScope},
    editor::{Editor, Field},
    project::{self, SaveMode},
    song::*,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn bytes(song: &Song) -> Vec<u8> {
    serde_json::to_vec(song).unwrap()
}
fn scope() -> RectScope {
    RectScope {
        pattern: PatternId(1),
        track: TrackId(1),
        row_start: 0,
        row_end: 2,
        column_start: 0,
        column_end: 2,
    }
}
fn destination() -> PasteDestination {
    PasteDestination {
        pattern: PatternId(2),
        track: TrackId(1),
        row_start: 10,
        column_start: 0,
    }
}
fn put(song: &mut Song, p: u64, row: u32, column: u16, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(p))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(column, cell);
}
fn editor() -> Editor {
    let mut s = example_song();
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .insert(TrackId(1), BTreeMap::new());
    put(
        &mut s,
        1,
        0,
        0,
        Cell {
            note: Note::Note(48),
            instrument: Some(InstrumentId(1)),
            velocity: Some(-0.0),
            pan: Some(-0.0),
            delay: Some(-0.0),
            ..Cell::default()
        },
    );
    put(&mut s, 1, 0, 1, Cell::default());
    put(
        &mut s,
        1,
        1,
        1,
        Cell {
            note: Note::Off,
            velocity: Some(0.0),
            pan: Some(0.25),
            delay: Some(0.125),
            ..Cell::default()
        },
    );
    s.validate().unwrap();
    let mut e = Editor::default();
    e.replace_document(s).unwrap();
    e.edit_mode = true;
    e.row = 7;
    e.column = 1;
    e.field = Field::Pan;
    e
}
#[derive(Debug, PartialEq)]
struct State {
    song: Vec<u8>,
    history: String,
    revision: u64,
    cursor: (usize, TrackId, u32, u16, Field),
    dirty: bool,
    edit: bool,
}
fn state(e: &Editor) -> State {
    State {
        song: bytes(&e.song),
        history: format!("{:?}", e.history),
        revision: e.revision(),
        cursor: (e.occurrence, e.track, e.row, e.column, e.field),
        dirty: e.dirty,
        edit: e.edit_mode,
    }
}
fn scalar(song: &mut Song, value: f32) {
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
        .pan = Some(value);
}

#[test]
fn exact_document_comparison_distinguishes_zero_and_lexical_paths_without_io() {
    let a = editor().song;
    let mut b = a.clone();
    scalar(&mut b, 0.0);
    assert_eq!(a, b);
    assert_ne!(bytes(&a), bytes(&b));
    assert!(!project::same_serialized_song(&a, &b).unwrap());
    assert!(project::same_serialized_song(&a, &a.clone()).unwrap());
    for (left, right) in [
        ("/not-present/a/./b.wav", "/not-present/a/b.wav"),
        ("/not-present/a//b.wav", "/not-present/a/b.wav"),
    ] {
        let mut a = a.clone();
        let mut b = a.clone();
        a.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(left.into());
        b.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(right.into());
        assert_eq!(a, b);
        assert!(!project::same_serialized_song(&a, &b).unwrap());
    }
    b.title = "obviously different".into();
    assert!(!project::same_serialized_song(&a, &b).unwrap());
}

#[test]
fn exact_editor_zero_edit_has_one_history_entry_and_correct_saved_dirty_state() {
    let mut e = editor();
    let before = bytes(&e.song);
    let rev = e.revision();
    let cursor = state(&e).cursor;
    let mut candidate = e.song.clone();
    scalar(&mut candidate, 0.0);
    e.apply_document_edit(candidate).unwrap();
    assert_eq!(e.revision(), rev + 1);
    assert!(e.dirty);
    assert_eq!(state(&e).cursor, cursor);
    let changed = bytes(&e.song);
    assert_ne!(changed, before);
    e.mark_saved();
    assert!(!e.dirty);
    e.undo();
    assert_eq!(bytes(&e.song), before);
    assert!(e.dirty);
    e.redo();
    assert_eq!(bytes(&e.song), changed);
    assert!(!e.dirty);
    e.undo();
    let snapshot = state(&e);
    e.apply_document_edit(e.song.clone()).unwrap();
    assert_eq!(state(&e), snapshot);
    assert!(e.can_redo());
}

#[test]
fn lexical_legacy_path_edits_are_exact_but_registered_source_paths_stay_immutable() {
    let mut e = editor();
    e.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some("/no-file/a/./b.wav".into());
    e.mark_saved();
    let before = bytes(&e.song);
    let mut candidate = e.song.clone();
    candidate
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .sample = Some("/no-file/a/b.wav".into());
    assert_eq!(candidate, e.song);
    e.apply_document_edit(candidate).unwrap();
    assert!(e.dirty);
    assert_ne!(bytes(&e.song), before);
    e.undo();
    assert_eq!(bytes(&e.song), before);
    assert!(!e.dirty);
    e.song.samples.insert(
        SampleId(9),
        SampleAsset {
            path: "/no-file/a/./b.wav".into(),
            sha256: "1".repeat(64),
            sample_rate: 8000,
            channels: 1,
            frames: 8,
        },
    );
    e.mark_saved();
    for path in ["/no-file/a/b.wav", "/no-file/a//b.wav"] {
        let mut candidate = e.song.clone();
        candidate.samples.get_mut(&SampleId(9)).unwrap().path = path.into();
        let before = state(&e);
        assert!(e
            .apply_document_edit(candidate)
            .unwrap_err()
            .contains("immutable"));
        assert_eq!(state(&e), before);
        let mut asset = e.song.samples[&SampleId(9)].clone();
        asset.path = path.into();
        assert!(e.register_sample(SampleId(9), asset).is_err());
        assert_eq!(state(&e), before);
    }
}

#[test]
fn bounded_equality_errors_refuse_before_history_and_restore_never_falsely_cleans() {
    let mut s = editor().song;
    s.title = "x".repeat(8 * 1024 * 1024);
    s.validate().unwrap();
    assert!(project::same_serialized_song(&s, &s)
        .unwrap_err()
        .contains("8 MiB"));
    let mut e = Editor::default();
    e.replace_document(s).unwrap();
    e.edit_mode = true;
    // An obviously different legacy edit does not acquire a new global storage cap.
    e.set_timing(121.0, 4).unwrap();
    assert!(e.dirty);
    // Different from current, but equal-looking to the oversized saved baseline:
    // the second (prospective dirty) comparison must fail before any history write.
    let mut saved_candidate = e.song.clone();
    saved_candidate.bpm = 120.0;
    let before_second_comparison = state(&e);
    assert!(e
        .apply_document_edit(saved_candidate)
        .unwrap_err()
        .contains("8 MiB"));
    assert_eq!(state(&e), before_second_comparison);
    e.undo();
    assert!(e.dirty);
    assert!(e.can_redo());
    let revision = e.revision();
    let before = bytes(&e.song);
    let history = format!("{:?}", e.history);
    assert!(e
        .apply_document_edit(e.song.clone())
        .unwrap_err()
        .contains("8 MiB"));
    assert_eq!(e.revision(), revision);
    assert_eq!(bytes(&e.song), before);
    assert_eq!(format!("{:?}", e.history), history);
    assert!(e.can_redo());
    e.redo();
    assert_eq!(e.song.bpm, 121.0);
    assert!(e.dirty);
}

#[test]
fn copy_is_read_only_exact_holes_default_cells_and_alias_resolution() {
    let mut e = editor();
    e.edit_mode = false;
    let before = state(&e);
    let clip = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    e.validate_cell_copy(&clip).unwrap();
    assert_eq!(state(&e), before);
    assert_eq!(clip.snapshot().cells().len(), 4);
    assert!(clip.snapshot().cells()[2].is_none());
    assert_eq!(clip.snapshot().cells()[1], Some(Cell::default()));
    assert_eq!(
        clip.snapshot().cells()[0]
            .as_ref()
            .unwrap()
            .pan
            .unwrap()
            .to_bits(),
        (-0.0f32).to_bits()
    );
    let mut alias = e.song.patterns[&PatternId(1)].clone();
    alias.tracks.remove(&TrackId(1));
    alias.track_aliases.insert(
        TrackId(1),
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        },
    );
    e.song.patterns.insert(PatternId(9), alias);
    e.song.validate().unwrap();
    let mut selected = scope();
    selected.pattern = PatternId(9);
    let a = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&selected)
        .unwrap();
    assert_eq!(
        serde_json::to_vec(a.snapshot().cells()).unwrap(),
        serde_json::to_vec(clip.snapshot().cells()).unwrap()
    );
    assert!(a.snapshot().source_chain().len() > 1);
    assert!(e
        .capture_cell_document()
        .unwrap()
        .clear_cells_preview(&selected)
        .is_err());
}

#[test]
fn paste_cross_pattern_same_track_preserves_whole_cells_cursor_and_one_undo() {
    let mut e = editor();
    let clip = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    e.validate_cell_copy(&clip).unwrap();
    put(
        &mut e.song,
        2,
        11,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            ..Cell::default()
        },
    );
    e.mark_saved();
    let before = bytes(&e.song);
    let cursor = state(&e).cursor;
    let plan = e
        .capture_cell_document()
        .unwrap()
        .paste_cells_preview(&clip, &destination())
        .unwrap();
    e.validate_cell_preview(&plan).unwrap();
    let expected = bytes(plan.preview().candidate());
    e.commit_cell_block_preview(&plan).unwrap();
    assert_eq!(bytes(&e.song), expected);
    assert_eq!(state(&e).cursor, cursor);
    assert!(e.dirty);
    let rows = &e.song.patterns[&PatternId(2)].tracks[&TrackId(1)];
    assert!(!rows[&11].contains_key(&0));
    assert_eq!(rows[&10][&0].pan.unwrap().to_bits(), (-0.0f32).to_bits());
    assert_eq!(rows[&10][&1], Cell::default());
    e.undo();
    assert_eq!(bytes(&e.song), before);
    assert!(!e.dirty);
    assert!(!e.can_undo());
    e.redo();
    assert_eq!(bytes(&e.song), expected);
}

#[test]
fn clipboard_owner_survives_edits_save_undo_but_not_new_open_or_other_editor() {
    let mut e = editor();
    let clip = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    let cloned = clip.clone();
    e.validate_cell_copy(&clip).unwrap();
    e.set_timing(137.3, 7).unwrap();
    assert!(e.cell_clipboard_is_current_document(&cloned));
    assert!(e.validate_cell_copy(&cloned).is_err());
    e.undo();
    e.redo();
    e.mark_saved();
    assert!(e.cell_clipboard_is_current_document(&clip));
    assert!(e
        .capture_cell_document()
        .unwrap()
        .paste_cells_preview(&clip, &destination())
        .is_ok());
    let mut invalid = e.song.clone();
    invalid.bpm = 0.0;
    assert!(e.replace_document(invalid).is_err());
    assert!(e.cell_clipboard_is_current_document(&clip));
    let same = e.song.clone();
    e.replace_document(same).unwrap();
    assert!(!e.cell_clipboard_is_current_document(&clip));
    assert!(e
        .capture_cell_document()
        .unwrap()
        .paste_cells_preview(&clip, &destination())
        .is_err());
    let other = editor();
    assert!(!other.cell_clipboard_is_current_document(&clip));
    // Explicit Editor cloning is a same-document fork, not New/Open. Workers
    // still use DocumentCapture rather than cloning the history-bearing Editor.
    let original = editor();
    let shared = original
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    let fork = original.clone();
    assert!(fork.cell_clipboard_is_current_document(&shared));
    fork.validate_cell_copy(&shared).unwrap();
}

#[test]
fn stale_copy_and_preview_refuse_aba_unchecked_bits_and_read_mode_atomically() {
    let mut e = editor();
    let cap = e.capture_cell_document().unwrap();
    let clip = cap.copy_cells(&scope()).unwrap();
    let plan = cap.clear_cells_preview(&scope()).unwrap();
    e.edit_mode = false;
    let before = state(&e);
    assert!(e.commit_cell_block_preview(&plan).is_err());
    assert_eq!(state(&e), before);
    e.edit_mode = true;
    e.set_timing(137.3, 7).unwrap();
    e.undo();
    let before = state(&e);
    assert!(e.validate_cell_copy(&clip).is_err());
    assert!(e.commit_cell_block_preview(&plan).is_err());
    assert_eq!(state(&e), before);
    let cap = e.capture_cell_document().unwrap();
    let clip = cap.copy_cells(&scope()).unwrap();
    let plan = cap.clear_cells_preview(&scope()).unwrap();
    let expected = e.song.clone();
    let revision = e.revision();
    scalar(&mut e.song, 0.0);
    assert_eq!(e.song, expected);
    let before = state(&e);
    assert!(e.validate_cell_copy(&clip).is_err());
    assert!(e.commit_cell_block_preview(&plan).is_err());
    assert!(e
        .apply_document_edit_if_current(revision, &expected, expected.clone())
        .is_err());
    assert_eq!(state(&e), before);
}

#[test]
fn clear_and_identity_paste_keep_redo_and_failure_does_not_replace_authority() {
    let mut e = editor();
    let clip = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    e.set_timing(137.3, 7).unwrap();
    e.undo();
    let dest = PasteDestination {
        pattern: PatternId(1),
        track: TrackId(1),
        row_start: 0,
        column_start: 0,
    };
    let plan = e
        .capture_cell_document()
        .unwrap()
        .paste_cells_preview(&clip, &dest)
        .unwrap();
    assert_eq!(plan.preview().summary().changed_cells, 0);
    let before = state(&e);
    e.commit_cell_block_preview(&plan).unwrap();
    assert_eq!(state(&e), before);
    assert!(e.can_redo());
    let empty = RectScope {
        row_start: 20,
        row_end: 22,
        ..scope()
    };
    let clear = e
        .capture_cell_document()
        .unwrap()
        .clear_cells_preview(&empty)
        .unwrap();
    let before = state(&e);
    e.commit_cell_block_preview(&clear).unwrap();
    assert_eq!(state(&e), before);
    for bad in [
        PasteDestination {
            track: TrackId(2),
            ..destination()
        },
        PasteDestination {
            row_start: u32::MAX,
            ..destination()
        },
        PasteDestination {
            column_start: 1,
            ..destination()
        },
    ] {
        assert!(e
            .capture_cell_document()
            .unwrap()
            .paste_cells_preview(&clip, &bad)
            .is_err());
        assert_eq!(state(&e), before);
    }
}

#[test]
fn clipboard_is_historical_not_a_live_reference_and_paste_revalidates_ids() {
    let mut e = editor();
    let clip = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    let mut changed = e.song.clone();
    for p in changed.patterns.values_mut() {
        for t in p.tracks.values_mut() {
            for row in t.values_mut() {
                row.retain(|_, c| c.instrument != Some(InstrumentId(1)));
            }
        }
    }
    changed.instruments.remove(&InstrumentId(1));
    changed.validate().unwrap();
    e.apply_document_edit(changed).unwrap();
    assert!(e.cell_clipboard_is_current_document(&clip));
    let before = state(&e);
    assert!(e
        .capture_cell_document()
        .unwrap()
        .paste_cells_preview(&clip, &destination())
        .is_err());
    assert_eq!(state(&e), before);
}

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "leviathan-cell-block-editor-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn cell_transactions_save_open_without_clipboard_schema_or_source_rewrite() {
    let dir = Dir::new();
    let path = dir.0.join("before.leviathan.json");
    let mut e = editor();
    project::save(&e.song, &path, SaveMode::CreateNew).unwrap();
    let old = std::fs::read(&path).unwrap();
    let clip = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&scope())
        .unwrap();
    let plan = e
        .capture_cell_document()
        .unwrap()
        .paste_cells_preview(&clip, &destination())
        .unwrap();
    e.commit_cell_block_preview(&plan).unwrap();
    let after = dir.0.join("after.leviathan.json");
    project::save(&e.song, &after, SaveMode::CreateNew).unwrap();
    let loaded = project::load(&after).unwrap().0;
    assert_eq!(bytes(&loaded), bytes(&e.song));
    let text = String::from_utf8(std::fs::read(&after).unwrap()).unwrap();
    for absent in [
        "clipboard",
        "document_owner",
        "cell_blocks",
        "paste_replace",
    ] {
        assert!(!text.contains(absent));
    }
    e.undo();
    let restored = dir.0.join("restored.leviathan.json");
    project::save(&e.song, &restored, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(restored).unwrap(), old);
    e.replace_document(loaded).unwrap();
    assert!(!e.cell_clipboard_is_current_document(&clip));
    assert!(!e.dirty);
}

#[cfg(unix)]
#[test]
fn non_utf8_legacy_path_comparison_error_is_atomic_without_filesystem_io() {
    use std::os::unix::ffi::OsStringExt;
    let mut e = editor();
    e.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some(std::ffi::OsString::from_vec(b"/no-file/invalid-\xff.wav".to_vec()).into());
    e.song.validate().unwrap();
    e.mark_saved();
    let revision = e.revision();
    let history = format!("{:?}", e.history);
    let song = format!("{:?}", e.song);
    assert!(project::same_serialized_song(&e.song, &e.song).is_err());
    assert!(e.apply_document_edit(e.song.clone()).is_err());
    assert_eq!(e.revision(), revision);
    assert_eq!(format!("{:?}", e.song), song);
    assert_eq!(format!("{:?}", e.history), history);
    assert!(!e.dirty);
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    assert_eq!(format!("{:?}", e.song), song);
    assert!(e.dirty); // Infallible restore cannot prove this unserializable state clean.
}

#[test]
fn exact_comparator_bounds_both_encodings_at_pretty_project_limit() {
    let mut left = editor().song;
    scalar(&mut left, 0.0);
    left.title.clear();
    let overhead = project::check_serialized_size(&left).unwrap();
    left.title = "x".repeat(8 * 1024 * 1024 - overhead);
    assert_eq!(
        project::check_serialized_size(&left).unwrap(),
        8 * 1024 * 1024
    );
    assert!(project::same_serialized_song(&left, &left).unwrap());
    let mut right = left.clone();
    scalar(&mut right, -0.0); // One extra serialized sign byte; structurally equal.
    assert_eq!(left, right);
    assert!(project::same_serialized_song(&left, &right)
        .unwrap_err()
        .contains("8 MiB"));
    assert!(project::same_serialized_song(&right, &left)
        .unwrap_err()
        .contains("8 MiB"));
}
