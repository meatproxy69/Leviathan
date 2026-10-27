use aqua_pattern_core::{
    editor::{Editor, Field},
    project::{self, SaveMode},
    session::{Action, Session},
    song::*,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "leviathan-m3-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn document(song: Song) -> serde_json::Value {
    serde_json::json!({"format": "leviathan-song", "version": 1, "song": song})
}
#[test]
fn project_roundtrip_resolves_relative_assets_and_save_as_preserves_target() {
    let dir = Dir::new();
    let asset = dir.path("sample.wav");
    std::fs::write(&asset, b"fixture asset reference").unwrap();
    let mut song = example_song();
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(asset.clone());
    let path = dir.path("song.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["song"]["instruments"]["1"]["sample"], "sample.wav");
    let (restored, _) = project::load(&path).unwrap();
    assert_eq!(restored, song);
    let sub = dir.path("other");
    std::fs::create_dir(&sub).unwrap();
    let copy = sub.join("copy.json");
    project::save(&restored, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(
        project::load(&copy).unwrap().0.instruments[&InstrumentId(1)].sample,
        Some(asset)
    );
}
#[test]
fn malformed_versions_references_ranges_and_unknown_fields_are_rejected() {
    let dir = Dir::new();
    let path = dir.path("bad.json");
    std::fs::write(&path, "not json").unwrap();
    assert!(project::load(&path).is_err());
    let base = document(example_song());
    for (pointer, value) in [
        ("/version", serde_json::json!(2)),
        ("/song/schema", serde_json::json!(2)),
        ("/song/bpm", serde_json::json!(0)),
        ("/song/order/0/pattern", serde_json::json!(99)),
        ("/song/patterns/1/rows", serde_json::json!(0)),
    ] {
        let mut data = base.clone();
        *data.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(project::load(&path).is_err(), "{pointer}");
    }
    let mut data = base;
    data["song"]["future_effects"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(project::load(&path).is_err());
}
#[test]
fn missing_assets_and_failed_open_preserve_current_document_history_and_path() {
    let dir = Dir::new();
    let path = dir.path("missing.json");
    let mut data = document(example_song());
    data["song"]["instruments"]["1"]["sample"] = serde_json::json!("missing.wav");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(project::load(&path)
        .unwrap_err()
        .contains("Instrument 1 missing/unreadable"));
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.enter_note(Note::Note(70)).unwrap();
    let before = e.song.clone();
    let mut session = Session::default();
    session.request(&mut e, Action::Open(path)).unwrap();
    assert!(session.discard(&mut e).is_err());
    assert_eq!(e.song, before);
    assert!(e.dirty && e.can_undo());
    assert!(session.pending.is_some());
    assert!(session.path.is_none());
}
#[test]
fn save_failures_never_clobber_existing_file_and_clean_up_temporary_files() {
    let dir = Dir::new();
    let path = dir.path("song.json");
    std::fs::write(&path, b"original").unwrap();
    let song = example_song();
    assert!(project::save(&song, &path, SaveMode::CreateNew).is_err());
    assert!(project::save(&song, &path, SaveMode::ReplaceConfirmed(b"wrong".to_vec())).is_err());
    assert!(project::save(&song, &dir.path("absent/song.json"), SaveMode::CreateNew).is_err());
    let mut bad = song.clone();
    bad.bpm = f64::NAN;
    assert!(project::save(
        &bad,
        &path,
        SaveMode::ReplaceConfirmed(b"original".to_vec())
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    project::save(
        &song,
        &path,
        SaveMode::ReplaceConfirmed(b"original".to_vec()),
    )
    .unwrap();
    assert_eq!(project::load(&path).unwrap().0, song);
}
#[cfg(unix)]
#[test]
fn symlink_targets_are_not_overwritten_or_loaded() {
    let dir = Dir::new();
    let real = dir.path("real");
    let link = dir.path("link");
    std::fs::write(&real, b"preserve").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(project::save(&example_song(), &link, SaveMode::CreateNew).is_err());
    assert!(project::save(
        &example_song(),
        &link,
        SaveMode::ReplaceConfirmed(b"preserve".to_vec())
    )
    .is_err());
    assert!(project::load(&link).is_err());
    assert_eq!(std::fs::read(real).unwrap(), b"preserve");
}
#[test]
fn save_discard_cancel_guard_new_open_and_close() {
    let dir = Dir::new();
    let mut session = Session::default();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.enter_note(Note::Note(72)).unwrap();
    let edited = e.song.clone();
    session.request(&mut e, Action::New).unwrap();
    assert_eq!(e.song, edited);
    session.cancel();
    assert_eq!(e.song, edited);
    session.request(&mut e, Action::Close).unwrap();
    assert!(!session.close_ready);
    assert!(session
        .save_request(&mut e, dir.path("absent/song.json"), true)
        .is_err());
    assert!(e.dirty);
    assert!(!session.close_ready);
    session
        .save_request(&mut e, dir.path("saved.json"), true)
        .unwrap();
    assert!(!e.dirty);
    assert!(session.close_ready);
    assert_eq!(project::load(&dir.path("saved.json")).unwrap().0, edited);
    session.close_ready = false;
    e.enter_note(Note::Note(73)).unwrap();
    session.request(&mut e, Action::New).unwrap();
    session.discard(&mut e).unwrap();
    assert!(!e.dirty && !e.can_undo());
    assert!(session.path.is_none());
    assert!(e.has_grid());
    assert_eq!(e.song.tracks.len(), 24);
    assert_eq!(e.song.instruments.len(), 24);
    session
        .request(&mut e, Action::Open(dir.path("saved.json")))
        .unwrap();
    assert_eq!(e.song, edited);
    assert!(session.path.is_some());
}
#[test]
fn overwrite_confirmation_is_explicit_and_rejects_external_changes() {
    let dir = Dir::new();
    let path = dir.path("song.json");
    project::save(&example_song(), &path, SaveMode::CreateNew).unwrap();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.enter_note(Note::Note(74)).unwrap();
    let mut session = Session::default();
    session.save_request(&mut e, path.clone(), false).unwrap();
    assert!(session.overwrite.is_some() && e.dirty);
    std::fs::write(&path, b"external change").unwrap();
    assert!(session.confirm_overwrite(&mut e).is_err());
    assert!(e.dirty);
    assert_eq!(std::fs::read(&path).unwrap(), b"external change");
    session.save_request(&mut e, path.clone(), false).unwrap();
    session.confirm_overwrite(&mut e).unwrap();
    assert!(!e.dirty);
    assert_eq!(project::load(&path).unwrap().0, e.song);
}
#[test]
fn undo_redo_saved_state_divergence_and_navigation() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 0,
        ..Editor::default()
    };
    let original = e.song.clone();
    e.select(2, TrackId(2)).unwrap();
    e.move_rows(2);
    assert!(!e.dirty && !e.can_undo());
    e.enter_note(Note::Note(60)).unwrap();
    let first = e.song.clone();
    e.mark_saved();
    e.field = Field::Pan;
    e.enter_value("0.25").unwrap();
    assert!(e.dirty);
    e.undo();
    assert_eq!(e.song, first);
    assert!(!e.dirty);
    e.undo();
    assert_eq!(e.song, original);
    assert!(e.dirty);
    e.redo();
    assert_eq!(e.song, first);
    assert!(!e.dirty);
    e.field = Field::Delay;
    e.enter_value("0.5").unwrap();
    assert!(!e.can_redo());
}
#[test]
fn history_covers_clone_repeat_resize_order_and_is_bounded() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let original = e.song.clone();
    e.append(true).unwrap();
    e.resize(128).unwrap();
    e.append(false).unwrap();
    e.move_occurrence(-1).unwrap();
    let edited = e.song.clone();
    for _ in 0..4 {
        e.undo();
    }
    assert_eq!(e.song, original);
    assert_eq!(e.occurrence, 0);
    for _ in 0..4 {
        e.redo();
    }
    assert_eq!(e.song, edited);
    e.song.validate().unwrap();
    let mut e = Editor {
        edit_mode: true,
        field: Field::Velocity,
        ..Editor::default()
    };
    for i in 0..70 {
        e.enter_value(&(i as f32 / 100.0).to_string()).unwrap();
    }
    for _ in 0..64 {
        assert!(e.can_undo());
        e.undo();
    }
    assert!(!e.can_undo());
    assert_eq!(e.cell().velocity, Some(0.05));
}
#[test]
fn empty_valid_project_loads_without_inventing_content() {
    let mut song = example_song();
    song.order.clear();
    song.patterns.clear();
    song.tracks.clear();
    song.track_order.clear();
    let dir = Dir::new();
    let path = dir.path("empty.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let mut e = Editor::default();
    let mut session = Session::default();
    session.request(&mut e, Action::Open(path)).unwrap();
    assert!(!e.has_grid());
    assert_eq!(e.song, song);
}
