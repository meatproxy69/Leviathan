//! Persisted track balance is one pure owner-bound transaction, not live delivery.
use aqua_pattern_core::{
    cell_blocks::RectScope,
    editor::{Editor, Field},
    project::{self, SaveMode},
    song::*,
    track_mix::{MixLayoutStatus, TrackMix, TrackMixConfig, TrackMixSetting},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn bytes(song: &Song) -> Vec<u8> {
    serde_json::to_vec(song).unwrap()
}
fn editor() -> Editor {
    Editor {
        edit_mode: true,
        row: 7,
        column: 1,
        field: Field::Pan,
        ..Editor::default()
    }
}
fn config(values: &[(u64, f64, f64, bool, bool)]) -> TrackMixConfig {
    TrackMixConfig {
        version: 1,
        settings: values
            .iter()
            .map(|&(track, gain, balance, mute, solo)| TrackMixSetting {
                track: TrackId(track),
                gain,
                balance,
                mute,
                solo,
            })
            .collect(),
    }
}
fn preview(e: &Editor, c: TrackMixConfig) -> aqua_pattern_core::editor::DocumentTrackMixPreview {
    e.capture_track_mix_document()
        .unwrap()
        .preview_track_mix(c)
        .unwrap()
}
fn cursor(e: &Editor) -> (usize, TrackId, u32, u16, Field) {
    (e.occurrence, e.track, e.row, e.column, e.field)
}
#[derive(Debug, PartialEq)]
struct State {
    song: Vec<u8>,
    history: String,
    revision: u64,
    cursor: (usize, TrackId, u32, u16, Field),
    dirty: bool,
    edit: bool,
    mix_revision: Option<u64>,
}
fn state(e: &Editor) -> State {
    State {
        song: bytes(&e.song),
        history: format!("{:?}", e.history),
        revision: e.revision(),
        cursor: cursor(e),
        dirty: e.dirty,
        edit: e.edit_mode,
        mix_revision: e.track_mix_change_revision(),
    }
}
fn apply(e: &mut Editor, c: TrackMixConfig) {
    let p = preview(e, c);
    e.commit_track_mix_preview(&p).unwrap();
}

#[test]
fn pure_read_mode_preview_requires_explicit_edit_commit_and_preserves_cursor() {
    let mut e = editor();
    e.edit_mode = false;
    let before = state(&e);
    let owner = e.document_owner();
    let p = preview(
        &e,
        config(&[(1, 0.5, -0.25, false, false), (2, 0.75, 0.5, true, true)]),
    );
    assert_eq!(p.revision(), e.revision());
    assert!(matches!(
        p.preview().summary().layout_status,
        MixLayoutStatus::Unavailable { .. }
    ));
    e.validate_track_mix_preview(&p).unwrap();
    assert_eq!(state(&e), before);
    assert!(e.commit_track_mix_preview(&p).is_err());
    assert_eq!(state(&e), before);
    e.edit_mode = true;
    let cursor = cursor(&e);
    let source = bytes(&e.song);
    e.commit_track_mix_preview(&p).unwrap();
    assert_eq!(cursor, crate::cursor(&e));
    assert!(e.owns_document(&owner));
    assert_eq!(e.track_mix_change_revision(), Some(e.revision()));
    assert!(e.can_undo());
    assert!(e.dirty);
    let changed = bytes(&e.song);
    let mut without_mix = e.song.clone();
    without_mix.track_mix.clear();
    assert_eq!(bytes(&without_mix), source);
    e.undo();
    assert_eq!(bytes(&e.song), source);
    assert!(!e.dirty);
    assert!(!e.can_undo());
    assert_eq!(cursor, crate::cursor(&e));
    assert_eq!(e.track_mix_change_revision(), Some(e.revision()));
    e.redo();
    assert_eq!(bytes(&e.song), changed);
    assert_eq!(cursor, crate::cursor(&e));
    assert_eq!(e.track_mix_change_revision(), Some(e.revision()));
}

#[test]
fn explicit_noop_apply_preserves_redo_revision_and_saved_state_for_live_retry() {
    let mut e = editor();
    apply(&mut e, config(&[(1, 0.25, 0.0, false, false)]));
    e.undo();
    assert!(e.can_redo());
    let before = state(&e);
    let p = preview(&e, TrackMixConfig::from_song(&e.song));
    assert_eq!(p.preview().summary().changed_tracks, 0);
    e.commit_track_mix_preview(&p).unwrap();
    assert_eq!(state(&e), before);
    // The valid plan can be reused for an explicit external live retry. No runtime
    // delivery is attempted or certified by this document API.
    e.commit_track_mix_preview(&p).unwrap();
    assert_eq!(state(&e), before);
    e.redo();
    assert_eq!(e.song.track_mix[&TrackId(1)].gain, 0.25);
}

#[test]
fn signed_zero_model_bits_are_real_history_changes_and_reset_omits_map() {
    let mut e = editor();
    apply(&mut e, config(&[(1, 0.0, 0.0, false, false)]));
    e.mark_saved();
    let positive = bytes(&e.song);
    let rev = e.revision();
    apply(&mut e, config(&[(1, -0.0, -0.0, false, false)]));
    assert_eq!(e.revision(), rev + 1);
    assert!(e.dirty);
    assert_eq!(
        e.song.track_mix[&TrackId(1)].gain.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        e.song.track_mix[&TrackId(1)].balance.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(e.track_mix_change_revision(), Some(e.revision()));
    let negative = bytes(&e.song);
    assert_ne!(positive, negative);
    e.undo();
    assert_eq!(bytes(&e.song), positive);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(bytes(&e.song), negative);
    apply(&mut e, config(&[]));
    assert!(e.song.track_mix.is_empty());
    assert!(!String::from_utf8(bytes(&e.song))
        .unwrap()
        .contains("track_mix"));
}

#[test]
fn invalid_full_sets_refuse_without_changing_history_or_old_ready_plan() {
    let mut e = editor();
    let good = preview(&e, config(&[(1, 0.5, 0.0, false, false)]));
    let before = state(&e);
    let mut bad = vec![
        config(&[(1, f64::NAN, 0.0, false, false)]),
        config(&[(1, f64::INFINITY, 0.0, false, false)]),
        config(&[(1, -0.000_000_1, 0.0, false, false)]),
        config(&[(1, 1.000_000_01, 0.0, false, false)]),
        config(&[(1, 1.0, 1.000_000_01, false, false)]),
        config(&[(1, 1.0, -1.000_000_01, false, false)]),
        config(&[(1, 1.0, 0.0, false, false), (1, 0.5, 0.0, false, false)]),
        config(&[(999, 0.5, 0.0, false, false)]),
    ];
    let mut version = config(&[]);
    version.version = 2;
    bad.push(version);
    let mut huge = config(&[(1, 1.0, 0.0, false, false)]);
    huge.settings = vec![huge.settings[0].clone(); 65];
    bad.push(huge);
    for c in bad {
        assert!(e
            .capture_track_mix_document()
            .unwrap()
            .preview_track_mix(c)
            .is_err());
        assert_eq!(state(&e), before);
    }
    e.validate_track_mix_preview(&good).unwrap();
    e.commit_track_mix_preview(&good).unwrap();
    assert_eq!(e.song.track_mix[&TrackId(1)].gain, 0.5);
}

#[test]
fn owner_revokes_only_successful_replacement_and_cloned_editors_share_identity() {
    let mut e = editor();
    let owner = e.document_owner();
    let p = preview(&e, config(&[]));
    let other = editor();
    assert!(!other.owns_document(&owner));
    assert!(other.validate_track_mix_preview(&p).is_err());
    let cloned = e.clone();
    assert!(cloned.owns_document(&owner));
    cloned.validate_track_mix_preview(&p).unwrap();
    let mut invalid = e.song.clone();
    invalid.bpm = 0.0;
    let before = state(&e);
    assert!(e.replace_document(invalid).is_err());
    assert_eq!(state(&e), before);
    assert!(e.owns_document(&owner));
    e.validate_track_mix_preview(&p).unwrap();
    e.replace_document(e.song.clone()).unwrap();
    assert!(!e.owns_document(&owner));
    assert!(e.validate_track_mix_preview(&p).is_err());
    assert_eq!(e.track_mix_change_revision(), None);
}

#[test]
fn preview_rejects_revision_aba_and_unchecked_exact_base_changes_atomically() {
    let mut e = editor();
    let p = preview(&e, config(&[(1, 0.5, 0.0, false, false)]));
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    assert_eq!(bytes(&e.song), bytes(p.preview().base()));
    let before = state(&e);
    assert!(e.commit_track_mix_preview(&p).is_err());
    assert_eq!(state(&e), before);
    apply(&mut e, config(&[(1, 0.0, 0.0, false, false)]));
    let p = preview(&e, config(&[]));
    let rev = e.revision();
    e.song.track_mix.get_mut(&TrackId(1)).unwrap().balance = -0.0;
    assert_eq!(&e.song, p.preview().base());
    assert_eq!(e.revision(), rev);
    let before = state(&e);
    assert!(e.commit_track_mix_preview(&p).is_err());
    assert_eq!(state(&e), before);
    let mut e = editor();
    e.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some("/not-present/a/./b.wav".into());
    let p = preview(&e, config(&[]));
    e.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some("/not-present/a/b.wav".into());
    assert_eq!(&e.song, p.preview().base());
    let before = state(&e);
    assert!(e.commit_track_mix_preview(&p).is_err());
    assert_eq!(state(&e), before);
}

#[test]
fn mix_transition_hint_covers_undo_redo_not_save_or_combined_edits() {
    let mut e = editor();
    let owner = e.document_owner();
    apply(&mut e, config(&[(1, 0.5, 0.0, false, false)]));
    let tagged = e.revision();
    assert_eq!(e.track_mix_change_revision(), Some(tagged));
    e.mark_saved();
    assert_eq!(e.revision(), tagged);
    assert_eq!(e.track_mix_change_revision(), Some(tagged));
    e.set_timing(121.0, 4).unwrap();
    assert_eq!(e.track_mix_change_revision(), None);
    e.undo();
    assert_eq!(e.track_mix_change_revision(), None);
    e.undo();
    assert_eq!(e.track_mix_change_revision(), Some(e.revision()));
    e.redo();
    assert_eq!(e.track_mix_change_revision(), Some(e.revision()));
    let mut both = e.song.clone();
    both.title.push('!');
    both.track_mix.get_mut(&TrackId(1)).unwrap().gain = 0.3;
    e.apply_document_edit(both).unwrap();
    assert_eq!(e.track_mix_change_revision(), None);
    e.undo();
    assert_eq!(e.track_mix_change_revision(), None);
    assert!(e.owns_document(&owner));
    let mut lexical = e.song.clone();
    lexical
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .sample = Some("/no-file/a/./b.wav".into());
    e.apply_document_edit(lexical).unwrap();
    let mut both = e.song.clone();
    both.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some("/no-file/a/b.wav".into());
    both.track_mix.get_mut(&TrackId(1)).unwrap().gain = 0.2;
    e.apply_document_edit(both).unwrap();
    assert_eq!(e.track_mix_change_revision(), None);
}

#[test]
fn history_hint_failure_does_not_add_a_global_size_restriction_or_block_restore() {
    let mut e = editor();
    e.song.title = "x".repeat(8 * 1024 * 1024);
    e.mark_saved();
    assert!(e.capture_track_mix_document().is_err());
    let mut changed = e.song.clone();
    changed.track_mix.insert(
        TrackId(1),
        TrackMix {
            gain: 0.5,
            ..TrackMix::default()
        },
    );
    // Obviously differing documents still follow the pre-existing history policy.
    e.apply_document_edit(changed).unwrap();
    assert_eq!(e.track_mix_change_revision(), None);
    assert!(e.dirty);
    e.undo();
    assert!(e.song.track_mix.is_empty());
    assert!(e.dirty);
    assert_eq!(e.track_mix_change_revision(), None);
    e.redo();
    assert_eq!(e.song.track_mix[&TrackId(1)].gain, 0.5);
    assert_eq!(e.track_mix_change_revision(), None);
}

#[cfg(unix)]
#[test]
fn unserializable_hint_is_conservative_without_changing_infallible_history() {
    use std::os::unix::ffi::OsStringExt;
    let mut e = editor();
    e.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some(std::ffi::OsString::from_vec(b"/missing/invalid-\xff.wav".to_vec()).into());
    e.mark_saved();
    let mut changed = e.song.clone();
    changed.track_mix.insert(
        TrackId(1),
        TrackMix {
            gain: 0.5,
            ..TrackMix::default()
        },
    );
    e.apply_document_edit(changed).unwrap();
    assert_eq!(e.track_mix_change_revision(), None);
    e.undo();
    assert!(e.song.track_mix.is_empty());
    assert!(e.dirty);
    e.redo();
    assert_eq!(e.song.track_mix[&TrackId(1)].gain, 0.5);
    assert_eq!(e.track_mix_change_revision(), None);
}

#[test]
fn installed_cell_clipboard_survives_mix_history_but_old_worker_plan_does_not() {
    let mut e = editor();
    let scope = RectScope {
        pattern: PatternId(1),
        track: TrackId(1),
        row_start: 0,
        row_end: 1,
        column_start: 0,
        column_end: 1,
    };
    let capture = e.capture_cell_document().unwrap();
    let clip = capture.copy_cells(&scope).unwrap();
    e.validate_cell_copy(&clip).unwrap();
    let old_mix = capture
        .preview_track_mix(config(&[(1, 0.5, 0.0, false, false)]))
        .unwrap();
    apply(&mut e, config(&[(1, 0.25, 0.0, false, false)]));
    assert!(e.cell_clipboard_is_current_document(&clip));
    assert!(e.validate_cell_copy(&clip).is_err());
    assert!(e.validate_track_mix_preview(&old_mix).is_err());
    e.mark_saved();
    e.undo();
    e.redo();
    assert!(e.cell_clipboard_is_current_document(&clip));
    e.replace_document(e.song.clone()).unwrap();
    assert!(!e.cell_clipboard_is_current_document(&clip));
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "leviathan-track-mix-editor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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
fn project_roundtrip_preserves_bits_and_empty_reset_restores_old_json_exactly() {
    let d = Dir::new();
    let mut e = editor();
    let old = d.0.join("old.json");
    project::save(&e.song, &old, SaveMode::CreateNew).unwrap();
    let old_bytes = std::fs::read(old).unwrap();
    assert!(!String::from_utf8_lossy(&old_bytes).contains("track_mix"));
    apply(
        &mut e,
        config(&[(1, -0.0, -0.0, false, true), (2, 0.75, 0.25, true, false)]),
    );
    let changed = d.0.join("changed.json");
    project::save(&e.song, &changed, SaveMode::CreateNew).unwrap();
    let loaded = project::load(&changed).unwrap().0;
    assert_eq!(bytes(&loaded), bytes(&e.song));
    assert_eq!(
        loaded.track_mix[&TrackId(1)].gain.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        loaded.track_mix[&TrackId(1)].balance.to_bits(),
        (-0.0_f32).to_bits()
    );
    let text = std::fs::read_to_string(&changed).unwrap();
    for forbidden in [
        "document_owner",
        "layout_status",
        "worker_enqueued",
        "track_mix_change_revision",
    ] {
        assert!(!text.contains(forbidden));
    }
    let mut replacement = editor();
    replacement.replace_document(loaded).unwrap();
    assert!(!replacement.dirty);
    assert!(!replacement.can_undo());
    apply(&mut e, config(&[]));
    let reset = d.0.join("reset.json");
    project::save(&e.song, &reset, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(reset).unwrap(), old_bytes);
}

#[test]
fn configured_unused_track_reference_requires_explicit_full_set_clear_before_delete() {
    let mut e = editor();
    e.song.tracks.insert(
        TrackId(99),
        Track {
            name: "Unused".into(),
            note_columns: 1,
        },
    );
    e.song.track_order.push(TrackId(99));
    e.mark_saved();
    apply(&mut e, config(&[(99, 0.5, 0.0, false, true)]));
    let mut deleted = e.song.clone();
    deleted.tracks.remove(&TrackId(99));
    deleted.track_order.retain(|&t| t != TrackId(99));
    let before = state(&e);
    assert!(e.apply_document_edit(deleted).is_err());
    assert_eq!(state(&e), before);
    apply(&mut e, config(&[]));
    let mut deleted = e.song.clone();
    deleted.tracks.remove(&TrackId(99));
    deleted.track_order.retain(|&t| t != TrackId(99));
    e.apply_document_edit(deleted).unwrap();
    assert_eq!(e.track_mix_change_revision(), None);
    e.undo();
    assert!(e.song.tracks.contains_key(&TrackId(99)));
    e.undo();
    assert!(e.song.track_mix.contains_key(&TrackId(99)));
}
