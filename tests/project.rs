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

#[test]
fn startup_and_new_song_provide_twenty_four_tracks_and_instruments() {
    let startup = startup_song();
    assert_eq!(startup.tracks.len(), NEW_SONG_CAPACITY);
    assert_eq!(startup.instruments.len(), NEW_SONG_CAPACITY);
    assert_eq!(startup.patterns[&PatternId(1)].tracks.len(), 4);
    assert_eq!(Editor::default().song.tracks.len(), 4);

    let mut editor = Editor::default();
    let mut session = Session::default();
    session.request(&mut editor, Action::New).unwrap();
    assert_eq!(editor.song.tracks.len(), NEW_SONG_CAPACITY);
    assert_eq!(editor.song.instruments.len(), NEW_SONG_CAPACITY);
    assert!(editor.song.patterns[&PatternId(1)].tracks.is_empty());
    assert_eq!(editor.song.order.len(), 1);
    editor.song.validate().unwrap();
}

struct Dir(PathBuf);
#[cfg(windows)]
fn ordinary_temp_dir() -> PathBuf {
    let path = std::env::temp_dir();
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{}", unc))
    } else if let Some(dos) = value.strip_prefix(r"\\?\") {
        PathBuf::from(dos)
    } else {
        path
    }
}
#[cfg(not(windows))]
fn ordinary_temp_dir() -> PathBuf {
    std::env::temp_dir()
}
impl Dir {
    fn new() -> Self {
        let p = ordinary_temp_dir().join(format!(
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
#[cfg(windows)]
#[test]
fn windows_project_paths_normalize_safe_verbatim_and_ordinary_paths() {
    let dir = Dir::new();
    let ordinary = project::save(&example_song(), &dir.path("song.json"), SaveMode::CreateNew)
        .unwrap()
        .0;
    assert!(!ordinary.to_string_lossy().starts_with(r"\\?\"));

    let verbatim = PathBuf::from(format!(r"\\?\{}", dir.path("song-verbatim.json").display()));
    let preserved = project::save(&example_song(), &verbatim, SaveMode::CreateNew)
        .unwrap()
        .0;
    assert!(!preserved.to_string_lossy().starts_with(r"\\?\"));
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

#[test]
fn project_confirmations_cannot_be_retargeted_by_overlapping_requests() {
    let dir = Dir::new();
    let existing = dir.path("existing.json");
    std::fs::write(&existing, b"existing").unwrap();
    let mut session = Session::default();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.enter_note(Note::Note(72)).unwrap();
    let edited = e.song.clone();
    session.request(&mut e, Action::New).unwrap();
    assert!(session.request(&mut e, Action::Close).is_err());
    assert!(matches!(session.pending, Some(Action::New)));
    assert!(session
        .save_request(&mut e, dir.path("unrelated.json"), false)
        .is_err());
    session
        .save_request(&mut e, existing.clone(), true)
        .unwrap();
    assert!(session.request(&mut e, Action::Close).is_err());
    assert!(session.discard(&mut e).is_err());
    assert!(session
        .save_request(&mut e, dir.path("unrelated.json"), true)
        .is_err());
    assert_eq!(session.overwrite.as_ref().unwrap().path, existing);
    assert_eq!(e.song, edited);
    assert!(e.dirty && e.can_undo());
    assert!(!session.close_ready && !dir.path("unrelated.json").exists());
    session.cancel();
    session.confirm_overwrite(&mut e).unwrap();
    assert_eq!(std::fs::read(&existing).unwrap(), b"existing");
    assert!(session.pending.is_none() && session.overwrite.is_none());
    assert_eq!(e.song, edited);
}

#[test]
fn save_then_failed_open_keeps_saved_song_history_path_and_pending_action() {
    let dir = Dir::new();
    let active = dir.path("active.json");
    let broken = dir.path("broken.json");
    std::fs::write(&broken, b"not a project").unwrap();
    let mut session = Session::default();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    session.save_request(&mut e, active.clone(), false).unwrap();
    e.enter_note(Note::Note(71)).unwrap();
    let edited = e.song.clone();
    session.request(&mut e, Action::Open(broken)).unwrap();
    session.save_request(&mut e, active.clone(), true).unwrap();
    assert!(session.confirm_overwrite(&mut e).is_err());
    assert_eq!(project::load(&active).unwrap().0, edited);
    assert_eq!(e.song, edited);
    assert!(!e.dirty && e.can_undo());
    assert_eq!(session.path, Some(active));
    assert!(matches!(session.pending, Some(Action::Open(_))));
    assert!(session.overwrite.is_none() && !session.close_ready);
    session.cancel();
    e.undo();
    assert!(e.dirty);
    e.redo();
    assert_eq!(e.song, edited);
    assert!(!e.dirty);
}

#[cfg(unix)]
#[test]
fn overwrite_confirmation_captures_the_resolved_parent_directory() {
    let dir = Dir::new();
    let first = dir.path("first");
    let second = dir.path("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    for base in [&first, &second] {
        std::fs::write(base.join("song.json"), b"same original bytes").unwrap();
    }
    let alias = dir.path("alias");
    std::os::unix::fs::symlink(&first, &alias).unwrap();
    let mut e = Editor::default();
    let mut session = Session::default();
    session
        .save_request(&mut e, alias.join("song.json"), false)
        .unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&second, &alias).unwrap();
    session.confirm_overwrite(&mut e).unwrap();
    assert_eq!(project::load(&first.join("song.json")).unwrap().0, e.song);
    assert_eq!(
        std::fs::read(second.join("song.json")).unwrap(),
        b"same original bytes"
    );
    assert_eq!(session.path, Some(first.join("song.json")));
}

#[cfg(unix)]
#[test]
fn staged_open_captures_the_resolved_parent_directory() {
    let dir = Dir::new();
    let first = dir.path("first");
    let second = dir.path("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    for (base, title) in [(&first, "first"), (&second, "second")] {
        let mut song = example_song();
        song.title = title.into();
        project::save(&song, &base.join("song.json"), SaveMode::CreateNew).unwrap();
    }
    let alias = dir.path("alias");
    std::os::unix::fs::symlink(&first, &alias).unwrap();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.enter_note(Note::Note(72)).unwrap();
    let mut session = Session::default();
    session
        .request(&mut e, Action::Open(alias.join("song.json")))
        .unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&second, &alias).unwrap();
    session.discard(&mut e).unwrap();
    assert_eq!(e.song.title, "first");
    assert_eq!(session.path, Some(first.join("song.json")));
}

#[test]
fn project_size_limit_rejects_reads_and_writes_without_publication() {
    let dir = Dir::new();
    let oversized = dir.path("oversized.json");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(project::existing_bytes(&oversized)
        .unwrap_err()
        .contains("8 MiB"));
    assert!(project::load(&oversized).unwrap_err().contains("8 MiB"));
    let mut song = example_song();
    song.title = "x".repeat(8 * 1024 * 1024);
    assert!(
        project::save(&song, &dir.path("new.json"), SaveMode::CreateNew)
            .unwrap_err()
            .contains("8 MiB")
    );
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn concurrent_new_saves_publish_one_complete_project_without_clobbering() {
    use std::sync::{Arc, Barrier};
    let dir = Dir::new();
    let path = dir.path("winner.json");
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["first writer", "second writer"]
        .into_iter()
        .map(|title| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut song = example_song();
                song.title = title.into();
                barrier.wait();
                (
                    project::save(&song, &path, SaveMode::CreateNew).is_ok(),
                    song,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|(saved, _)| *saved).count(), 1);
    let winner = &results.iter().find(|(saved, _)| *saved).unwrap().1;
    assert_eq!(&project::load(&path).unwrap().0, winner);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn non_file_assets_are_rejected_before_project_publication() {
    let dir = Dir::new();
    let mut song = example_song();
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(dir.0.clone());
    let path = dir.path("song.json");
    assert!(project::save(&song, &path, SaveMode::CreateNew)
        .unwrap_err()
        .contains("sample is not a file"));
    assert!(!path.exists());
    std::fs::write(&path, serde_json::to_vec(&document(song)).unwrap()).unwrap();
    assert!(project::load(&path)
        .unwrap_err()
        .contains("sample is not a file"));
}

#[test]
fn failed_confirmed_save_preserves_active_document_cursor_and_both_history_stacks() {
    let dir = Dir::new();
    let active = dir.path("active.json");
    let other = dir.path("other.json");
    std::fs::write(&other, b"original target").unwrap();
    let mut session = Session::default();
    let mut e = Editor {
        edit_mode: true,
        edit_step: 0,
        ..Editor::default()
    };
    session.save_request(&mut e, active.clone(), false).unwrap();
    e.enter_note(Note::Note(73)).unwrap();
    let edited = e.song.clone();
    e.field = Field::Pan;
    e.enter_value("0.5").unwrap();
    e.undo();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    session.request(&mut e, Action::Close).unwrap();
    session.save_request(&mut e, other.clone(), true).unwrap();
    std::fs::write(&other, b"external writer").unwrap();
    assert!(session.confirm_overwrite(&mut e).is_err());
    assert_eq!(e.song, edited);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(e.dirty && e.can_undo() && e.can_redo());
    assert_eq!(session.path, Some(active));
    assert!(matches!(session.pending, Some(Action::Close)));
    assert!(!session.close_ready);
    assert_eq!(std::fs::read(&other).unwrap(), b"external writer");
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn approved_close_does_not_accept_new_project_actions_while_cleanup_is_pending() {
    let dir = Dir::new();
    let mut session = Session::default();
    let mut e = Editor::default();
    let original = e.song.clone();
    session.request(&mut e, Action::Close).unwrap();
    assert!(session.close_ready);
    assert!(session.request(&mut e, Action::New).is_err());
    assert!(session
        .save_request(&mut e, dir.path("late-save.json"), false)
        .is_err());
    assert_eq!(e.song, original);
    assert!(session.path.is_none() && session.pending.is_none());
    assert!(!dir.path("late-save.json").exists());
}

#[test]
fn step_effects_roundtrip_preserves_all_fields_and_saved_state() {
    let dir = Dir::new();
    let path = dir.path("effects.json");
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.field = Field::Delay;
    e.enter_value("0.1").unwrap();
    e.set_effects(StepEffects {
        retrigger: Some(Retrigger {
            repeats: 2,
            spacing: 0.1,
        }),
        // Decimal equality with delay + 2 * spacing must survive f64 rounding.
        release: Some(0.3),
        glide: Some(Glide {
            target: 72,
            semitones_per_second: 12.0,
        }),
        ..Default::default()
    })
    .unwrap();
    e.row = 1;
    e.set_effects(StepEffects {
        release: Some(0.4),
        glide: Some(Glide {
            target: 48,
            semitones_per_second: 3.0,
        }),
        ..Default::default()
    })
    .unwrap();
    let expected = e.song.clone();
    let mut session = Session::default();
    session.save_request(&mut e, path.clone(), false).unwrap();
    assert!(!e.dirty);
    let encoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(encoded["version"], 1);
    assert_eq!(encoded["song"]["schema"], 1);
    let encoded_cell = &encoded["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"];
    assert_eq!(encoded_cell["effects"]["retrigger"]["repeats"], 2);
    assert_eq!(encoded_cell["effects"]["release"], 0.3);
    assert_eq!(encoded_cell["effects"]["glide"]["target"], 72);
    assert_eq!(encoded_cell["delay"], 0.1);
    assert_eq!(project::load(&path).unwrap().0, expected);
    e.undo();
    assert!(e.dirty);
    e.redo();
    assert!(!e.dirty);
    session.request(&mut e, Action::New).unwrap();
    session.request(&mut e, Action::Open(path)).unwrap();
    assert_eq!(e.song, expected);
    assert!(!e.dirty && !e.can_undo() && !e.can_redo());
}

#[test]
fn legacy_schema_one_projects_default_to_no_effects_and_omit_empty_effects_on_save() {
    let dir = Dir::new();
    let path = dir.path("legacy.json");
    let expected = example_song();
    let mut legacy = document(expected.clone());
    // The old five-field cell shape is intentionally explicit, independent of serde defaults.
    legacy["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"] = serde_json::json!({
        "note": {"Note": 49}, "instrument": 1, "velocity": 0.7,
        "pan": null, "delay": null
    });
    let bytes = serde_json::to_vec(&legacy).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("effects"));
    std::fs::write(&path, &bytes).unwrap();
    let (loaded, _) = project::load(&path).unwrap();
    assert_eq!(loaded, expected);
    for pattern in loaded.patterns.values() {
        for track in pattern.tracks.values() {
            for row in track.values() {
                for cell in row.values() {
                    assert!(cell.effects.is_empty());
                }
            }
        }
    }
    project::save(&loaded, &dir.path("roundtrip.json"), SaveMode::CreateNew).unwrap();
    assert!(!std::fs::read_to_string(dir.path("roundtrip.json"))
        .unwrap()
        .contains("effects"));
    // An explicit empty object is also a supported no-command representation.
    legacy["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["effects"] = serde_json::json!({});
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(project::load(&path).unwrap().0, expected);
}

#[test]
fn invalid_or_unknown_project_effects_preserve_current_document_and_history() {
    let dir = Dir::new();
    let active = dir.path("active.json");
    let bad = dir.path("invalid-effects.json");
    let mut session = Session::default();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    session.save_request(&mut e, active.clone(), false).unwrap();
    e.set_effects(StepEffects {
        release: Some(0.5),
        ..Default::default()
    })
    .unwrap();
    e.field = Field::Pan;
    e.enter_value("0.5").unwrap();
    e.undo();
    let before = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for effects in [
        serde_json::json!({"offset": 0.25}),
        serde_json::json!({"reverse": true}),
        serde_json::json!({"cut": 1.0}),
        serde_json::json!({"retrigger": {"repeats": 0, "spacing": 0.1}}),
        serde_json::json!({"retrigger": {"repeats": 2, "spacing": 0.5}}),
        serde_json::json!({"retrigger": {"repeats": 2, "spacing": 0.1, "future": true}}),
        serde_json::json!({"release": 1.0}),
        serde_json::json!({"glide": {"target": 128, "semitones_per_second": 1.0}}),
        serde_json::json!({"glide": {"target": 60, "semitones_per_second": -1.0}}),
    ] {
        let mut invalid = document(example_song());
        invalid["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["effects"] = effects;
        std::fs::write(&bad, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session.request(&mut e, Action::Open(bad.clone())).unwrap();
        assert!(session.discard(&mut e).is_err());
        assert_eq!(e.song, before);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(e.dirty && e.can_undo() && e.can_redo());
        assert_eq!(session.path, Some(active.clone()));
        assert!(session.pending.is_some());
        session.cancel();
    }
}

#[test]
fn invalid_effect_save_never_replaces_existing_project_or_publishes_temp_files() {
    let dir = Dir::new();
    let path = dir.path("existing.json");
    let mut song = example_song();
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let expected = std::fs::read(&path).unwrap();
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
        .effects
        .release = Some(f64::NAN);
    assert!(project::save(&song, &path, SaveMode::ReplaceConfirmed(expected.clone())).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn asset_policy_rejects_before_metadata_or_open_of_a_missing_asset() {
    use std::cell::Cell as Count;
    let dir = Dir::new();
    let path = dir.path("policy.json");
    let mut song = example_song();
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some("missing.wav".into());
    std::fs::write(&path, serde_json::to_vec(&document(song)).unwrap()).unwrap();
    let calls = Count::new(0);
    let error = project::load_with_asset_policy(&path, |asset| {
        calls.set(calls.get() + 1);
        assert_eq!(asset, dir.path("missing.wav"));
        Err("test policy denied access".into())
    })
    .unwrap_err();
    assert_eq!(calls.get(), 1);
    assert!(error.contains("Instrument 1") && error.contains("test policy denied access"));
    assert!(!error.contains("missing/unreadable"));
    // Default loading still performs its ordinary file checks, without this policy.
    assert!(project::load(&path)
        .unwrap_err()
        .contains("missing/unreadable"));
}

#[test]
fn asset_policy_receives_every_resolved_reference_and_preserves_normal_load() {
    use std::cell::RefCell;
    let dir = Dir::new();
    let first = dir.path("first.wav");
    let second = dir.path("second.wav");
    std::fs::write(&first, b"first fixture reference").unwrap();
    std::fs::write(&second, b"second fixture reference").unwrap();
    let path = dir.path("policy.json");
    let mut song = example_song();
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some("first.wav".into());
    song.instruments.get_mut(&InstrumentId(2)).unwrap().sample = Some(second.clone());
    std::fs::write(&path, serde_json::to_vec(&document(song)).unwrap()).unwrap();
    let seen = RefCell::new(Vec::new());
    let loaded = project::load_with_asset_policy(&path, |asset| {
        assert!(asset.is_absolute() && asset.starts_with(&dir.0));
        seen.borrow_mut().push(asset.to_owned());
        Ok(())
    })
    .unwrap();
    assert_eq!(*seen.borrow(), vec![first, second]);
    assert_eq!(loaded, project::load(&path).unwrap());
}

#[cfg(unix)]
#[test]
fn asset_policy_can_deny_parent_traversal_and_symlink_escape() {
    let dir = Dir::new();
    let allowed = dir.path("allowed");
    let outside = dir.path("outside");
    std::fs::create_dir(&allowed).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let external_asset = outside.join("sample.wav");
    std::fs::write(&external_asset, b"outside fixture reference").unwrap();
    std::os::unix::fs::symlink(&external_asset, allowed.join("alias.wav")).unwrap();
    let path = allowed.join("policy.json");
    for reference in ["../outside/sample.wav", "alias.wav"] {
        let mut song = example_song();
        song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(reference.into());
        std::fs::write(&path, serde_json::to_vec(&document(song)).unwrap()).unwrap();
        let error = project::load_with_asset_policy(&path, |asset| {
            let resolved = asset.canonicalize().map_err(|e| e.to_string())?;
            if !resolved.starts_with(&allowed) {
                return Err("outside authorized root".into());
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("outside authorized root"));
    }
    assert_eq!(
        std::fs::read(external_asset).unwrap(),
        b"outside fixture reference"
    );
}

#[test]
fn advanced_step_commands_save_and_reopen_without_losing_musical_fields() {
    let dir = Dir::new();
    let path = dir.path("advanced-effects.json");
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let original_cell = e.cell();
    let advanced = StepEffects {
        retrigger: Some(Retrigger {
            repeats: 2,
            spacing: 0.1,
        }),
        cut: Some(0.6),
        offset: Some(42),
        direction: Some(Direction::Reverse),
        glide: Some(Glide {
            target: 72,
            semitones_per_second: 12.0,
        }),
        ..Default::default()
    };
    e.set_effects(advanced.clone()).unwrap();
    let expected = e.song.clone();
    let mut session = Session::default();
    session.save_request(&mut e, path.clone(), false).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let encoded = &json["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["effects"];
    assert_eq!(encoded["cut"], 0.6);
    assert_eq!(encoded["offset"], 42);
    assert_eq!(encoded["direction"], "Reverse");
    assert_eq!(encoded["retrigger"]["repeats"], 2);
    assert_eq!(encoded["glide"]["target"], 72);
    assert_eq!(project::load(&path).unwrap().0, expected);
    session.request(&mut e, Action::New).unwrap();
    session.request(&mut e, Action::Open(path)).unwrap();
    assert_eq!(e.song, expected);
    assert_eq!(e.cell().effects, advanced);
    let mut musical_fields = e.cell();
    musical_fields.effects = StepEffects::default();
    assert_eq!(musical_fields, original_cell);
    assert!(!e.dirty && !e.can_undo());
}

#[test]
fn earlier_effect_objects_roundtrip_without_inserting_new_command_fields() {
    let dir = Dir::new();
    let original = dir.path("earlier-effects.json");
    let copy = dir.path("earlier-effects-copy.json");
    let mut data = document(example_song());
    let earlier = serde_json::json!({
        "retrigger": {"repeats": 2, "spacing": 0.1},
        "release": 0.5,
        "glide": {"target": 72, "semitones_per_second": 12.0}
    });
    data["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["effects"] = earlier.clone();
    std::fs::write(&original, serde_json::to_vec(&data).unwrap()).unwrap();
    let (song, _) = project::load(&original).unwrap();
    let effects = &song.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0].effects;
    assert!(effects.cut.is_none() && effects.offset.is_none() && effects.direction.is_none());
    project::save(&song, &copy, SaveMode::CreateNew).unwrap();
    let encoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&copy).unwrap()).unwrap();
    assert_eq!(
        encoded["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["effects"],
        earlier
    );
    assert_eq!(project::load(&copy).unwrap().0, song);
}

fn persisted_phrase() -> Phrase {
    use std::collections::BTreeMap;
    Phrase {
        name: "Persisted motif".into(),
        instrument: InstrumentId(1),
        rows: 8,
        lines_per_beat: 8,
        note_columns: 2,
        root_note: 60,
        cells: BTreeMap::from([
            (
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        velocity: Some(0.6),
                        pan: Some(-0.2),
                        delay: Some(0.1),
                        effects: StepEffects {
                            cut: Some(0.8),
                            offset: Some(3),
                            direction: Some(Direction::Reverse),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )]),
            ),
            (
                7,
                BTreeMap::from([(
                    1,
                    Cell {
                        note: Note::Off,
                        ..Default::default()
                    },
                )]),
            ),
        ]),
    }
}

#[test]
fn phrase_definitions_and_stable_triggers_roundtrip_through_session_and_asset_policy() {
    use std::cell::Cell as Count;
    let dir = Dir::new();
    let sample = dir.path("owner.wav");
    std::fs::write(&sample, b"asset reference fixture").unwrap();
    let path = dir.path("phrases.json");
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.assign_sample(InstrumentId(1), Some(sample.clone()))
        .unwrap();
    e.upsert_phrase(PhraseId(17), persisted_phrase()).unwrap();
    e.set_phrase_trigger(Some(PhraseTrigger {
        phrase: PhraseId(17),
        cycles: 3,
    }))
    .unwrap();
    let expected = e.song.clone();
    let mut session = Session::default();
    session.save_request(&mut e, path.clone(), false).unwrap();
    assert!(!e.dirty);
    let encoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(encoded["version"], 1);
    assert_eq!(encoded["song"]["schema"], 1);
    assert_eq!(encoded["song"]["phrases"]["17"]["instrument"], 1);
    assert_eq!(encoded["song"]["phrases"]["17"]["lines_per_beat"], 8);
    assert_eq!(
        encoded["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["phrase"]["cycles"],
        3
    );
    assert_eq!(encoded["song"]["instruments"]["1"]["sample"], "owner.wav");
    let calls = Count::new(0);
    let (loaded, _) = project::load_with_asset_policy(&path, |asset| {
        assert_eq!(asset, sample);
        calls.set(calls.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        calls.get(),
        1,
        "phrase uses its owner's existing sample-policy path"
    );
    assert_eq!(loaded, expected);
    assert!(
        project::load_with_asset_policy(&path, |_| Err("owner denied".into()))
            .unwrap_err()
            .contains("owner denied")
    );
    e.undo();
    assert!(e.dirty);
    e.redo();
    assert!(!e.dirty);
    session.request(&mut e, Action::New).unwrap();
    assert!(e.song.phrases.is_empty());
    session.request(&mut e, Action::Open(path)).unwrap();
    assert_eq!(e.song, expected);
    assert_eq!(
        e.cell().phrase,
        Some(PhraseTrigger {
            phrase: PhraseId(17),
            cycles: 3
        })
    );
    assert!(!e.dirty && !e.can_undo());
}

#[test]
fn legacy_projects_without_phrases_keep_their_serialized_shape() {
    let dir = Dir::new();
    let original = dir.path("legacy.json");
    let copy = dir.path("copy.json");
    let song = example_song();
    project::save(&song, &original, SaveMode::CreateNew).unwrap();
    let original_bytes = std::fs::read(&original).unwrap();
    let text = String::from_utf8(original_bytes.clone()).unwrap();
    assert!(!text.contains("\"phrases\"") && !text.contains("\"phrase\""));
    assert!(!text.contains("\"samples\"") && !text.contains("\"regions\""));
    assert!(!text.contains("\"region\""));
    assert!(!text.contains("\"marker_sets\""));
    assert!(!text.contains("\"drum_kits\"") && !text.contains("\"kind\""));
    assert!(!text.contains("\"track_aliases\"") && !text.contains("\"muted_tracks\""));
    let (loaded, _) = project::load(&original).unwrap();
    assert!(loaded.phrases.is_empty());
    assert!(loaded.samples.is_empty() && loaded.regions.is_empty());
    assert!(loaded.marker_sets.is_empty() && loaded.drum_kits.is_empty());
    assert_eq!(loaded, song);
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), original_bytes);
    let mut explicit: serde_json::Value = serde_json::from_slice(&original_bytes).unwrap();
    explicit["song"]["phrases"] = serde_json::json!({});
    explicit["song"]["samples"] = serde_json::json!({});
    explicit["song"]["regions"] = serde_json::json!({});
    explicit["song"]["marker_sets"] = serde_json::json!({});
    explicit["song"]["drum_kits"] = serde_json::json!({});
    explicit["song"]["patterns"]["1"]["track_aliases"] = serde_json::json!({});
    explicit["song"]["order"][0]["muted_tracks"] = serde_json::json!([]);
    explicit["song"]["instruments"]["1"]["kind"] = serde_json::json!({"type":"Pitched"});
    explicit["song"]["instruments"]["1"]["region"] = serde_json::Value::Null;
    explicit["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["phrase"] = serde_json::Value::Null;
    std::fs::write(&original, serde_json::to_vec(&explicit).unwrap()).unwrap();
    assert_eq!(project::load(&original).unwrap().0, song);
}

#[test]
fn invalid_phrase_projects_preserve_current_dirty_document_and_history() {
    let dir = Dir::new();
    let active = dir.path("active.json");
    let invalid_path = dir.path("invalid-phrase.json");
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let mut session = Session::default();
    session.save_request(&mut e, active.clone(), false).unwrap();
    e.upsert_phrase(PhraseId(17), persisted_phrase()).unwrap();
    e.set_phrase_trigger(Some(PhraseTrigger {
        phrase: PhraseId(17),
        cycles: 1,
    }))
    .unwrap();
    e.undo();
    let expected = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut base = document(expected.clone());
    base["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["phrase"] =
        serde_json::json!({"phrase":17,"cycles":1});
    for (pointer, value) in [
        ("/song/phrases/17/instrument", serde_json::json!(999)),
        ("/song/phrases/17/rows", serde_json::json!(0)),
        ("/song/phrases/17/rows", serde_json::json!(7)),
        ("/song/phrases/17/lines_per_beat", serde_json::json!(0)),
        ("/song/phrases/17/note_columns", serde_json::json!(1)),
        ("/song/phrases/17/root_note", serde_json::json!(128)),
        (
            "/song/phrases/17/cells/0/0/instrument",
            serde_json::json!(2),
        ),
        (
            "/song/patterns/1/tracks/1/0/0/phrase/phrase",
            serde_json::json!(999),
        ),
        (
            "/song/patterns/1/tracks/1/0/0/phrase/cycles",
            serde_json::json!(0),
        ),
    ] {
        let mut invalid = base.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session
            .request(&mut e, Action::Open(invalid_path.clone()))
            .unwrap();
        assert!(session.discard(&mut e).is_err(), "{pointer}");
        assert_eq!(e.song, expected);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(e.dirty && e.can_undo() && e.can_redo());
        assert_eq!(session.path, Some(active.clone()));
        session.cancel();
    }
    let mut nested = base;
    nested["song"]["phrases"]["17"]["cells"]["0"]["0"]["phrase"] =
        serde_json::json!({"phrase":17,"cycles":1});
    std::fs::write(&invalid_path, serde_json::to_vec(&nested).unwrap()).unwrap();
    assert!(project::load(&invalid_path).is_err());
    assert_eq!(e.song, expected);
    let active_bytes = std::fs::read(&active).unwrap();
    let mut invalid_save = expected.clone();
    invalid_save.phrases.get_mut(&PhraseId(17)).unwrap().rows = 0;
    assert!(project::save(
        &invalid_save,
        &active,
        SaveMode::ReplaceConfirmed(active_bytes.clone())
    )
    .is_err());
    assert_eq!(std::fs::read(&active).unwrap(), active_bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

fn imported_source(dir: &Dir) -> SampleAsset {
    let path = dir.path("source.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..32i16 {
        writer.write_sample(n * 100).unwrap();
    }
    writer.finalize().unwrap();
    aqua_pattern_core::sample_asset::import(&path)
        .unwrap()
        .asset
}
fn sourced_song(dir: &Dir) -> Song {
    let mut song = example_song();
    song.register_sample(SampleId(11), imported_source(dir))
        .unwrap();
    song.upsert_region(
        RegionId(21),
        SampleRegion {
            name: "Tail".into(),
            amp_envelope: None,
            forward_loop: None,
            sample: SampleId(11),
            start_frame: 4,
            end_frame: 32,
            root_note: 60,
        },
    )
    .unwrap();
    song.assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    song.upsert_phrase(PhraseId(17), persisted_phrase())
        .unwrap();
    song
}

#[test]
fn imported_sources_regions_and_phrase_owner_mappings_roundtrip_without_copying_audio() {
    use std::cell::RefCell;
    let dir = Dir::new();
    let song = sourced_song(&dir);
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let path = dir.path("regions.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["version"], 1);
    assert_eq!(stored["song"]["samples"]["11"]["path"], "source.wav");
    assert_eq!(stored["song"]["samples"]["11"]["frames"], 32);
    assert_eq!(stored["song"]["instruments"]["1"]["region"], 21);
    assert_eq!(
        stored["song"]["instruments"]["1"]["sample"],
        serde_json::Value::Null
    );
    let calls = RefCell::new(Vec::new());
    let loaded = project::load_with_asset_policy(&path, |p| {
        calls.borrow_mut().push(p.to_owned());
        Ok(())
    })
    .unwrap()
    .0;
    assert_eq!(loaded, song);
    assert_eq!(*calls.borrow(), vec![dir.path("source.wav")]);
    let elsewhere = dir.path("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let copy = elsewhere.join("copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(project::load(&copy).unwrap().0, song);
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 1);
    let mut editor = Editor::default();
    let mut session = Session::default();
    session.request(&mut editor, Action::Open(path)).unwrap();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty && !editor.can_undo());
}

#[test]
fn registered_source_policy_precedes_file_access_even_for_unused_definitions() {
    use std::cell::Cell as Counter;
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.assign_region(InstrumentId(1), None).unwrap();
    song.remove_region(RegionId(21)).unwrap();
    song.samples.get_mut(&SampleId(11)).unwrap().path = "missing.wav".into();
    let path = dir.path("outside.json");
    std::fs::write(&path, serde_json::to_vec(&document(song)).unwrap()).unwrap();
    let calls = Counter::new(0);
    let err = project::load_with_asset_policy(&path, |p| {
        calls.set(calls.get() + 1);
        assert_eq!(p, dir.path("missing.wav"));
        Err("source is outside allowed root".into())
    })
    .unwrap_err();
    assert!(
        err.contains("Source 11") && err.contains("denied") && !err.contains("missing/unreadable")
    );
    assert_eq!(calls.get(), 1);
    assert!(project::load(&path)
        .unwrap_err()
        .contains("missing/unreadable"));
}

#[test]
fn changed_registered_bytes_fail_save_and_open_without_history_or_file_loss() {
    let dir = Dir::new();
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    editor.apply_document_edit(sourced_song(&dir)).unwrap();
    let path = dir.path("registered.json");
    let mut session = Session::default();
    session
        .save_request(&mut editor, path.clone(), false)
        .unwrap();
    let saved_bytes = std::fs::read(&path).unwrap();
    editor
        .assign_region(InstrumentId(2), Some(RegionId(21)))
        .unwrap();
    editor
        .assign_region(InstrumentId(3), Some(RegionId(21)))
        .unwrap();
    editor.undo();
    let before = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let source = dir.path("source.wav");
    let original = std::fs::read(&source).unwrap();
    let mut changed = original.clone();
    *changed.last_mut().unwrap() ^= 1;
    std::fs::write(&source, changed).unwrap();
    let error = project::save(
        &editor.song,
        &path,
        SaveMode::ReplaceConfirmed(saved_bytes.clone()),
    )
    .unwrap_err();
    assert!(error.contains("SHA-256"));
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    assert!(project::load(&path).unwrap_err().contains("SHA-256"));
    session
        .request(&mut editor, Action::Open(path.clone()))
        .unwrap();
    assert!(session
        .discard(&mut editor)
        .unwrap_err()
        .contains("SHA-256"));
    assert_eq!(editor.song, before);
    assert_eq!(editor.revision(), revision);
    assert_eq!(
        (
            editor.occurrence,
            editor.track,
            editor.column,
            editor.row,
            editor.field
        ),
        cursor
    );
    assert!(editor.dirty && editor.can_undo() && editor.can_redo());
    assert_eq!(session.path, Some(path.clone()));
    session.cancel();
    std::fs::write(&source, original).unwrap();
    session.request(&mut editor, Action::Open(path)).unwrap();
    session.discard(&mut editor).unwrap();
    assert!(!editor.dirty && !editor.can_undo());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn explicit_legacy_and_region_assignment_switches_preserve_phrase_definitions() {
    let dir = Dir::new();
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    editor.apply_document_edit(sourced_song(&dir)).unwrap();
    let region_song = editor.song.clone();
    editor.assign_sample(InstrumentId(1), None).unwrap();
    assert_eq!(editor.song, region_song);
    assert!(editor
        .assign_sample(InstrumentId(1), Some(dir.path("absent.wav")))
        .is_err());
    assert_eq!(editor.song, region_song);
    editor
        .assign_sample(InstrumentId(1), Some(dir.path("source.wav")))
        .unwrap();
    let legacy = editor.song.clone();
    assert_eq!(legacy.instruments[&InstrumentId(1)].region, None);
    assert_eq!(
        legacy.instruments[&InstrumentId(1)].sample,
        Some(dir.path("source.wav"))
    );
    assert_eq!(legacy.phrases, region_song.phrases);
    assert_eq!(legacy.regions, region_song.regions);
    editor.undo();
    assert_eq!(editor.song, region_song);
    editor.redo();
    assert_eq!(editor.song, legacy);
    editor
        .assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    assert_eq!(editor.song, region_song);
    editor.assign_region(InstrumentId(1), None).unwrap();
    assert!(editor.song.instruments[&InstrumentId(1)].region.is_none());
    assert!(editor.song.instruments[&InstrumentId(1)].sample.is_none());
    editor.undo();
    assert_eq!(editor.song, region_song);
}

#[test]
fn registered_source_storage_budgets_preflight_before_hashing_or_publishing() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    let path = dir.path("too-large.wav");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    song.samples.get_mut(&SampleId(11)).unwrap().path = path.clone();
    let target = dir.path("budget.json");
    assert!(project::save(&song, &target, SaveMode::CreateNew)
        .unwrap_err()
        .contains("64 MiB encoded budget"));
    assert!(!target.exists());
    file.set_len(64 * 1024 * 1024).unwrap();
    let source = song.samples[&SampleId(11)].clone();
    for id in 12..=15 {
        song.register_sample(SampleId(id), source.clone()).unwrap();
    }
    assert!(project::save(&song, &target, SaveMode::CreateNew)
        .unwrap_err()
        .contains("256 MiB aggregate"));
    assert!(!target.exists());
    std::fs::write(&target, serde_json::to_vec(&document(song)).unwrap()).unwrap();
    assert!(project::load(&target)
        .unwrap_err()
        .contains("256 MiB aggregate"));
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

#[test]
fn invalid_source_metadata_regions_and_conflicting_mappings_never_replace_document() {
    let dir = Dir::new();
    let path = dir.path("invalid-source.json");
    let song = sourced_song(&dir);
    let valid = document(song);
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    editor.enter_note(Note::Note(70)).unwrap();
    editor.enter_note(Note::Note(71)).unwrap();
    editor.undo();
    let before = editor.song.clone();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let mut session = Session::default();
    for (pointer, value) in [
        ("/song/samples/11/sha256", serde_json::json!("not-a-sha")),
        ("/song/samples/11/sample_rate", serde_json::json!(0)),
        ("/song/samples/11/channels", serde_json::json!(3)),
        ("/song/samples/11/frames", serde_json::json!(0)),
        ("/song/regions/21/sample", serde_json::json!(999)),
        ("/song/regions/21/start_frame", serde_json::json!(32)),
        ("/song/regions/21/end_frame", serde_json::json!(33)),
        ("/song/regions/21/root_note", serde_json::json!(128)),
        ("/song/instruments/1/region", serde_json::json!(999)),
        (
            "/song/instruments/1/sample",
            serde_json::json!("source.wav"),
        ),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session
            .request(&mut editor, Action::Open(path.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err(), "{pointer}");
        assert_eq!(editor.song, before);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_undo() && editor.can_redo());
        assert!(session.path.is_none());
        session.cancel();
    }
}

#[cfg(unix)]
#[test]
fn registered_source_root_policy_catches_symlink_escape_before_hashing() {
    let dir = Dir::new();
    let song = sourced_song(&dir);
    let allowed = dir.path("allowed");
    std::fs::create_dir(&allowed).unwrap();
    std::os::unix::fs::symlink(dir.path("source.wav"), allowed.join("link.wav")).unwrap();
    let mut data = document(song);
    data["song"]["samples"]["11"]["path"] = serde_json::json!("link.wav");
    let path = allowed.join("song.json");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let error = project::load_with_asset_policy(&path, |p| {
        if p.canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&allowed)
        {
            Ok(())
        } else {
            Err("escapes the allowed source root".into())
        }
    })
    .unwrap_err();
    assert!(error.contains("Source 11") && error.contains("denied"));
    assert!(
        project::load(&path).is_ok(),
        "normal project load allows valid asset symlinks"
    );
}

fn persisted_markers() -> MarkerSet {
    MarkerSet {
        name: "User-edited break boundaries".into(),
        locked_frames: Vec::new(),
        sample: SampleId(11),
        source_bpm: None,
        markers: vec![0, 8, 16, 32],
    }
}
fn persisted_compile_config() -> aqua_pattern_core::break_compiler::CompileConfig {
    use aqua_pattern_core::break_compiler::{CompileConfig, CompileTiming, TailPolicy};
    CompileConfig {
        marker_set: MarkerSetId(31),
        target_bpm: 175.0,
        timing: CompileTiming::SourceRhythm,
        cycles: 1,
        roll: None,
        tail: TailPolicy::Overlap,
        minimum_slice_frames: 1,
        root_note: 60,
        pad_tails: false,
        name: "Saved compiler output".into(),
    }
}

#[test]
fn accepted_manual_markers_and_explicit_source_tempo_roundtrip_independently() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.upsert_marker_set(MarkerSetId(31), persisted_markers())
        .unwrap();
    let unknown = dir.path("unknown-source-tempo.json");
    project::save(&song, &unknown, SaveMode::CreateNew).unwrap();
    assert_eq!(project::load(&unknown).unwrap().0, song);
    assert_eq!(
        project::load(&unknown).unwrap().0.marker_sets[&MarkerSetId(31)].source_bpm,
        None
    );
    let mut known = persisted_markers();
    known.source_bpm = Some(122.5);
    known.markers = vec![0, 7, 19, 32];
    song.upsert_marker_set(MarkerSetId(31), known.clone())
        .unwrap();
    song.bpm = 175.0;
    song.lines_per_beat = 8;
    let path = dir.path("known-source-tempo.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let (loaded, _) = project::load(&path).unwrap();
    assert_eq!(loaded, song);
    assert_eq!(loaded.marker_sets[&MarkerSetId(31)], known);
    assert_eq!(loaded.bpm, 175.0);
    assert_eq!(loaded.lines_per_beat, 8);
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["song"]["marker_sets"]["31"]["source_bpm"], 122.5);
    assert_eq!(stored["song"]["samples"]["11"]["path"], "source.wav");
    assert!(stored["song"]["marker_sets"]["31"]
        .get("suggestions")
        .is_none());
}

#[test]
fn compiler_commit_save_reopen_and_undo_preserve_manual_markers_and_distinct_tempos() {
    use aqua_pattern_core::break_compiler;
    let dir = Dir::new();
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    editor.apply_document_edit(sourced_song(&dir)).unwrap();
    let mut markers = persisted_markers();
    markers.source_bpm = Some(120.0);
    editor
        .upsert_marker_set(MarkerSetId(31), markers.clone())
        .unwrap();
    let mut session = Session::default();
    let path = dir.path("compiled.json");
    session
        .save_request(&mut editor, path.clone(), false)
        .unwrap();
    let base = editor.song.clone();
    let revision = editor.revision();
    let preview = break_compiler::preview(&base, &persisted_compile_config()).unwrap();
    assert_eq!(editor.song, base);
    assert!(!editor.dirty);
    editor.commit_preview(revision, &preview).unwrap();
    let compiled = editor.song.clone();
    assert_eq!(compiled.marker_sets[&MarkerSetId(31)], markers);
    assert_eq!(compiled.bpm, 175.0);
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(!editor.dirty);
    editor.redo();
    assert_eq!(editor.song, compiled);
    session
        .save_request(&mut editor, path.clone(), false)
        .unwrap();
    session.confirm_overwrite(&mut editor).unwrap();
    assert!(!editor.dirty);
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(editor.dirty, "saved baseline is the generated result");
    editor.redo();
    assert!(!editor.dirty);
    session.request(&mut editor, Action::New).unwrap();
    session.request(&mut editor, Action::Open(path)).unwrap();
    assert_eq!(editor.song, compiled);
    assert!(!editor.dirty && !editor.can_undo());
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        2,
        "compilation writes no derived audio files"
    );
}

#[test]
fn invalid_marker_project_load_and_save_preserve_active_history_revision_and_bytes() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.upsert_marker_set(MarkerSetId(31), persisted_markers())
        .unwrap();
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    editor.apply_document_edit(song.clone()).unwrap();
    let path = dir.path("markers.json");
    let mut session = Session::default();
    session
        .save_request(&mut editor, path.clone(), false)
        .unwrap();
    let saved_bytes = std::fs::read(&path).unwrap();
    editor.set_timing(175.0, 8).unwrap();
    editor.set_timing(176.0, 8).unwrap();
    editor.undo();
    let current = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let valid = document(song.clone());
    let invalid_path = dir.path("invalid-markers.json");
    for (pointer, value) in [
        ("/song/marker_sets/31/sample", serde_json::json!(999)),
        ("/song/marker_sets/31/source_bpm", serde_json::json!(0)),
        ("/song/marker_sets/31/source_bpm", serde_json::json!(401)),
        (
            "/song/marker_sets/31/markers",
            serde_json::json!([1, 8, 32]),
        ),
        (
            "/song/marker_sets/31/markers",
            serde_json::json!([0, 8, 31]),
        ),
        (
            "/song/marker_sets/31/markers",
            serde_json::json!([0, 8, 8, 32]),
        ),
        (
            "/song/marker_sets/31/markers",
            serde_json::json!([0, 16, 8, 32]),
        ),
        (
            "/song/marker_sets/31/markers",
            serde_json::json!([0, 32, 33]),
        ),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session
            .request(&mut editor, Action::Open(invalid_path.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err(), "{pointer}");
        assert_eq!(editor.song, current);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_redo());
        session.cancel();
    }
    song.marker_sets.get_mut(&MarkerSetId(31)).unwrap().markers = vec![0, 32, 32];
    assert!(project::save(
        &song,
        &path,
        SaveMode::ReplaceConfirmed(saved_bytes.clone())
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    assert_eq!(editor.revision(), revision);
}

#[test]
fn persistent_marker_locks_roundtrip_and_old_unlocked_projects_keep_exact_json() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.upsert_marker_set(MarkerSetId(31), persisted_markers())
        .unwrap();
    let old_path = dir.path("old-unlocked.json");
    project::save(&song, &old_path, SaveMode::CreateNew).unwrap();
    let old_bytes = std::fs::read(&old_path).unwrap();
    assert!(!String::from_utf8(old_bytes.clone())
        .unwrap()
        .contains("locked_frames"));
    let (unlocked, _) = project::load(&old_path).unwrap();
    assert!(unlocked.marker_sets[&MarkerSetId(31)]
        .locked_frames
        .is_empty());
    let copy = dir.path("old-copy.json");
    project::save(&unlocked, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), old_bytes);
    song.set_marker_lock(MarkerSetId(31), 16, true).unwrap();
    song.set_marker_lock(MarkerSetId(31), 8, true).unwrap();
    let path = dir.path("locked.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        stored["song"]["marker_sets"]["31"]["locked_frames"],
        serde_json::json!([8, 16])
    );
    let mut editor = Editor::default();
    let mut session = Session::default();
    session.request(&mut editor, Action::Open(path)).unwrap();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty);
    editor.edit_mode = true;
    assert!(editor.move_marker(MarkerSetId(31), 8, 9).is_err());
    assert!(editor.remove_marker(MarkerSetId(31), 16).is_err());
    editor.set_marker_lock(MarkerSetId(31), 8, false).unwrap();
    assert!(editor.dirty);
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty);
}

#[test]
fn malformed_marker_lock_projects_fail_without_replacing_dirty_document_or_saved_bytes() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.upsert_marker_set(MarkerSetId(31), persisted_markers())
        .unwrap();
    song.set_marker_lock(MarkerSetId(31), 8, true).unwrap();
    let active = dir.path("locked.json");
    project::save(&song, &active, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&active).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(170.0, 8).unwrap();
    editor.set_timing(175.0, 8).unwrap();
    editor.undo();
    let before = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let mut session = Session {
        path: Some(active.clone()),
        ..Session::default()
    };
    let invalid_path = dir.path("bad-locks.json");
    for locks in [vec![0], vec![32], vec![7], vec![8, 8], vec![16, 8]] {
        let mut invalid = document(song.clone());
        invalid["song"]["marker_sets"]["31"]["locked_frames"] = serde_json::json!(locks);
        std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session
            .request(&mut editor, Action::Open(invalid_path.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err());
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_redo());
        assert_eq!(session.path, Some(active.clone()));
        session.cancel();
    }
    song.marker_sets
        .get_mut(&MarkerSetId(31))
        .unwrap()
        .locked_frames = vec![7];
    assert!(project::save(&song, &active, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(&active).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

fn persisted_drum_pad() -> DrumPad {
    DrumPad {
        slot: 0,
        name: "Snare".into(),
        velocity_zones: Vec::new(),
        note: 36,
        region: Some(RegionId(21)),
        root_note: 60,
        tune_semitones: 12,
        gain: 0.7,
        pan: -0.2,
        mode: PadMode::Gated,
        new_note: PadNewNote::ReleasePrevious,
        choke_group: Some(3),
    }
}
fn kit_song(dir: &Dir) -> Song {
    let mut song = sourced_song(dir);
    song.instruments.insert(
        InstrumentId(99),
        Instrument {
            name: "Drum kit instrument".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: None,
        },
    );
    let mut empty_pad = persisted_drum_pad();
    empty_pad.slot = 15;
    empty_pad.note = 51;
    empty_pad.region = None;
    empty_pad.tune_semitones = 0;
    empty_pad.mode = PadMode::OneShot;
    empty_pad.new_note = PadNewNote::Overlap;
    empty_pad.choke_group = None;
    song.upsert_drum_kit(
        DrumKitId(41),
        DrumKitInstrument {
            name: "Stored kit".into(),
            pads: [(PadId(501), persisted_drum_pad()), (PadId(999), empty_pad)].into(),
        },
    )
    .unwrap();
    song.assign_kit(InstrumentId(99), Some(DrumKitId(41)))
        .unwrap();
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .insert(
            0,
            Cell {
                note: Note::Note(36),
                instrument: Some(InstrumentId(99)),
                velocity: Some(0.4),
                pan: Some(-0.1),
                delay: Some(0.25),
                effects: StepEffects {
                    release: Some(0.75),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    song.validate().unwrap();
    song
}

#[test]
fn drum_kit_pad_parameters_and_ordinary_notes_roundtrip_with_existing_sources_and_phrases() {
    use std::cell::RefCell;
    let dir = Dir::new();
    let song = kit_song(&dir);
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let path = dir.path("kit.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["version"], 1);
    assert_eq!(
        stored["song"]["instruments"]["99"]["kind"],
        serde_json::json!({"type":"DrumKit","data":{"kit":41}})
    );
    assert!(stored["song"]["instruments"]["1"].get("kind").is_none());
    assert_eq!(
        stored["song"]["drum_kits"]["41"]["pads"]["501"]["tune_semitones"],
        12
    );
    assert!(stored["song"]["drum_kits"]["41"]["pads"]["999"]
        .get("tune_semitones")
        .is_none());
    assert_eq!(stored["song"]["samples"]["11"]["path"], "source.wav");
    let calls = RefCell::new(Vec::new());
    let loaded = project::load_with_asset_policy(&path, |p| {
        calls.borrow_mut().push(p.to_owned());
        Ok(())
    })
    .unwrap()
    .0;
    assert_eq!(loaded, song);
    assert_eq!(*calls.borrow(), vec![dir.path("source.wav")]);
    assert!(
        project::load_with_asset_policy(&path, |_| Err("outside allowed root".into())).is_err()
    );
    let mut e = Editor::default();
    let mut session = Session::default();
    session.request(&mut e, Action::Open(path)).unwrap();
    assert_eq!(e.song, song);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    let mut pad = persisted_drum_pad();
    pad.gain = 0.3;
    e.upsert_drum_pad(DrumKitId(41), PadId(501), pad).unwrap();
    assert!(e.dirty);
    e.undo();
    assert_eq!(e.song, song);
    assert!(!e.dirty);
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn kit_region_and_legacy_source_switches_are_explicit_and_undoable() {
    let dir = Dir::new();
    let mut e = Editor::default();
    e.replace_document(kit_song(&dir)).unwrap();
    e.edit_mode = true;
    let mapped = e.song.clone();
    let revision = e.revision();
    e.assign_sample(InstrumentId(99), None).unwrap();
    e.assign_region(InstrumentId(99), None).unwrap();
    assert_eq!(e.song, mapped);
    assert_eq!(e.revision(), revision);
    assert!(e
        .assign_kit(InstrumentId(99), Some(DrumKitId(999)))
        .is_err());
    assert!(e
        .assign_sample(InstrumentId(99), Some(dir.path("missing.wav")))
        .is_err());
    assert_eq!(e.song, mapped);
    e.assign_sample(InstrumentId(99), Some(dir.path("source.wav")))
        .unwrap();
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].kind,
        InstrumentKind::Pitched
    );
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].sample,
        Some(dir.path("source.wav"))
    );
    assert!(e.song.instruments[&InstrumentId(99)].region.is_none());
    assert_eq!(e.song.drum_kits, mapped.drum_kits);
    e.undo();
    assert_eq!(e.song, mapped);
    e.assign_region(InstrumentId(99), Some(RegionId(21)))
        .unwrap();
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].kind,
        InstrumentKind::Pitched
    );
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].region,
        Some(RegionId(21))
    );
    e.assign_kit(InstrumentId(99), Some(DrumKitId(41))).unwrap();
    assert_eq!(e.song, mapped);
    e.assign_kit(InstrumentId(99), None).unwrap();
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].kind,
        InstrumentKind::Pitched
    );
    assert!(e.song.instruments[&InstrumentId(99)].sample.is_none());
    assert!(e.song.instruments[&InstrumentId(99)].region.is_none());
    e.undo();
    assert_eq!(e.song, mapped);
}

#[test]
fn malformed_kit_projects_and_used_pad_unmapping_preserve_dirty_document_and_disk() {
    let dir = Dir::new();
    let song = kit_song(&dir);
    let active = dir.path("kit.json");
    project::save(&song, &active, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&active).unwrap();
    let mut e = Editor::default();
    e.replace_document(song.clone()).unwrap();
    e.edit_mode = true;
    e.set_timing(170.0, 8).unwrap();
    e.set_timing(175.0, 8).unwrap();
    e.undo();
    let before = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut session = Session {
        path: Some(active.clone()),
        ..Session::default()
    };
    let valid = document(song.clone());
    let invalid_path = dir.path("bad-kit.json");
    for (pointer, value) in [
        ("/song/instruments/99/kind/data/kit", serde_json::json!(999)),
        (
            "/song/instruments/99/sample",
            serde_json::json!("source.wav"),
        ),
        ("/song/drum_kits/41/pads/501/slot", serde_json::json!(16)),
        ("/song/drum_kits/41/pads/501/note", serde_json::json!(128)),
        ("/song/drum_kits/41/pads/501/region", serde_json::json!(999)),
        (
            "/song/drum_kits/41/pads/501/region",
            serde_json::Value::Null,
        ),
        (
            "/song/drum_kits/41/pads/501/tune_semitones",
            serde_json::json!(49),
        ),
        ("/song/drum_kits/41/pads/501/gain", serde_json::json!(1.1)),
        ("/song/drum_kits/41/pads/501/pan", serde_json::json!(-1.1)),
        (
            "/song/drum_kits/41/pads/501/mode",
            serde_json::json!("Loop"),
        ),
        (
            "/song/drum_kits/41/pads/501/choke_group",
            serde_json::json!(17),
        ),
        ("/song/drum_kits/41/pads/999/note", serde_json::json!(36)),
        ("/song/drum_kits/41/pads/999/slot", serde_json::json!(0)),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session
            .request(&mut e, Action::Open(invalid_path.clone()))
            .unwrap();
        assert!(session.discard(&mut e).is_err(), "{pointer}");
        assert_eq!(e.song, before);
        assert_eq!(e.revision(), revision);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(e.dirty && e.can_redo());
        assert_eq!(session.path, Some(active.clone()));
        session.cancel();
    }
    let mut invalid = song;
    invalid
        .drum_kits
        .get_mut(&DrumKitId(41))
        .unwrap()
        .pads
        .remove(&PadId(501));
    assert!(project::save(&invalid, &active, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(&active).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

#[test]
fn fresh_empty_kit_instrument_roundtrips_without_rebinding_existing_content() {
    let dir = Dir::new();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let original = e.song.clone();
    let (instrument, kit) = e
        .create_drum_kit_instrument("Independent drums".into())
        .unwrap();
    let path = dir.path("fresh-kit.json");
    project::save(&e.song, &path, SaveMode::CreateNew).unwrap();
    let (restored, _) = project::load(&path).unwrap();
    assert_eq!(restored, e.song);
    assert!(restored.drum_kits[&kit].pads.is_empty());
    assert_eq!(
        restored.instruments[&instrument].kind,
        InstrumentKind::DrumKit { kit }
    );
    let mut existing = restored;
    existing.instruments.remove(&instrument);
    existing.drum_kits.remove(&kit);
    assert_eq!(existing, original);
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        1,
        "no asset or auxiliary pattern files created"
    );
    e.undo();
    assert_eq!(e.song, original);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(project::load(&path).unwrap().0, e.song);
}

#[test]
fn drum_step_exact_offgrid_edit_saves_only_ordinary_cells_and_reopens_without_overlay_state() {
    use aqua_pattern_core::drum_steps::{self, StepDivision, StepEdit, StepView};
    let dir = Dir::new();
    let song = kit_song(&dir);
    let source = std::fs::read(dir.path("source.wav")).unwrap();
    let view = StepView {
        pattern: PatternId(1),
        track: TrackId(1),
        instrument: InstrumentId(99),
        division: StepDivision::Sixteenth,
        steps: 16,
        page: 0,
    };
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    let projection = drum_steps::project(&editor.song, &view).unwrap();
    let event = projection.lanes[0].steps[0].events[0].clone();
    assert!(!event.on_grid);
    editor
        .edit_drum_step(
            editor.revision(),
            &projection,
            &StepEdit::SetRetrigger {
                event,
                value: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.125,
                }),
            },
        )
        .unwrap();
    let edited = editor.song.clone();
    assert_eq!(edited.drum_kits, song.drum_kits);
    assert_eq!(edited.samples, song.samples);
    assert_eq!(edited.regions, song.regions);
    assert_eq!(edited.phrases, song.phrases);
    assert_eq!(edited.order, song.order);
    let path = dir.path("step-edit.json");
    project::save(&edited, &path, SaveMode::CreateNew).unwrap();
    let (reopened, _) = project::load(&path).unwrap();
    assert_eq!(reopened, edited);
    let json = std::fs::read_to_string(&path).unwrap();
    for transient_key in [
        "total_steps",
        "base_view",
        "on_grid",
        "conflicts",
        "projection_token",
    ] {
        assert!(!json.contains(transient_key));
    }
    assert_eq!(
        drum_steps::project(&reopened, &view).unwrap().lanes,
        drum_steps::project(&edited, &view).unwrap().lanes
    );
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source);
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty);
    editor.redo();
    assert_eq!(editor.song, reopened);
}

fn aliased_project() -> Song {
    let mut song = example_song();
    song.patterns.insert(
        PatternId(3),
        Pattern {
            generation: None,
            name: "Cross-track alias".into(),
            rows: 32,
            tracks: Default::default(),
            track_aliases: [(
                TrackId(2),
                TrackAlias {
                    pattern: PatternId(1),
                    track: TrackId(1),
                },
            )]
            .into(),
        },
    );
    song.order.push(Occurrence {
        pattern: PatternId(3),
        muted_tracks: [TrackId(2)].into(),
        section: None,
    });
    song.order[0].muted_tracks.insert(TrackId(1));
    song.validate().unwrap();
    song
}
#[test]
fn alias_graph_and_occurrence_mutes_roundtrip_without_shadow_rows_or_shared_mute_state() {
    let dir = Dir::new();
    let path = dir.path("aliases.json");
    let song = aliased_project();
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        json["song"]["patterns"]["3"]["track_aliases"]["2"],
        serde_json::json!({"pattern":1,"track":1})
    );
    assert_eq!(
        json["song"]["patterns"]["3"]["tracks"],
        serde_json::json!({})
    );
    assert_eq!(
        json["song"]["order"][0]["muted_tracks"],
        serde_json::json!([1])
    );
    assert!(json["song"]["order"][2].get("muted_tracks").is_none());
    let (loaded, _) = project::load(&path).unwrap();
    assert_eq!(loaded, song);
    let mut editor = Editor::default();
    editor.replace_document(loaded).unwrap();
    editor.select(3, TrackId(2)).unwrap();
    assert_eq!(editor.content_source().unwrap(), (PatternId(1), TrackId(1)));
    assert_eq!(
        editor.cell(),
        song.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0]
    );
    editor.edit_mode = true;
    editor.make_track_unique(PatternId(3), TrackId(2)).unwrap();
    let unique = editor.song.clone();
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty);
    editor.redo();
    assert_eq!(editor.song, unique);
    let copy = dir.path("unique.json");
    project::save(&editor.song, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(project::load(&copy).unwrap().0, unique);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
#[test]
fn invalid_alias_or_mute_load_and_overwrite_preserve_active_history_and_bytes() {
    let dir = Dir::new();
    let path = dir.path("active.json");
    let invalid_path = dir.path("invalid.json");
    let song = aliased_project();
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.select(3, TrackId(2)).unwrap();
    editor.set_timing(130.0, 4).unwrap();
    editor.set_timing(140.0, 4).unwrap();
    editor.undo();
    let before = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let mut session = Session {
        path: Some(path.clone()),
        ..Session::default()
    };
    let valid = document(song.clone());
    for (pointer, value) in [
        (
            "/song/patterns/3/track_aliases/2/pattern",
            serde_json::json!(999),
        ),
        (
            "/song/patterns/3/track_aliases/2/track",
            serde_json::json!(999),
        ),
        (
            "/song/patterns/3/track_aliases/2",
            serde_json::json!({"pattern":3,"track":2}),
        ),
        ("/song/patterns/3/rows", serde_json::json!(64)),
        ("/song/tracks/1/note_columns", serde_json::json!(3)),
        ("/song/patterns/3/tracks", serde_json::json!({"2":{}})),
        ("/song/order/3/muted_tracks", serde_json::json!([999])),
        (
            "/song/patterns/3/track_aliases/2",
            serde_json::json!({"pattern":1,"track":1,"offset":0}),
        ),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        session
            .request(&mut editor, Action::Open(invalid_path.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err(), "{pointer}");
        session.cancel();
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_redo());
        assert_eq!(session.path, Some(path.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    let mut invalid = song;
    invalid.patterns.get_mut(&PatternId(3)).unwrap().rows = 64;
    assert!(project::save(&invalid, &path, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn committed_transform_roundtrips_exact_generated_delay_without_persisting_preview_or_touching_assets(
) {
    use aqua_pattern_core::transforms::{self, TransformConfig, TransformFields, TransformScope};
    let dir = Dir::new();
    let path = dir.path("transformed.json");
    let mut song = sourced_song(&dir);
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .insert(
            1,
            [(
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(0.6),
                    pan: Some(-0.4),
                    delay: Some(0.25),
                    ..Default::default()
                },
            )]
            .into(),
        );
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    let config = TransformConfig {
        scope: TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 1,
            row_end: 2,
            columns: vec![0],
        },
        fields: TransformFields {
            notes: true,
            glide_targets: false,
            velocity: true,
            timing: true,
        },
        transpose_semitones: 2,
        velocity_scale: 0.5,
        nudge_frames: 37,
        swing: None,
    };
    let revision = editor.revision();
    let plan = transforms::preview(&song, config).unwrap();
    assert_eq!(editor.song, song);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty);
    editor.commit_transform_preview(revision, &plan).unwrap();
    let transformed = editor.song.clone();
    let changed = &transformed.patterns[&PatternId(1)].tracks[&TrackId(1)][&1][&0];
    assert_eq!(changed.note, Note::Note(62));
    assert_eq!(changed.velocity, Some(0.3));
    assert_eq!(changed.pan, Some(-0.4));
    assert_ne!(changed.delay, Some(0.25));
    assert_eq!(transformed.samples, song.samples);
    assert_eq!(transformed.regions, song.regions);
    assert_eq!(transformed.phrases, song.phrases);
    assert_eq!(transformed.order, song.order);
    assert_eq!(transformed.bpm, song.bpm);
    assert_eq!(transformed.lines_per_beat, song.lines_per_beat);
    project::save(&transformed, &path, SaveMode::CreateNew).unwrap();
    let (reopened, _) = project::load(&path).unwrap();
    assert_eq!(reopened, transformed);
    assert_eq!(
        reopened.patterns[&PatternId(1)].tracks[&TrackId(1)][&1][&0]
            .delay
            .unwrap()
            .to_bits(),
        changed.delay.unwrap().to_bits()
    );
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for key in [
        "transform",
        "transform_preview",
        "scope",
        "fields",
        "seed",
        "groove",
    ] {
        assert!(value["song"].get(key).is_none());
    }
    let copy = dir.path("copy.json");
    project::save(&reopened, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&path).unwrap());
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty);
    editor.redo();
    assert_eq!(editor.song, reopened);
}

#[test]
fn legacy_schema_one_literal_stays_byte_exact_without_generation_metadata() {
    // Frozen pre-generation schema-1 bytes, not produced by the current Song serializer.
    // The non-dyadic delay also pins the exact f64 roundtrip feature.
    let legacy = r#"{
  "format": "leviathan-song",
  "version": 1,
  "song": {
    "schema": 1,
    "title": "Legacy ordinary cell",
    "bpm": 120.0,
    "lines_per_beat": 4,
    "tracks": {
      "1": {
        "name": "Legacy track",
        "note_columns": 1
      }
    },
    "track_order": [
      1
    ],
    "instruments": {
      "1": {
        "name": "Legacy tone",
        "sample": null
      }
    },
    "patterns": {
      "1": {
        "name": "Legacy pattern",
        "rows": 4,
        "tracks": {
          "1": {
            "0": {
              "0": {
                "note": {
                  "Note": 60
                },
                "instrument": 1,
                "velocity": 0.0,
                "pan": null,
                "delay": 0.23703703703703694
              }
            }
          }
        }
      }
    },
    "order": [
      {
        "pattern": 1
      }
    ]
  }
}"#;
    let dir = Dir::new();
    let path = dir.path("legacy.json");
    std::fs::write(&path, legacy).unwrap();
    let (song, _) = project::load(&path).unwrap();
    let cell = &song.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0];
    assert_eq!(cell.velocity, Some(0.0));
    assert_eq!(cell.pan, None);
    assert_eq!(
        cell.delay.unwrap().to_bits(),
        0.23703703703703694f64.to_bits()
    );
    let copy = dir.path("legacy-copy.json");
    project::save(&song, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), legacy.as_bytes());
}

fn persisted_generation_config() -> aqua_pattern_core::generation::GenerationConfig {
    use aqua_pattern_core::generation::{GenerationConfig, GenerationNote, GenerationPreset};
    GenerationConfig {
        version: 1,
        name: "Persisted seeded take".into(),
        bars: 1,
        instrument: InstrumentId(1),
        seed: u64::MAX,
        preset: GenerationPreset::Euclidean {
            steps: 7,
            pulses: 7,
            rotation: 0,
            probability: 1.0,
        },
        sequence: vec![
            GenerationNote {
                note: 60,
                velocity: 0.35,
            },
            GenerationNote {
                note: 67,
                velocity: 0.9,
            },
        ],
    }
}
fn generation_project_song(dir: &Dir) -> Song {
    let mut song = sourced_song(dir);
    for instrument in song.instruments.values_mut() {
        if instrument.region.is_none() {
            instrument.sample = Some(dir.path("source.wav"));
        }
    }
    song
}
#[test]
fn generated_content_and_historical_seed_recipe_roundtrip_exactly_without_regeneration() {
    use aqua_pattern_core::generation;
    let dir = Dir::new();
    let song = generation_project_song(&dir);
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    let plan = generation::preview(&song, persisted_generation_config()).unwrap();
    let summary = plan.summary().clone();
    let first = plan.cells()[0].clone();
    assert!(plan.cells().iter().any(|event| event.cell.delay.is_some()));
    let provenance = plan.provenance().clone();
    editor
        .commit_generation_preview(editor.revision(), &plan)
        .unwrap();
    let generated = editor.song.clone();
    assert_eq!(generated.samples, song.samples);
    assert_eq!(generated.regions, song.regions);
    assert_eq!(generated.phrases, song.phrases);
    assert_eq!(generated.instruments, song.instruments);
    let path = dir.path("generated.json");
    project::save(&generated, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let history = &stored["song"]["patterns"][summary.pattern.0.to_string()]["generation"];
    assert_eq!(history["config"]["version"], 1);
    assert_eq!(history["config"]["seed"].as_u64(), Some(u64::MAX));
    assert_eq!(history["algorithm"], generation::ALGORITHM);
    assert_eq!(stored["song"]["samples"]["11"]["path"], "source.wav");
    for old_id in song.patterns.keys() {
        assert!(stored["song"]["patterns"][old_id.0.to_string()]
            .get("generation")
            .is_none());
    }
    let (reopened, _) = project::load(&path).unwrap();
    assert_eq!(reopened, generated);
    for event in plan.cells() {
        let cell =
            &reopened.patterns[&summary.pattern].tracks[&summary.track][&event.row][&event.column];
        assert_eq!(
            cell.delay.map(f64::to_bits),
            event.cell.delay.map(f64::to_bits)
        );
        assert_eq!(
            cell.velocity.map(f32::to_bits),
            event.cell.velocity.map(f32::to_bits)
        );
    }
    let copy = dir.path("generated-copy.json");
    project::save(&reopened, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    editor.select(summary.occurrence, summary.track).unwrap();
    editor.row = first.row;
    editor.column = first.column;
    let mut manual = editor.cell();
    manual.note = Note::Note(73);
    manual.velocity = Some(0.0);
    editor.set_cell(manual.clone()).unwrap();
    assert_eq!(editor.pattern().generation.as_ref(), Some(&provenance));
    assert_eq!(
        generation::content_matches(editor.pattern()).unwrap(),
        Some(false)
    );
    let edited = editor.song.clone();
    let edited_path = dir.path("edited.json");
    project::save(&edited, &edited_path, SaveMode::CreateNew).unwrap();
    let mut session = Session::default();
    let mut opened = Editor::default();
    session
        .request(&mut opened, Action::Open(edited_path))
        .unwrap();
    assert_eq!(opened.song, edited);
    assert!(!opened.dirty && !opened.can_undo());
    assert_eq!(
        opened.song.patterns[&summary.pattern].tracks[&summary.track][&first.row][&first.column],
        manual
    );
    assert_eq!(
        generation::content_matches(&opened.song.patterns[&summary.pattern]).unwrap(),
        Some(false)
    );
    assert_eq!(
        opened.song.patterns[&summary.pattern].generation.as_ref(),
        Some(&provenance)
    );
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
    editor.undo();
    assert_eq!(editor.song, generated);
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty);
    editor.redo();
    assert_eq!(editor.song, generated);
}
#[test]
fn malformed_generation_history_cannot_replace_dirty_song_or_existing_project_bytes() {
    use aqua_pattern_core::generation;
    let dir = Dir::new();
    let song = generation_project_song(&dir);
    let plan = generation::preview(&song, persisted_generation_config()).unwrap();
    let generated = plan.candidate().clone();
    let id = plan.summary().pattern;
    let path = dir.path("active.json");
    project::save(&generated, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut editor = Editor::default();
    let mut session = Session::default();
    session
        .request(&mut editor, Action::Open(path.clone()))
        .unwrap();
    editor.edit_mode = true;
    editor.set_timing(121.0, 4).unwrap();
    editor.set_timing(122.0, 4).unwrap();
    editor.undo();
    let current = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    for (suffix, value) in [
        ("/config/version", serde_json::json!(2)),
        ("/config/seed", serde_json::json!(-1)),
        ("/config/sequence/0/note", serde_json::json!(11)),
        ("/algorithm", serde_json::json!("arbitrary-code-generator")),
        ("/content_sha256", serde_json::json!("not-a-digest")),
        ("/raw_ticks", serde_json::json!(0)),
    ] {
        let mut data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let pointer = format!("/song/patterns/{}/generation{suffix}", id.0);
        *data.pointer_mut(&pointer).unwrap() = value;
        let invalid = dir.path("invalid.json");
        std::fs::write(&invalid, serde_json::to_vec(&data).unwrap()).unwrap();
        session.request(&mut editor, Action::Open(invalid)).unwrap();
        assert!(session.discard(&mut editor).is_err(), "{pointer}");
        assert_eq!(editor.song, current);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_undo() && editor.can_redo());
        assert_eq!(session.path, Some(path.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        session.cancel();
    }
    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown["song"]["patterns"][id.0.to_string()]["generation"]["config"]["script"] =
        serde_json::json!("not executable");
    let invalid = dir.path("unknown.json");
    std::fs::write(&invalid, serde_json::to_vec(&unknown).unwrap()).unwrap();
    assert!(project::load(&invalid).is_err());
    let mut invalid = generated;
    invalid
        .patterns
        .get_mut(&id)
        .unwrap()
        .generation
        .as_mut()
        .unwrap()
        .content_sha256 = "invalid".into();
    assert!(project::save(&invalid, &path, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(editor.song, current);
    assert_eq!(editor.revision(), revision);
}

#[test]
fn pure_project_size_preflight_matches_pretty_wrapper_escaping_and_exact_limit() {
    let dir = Dir::new();
    let mut song = example_song();
    song.title = "Quoted \"line\"\nslash\\ Unicode é 🎛".into();
    let path = dir.path("counted.json");
    let count = project::check_serialized_size(&song).unwrap();
    assert!(count > serde_json::to_vec(&song).unwrap().len());
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    assert_eq!(count, std::fs::read(&path).unwrap().len());
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(dir.path("missing.wav"));
    assert!(
        project::check_serialized_size(&song).is_ok(),
        "size preflight must not inspect assets"
    );
    assert!(project::save(&song, &dir.path("missing.json"), SaveMode::CreateNew).is_err());
    assert!(!dir.path("missing.json").exists());
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = None;
    song.title.clear();
    let overhead = project::check_serialized_size(&song).unwrap();
    let limit = 8 * 1024 * 1024;
    song.title = "x".repeat(limit - overhead);
    assert_eq!(project::check_serialized_size(&song).unwrap(), limit);
    let boundary = dir.path("boundary.json");
    project::save(&song, &boundary, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&boundary).unwrap();
    assert_eq!(bytes.len(), limit);
    assert_eq!(project::load(&boundary).unwrap().0, song);
    song.title.push('x');
    assert!(project::check_serialized_size(&song)
        .unwrap_err()
        .contains("8 MiB"));
    assert!(project::save(&song, &boundary, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(boundary).unwrap(), bytes);
    // Count escaped JSON bytes, not UTF-8 source-string length or compact Song.
    song.title = "\"".repeat(limit / 2);
    assert!(project::check_serialized_size(&song).is_err());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}
#[test]
fn generation_rejects_pretty_project_budget_overflow_before_preview_or_history_commit() {
    use aqua_pattern_core::generation::{self, GenerationPreset};
    let dir = Dir::new();
    let mut song = generation_project_song(&dir);
    song.order.clear();
    song.patterns.clear();
    song.title = "x".repeat(7_900_000);
    let path = dir.path("near-limit.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() < 8 * 1024 * 1024);
    assert!(project::check_serialized_size(&song).is_ok());
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(121.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let mut config = persisted_generation_config();
    config.bars = 16;
    config.preset = GenerationPreset::Euclidean {
        steps: 128,
        pulses: 128,
        rotation: 0,
        probability: 1.0,
    };
    let error = generation::preview(&editor.song, config).unwrap_err();
    assert!(error.contains("8 MiB"), "{error}");
    assert_eq!(editor.song, song);
    assert_eq!(editor.revision(), revision);
    assert_eq!(
        (
            editor.occurrence,
            editor.track,
            editor.column,
            editor.row,
            editor.field
        ),
        cursor
    );
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!dir.path("generated.json").exists());
}

#[test]
fn legacy_region_json_omits_sampler_options_and_reopens_byte_exactly() {
    // Frozen pre-sampler region shape, not a value produced by the new serializer.
    let legacy = "{\n  \"name\": \"Tail\",\n  \"sample\": 11,\n  \"start_frame\": 4,\n  \"end_frame\": 32,\n  \"root_note\": 60\n}";
    let region: SampleRegion = serde_json::from_str(legacy).unwrap();
    assert_eq!(region.amp_envelope, None);
    assert_eq!(region.forward_loop, None);
    assert_eq!(serde_json::to_string_pretty(&region).unwrap(), legacy);
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.regions.insert(RegionId(21), region);
    let path = dir.path("legacy-regions.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("amp_envelope") && !text.contains("forward_loop"));
    let loaded = project::load(&path).unwrap().0;
    assert_eq!(loaded, song);
    let copy = dir.path("legacy-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    let mut editor = Editor::default();
    editor.replace_document(loaded).unwrap();
    editor.edit_mode = true;
    let revision = editor.revision();
    editor
        .set_region_playback(RegionId(21), None, None)
        .unwrap();
    assert!(!editor.dirty && !editor.can_undo());
    assert_eq!(editor.revision(), revision);
}

#[test]
fn region_sampler_options_roundtrip_exactly_with_shared_users_and_unchanged_source_policy() {
    use std::cell::RefCell;
    let dir = Dir::new();
    let mut base = sourced_song(&dir);
    base.assign_region(InstrumentId(2), Some(RegionId(21)))
        .unwrap();
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let boundary = AmpEnvelope {
        attack_ms: 10_000,
        decay_ms: 10_000,
        sustain_percent: 0,
        release_ms: 500,
    };
    let short_loop = ForwardLoop {
        start_frame: 30,
        end_frame: 32,
    };
    let variants = [
        (None, None),
        (Some(AmpEnvelope::default()), None),
        (None, Some(short_loop)),
        (Some(boundary), Some(short_loop)),
        (
            Some(AmpEnvelope {
                attack_ms: 0,
                decay_ms: 0,
                sustain_percent: 100,
                release_ms: 0,
            }),
            Some(ForwardLoop {
                start_frame: 4,
                end_frame: 32,
            }),
        ),
    ];
    for (index, (envelope, forward_loop)) in variants.into_iter().enumerate() {
        let mut editor = Editor::default();
        editor.replace_document(base.clone()).unwrap();
        editor.edit_mode = true;
        editor
            .set_region_playback(RegionId(21), envelope, forward_loop)
            .unwrap();
        let song = editor.song.clone();
        assert_eq!(song.samples, base.samples);
        assert_eq!(song.instruments, base.instruments);
        assert_eq!(song.phrases, base.phrases);
        assert_eq!(song.patterns, base.patterns);
        let path = dir.path(&format!("sampler-{index}.json"));
        project::save(&song, &path, SaveMode::CreateNew).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["version"], 1);
        assert_eq!(stored["song"]["samples"]["11"]["path"], "source.wav");
        let encoded_region = &stored["song"]["regions"]["21"];
        assert_eq!(
            encoded_region.get("amp_envelope").is_some(),
            envelope.is_some()
        );
        assert_eq!(
            encoded_region.get("forward_loop").is_some(),
            forward_loop.is_some()
        );
        let authorized = RefCell::new(Vec::new());
        let loaded = project::load_with_asset_policy(&path, |p| {
            authorized.borrow_mut().push(p.to_owned());
            Ok(())
        })
        .unwrap()
        .0;
        assert_eq!(loaded, song);
        assert_eq!(*authorized.borrow(), vec![dir.path("source.wav")]);
        let copy = dir.path(&format!("sampler-{index}-copy.json"));
        project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
        assert_eq!(std::fs::read(copy).unwrap(), bytes);
        let mut reopened = Editor::default();
        let mut session = Session::default();
        session.request(&mut reopened, Action::Open(path)).unwrap();
        assert_eq!(reopened.song, song);
        assert!(!reopened.dirty && !reopened.can_undo());
        assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
    }
}

#[test]
fn malformed_sampler_storage_never_replaces_dirty_song_or_existing_project() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    song.set_region_playback(
        RegionId(21),
        Some(AmpEnvelope::default()),
        Some(ForwardLoop {
            start_frame: 8,
            end_frame: 24,
        }),
    )
    .unwrap();
    let active = dir.path("active.json");
    project::save(&song, &active, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&active).unwrap();
    let valid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.set_timing(124.0, 4).unwrap();
    editor.undo();
    let current = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let bad = dir.path("bad.json");
    let mut session = Session {
        path: Some(active.clone()),
        ..Default::default()
    };
    for (field, value) in [
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 10001, "decay_ms": 0, "sustain_percent": 100, "release_ms": 50}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 0, "decay_ms": 10001, "sustain_percent": 100, "release_ms": 50}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 0, "decay_ms": 0, "sustain_percent": 101, "release_ms": 50}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 0, "decay_ms": 0, "sustain_percent": 100, "release_ms": 501}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": -1, "decay_ms": 0, "sustain_percent": 100, "release_ms": 50}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 0.5, "decay_ms": 0, "sustain_percent": 100, "release_ms": 50}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 0, "decay_ms": 0, "sustain_percent": 100}),
        ),
        (
            "amp_envelope",
            serde_json::json!({"attack_ms": 0, "decay_ms": 0, "sustain_percent": 100, "release_ms": 50, "curve": "arbitrary"}),
        ),
        (
            "forward_loop",
            serde_json::json!({"start_frame": 3, "end_frame": 24}),
        ),
        (
            "forward_loop",
            serde_json::json!({"start_frame": 8, "end_frame": 33}),
        ),
        (
            "forward_loop",
            serde_json::json!({"start_frame": 8, "end_frame": 9}),
        ),
        (
            "forward_loop",
            serde_json::json!({"start_frame": 24, "end_frame": 8}),
        ),
        (
            "forward_loop",
            serde_json::json!({"start_frame": 8, "end_frame": 24, "mode": "ping_pong"}),
        ),
    ] {
        let mut malformed = valid.clone();
        malformed["song"]["regions"]["21"][field] = value;
        std::fs::write(&bad, serde_json::to_vec_pretty(&malformed).unwrap()).unwrap();
        assert!(project::load(&bad).is_err(), "{malformed}");
        session
            .request(&mut editor, Action::Open(bad.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err());
        assert!(session.pending.is_some());
        session.cancel();
        assert_eq!(session.path, Some(active.clone()));
        assert_eq!(editor.song, current);
        assert_eq!(editor.revision(), revision);
        assert!(editor.dirty && editor.can_undo() && editor.can_redo());
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert_eq!(std::fs::read(&active).unwrap(), bytes);
    }
    let mut invalid = song;
    invalid.regions.get_mut(&RegionId(21)).unwrap().end_frame = 16;
    assert!(project::save(&invalid, &active, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(&active).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        3,
        "no failed-save temporary output"
    );
    // Playback metadata adds no bypass around the existing pre-read asset policy.
    std::fs::remove_file(dir.path("source.wav")).unwrap();
    let error = project::load_with_asset_policy(&active, |_| Err("outside permitted roots".into()))
        .unwrap_err();
    assert!(
        error.contains("denied") && error.contains("outside permitted roots"),
        "{error}"
    );
    assert_eq!(editor.song, current);
}

fn persisted_velocity_zones() -> Vec<VelocityZone> {
    vec![
        VelocityZone {
            min_velocity: 0,
            max_velocity: 63,
            alternatives: vec![
                PadLayer {
                    id: PadLayerId(41),
                    region: RegionId(21),
                },
                PadLayer {
                    id: PadLayerId(17),
                    region: RegionId(22),
                },
            ],
        },
        VelocityZone {
            min_velocity: 64,
            max_velocity: 127,
            alternatives: vec![PadLayer {
                id: PadLayerId(99),
                region: RegionId(22),
            }],
        },
    ]
}
fn zoned_kit_song(dir: &Dir) -> Song {
    let mut song = kit_song(dir);
    song.register_sample(SampleId(12), song.samples[&SampleId(11)].clone())
        .unwrap();
    let mut alternate = song.regions[&RegionId(21)].clone();
    alternate.name = "Sampler alternate".into();
    alternate.sample = SampleId(12);
    alternate.amp_envelope = Some(AmpEnvelope {
        attack_ms: 1,
        decay_ms: 2,
        sustain_percent: 63,
        release_ms: 75,
    });
    alternate.forward_loop = Some(ForwardLoop {
        start_frame: 8,
        end_frame: 24,
    });
    song.upsert_region(RegionId(22), alternate).unwrap();
    song.set_pad_sources(DrumKitId(41), PadId(501), None, persisted_velocity_zones())
        .unwrap();
    song
}

#[test]
fn legacy_single_region_pad_json_remains_exact_without_velocity_zones() {
    let legacy = r#"{"slot":0,"name":"Kick","note":36,"region":21,"root_note":60,"gain":0.75,"pan":-0.25,"mode":"OneShot","new_note":"Overlap","choke_group":null}"#;
    let pad: DrumPad = serde_json::from_str(legacy).unwrap();
    assert!(pad.velocity_zones.is_empty());
    assert!(pad.has_source());
    assert_eq!(serde_json::to_string(&pad).unwrap(), legacy);
    let dir = Dir::new();
    let mut song = kit_song(&dir);
    song.upsert_drum_pad(DrumKitId(41), PadId(501), pad)
        .unwrap();
    let path = dir.path("legacy-kit.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!std::str::from_utf8(&bytes)
        .unwrap()
        .contains("velocity_zones"));
    let restored = project::load(&path).unwrap().0;
    assert_eq!(restored, song);
    let copy = dir.path("legacy-kit-copy.json");
    project::save(&restored, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
}

#[test]
fn zoned_sources_stable_alternative_order_and_sampler_options_roundtrip_without_runtime_counters() {
    use std::cell::RefCell;
    let dir = Dir::new();
    let song = zoned_kit_song(&dir);
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let path = dir.path("zoned-kit.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["version"], 1);
    assert_eq!(
        stored["song"]["drum_kits"]["41"]["pads"]["501"]["region"],
        serde_json::Value::Null
    );
    assert_eq!(
        stored["song"]["drum_kits"]["41"]["pads"]["501"]["velocity_zones"][0]["alternatives"][0]
            ["id"],
        41
    );
    assert_eq!(
        stored["song"]["drum_kits"]["41"]["pads"]["501"]["velocity_zones"][0]["alternatives"][1]
            ["id"],
        17,
        "alternative list order is meaningful; do not sort stable IDs"
    );
    assert!(stored["song"]["drum_kits"]["41"]["pads"]["999"]
        .get("velocity_zones")
        .is_none());
    assert_eq!(stored["song"]["samples"]["12"]["path"], "source.wav");
    let authorized = RefCell::new(Vec::new());
    let loaded = project::load_with_asset_policy(&path, |p| {
        authorized.borrow_mut().push(p.to_owned());
        Ok(())
    })
    .unwrap()
    .0;
    assert_eq!(loaded, song);
    assert_eq!(
        *authorized.borrow(),
        vec![dir.path("source.wav"), dir.path("source.wav")]
    );
    let copy = dir.path("zoned-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    let mut editor = Editor::default();
    let mut session = Session::default();
    session.request(&mut editor, Action::Open(path)).unwrap();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty && !editor.can_undo());
    editor.edit_mode = true;
    let mut edited_pad = editor.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)].clone();
    edited_pad.name = "Metadata-only edit".into();
    edited_pad.gain = 0.5;
    editor
        .upsert_drum_pad(DrumKitId(41), PadId(501), edited_pad)
        .unwrap();
    assert_eq!(
        editor.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)].velocity_zones,
        persisted_velocity_zones()
    );
    editor.undo();
    assert_eq!(editor.song, song);
    editor
        .set_pad_sources(DrumKitId(41), PadId(501), Some(RegionId(21)), Vec::new())
        .unwrap();
    let single = dir.path("explicit-single.json");
    project::save(&editor.song, &single, SaveMode::CreateNew).unwrap();
    assert!(!std::fs::read_to_string(single)
        .unwrap()
        .contains("velocity_zones"));
    editor.undo();
    assert_eq!(editor.song, song);
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
}

#[test]
fn malformed_zone_projects_and_alternative_deletion_preserve_active_history_and_saved_bytes() {
    let dir = Dir::new();
    let song = zoned_kit_song(&dir);
    let active = dir.path("active-zones.json");
    project::save(&song, &active, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&active).unwrap();
    let valid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.set_timing(124.0, 4).unwrap();
    editor.undo();
    let current = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let mut session = Session {
        path: Some(active.clone()),
        ..Default::default()
    };
    let bad = dir.path("bad-zones.json");
    for (pointer, value) in [
        ("/song/drum_kits/41/pads/501/region", serde_json::json!(21)),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/min_velocity",
            serde_json::json!(1),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/max_velocity",
            serde_json::json!(64),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/1/max_velocity",
            serde_json::json!(126),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/min_velocity",
            serde_json::json!(-1),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/min_velocity",
            serde_json::json!(0.5),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/alternatives",
            serde_json::json!([]),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/alternatives/1/id",
            serde_json::json!(0),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/0/alternatives/1/id",
            serde_json::json!(41),
        ),
        (
            "/song/drum_kits/41/pads/501/velocity_zones/1/alternatives/0/region",
            serde_json::json!(999),
        ),
    ] {
        let mut data = valid.clone();
        *data.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&bad, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(project::load(&bad).is_err(), "{pointer}");
        session
            .request(&mut editor, Action::Open(bad.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err());
        session.cancel();
        assert_eq!(editor.song, current);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_undo() && editor.can_redo());
        assert_eq!(session.path, Some(active.clone()));
    }
    for extra in ["round_robin_counter", "random_seed"] {
        let mut data = valid.clone();
        data["song"]["drum_kits"]["41"]["pads"]["501"]["velocity_zones"][0][extra] =
            serde_json::json!(1);
        std::fs::write(&bad, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(
            project::load(&bad).is_err(),
            "runtime counter/unsupported random state must not become stored schema"
        );
    }
    let mut dangling = song;
    dangling.regions.remove(&RegionId(22));
    assert!(project::save(
        &dangling,
        &active,
        SaveMode::ReplaceConfirmed(bytes.clone())
    )
    .is_err());
    assert_eq!(std::fs::read(&active).unwrap(), bytes);
    assert_eq!(editor.song, current);
    assert_eq!(editor.revision(), revision);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

fn persisted_groove_capture() -> aqua_pattern_core::groove::CaptureConfig {
    aqua_pattern_core::groove::CaptureConfig {
        version: 1,
        name: "Frozen manual timing".into(),
        marker_set: MarkerSetId(31),
        start_marker: 0,
        end_marker: 4,
        include_start_anchor: true,
        division: aqua_pattern_core::break_compiler::Subdivision::Sixteenth,
        cycle_steps: 4,
    }
}
fn persisted_groove_apply() -> aqua_pattern_core::groove::ApplyConfig {
    aqua_pattern_core::groove::ApplyConfig {
        groove: GrooveId(9),
        strength_percent: 100,
        scope: aqua_pattern_core::transforms::TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 4,
            columns: vec![0],
        },
    }
}
fn groove_project_song(dir: &Dir) -> Song {
    let source = dir.path("groove-source.wav");
    let mut writer = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for frame in 0..8_000i16 {
        writer.write_sample((frame % 17 - 8) * 100).unwrap();
    }
    writer.finalize().unwrap();
    let mut song = example_song();
    song.bpm = 120.0;
    song.lines_per_beat = 4;
    for pattern in song.patterns.values_mut() {
        pattern.tracks.clear();
    }
    song.patterns.get_mut(&PatternId(1)).unwrap().rows = 8;
    song.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: Default::default(),
        section: None,
    }];
    for row in 0..4 {
        song.patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .insert(
                row,
                [(
                    0,
                    Cell {
                        note: Note::Note(60 + row as u8),
                        instrument: Some(InstrumentId(1)),
                        velocity: if row == 0 { Some(0.0) } else { None },
                        pan: Some(0.0),
                        delay: if row == 0 { Some(0.0) } else { None },
                        ..Default::default()
                    },
                )]
                .into(),
            );
    }
    song.register_sample(
        SampleId(11),
        aqua_pattern_core::sample_asset::import(&source)
            .unwrap()
            .asset,
    )
    .unwrap();
    song.upsert_marker_set(
        MarkerSetId(31),
        MarkerSet {
            name: "Accepted boundaries, not detector output".into(),
            sample: SampleId(11),
            source_bpm: Some(120.0),
            markers: vec![0, 1_100, 2_000, 3_100, 4_000, 8_000],
            locked_frames: Vec::new(),
        },
    )
    .unwrap();
    song
}

#[test]
fn legacy_project_with_no_grooves_retains_exact_save_bytes_and_optional_cells() {
    let dir = Dir::new();
    let song = groove_project_song(&dir);
    assert!(song.grooves.is_empty());
    let path = dir.path("legacy-no-groove.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(stored["song"].get("grooves").is_none());
    let loaded = project::load(&path).unwrap().0;
    assert_eq!(loaded, song);
    let copy = dir.path("legacy-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    assert_eq!(
        loaded.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0].delay,
        Some(0.0)
    );
    assert_eq!(
        loaded.patterns[&PatternId(1)].tracks[&TrackId(1)][&1][&0].delay,
        None
    );
}

#[test]
fn groove_capture_apply_and_historical_provenance_survive_reopen_without_live_asset_dependencies() {
    use aqua_pattern_core::groove;
    let dir = Dir::new();
    let base = groove_project_song(&dir);
    let source_path = dir.path("groove-source.wav");
    let source_bytes = std::fs::read(&source_path).unwrap();
    let capture = groove::capture_preview(&base, GrooveId(9), persisted_groove_capture()).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    editor
        .commit_groove_capture_preview(editor.revision(), &capture)
        .unwrap();
    let captured = editor.song.clone();
    let plan = groove::preview(&captured, persisted_groove_apply()).unwrap();
    editor
        .commit_groove_preview(editor.revision(), &plan)
        .unwrap();
    let applied = editor.song.clone();
    let path = dir.path("groove.json");
    project::save(&applied, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["version"], 1);
    assert_eq!(stored["song"]["samples"]["11"]["path"], "groove-source.wav");
    assert_eq!(
        stored["song"]["grooves"]["9"]["anchors"][1]["source_frame_delta"],
        1100
    );
    let loaded = project::load(&path).unwrap().0;
    assert_eq!(loaded, applied);
    for row in [1, 3] {
        assert_eq!(
            loaded.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0]
                .delay
                .unwrap()
                .to_bits(),
            applied.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0]
                .delay
                .unwrap()
                .to_bits()
        );
    }
    let copy = dir.path("groove-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    editor.undo();
    assert_eq!(editor.song, captured);
    editor.undo();
    assert_eq!(editor.song, base);
    editor.redo();
    editor.redo();
    assert_eq!(editor.song, applied);
    let mut reopened = Editor::default();
    let mut session = Session::default();
    session.request(&mut reopened, Action::Open(path)).unwrap();
    assert_eq!(reopened.song, applied);
    assert!(!reopened.dirty && !reopened.can_undo());
    // Historical references do not pin a source or marker set after capture.
    let mut independent = captured.clone();
    let mut changed_markers = independent.marker_sets[&MarkerSetId(31)].clone();
    changed_markers.markers[1] = 1_200;
    changed_markers.source_bpm = Some(100.0);
    independent
        .upsert_marker_set(MarkerSetId(31), changed_markers)
        .unwrap();
    assert_eq!(independent.grooves, captured.grooves);
    independent.remove_marker_set(MarkerSetId(31)).unwrap();
    independent.remove_sample(SampleId(11)).unwrap();
    independent.validate().unwrap();
    let independent_plan = groove::preview(&independent, persisted_groove_apply()).unwrap();
    assert_eq!(independent_plan.candidate().patterns, applied.patterns);
    assert_eq!(independent_plan.candidate().grooves, captured.grooves);
    assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    std::fs::remove_file(&source_path).unwrap();
    let without_source = dir.path("historical-only.json");
    project::save(
        independent_plan.candidate(),
        &without_source,
        SaveMode::CreateNew,
    )
    .unwrap();
    let historical = project::load_with_asset_policy(&without_source, |_| {
        panic!("historical groove provenance must not perform asset IO")
    })
    .unwrap()
    .0;
    assert_eq!(&historical, independent_plan.candidate());
    reopened.replace_document(historical.clone()).unwrap();
    reopened.edit_mode = true;
    reopened.remove_groove(GrooveId(9)).unwrap();
    assert_eq!(reopened.song.patterns, historical.patterns);
    reopened.undo();
    assert_eq!(reopened.song, historical);
}

#[test]
fn malformed_groove_projects_never_replace_dirty_history_or_existing_save() {
    use aqua_pattern_core::groove;
    let dir = Dir::new();
    let base = groove_project_song(&dir);
    let capture = groove::capture_preview(&base, GrooveId(9), persisted_groove_capture()).unwrap();
    let song = capture.candidate().clone();
    let active = dir.path("active-groove.json");
    project::save(&song, &active, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&active).unwrap();
    let valid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.set_timing(124.0, 4).unwrap();
    editor.undo();
    let current = editor.song.clone();
    let revision = editor.revision();
    let mut session = Session {
        path: Some(active.clone()),
        ..Default::default()
    };
    let bad = dir.path("bad-groove.json");
    for (pointer, value) in [
        ("/song/grooves/9/version", serde_json::json!(2)),
        ("/song/grooves/9/cycle_steps", serde_json::json!(0)),
        ("/song/grooves/9/source_bpm", serde_json::json!(0)),
        ("/song/grooves/9/source_rate", serde_json::json!(0)),
        ("/song/grooves/9/anchors/1/step", serde_json::json!(0)),
        (
            "/song/grooves/9/anchors/1/source_frame_delta",
            serde_json::json!(4000),
        ),
        (
            "/song/grooves/9/provenance/source_sha256",
            serde_json::json!("not-a-hash"),
        ),
        (
            "/song/grooves/9/provenance/marker_set_sha256",
            serde_json::json!("A".repeat(64)),
        ),
        (
            "/song/grooves/9/provenance/window_end_frame",
            serde_json::json!(8001),
        ),
        (
            "/song/grooves/9/provenance/include_start_anchor",
            serde_json::json!(false),
        ),
        ("/song/grooves/9/provenance/sample", serde_json::json!(0)),
    ] {
        let mut malformed = valid.clone();
        *malformed.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&bad, serde_json::to_vec(&malformed).unwrap()).unwrap();
        assert!(project::load(&bad).is_err(), "{pointer}");
        session
            .request(&mut editor, Action::Open(bad.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err());
        session.cancel();
        assert_eq!(editor.song, current);
        assert_eq!(editor.revision(), revision);
        assert!(editor.dirty && editor.can_undo() && editor.can_redo());
        assert_eq!(session.path, Some(active.clone()));
        assert_eq!(std::fs::read(&active).unwrap(), bytes);
    }
    let mut unknown = valid;
    unknown["song"]["grooves"]["9"]["regenerate_on_open"] = serde_json::json!(true);
    std::fs::write(&bad, serde_json::to_vec(&unknown).unwrap()).unwrap();
    assert!(project::load(&bad).is_err());
    let mut invalid = song;
    invalid
        .grooves
        .get_mut(&GrooveId(9))
        .unwrap()
        .provenance
        .source_frames = 0;
    assert!(project::save(&invalid, &active, SaveMode::ReplaceConfirmed(bytes.clone())).is_err());
    assert_eq!(std::fs::read(&active).unwrap(), bytes);
    assert_eq!(editor.song, current);
    assert_eq!(editor.revision(), revision);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

#[test]
fn groove_capture_rejects_unsaveable_pretty_wrapper_before_exposing_a_commit_plan() {
    use aqua_pattern_core::groove;
    let dir = Dir::new();
    let mut song = groove_project_song(&dir);
    song.title.clear();
    let overhead = project::check_serialized_size(&song).unwrap();
    song.title = "x".repeat(8 * 1024 * 1024 - overhead - 16);
    let path = dir.path("near-limit-groove.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let saved = std::fs::read(&path).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(121.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    let error =
        groove::capture_preview(&editor.song, GrooveId(9), persisted_groove_capture()).unwrap_err();
    assert!(error.contains("8 MiB"), "{error}");
    assert_eq!(editor.song, song);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    assert_eq!(std::fs::read(path).unwrap(), saved);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

fn persisted_melodic_map() -> MultisampleInstrument {
    MultisampleInstrument {
        name: "Shared melodic map".into(),
        zones: [
            (
                KeyZoneId(41),
                KeyZone {
                    name: "Soft native root".into(),
                    note_min: 0,
                    note_max: 127,
                    velocity_min: 0,
                    velocity_max: 63,
                    region: RegionId(21),
                    root_note: 48,
                },
            ),
            (
                KeyZoneId(17),
                KeyZone {
                    name: "Hard native root".into(),
                    note_min: 0,
                    note_max: 127,
                    velocity_min: 64,
                    velocity_max: 127,
                    region: RegionId(22),
                    root_note: 72,
                },
            ),
        ]
        .into(),
    }
}
fn melodic_project_song(dir: &Dir) -> Song {
    let mut song = sourced_song(dir);
    let mut region = song.regions[&RegionId(21)].clone();
    region.name = "Other source range".into();
    region.start_frame = 8;
    region.root_note = 61;
    region.amp_envelope = Some(AmpEnvelope {
        attack_ms: 2,
        decay_ms: 3,
        sustain_percent: 50,
        release_ms: 17,
    });
    region.forward_loop = Some(ForwardLoop {
        start_frame: 8,
        end_frame: 20,
    });
    song.upsert_region(RegionId(22), region).unwrap();
    song.upsert_multisample(MultisampleId(51), persisted_melodic_map())
        .unwrap();
    song.assign_multisample(InstrumentId(1), Some(MultisampleId(51)))
        .unwrap();
    song.instruments.insert(
        InstrumentId(99),
        Instrument {
            name: "Second shared-map user".into(),
            kind: InstrumentKind::Multisample {
                map: MultisampleId(51),
            },
            sample: None,
            region: None,
        },
    );
    song.upsert_multisample(
        MultisampleId(52),
        MultisampleInstrument {
            name: "Explicit empty draft map".into(),
            zones: Default::default(),
        },
    )
    .unwrap();
    song.validate().unwrap();
    song
}

#[test]
fn legacy_project_bytes_remain_exact_when_multisample_maps_are_absent() {
    let dir = Dir::new();
    let song = sourced_song(&dir);
    let path = dir.path("legacy-no-keyzones.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["song"].get("multisamples").is_none());
    for instrument in json["song"]["instruments"].as_object().unwrap().values() {
        assert!(
            instrument.get("kind").is_none(),
            "legacy Pitched discriminant stays omitted"
        );
    }
    let loaded = project::load(&path).unwrap().0;
    assert_eq!(loaded, song);
    assert!(loaded.multisamples.is_empty());
    let copy = dir.path("legacy-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    let mut explicit_empty = json;
    explicit_empty["song"]["multisamples"] = serde_json::json!({});
    let decoded: Song = serde_json::from_value(explicit_empty["song"].clone()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.multisamples, song.multisamples);
}

#[test]
fn melodic_keyzone_ids_roots_shared_maps_and_empty_coverage_roundtrip_without_new_asset_io() {
    use std::cell::RefCell;
    let dir = Dir::new();
    let song = melodic_project_song(&dir);
    let source_bytes = std::fs::read(dir.path("source.wav")).unwrap();
    let path = dir.path("keyzones.json");
    project::save(&song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["song"]["schema"], 1);
    assert_eq!(json["song"]["samples"]["11"]["path"], "source.wav");
    assert_eq!(
        json["song"]["instruments"]["1"]["kind"],
        serde_json::json!({"type":"Multisample","data":{"map":51}})
    );
    assert_eq!(
        json["song"]["multisamples"]["51"]["zones"]["41"]["root_note"],
        48
    );
    assert_eq!(
        json["song"]["multisamples"]["51"]["zones"]["17"]["root_note"],
        72
    );
    assert_eq!(
        json["song"]["multisamples"]["52"]["zones"],
        serde_json::json!({})
    );
    let policy_paths = RefCell::new(Vec::new());
    let loaded = project::load_with_asset_policy(&path, |p| {
        policy_paths.borrow_mut().push(p.to_owned());
        Ok(())
    })
    .unwrap()
    .0;
    assert_eq!(loaded, song);
    assert_eq!(
        *policy_paths.borrow(),
        vec![dir.path("source.wav")],
        "map zones reuse existing registered asset authority, not new IO paths"
    );
    let denied =
        project::load_with_asset_policy(&path, |_| Err("keyzone source policy denied".into()))
            .unwrap_err();
    assert!(denied.contains("keyzone source policy denied"));
    let copy = dir.path("keyzones-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    let mut editor = Editor::default();
    let mut session = Session::default();
    session
        .request(&mut editor, Action::Open(path.clone()))
        .unwrap();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty && !editor.can_undo());
    editor.edit_mode = true;
    let mut zones = editor.song.multisamples[&MultisampleId(51)].zones.clone();
    zones.get_mut(&KeyZoneId(41)).unwrap().velocity_max = 79;
    zones.get_mut(&KeyZoneId(17)).unwrap().velocity_min = 80;
    editor.set_keyzones(MultisampleId(51), zones).unwrap();
    let changed = editor.song.clone();
    assert_eq!(changed.patterns, song.patterns);
    assert_eq!(changed.phrases, song.phrases);
    assert_eq!(changed.samples, song.samples);
    assert_eq!(changed.regions, song.regions);
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty && editor.can_redo());
    editor.redo();
    assert_eq!(editor.song, changed);
    let newer = dir.path("changed-keyzones.json");
    project::save(&changed, &newer, SaveMode::CreateNew).unwrap();
    assert_eq!(project::load(&newer).unwrap().0, changed);
    // Explicit source-kind switching uses old APIs without rewriting map content.
    editor
        .assign_sample(InstrumentId(1), Some(dir.path("source.wav")))
        .unwrap();
    assert_eq!(
        editor.song.instruments[&InstrumentId(1)].kind,
        InstrumentKind::Pitched
    );
    assert_eq!(editor.song.multisamples, changed.multisamples);
    editor.undo();
    assert_eq!(editor.song, changed);
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), source_bytes);
}

#[test]
fn malformed_keyzone_projects_and_unused_map_references_preserve_dirty_history_and_disk() {
    let dir = Dir::new();
    let song = melodic_project_song(&dir);
    let active = dir.path("active-keyzones.json");
    project::save(&song, &active, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&active).unwrap();
    let valid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.row = 8;
    editor.field = Field::Pan;
    editor.set_timing(123.0, 4).unwrap();
    editor.set_timing(124.0, 4).unwrap();
    editor.undo();
    let current = editor.song.clone();
    let revision = editor.revision();
    let mut session = Session {
        path: Some(active.clone()),
        ..Default::default()
    };
    let bad = dir.path("bad-keyzones.json");
    for (pointer, value) in [
        ("/song/multisamples/51/name", serde_json::json!(" ")),
        (
            "/song/multisamples/51/zones/41/name",
            serde_json::json!("é".repeat(65)),
        ),
        (
            "/song/multisamples/51/zones/41/note_max",
            serde_json::json!(128),
        ),
        (
            "/song/multisamples/51/zones/41/note_min",
            serde_json::json!(-1),
        ),
        (
            "/song/multisamples/51/zones/41/velocity_min",
            serde_json::json!(0.5),
        ),
        (
            "/song/multisamples/51/zones/41/velocity_max",
            serde_json::json!(64),
        ),
        (
            "/song/multisamples/51/zones/17/velocity_max",
            serde_json::json!(63),
        ),
        (
            "/song/multisamples/51/zones/41/root_note",
            serde_json::json!(128),
        ),
        (
            "/song/multisamples/51/zones/17/region",
            serde_json::json!(999),
        ),
        ("/song/instruments/1/kind/data/map", serde_json::json!(999)),
        ("/song/instruments/1/region", serde_json::json!(21)),
        (
            "/song/instruments/1/sample",
            serde_json::json!("source.wav"),
        ),
    ] {
        let mut data = valid.clone();
        if pointer == "/song/instruments/1/region" {
            data["song"]["instruments"]["1"]["region"] = value;
        } else {
            *data.pointer_mut(pointer).unwrap() = value;
        }
        std::fs::write(&bad, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(project::load(&bad).is_err(), "{pointer}");
        session
            .request(&mut editor, Action::Open(bad.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err());
        session.cancel();
        assert_eq!(editor.song, current);
        assert_eq!(editor.revision(), revision);
        assert_eq!((editor.row, editor.field), (8, Field::Pan));
        assert!(editor.dirty && editor.can_undo() && editor.can_redo());
        assert_eq!(session.path, Some(active.clone()));
    }
    for extra in ["crossfade", "round_robin_counter", "stack"] {
        let mut data = valid.clone();
        data["song"]["multisamples"]["51"]["zones"]["41"][extra] = serde_json::json!(true);
        std::fs::write(&bad, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(project::load(&bad).is_err());
    }
    let mut zero_id = valid.clone();
    let zone = zero_id["song"]["multisamples"]["51"]["zones"]
        .as_object_mut()
        .unwrap()
        .remove("41")
        .unwrap();
    zero_id["song"]["multisamples"]["51"]["zones"]["0"] = zone;
    std::fs::write(&bad, serde_json::to_vec(&zero_id).unwrap()).unwrap();
    assert!(project::load(&bad).is_err());
    let mut dangling = song;
    dangling.instruments.get_mut(&InstrumentId(1)).unwrap().kind = InstrumentKind::Pitched;
    dangling
        .instruments
        .get_mut(&InstrumentId(99))
        .unwrap()
        .kind = InstrumentKind::Pitched;
    dangling.regions.remove(&RegionId(22));
    assert!(
        project::save(
            &dangling,
            &active,
            SaveMode::ReplaceConfirmed(bytes.clone())
        )
        .is_err(),
        "an unused map's zone still protects its region"
    );
    assert_eq!(std::fs::read(active).unwrap(), bytes);
    assert_eq!(editor.song, current);
    assert_eq!(editor.revision(), revision);
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        3,
        "failed save leaves no temporary project"
    );
}

fn resample_project_song(dir: &Dir) -> Song {
    let mut song = sourced_song(dir);
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 4;
    pattern.tracks.clear();
    pattern.tracks.insert(
        TrackId(1),
        [
            (
                0,
                [(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        velocity: Some(0.25),
                        pan: Some(0.0),
                        delay: Some(0.0),
                        ..Default::default()
                    },
                )]
                .into(),
            ),
            (
                2,
                [(
                    0,
                    Cell {
                        note: Note::Off,
                        ..Default::default()
                    },
                )]
                .into(),
            ),
        ]
        .into(),
    );
    song.validate().unwrap();
    song
}
fn resample_config(dir: &Dir, name: &str) -> aqua_pattern_core::resample::ResampleConfig {
    aqua_pattern_core::resample::ResampleConfig {
        version: 1,
        occurrence: 0,
        name: "Printed occurrence".into(),
        root_note: 60,
        tail: aqua_pattern_core::resample::TailPolicy::ReleaseTail,
        output: dir.path(name),
    }
}

#[cfg(unix)]
#[test]
fn published_resample_registers_exactly_three_fresh_objects_as_one_protected_undo_and_roundtrips() {
    use aqua_pattern_core::resample::{self, Access};
    use std::cell::RefCell;
    let dir = Dir::new();
    let base = resample_project_song(&dir);
    let old_audio = std::fs::read(dir.path("source.wav")).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.row = 3;
    editor.field = Field::Delay;
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let revision = editor.revision();
    let config = resample_config(&dir, "printed.wav");
    let plan = resample::prepare(&base, config.clone()).unwrap();
    assert_eq!(plan.base(), &base);
    assert!(
        !config.output.exists(),
        "pure preparation cannot publish or reserve a file"
    );
    assert_eq!(
        (
            plan.summary().sample,
            plan.summary().region,
            plan.summary().instrument
        ),
        (SampleId(12), RegionId(22), InstrumentId(5))
    );
    let result = resample::render(&plan, |_| Ok(()), || false).unwrap();
    let artifact = std::fs::read(&config.output).unwrap();
    assert_eq!(
        editor.song, base,
        "render in Read mode does not mutate document/history"
    );
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && !editor.can_undo());
    assert!(editor.commit_resample_preview(revision, &result).is_err());
    assert_eq!(std::fs::read(&config.output).unwrap(), artifact);
    let authorized = RefCell::new(Vec::new());
    resample::verify_artifact(&result, |access| {
        match access {
            Access::PublishOutput(path) => {
                authorized.borrow_mut().push(("publish", path.to_owned()))
            }
            Access::ReadSource(path) => authorized.borrow_mut().push(("read", path.to_owned())),
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        *authorized.borrow(),
        vec![
            ("publish", config.output.clone()),
            ("read", dir.path("source.wav")),
            ("read", config.output.clone())
        ]
    );
    editor.edit_mode = true;
    editor.commit_resample_preview(revision, &result).unwrap();
    let registered = editor.song.clone();
    assert_eq!(&registered, result.candidate());
    assert_eq!(registered.samples[&SampleId(12)], *result.asset());
    let region = &registered.regions[&RegionId(22)];
    assert_eq!(
        (
            region.sample,
            region.start_frame,
            region.end_frame,
            region.root_note
        ),
        (SampleId(12), 0, result.asset().frames, 60)
    );
    assert_eq!((region.amp_envelope, region.forward_loop), (None, None));
    assert_eq!(
        registered.instruments[&InstrumentId(5)].kind,
        InstrumentKind::Pitched
    );
    assert_eq!(
        registered.instruments[&InstrumentId(5)].region,
        Some(RegionId(22))
    );
    assert!(registered.instruments[&InstrumentId(5)].sample.is_none());
    let mut stripped = registered.clone();
    stripped.samples.remove(&SampleId(12));
    stripped.regions.remove(&RegionId(22));
    stripped.instruments.remove(&InstrumentId(5));
    assert_eq!(
        stripped, base,
        "no order, cells, sources, tempo or existing mapping may change"
    );
    assert_eq!(
        (
            editor.occurrence,
            editor.track,
            editor.column,
            editor.row,
            editor.field
        ),
        cursor
    );
    assert_eq!(editor.revision(), revision + 1);
    assert!(editor.dirty && editor.can_undo());
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    assert_eq!(
        std::fs::read(&config.output).unwrap(),
        artifact,
        "Undo removes references only"
    );
    editor.redo();
    assert_eq!(editor.song, registered);
    let path = dir.path("resampled-project.json");
    project::save(&registered, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["song"]["samples"]["12"]["path"], "printed.wav");
    for transient in ["resample", "resample_receipt", "resample_preview"] {
        assert!(json["song"].get(transient).is_none());
    }
    let paths = RefCell::new(Vec::new());
    let loaded = project::load_with_asset_policy(&path, |p| {
        paths.borrow_mut().push(p.to_owned());
        Ok(())
    })
    .unwrap()
    .0;
    assert_eq!(loaded, registered);
    assert_eq!(
        *paths.borrow(),
        vec![dir.path("source.wav"), config.output.clone()]
    );
    let copy = dir.path("resampled-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    let mut restored = Editor::default();
    let mut session = Session::default();
    session.request(&mut restored, Action::Open(path)).unwrap();
    assert_eq!(restored.song, registered);
    assert!(!restored.dirty && !restored.can_undo());
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), old_audio);
    assert_eq!(std::fs::read(config.output).unwrap(), artifact);
}

#[cfg(unix)]
#[test]
fn stale_resample_registration_aba_replacement_and_discard_never_remove_published_audio() {
    use aqua_pattern_core::resample;
    let dir = Dir::new();
    let base = resample_project_song(&dir);
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    editor.row = 3;
    editor.field = Field::Pan;
    let revision = editor.revision();
    let config = resample_config(&dir, "stale-print.wav");
    let plan = resample::prepare(&base, config.clone()).unwrap();
    let result = resample::render(&plan, |_| Ok(()), || false).unwrap();
    let artifact = std::fs::read(&config.output).unwrap();
    editor.set_timing(123.0, 4).unwrap();
    let changed = editor.song.clone();
    let changed_revision = editor.revision();
    assert!(editor.commit_resample_preview(revision, &result).is_err());
    assert!(
        editor
            .commit_resample_preview(changed_revision, &result)
            .is_err(),
        "opaque full-base guard also rejects current revision with changed content"
    );
    assert_eq!(editor.song, changed);
    assert_eq!(editor.revision(), changed_revision);
    editor.undo();
    assert_eq!(editor.song, base);
    let aba_revision = editor.revision();
    assert!(editor.commit_resample_preview(revision, &result).is_err());
    assert_eq!(editor.revision(), aba_revision);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    assert_eq!((editor.row, editor.field), (3, Field::Pan));
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    assert!(
        editor.commit_resample_preview(revision, &result).is_err(),
        "same-content Open invalidates the captured generation"
    );
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && !editor.can_undo());
    drop(result); // Closing/discarding the review is not ownership of the external artifact.
    assert_eq!(std::fs::read(config.output).unwrap(), artifact);
}

#[cfg(unix)]
#[test]
fn resample_reauthorization_hash_failure_and_redo_restore_refs_only_without_clobbering_saved_project(
) {
    use aqua_pattern_core::resample;
    let dir = Dir::new();
    let base = resample_project_song(&dir);
    let active = dir.path("active-before-print.json");
    project::save(&base, &active, SaveMode::CreateNew).unwrap();
    let saved = std::fs::read(&active).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    let config = resample_config(&dir, "verify-print.wav");
    let result = resample::render(
        &resample::prepare(&base, config.clone()).unwrap(),
        |_| Ok(()),
        || false,
    )
    .unwrap();
    let bytes = std::fs::read(&config.output).unwrap();
    assert!(
        resample::verify_artifact(&result, |_| Err("output no longer authorized".into()))
            .unwrap_err()
            .contains("no longer authorized")
    );
    assert_eq!(editor.song, base);
    assert_eq!(editor.revision(), revision);
    assert!(editor.can_redo() && !editor.dirty);
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&config.output, &corrupt).unwrap();
    assert!(resample::verify_artifact(&result, |_| Ok(())).is_err());
    // The caller withholds registration after failed off-thread verification.
    assert_eq!(editor.song, base);
    assert_eq!(editor.revision(), revision);
    assert!(editor.can_redo() && !editor.dirty);
    std::fs::write(&config.output, &bytes).unwrap();
    resample::verify_artifact(&result, |_| Ok(())).unwrap();
    editor.commit_resample_preview(revision, &result).unwrap();
    let registered = editor.song.clone();
    editor.undo();
    assert_eq!(editor.song, base);
    std::fs::remove_file(&config.output).unwrap(); // External deletion, never Editor cleanup.
    editor.redo();
    assert_eq!(
        editor.song, registered,
        "Redo restores references only; it does not rerender or perform IO"
    );
    assert!(!config.output.exists());
    assert!(project::save(
        &editor.song,
        &active,
        SaveMode::ReplaceConfirmed(saved.clone())
    )
    .is_err());
    assert_eq!(std::fs::read(&active).unwrap(), saved);
    assert_eq!(editor.song, registered);
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && editor.can_redo());
}

#[cfg(unix)]
#[test]
fn resample_prepublication_failures_leave_document_sources_and_destination_ownership_intact() {
    use aqua_pattern_core::resample::{self, Access};
    let dir = Dir::new();
    let base = resample_project_song(&dir);
    let original = std::fs::read(dir.path("source.wav")).unwrap();
    let config = resample_config(&dir, "never-clobber.wav");
    let plan = resample::prepare(&base, config.clone()).unwrap();
    assert!(resample::render(&plan, |_| Ok(()), || true).is_err());
    assert!(!config.output.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    assert!(resample::render(
        &plan,
        |access| match access {
            Access::PublishOutput(_) => Err("publish denied".into()),
            Access::ReadSource(_) => Ok(()),
        },
        || false
    )
    .unwrap_err()
    .contains("publish denied"));
    assert!(!config.output.exists());
    std::fs::write(&config.output, b"independent existing file").unwrap();
    assert!(resample::render(&plan, |_| Ok(()), || false).is_err());
    assert_eq!(
        std::fs::read(&config.output).unwrap(),
        b"independent existing file"
    );
    std::fs::remove_file(&config.output).unwrap();
    // The final publication callback may revoke authorization after all DSP/import
    // work. Only the owned temporary is cleaned; no candidate/file is published.
    let mut publications = 0;
    assert!(resample::render(
        &plan,
        |access| {
            if matches!(access, Access::PublishOutput(_)) {
                publications += 1;
                if publications == 2 {
                    return Err("final publication denied".into());
                }
            }
            Ok(())
        },
        || false
    )
    .unwrap_err()
    .contains("final publication denied"));
    assert_eq!(publications, 2);
    assert!(!config.output.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    std::fs::remove_file(dir.path("source.wav")).unwrap();
    let error = resample::render(
        &plan,
        |access| match access {
            Access::ReadSource(_) => Err("source policy before missing-file access".into()),
            Access::PublishOutput(_) => Ok(()),
        },
        || false,
    )
    .unwrap_err();
    assert!(error.contains("source policy before missing-file access"));
    assert!(!config.output.exists());
    std::fs::write(dir.path("source.wav"), &original).unwrap();
    let mut changed = original.clone();
    *changed.last_mut().unwrap() ^= 1;
    std::fs::write(dir.path("source.wav"), changed).unwrap();
    assert!(
        resample::render(&plan, |_| Ok(()), || false).is_err(),
        "source identity is reverified before publication"
    );
    assert!(!config.output.exists());
    std::fs::write(dir.path("source.wav"), &original).unwrap();
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), original);
}

#[test]
fn resample_preparation_id_exhaustion_and_capacity_errors_do_not_allocate_or_touch_history() {
    use aqua_pattern_core::resample;
    let dir = Dir::new();
    let base = resample_project_song(&dir);
    let config = resample_config(&dir, "unallocated.wav");
    for kind in 0..3 {
        let mut exhausted = base.clone();
        match kind {
            0 => {
                exhausted
                    .samples
                    .insert(SampleId(u64::MAX), exhausted.samples[&SampleId(11)].clone());
            }
            1 => {
                exhausted
                    .regions
                    .insert(RegionId(u64::MAX), exhausted.regions[&RegionId(21)].clone());
            }
            _ => {
                exhausted.instruments.insert(
                    InstrumentId(u64::MAX),
                    exhausted.instruments[&InstrumentId(4)].clone(),
                );
            }
        }
        exhausted.validate().unwrap();
        let mut editor = Editor::default();
        editor.replace_document(exhausted.clone()).unwrap();
        editor.edit_mode = true;
        editor.set_timing(123.0, 4).unwrap();
        editor.undo();
        let revision = editor.revision();
        assert!(resample::prepare(&editor.song, config.clone()).is_err());
        assert_eq!(editor.song, exhausted);
        assert_eq!(editor.revision(), revision);
        assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
        assert!(!config.output.exists());
    }
    let mut full = base.clone();
    for id in 1..=128 {
        full.samples
            .insert(SampleId(id), base.samples[&SampleId(11)].clone());
    }
    full.validate().unwrap();
    assert!(
        resample::prepare(&full, config.clone()).is_err(),
        "registration capacity is checked before rendering"
    );
    let mut too_long = base.clone();
    too_long.patterns.get_mut(&PatternId(1)).unwrap().rows = 65536;
    assert!(resample::prepare(&too_long, config.clone()).is_err());
    for invalid in [
        resample::ResampleConfig {
            version: 2,
            ..config.clone()
        },
        resample::ResampleConfig {
            root_note: 128,
            ..config.clone()
        },
        resample::ResampleConfig {
            occurrence: u32::MAX,
            ..config.clone()
        },
        resample::ResampleConfig {
            name: " ".into(),
            ..config.clone()
        },
        resample::ResampleConfig {
            output: PathBuf::from("relative.wav"),
            ..config.clone()
        },
    ] {
        assert!(resample::prepare(&base, invalid).is_err());
    }
    assert!(!config.output.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn registration_source_preflight_reuses_save_policy_without_rewriting_paths_or_decoding_legacy_files(
) {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    let legacy = dir.path("legacy-not-decoded.wav");
    std::fs::write(
        &legacy,
        b"legacy save policy requires readability, not decoding",
    )
    .unwrap();
    song.instruments.get_mut(&InstrumentId(4)).unwrap().sample = Some(legacy.clone());
    let before = song.clone();
    let mut authorized = Vec::new();
    project::check_registration_sources(&song, |path| {
        authorized.push(path.to_owned());
        Ok(())
    })
    .unwrap();
    assert_eq!(authorized, vec![legacy.clone(), dir.path("source.wav")]);
    assert_eq!(song, before);
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        2,
        "preflight publishes no project or repairs"
    );
    let saved = dir.path("same-policy.json");
    project::save(&song, &saved, SaveMode::CreateNew).unwrap();
    assert_eq!(project::load(&saved).unwrap().0, song);
    let mut relative = song.clone();
    relative
        .instruments
        .get_mut(&InstrumentId(4))
        .unwrap()
        .sample = Some("legacy-not-decoded.wav".into());
    assert!(project::check_registration_sources(&relative, |_| panic!(
        "relative legacy path must not infer CWD"
    ))
    .unwrap_err()
    .contains("must be absolute"));
    relative
        .instruments
        .get_mut(&InstrumentId(4))
        .unwrap()
        .sample = None;
    relative.samples.get_mut(&SampleId(11)).unwrap().path = "source.wav".into();
    assert!(project::check_registration_sources(&relative, |_| panic!(
        "relative source path must not infer CWD"
    ))
    .unwrap_err()
    .contains("must be absolute"));
    let missing = dir.path("unused-missing.wav");
    song.instruments.get_mut(&InstrumentId(4)).unwrap().sample = Some(missing.clone());
    let error = project::check_registration_sources(&song, |path| {
        if path == missing {
            Err("authorization before missing-file metadata".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(error.contains("authorization before missing-file metadata"));
    assert!(!missing.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

#[test]
fn registration_preflight_exact_size_and_all_source_budgets_precede_hashing_or_publication() {
    let dir = Dir::new();
    let mut song = sourced_song(&dir);
    let sparse = dir.path("sparse-size-only.wav");
    let file = std::fs::File::create(&sparse).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    song.samples.get_mut(&SampleId(11)).unwrap().path = sparse.clone();
    assert!(project::check_registration_sources(&song, |_| Ok(()))
        .unwrap_err()
        .contains("64 MiB encoded budget"));
    file.set_len(64 * 1024 * 1024).unwrap();
    let source = song.samples[&SampleId(11)].clone();
    for id in 12..=14 {
        song.samples.insert(SampleId(id), source.clone());
    }
    // Exactly 256 MiB reaches the hash stage; the intentionally stale hash then
    // rejects. At 256 MiB+1 it must fail the size stage before hashing even one file.
    assert!(project::check_registration_sources(&song, |_| Ok(()))
        .unwrap_err()
        .contains("SHA-256"));
    let extra = dir.path("one-extra-byte.wav");
    std::fs::write(&extra, [0]).unwrap();
    let mut one_byte = source;
    one_byte.path = extra;
    song.samples.insert(SampleId(15), one_byte);
    assert!(project::check_registration_sources(&song, |_| Ok(()))
        .unwrap_err()
        .contains("256 MiB aggregate"));
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
    let mut bounded = example_song();
    bounded.title.clear();
    let limit = 8 * 1024 * 1024;
    let overhead = project::check_serialized_size(&bounded).unwrap();
    bounded.title = "x".repeat(limit - overhead);
    assert_eq!(project::check_serialized_size(&bounded).unwrap(), limit);
    project::check_registration_sources(&bounded, |_| {
        panic!("no source IO in this exact-size fixture")
    })
    .unwrap();
    bounded.title.push('x');
    assert!(project::check_registration_sources(&bounded, |_| panic!(
        "oversize must reject before source IO"
    ))
    .unwrap_err()
    .contains("8 MiB"));
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 3);
}

#[cfg(unix)]
#[test]
fn printed_artifact_survives_registration_refusal_for_unused_missing_relative_or_overbudget_sources(
) {
    use aqua_pattern_core::resample::{self, Access};
    let dir = Dir::new();
    let base = resample_project_song(&dir);
    let original = std::fs::read(dir.path("source.wav")).unwrap();
    let sparse = dir.path("unused-sparse.wav");
    std::fs::File::create(&sparse)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    for case in 0..5 {
        let mut song = base.clone();
        let mut unused = song.samples[&SampleId(11)].clone();
        let expected = match case {
            0 => {
                unused.path = dir.path("unused-missing.wav");
                song.samples.insert(SampleId(12), unused);
                "missing/unreadable"
            }
            1 => {
                unused.path = "unused-relative.wav".into();
                song.samples.insert(SampleId(12), unused);
                "must be absolute"
            }
            2 => {
                song.instruments.get_mut(&InstrumentId(4)).unwrap().sample =
                    Some(dir.path("unused-legacy-missing.wav"));
                "missing/unreadable"
            }
            3 => {
                song.instruments.get_mut(&InstrumentId(4)).unwrap().sample =
                    Some("unused-legacy-relative.wav".into());
                "must be absolute"
            }
            _ => {
                unused.path = sparse.clone();
                for id in 12..=15 {
                    song.samples.insert(SampleId(id), unused.clone());
                }
                "256 MiB aggregate"
            }
        };
        song.validate().unwrap();
        let mut editor = Editor::default();
        editor.replace_document(song.clone()).unwrap();
        editor.edit_mode = true;
        editor.set_timing(123.0, 4).unwrap();
        editor.undo();
        let revision = editor.revision();
        let config = resample_config(&dir, &format!("registration-refused-{case}.wav"));
        let plan = resample::prepare(&song, config.clone()).unwrap();
        let result = resample::render(&plan, |_| Ok(()), || false).unwrap();
        let bytes = std::fs::read(&config.output).unwrap();
        assert!(
            resample::verify_artifact(&result, |_| Ok(()))
                .unwrap_err()
                .contains(expected),
            "case {case}"
        );
        if case == 0 {
            assert!(resample::verify_artifact(&result, |access| match access {
                Access::ReadSource(path) if path.ends_with("unused-missing.wav") =>
                    Err("unused source root denied".into()),
                _ => Ok(()),
            })
            .unwrap_err()
            .contains("unused source root denied"));
        }
        assert_eq!(editor.song, song);
        assert_eq!(editor.revision(), revision);
        assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
        assert_eq!(std::fs::read(&config.output).unwrap(), bytes);
        drop(result);
        assert_eq!(std::fs::read(config.output).unwrap(), bytes);
    }
    assert_eq!(std::fs::read(dir.path("source.wav")).unwrap(), original);
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        7,
        "only original source, sparse fixture and five explicitly published artifacts"
    );
}

fn tempo_project_song(dir: &Dir) -> Song {
    let mut song = groove_project_song(dir);
    let path = dir.path("groove-source.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for frame in 0..32_000i32 {
        writer
            .write_sample(((frame % 19 - 9) * 100) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
    song.samples.insert(
        SampleId(11),
        aqua_pattern_core::sample_asset::import(&path)
            .unwrap()
            .asset,
    );
    song.marker_sets.clear();
    song.upsert_marker_set(
        MarkerSetId(31),
        MarkerSet {
            name: "Accepted timing points, not automatic beat labels".into(),
            sample: SampleId(11),
            source_bpm: None,
            markers: (0..=8).map(|i| i * 4_000).collect(),
            locked_frames: vec![8_000, 24_000],
        },
    )
    .unwrap();
    song.bpm = 175.25;
    song.lines_per_beat = 8;
    song.validate().unwrap();
    song
}
fn tempo_project_analysis(song: &Song) -> aqua_pattern_core::tempo::TempoPreview {
    use aqua_pattern_core::tempo::{self, MarkerRequest, TempoConfig};
    tempo::analyze_markers(
        song,
        MarkerRequest {
            marker_set: MarkerSetId(31),
            start_index: 0,
            end_index: 8,
            include_start: true,
        },
        TempoConfig {
            version: 1,
            min_bpm_tenths: 1190,
            max_bpm_tenths: 1210,
        },
    )
    .unwrap()
}

#[test]
fn tempo_review_is_transient_and_only_explicit_source_bpm_roundtrips_in_legacy_schema() {
    let dir = Dir::new();
    let base = tempo_project_song(&dir);
    let source_path = dir.path("groove-source.wav");
    let original_audio = std::fs::read(&source_path).unwrap();
    let old_path = dir.path("old-markers.json");
    project::save(&base, &old_path, SaveMode::CreateNew).unwrap();
    let old_bytes = std::fs::read(&old_path).unwrap();
    let analysis = tempo_project_analysis(&base);
    let preview = analysis.use_bpm(MarkerSetId(31), 0).unwrap();
    let unchanged = dir.path("review-without-apply.json");
    project::save(&base, &unchanged, SaveMode::CreateNew).unwrap();
    assert_eq!(
        std::fs::read(unchanged).unwrap(),
        old_bytes,
        "analysis is not persisted or automatically applied"
    );
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let mut expected = base.clone();
    expected
        .marker_sets
        .get_mut(&MarkerSetId(31))
        .unwrap()
        .source_bpm = preview.candidate().marker_sets[&MarkerSetId(31)].source_bpm;
    editor
        .commit_tempo_preview(editor.revision(), &preview)
        .unwrap();
    assert_eq!(editor.song, expected);
    let path = dir.path("explicit-tempo.json");
    project::save(&editor.song, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut previous: serde_json::Value = serde_json::from_slice(&old_bytes).unwrap();
    previous["song"]["marker_sets"]["31"]["source_bpm"] =
        stored["song"]["marker_sets"]["31"]["source_bpm"].clone();
    assert_eq!(
        stored, previous,
        "no phase/score/analysis/result token or new Song field persists"
    );
    assert_eq!(stored["version"], 1);
    let loaded = project::load(&path).unwrap().0;
    assert_eq!(loaded, expected);
    assert_eq!(
        loaded.marker_sets[&MarkerSetId(31)]
            .source_bpm
            .unwrap()
            .to_bits(),
        expected.marker_sets[&MarkerSetId(31)]
            .source_bpm
            .unwrap()
            .to_bits()
    );
    let copy = dir.path("explicit-tempo-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    editor.redo();
    assert_eq!(editor.song, expected);
    let mut reopened = Editor::default();
    let mut session = Session::default();
    session.request(&mut reopened, Action::Open(path)).unwrap();
    assert_eq!(reopened.song, expected);
    assert!(!reopened.dirty && !reopened.can_undo());
    assert_eq!(std::fs::read(source_path).unwrap(), original_audio);
    assert_eq!(std::fs::read(old_path).unwrap(), old_bytes);
}

#[test]
fn tempo_metadata_review_does_not_bypass_later_asset_policy_or_failed_save_atomicity() {
    let dir = Dir::new();
    let base = tempo_project_song(&dir);
    let path = dir.path("saved.json");
    project::save(&base, &path, SaveMode::CreateNew).unwrap();
    let saved_bytes = std::fs::read(&path).unwrap();
    let source_path = dir.path("groove-source.wav");
    let original_audio = std::fs::read(&source_path).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let revision = editor.revision();
    let analysis = tempo_project_analysis(&base);
    let preview = analysis.use_bpm(MarkerSetId(31), 0).unwrap();
    std::fs::remove_file(&source_path).unwrap();
    // Marker-only use is metadata, not a fresh verification claim; IO policy still
    // applies independently when a project is saved or reopened.
    editor.commit_tempo_preview(revision, &preview).unwrap();
    let committed = editor.song.clone();
    assert!(project::save(
        &committed,
        &path,
        SaveMode::ReplaceConfirmed(saved_bytes.clone())
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    let mut session = Session::default();
    session
        .request(&mut editor, Action::Open(path.clone()))
        .unwrap();
    let failed_revision = editor.revision();
    assert!(session.discard(&mut editor).is_err());
    assert_eq!(editor.song, committed);
    assert_eq!(editor.revision(), failed_revision);
    assert!(editor.dirty && editor.can_undo());
    session.cancel();
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && editor.can_redo());
    std::fs::write(&source_path, &original_audio).unwrap();
    assert!(project::load_with_asset_policy(&path, |_| Err(
        "tempo source outside authorized root".into()
    ))
    .unwrap_err()
    .contains("outside authorized root"));
    session.request(&mut editor, Action::Open(path)).unwrap();
    editor.edit_mode = true;
    let opened_revision = editor.revision();
    assert!(
        editor.commit_tempo_preview(revision, &preview).is_err(),
        "even identical-content Open invalidates the old review generation"
    );
    assert_eq!(editor.revision(), opened_revision);
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && !editor.can_undo());
}

#[test]
fn malformed_saved_tempo_or_injected_analysis_never_replaces_active_document() {
    let dir = Dir::new();
    let base = tempo_project_song(&dir);
    let analysis = tempo_project_analysis(&base);
    let preview = analysis.use_bpm(MarkerSetId(31), 0).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base).unwrap();
    editor.edit_mode = true;
    editor
        .commit_tempo_preview(editor.revision(), &preview)
        .unwrap();
    let expected = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let path = dir.path("malformed-tempo.json");
    let valid = document(expected.clone());
    for case in 0..4 {
        let mut invalid = valid.clone();
        match case {
            0 => invalid["song"]["marker_sets"]["31"]["source_bpm"] = serde_json::json!(400.1),
            1 => invalid["song"]["marker_sets"]["31"]["source_bpm"] = serde_json::json!("NaN"),
            2 => invalid["song"]["marker_sets"]["31"]["phase_frames"] = serde_json::json!(123),
            _ => {
                invalid["song"]["tempo_analysis"] = serde_json::json!({"candidate": 0, "bpm": 120})
            }
        }
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let mut session = Session::default();
        session
            .request(&mut editor, Action::Open(path.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err(), "invalid case {case}");
        assert_eq!(editor.song, expected);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            (
                editor.occurrence,
                editor.track,
                editor.column,
                editor.row,
                editor.field
            ),
            cursor
        );
        assert!(editor.dirty && editor.can_undo());
    }
}

#[test]
fn tempo_use_bpm_pretty_size_preflight_preserves_clean_redo_on_one_byte_overflow() {
    use aqua_pattern_core::tempo::{self, MarkerRequest, TempoConfig};
    let dir = Dir::new();
    let mut song = tempo_project_song(&dir);
    // A reviewed 119.9 BPM value is one JSON byte longer than the existing null.
    // Use independently rounded 119.9-BPM source positions, not 120-BPM evidence
    // outside this exact single-tick search range (which correctly has no fallback).
    let set = song.marker_sets.get_mut(&MarkerSetId(31)).unwrap();
    set.markers = (0..8)
        .map(|i| (i * 4_800_000 + 599) / 1199)
        .chain([32_000])
        .collect();
    set.locked_frames = vec![set.markers[2], set.markers[6]];
    // Analysis fits exactly; presenting a commit plan must still enforce Save's
    // actual pretty wrapper rather than compact Song serialization.
    song.title.clear();
    let overhead = project::check_serialized_size(&song).unwrap();
    song.title = "x".repeat(8 * 1024 * 1024 - overhead);
    assert_eq!(
        project::check_serialized_size(&song).unwrap(),
        8 * 1024 * 1024
    );
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(176.25, 8).unwrap();
    editor.undo();
    let revision = editor.revision();
    let analysis = tempo::analyze_markers(
        &song,
        MarkerRequest {
            marker_set: MarkerSetId(31),
            start_index: 0,
            end_index: 8,
            include_start: true,
        },
        TempoConfig {
            version: 1,
            min_bpm_tenths: 1199,
            max_bpm_tenths: 1199,
        },
    )
    .unwrap();
    assert_eq!(analysis.analysis().candidates[0].bpm_tenths, 1199);
    assert!(analysis
        .use_bpm(MarkerSetId(31), 0)
        .unwrap_err()
        .contains("8 MiB"));
    assert_eq!(editor.song, song);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        1,
        "pure analysis and failed review publish no extra file"
    );
}

fn catalog_project_song(dir: &Dir) -> Song {
    let mut song = groove_project_song(dir);
    song.marker_sets.clear();
    for (id, start_frame, end_frame) in [(21, 0, 4_000), (22, 4_000, 8_000)] {
        song.upsert_region(
            RegionId(id),
            SampleRegion {
                name: format!("Native review slice {id}"),
                sample: SampleId(11),
                start_frame,
                end_frame,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
    }
    song.assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    song
}
fn catalog_feature_preview(song: &Song) -> aqua_pattern_core::slice_features::FeaturePreview {
    use aqua_pattern_core::slice_features::{self, FeatureRequest};
    let plan = slice_features::prepare(
        song,
        FeatureRequest {
            version: 1,
            sample: SampleId(11),
            regions: vec![RegionId(21), RegionId(22)],
        },
    )
    .unwrap();
    slice_features::analyze_verified(&plan, &song.samples[&SampleId(11)].path).unwrap()
}
fn catalog_role() -> aqua_pattern_core::slice_features::RoleLabel {
    use aqua_pattern_core::slice_features::{Articulation, PercussionRole, RoleLabel};
    RoleLabel {
        role: PercussionRole::Unknown,
        articulation: Articulation::Ghost,
        note: "Explicit contextual review, not an automatic amplitude label".into(),
    }
}

#[test]
fn measured_catalog_and_explicit_roles_commit_once_without_changing_playback_or_audio() {
    use aqua_pattern_core::slice_features::{self, RoleChoice};
    let dir = Dir::new();
    let mut song = catalog_project_song(&dir);
    song.set_region_role(RegionId(21), Some(catalog_role()))
        .unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.row = 6;
    editor.field = Field::Velocity;
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let audio = std::fs::read(dir.path("groove-source.wav")).unwrap();
    let before_manifest =
        serde_json::to_value(aqua_pattern_core::song_render::evaluate(&song, None).unwrap())
            .unwrap();
    let features = catalog_feature_preview(&song);
    assert_eq!(features.records().len(), 2);
    assert_eq!(
        editor.song, song,
        "measurement itself is not a document edit"
    );
    let preview = features
        .catalog_preview(vec![RoleChoice {
            region: RegionId(22),
            label: catalog_role(),
        }])
        .unwrap();
    assert!(editor
        .commit_region_catalog_preview(revision, &preview)
        .is_err());
    assert_eq!(editor.song, song);
    editor.edit_mode = true;
    let mut changed_audio = audio.clone();
    changed_audio[0] ^= 1;
    std::fs::write(&features.asset().path, changed_audio).unwrap();
    assert!(aqua_pattern_core::sample_asset::verify_identity(
        features.asset(),
        &features.asset().path
    )
    .is_err());
    assert_eq!(
        editor.song, song,
        "caller withholds pure commit after failed source re-verification"
    );
    assert_eq!(editor.revision(), revision);
    std::fs::write(&features.asset().path, &audio).unwrap();
    aqua_pattern_core::sample_asset::verify_identity(features.asset(), &features.asset().path)
        .unwrap();
    editor
        .commit_region_catalog_preview(revision, &preview)
        .unwrap();
    let stored = editor.song.clone();
    assert_eq!(&stored, preview.candidate());
    assert_eq!(
        stored.region_catalog[&RegionId(21)].reviewed_role,
        song.region_catalog[&RegionId(21)].reviewed_role
    );
    assert!(stored.region_catalog[&RegionId(22)]
        .reviewed_role
        .as_ref()
        .unwrap()
        .based_on_feature_sha256
        .is_some());
    for id in [RegionId(21), RegionId(22)] {
        let status = slice_features::inspect_catalog(&stored, id).unwrap();
        assert_eq!(status.features_current_match, Some(true));
        assert_eq!(status.reviewed_role_current_match, Some(true));
    }
    let mut stripped = stored.clone();
    stripped.region_catalog = song.region_catalog.clone();
    assert_eq!(stripped, song);
    assert_eq!(
        serde_json::to_value(aqua_pattern_core::song_render::evaluate(&stored, None).unwrap())
            .unwrap(),
        before_manifest
    );
    assert_eq!(
        (
            editor.occurrence,
            editor.track,
            editor.column,
            editor.row,
            editor.field
        ),
        cursor
    );
    editor.undo();
    assert_eq!(editor.song, song);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    editor.redo();
    assert_eq!(editor.song, stored);
    editor.clear_region_features(RegionId(21)).unwrap();
    assert!(editor.song.region_catalog[&RegionId(21)].features.is_none());
    assert_eq!(
        editor.song.region_catalog[&RegionId(21)].reviewed_role,
        stored.region_catalog[&RegionId(21)].reviewed_role
    );
    editor.undo();
    assert_eq!(editor.song, stored);
    editor.set_region_role(RegionId(22), None).unwrap();
    assert!(editor.song.region_catalog[&RegionId(22)]
        .reviewed_role
        .is_none());
    assert_eq!(
        editor.song.region_catalog[&RegionId(22)].features,
        stored.region_catalog[&RegionId(22)].features
    );
    editor.undo();
    assert_eq!(editor.song, stored);
    assert_eq!(std::fs::read(dir.path("groove-source.wav")).unwrap(), audio);
}

#[test]
fn catalog_roundtrip_preserves_separate_historical_bindings_without_new_asset_reads() {
    use aqua_pattern_core::slice_features::{self, RoleChoice};
    let dir = Dir::new();
    let base = catalog_project_song(&dir);
    let path = dir.path("legacy-no-catalog.json");
    project::save(&base, &path, SaveMode::CreateNew).unwrap();
    let old_bytes = std::fs::read(&path).unwrap();
    let old: serde_json::Value = serde_json::from_slice(&old_bytes).unwrap();
    assert!(old["song"].get("region_catalog").is_none());
    let features = catalog_feature_preview(&base);
    let unchanged = dir.path("analysis-only.json");
    project::save(&base, &unchanged, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(unchanged).unwrap(), old_bytes);
    let preview = features
        .catalog_preview(vec![RoleChoice {
            region: RegionId(21),
            label: catalog_role(),
        }])
        .unwrap();
    let stored = preview.candidate().clone();
    let path = dir.path("catalog.json");
    project::save(&stored, &path, SaveMode::CreateNew).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let loaded = project::load(&path).unwrap().0;
    assert_eq!(
        loaded, stored,
        "native metric f64 values and separate role provenance roundtrip exactly"
    );
    let copy = dir.path("catalog-copy.json");
    project::save(&loaded, &copy, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), bytes);
    let mut historical = loaded.clone();
    let mut region = historical.regions[&RegionId(21)].clone();
    region.start_frame += 1;
    historical.upsert_region(RegionId(21), region).unwrap();
    assert_eq!(
        historical.region_catalog, loaded.region_catalog,
        "bounds edit retains evidence rather than silently rewriting/deleting it"
    );
    assert_eq!(
        slice_features::inspect_catalog(&historical, RegionId(21))
            .unwrap()
            .features_current_match,
        Some(false)
    );
    let new_path = dir.path("new-current-source.wav");
    std::fs::copy(dir.path("groove-source.wav"), &new_path).unwrap();
    historical
        .register_sample(
            SampleId(12),
            aqua_pattern_core::sample_asset::import(&new_path)
                .unwrap()
                .asset,
        )
        .unwrap();
    for id in [RegionId(21), RegionId(22)] {
        let mut region = historical.regions[&id].clone();
        region.sample = SampleId(12);
        historical.upsert_region(id, region).unwrap();
    }
    historical.remove_sample(SampleId(11)).unwrap();
    std::fs::remove_file(dir.path("groove-source.wav")).unwrap();
    let history_path = dir.path("historical-catalog.json");
    project::save(&historical, &history_path, SaveMode::CreateNew).unwrap();
    let calls = std::cell::RefCell::new(Vec::new());
    let reopened = project::load_with_asset_policy(&history_path, |path| {
        calls.borrow_mut().push(path.to_owned());
        Ok(())
    })
    .unwrap()
    .0;
    assert_eq!(reopened, historical);
    assert_eq!(*calls.borrow(), vec![new_path]);
    assert_eq!(reopened.region_catalog, stored.region_catalog);
    for id in [RegionId(21), RegionId(22)] {
        assert_eq!(
            slice_features::inspect_catalog(&reopened, id)
                .unwrap()
                .features_current_match,
            Some(false)
        );
    }
}

#[test]
fn measured_catalog_reviews_are_stale_safe_and_recomputing_preserves_separate_manual_roles() {
    use aqua_pattern_core::slice_features::{self, RoleChoice};
    let dir = Dir::new();
    let base = catalog_project_song(&dir);
    let features = catalog_feature_preview(&base);
    assert!(features
        .catalog_preview(vec![RoleChoice {
            region: RegionId(999),
            label: catalog_role()
        }])
        .is_err());
    assert!(features
        .catalog_preview(vec![
            RoleChoice {
                region: RegionId(21),
                label: catalog_role()
            },
            RoleChoice {
                region: RegionId(21),
                label: catalog_role()
            }
        ])
        .is_err());
    let preview = features.catalog_preview(Vec::new()).unwrap();
    assert!(
        preview
            .candidate()
            .region_catalog
            .values()
            .all(|e| e.reviewed_role.is_none()),
        "suggestions are not automatically accepted labels"
    );
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let revision = editor.revision();
    editor
        .set_region_role(RegionId(21), Some(catalog_role()))
        .unwrap();
    let changed = editor.song.clone();
    assert!(editor
        .commit_region_catalog_preview(editor.revision(), &preview)
        .is_err());
    assert_eq!(editor.song, changed);
    editor.undo();
    assert_eq!(editor.song, base);
    let current_revision = editor.revision();
    assert!(editor
        .commit_region_catalog_preview(revision, &preview)
        .is_err());
    assert_eq!(editor.revision(), current_revision);
    assert!(!editor.dirty && editor.can_redo());
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    assert!(editor
        .commit_region_catalog_preview(revision, &preview)
        .is_err());
    let before_bounds = editor.song.clone();
    let mut changed_region = editor.song.regions[&RegionId(21)].clone();
    changed_region.end_frame -= 1;
    editor.upsert_region(RegionId(21), changed_region).unwrap();
    let changed_bounds = editor.song.clone();
    assert!(editor
        .commit_region_catalog_preview(editor.revision(), &preview)
        .is_err());
    assert_eq!(editor.song, changed_bounds);
    editor.undo();
    assert_eq!(editor.song, before_bounds);
    let fresh = catalog_feature_preview(&editor.song)
        .catalog_preview(vec![RoleChoice {
            region: RegionId(21),
            label: catalog_role(),
        }])
        .unwrap();
    editor
        .commit_region_catalog_preview(editor.revision(), &fresh)
        .unwrap();
    let stored = editor.song.clone();
    let reviewed = stored.region_catalog[&RegionId(21)].reviewed_role.clone();
    editor.mark_saved();
    editor.set_timing(121.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    let recomputed = catalog_feature_preview(&editor.song)
        .catalog_preview(Vec::new())
        .unwrap();
    assert_eq!(recomputed.candidate(), &stored);
    editor
        .commit_region_catalog_preview(revision, &recomputed)
        .unwrap();
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && editor.can_redo());
    editor
        .set_region_role(RegionId(21), Some(catalog_role()))
        .unwrap();
    assert_eq!(
        editor.revision(),
        revision,
        "identical current manual control does not erase measurement provenance or redo"
    );
    assert!(!editor.dirty && editor.can_redo());
    assert_eq!(
        editor.song.region_catalog[&RegionId(21)].reviewed_role,
        reviewed
    );
    let mut region = editor.song.regions[&RegionId(21)].clone();
    region.start_frame += 1;
    editor.upsert_region(RegionId(21), region).unwrap();
    let refreshed = catalog_feature_preview(&editor.song)
        .catalog_preview(Vec::new())
        .unwrap();
    editor
        .commit_region_catalog_preview(editor.revision(), &refreshed)
        .unwrap();
    assert_eq!(
        editor.song.region_catalog[&RegionId(21)].reviewed_role,
        reviewed,
        "fresh features do not silently relabel/rebind a user's old review"
    );
    let status = slice_features::inspect_catalog(&editor.song, RegionId(21)).unwrap();
    assert_eq!(status.features_current_match, Some(true));
    assert_eq!(status.reviewed_role_current_match, Some(false));
    let stale_review = editor.song.clone();
    editor
        .set_region_role(RegionId(21), Some(catalog_role()))
        .unwrap();
    let reconfirmed = slice_features::inspect_catalog(&editor.song, RegionId(21)).unwrap();
    assert_eq!(reconfirmed.reviewed_role_current_match, Some(true));
    assert!(
        editor.song.region_catalog[&RegionId(21)]
            .reviewed_role
            .as_ref()
            .unwrap()
            .based_on_feature_sha256
            .is_none(),
        "explicit stale-role reconfirmation is manual, not fabricated feature authority"
    );
    editor.undo();
    assert_eq!(editor.song, stale_review);
}

#[test]
fn malformed_catalog_metadata_never_replaces_dirty_history_or_existing_project() {
    use aqua_pattern_core::slice_features::RoleChoice;
    let dir = Dir::new();
    let base = catalog_project_song(&dir);
    let preview = catalog_feature_preview(&base)
        .catalog_preview(vec![RoleChoice {
            region: RegionId(21),
            label: catalog_role(),
        }])
        .unwrap();
    let saved = preview.candidate().clone();
    let path = dir.path("valid-catalog.json");
    project::save(&saved, &path, SaveMode::CreateNew).unwrap();
    let saved_bytes = std::fs::read(&path).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(saved.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(121.0, 4).unwrap();
    editor.set_timing(122.0, 4).unwrap();
    editor.undo();
    let active = editor.song.clone();
    let revision = editor.revision();
    let invalid_path = dir.path("invalid-catalog.json");
    let valid = document(saved);
    for (pointer, value) in [
        (
            "/song/region_catalog/21/features/version",
            serde_json::json!(2),
        ),
        (
            "/song/region_catalog/21/features/binding/source_sha256",
            serde_json::json!("not a hash"),
        ),
        (
            "/song/region_catalog/21/features/binding/channels",
            serde_json::json!(3),
        ),
        (
            "/song/region_catalog/21/features/metrics/rms",
            serde_json::json!(-0.5),
        ),
        (
            "/song/region_catalog/21/features/metrics/peak",
            serde_json::json!("NaN"),
        ),
        (
            "/song/region_catalog/21/reviewed_role/label/role",
            serde_json::json!("automatically_proven_kick"),
        ),
        (
            "/song/region_catalog/21/reviewed_role/label/note",
            serde_json::json!("é".repeat(129)),
        ),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let mut session = Session::default();
        session
            .request(&mut editor, Action::Open(invalid_path.clone()))
            .unwrap();
        assert!(session.discard(&mut editor).is_err(), "{pointer}");
        assert_eq!(editor.song, active);
        assert_eq!(editor.revision(), revision);
        assert!(editor.dirty && editor.can_redo());
        assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    }
    let mut dangling = valid;
    let entries = dangling["song"]["region_catalog"].as_object_mut().unwrap();
    let entry = entries.remove("21").unwrap();
    entries.insert("999".into(), entry);
    std::fs::write(&invalid_path, serde_json::to_vec(&dangling).unwrap()).unwrap();
    assert!(
        project::load(&invalid_path).is_err(),
        "catalog keys must refer to a current region, even though audio bindings are historical"
    );
    let mut invalid = active.clone();
    invalid
        .region_catalog
        .get_mut(&RegionId(21))
        .unwrap()
        .features
        .as_mut()
        .unwrap()
        .metrics
        .rms = f64::NAN;
    assert!(project::save(
        &invalid,
        &path,
        SaveMode::ReplaceConfirmed(saved_bytes.clone())
    )
    .is_err());
    assert_eq!(std::fs::read(path).unwrap(), saved_bytes);
    assert_eq!(editor.song, active);
    assert_eq!(editor.revision(), revision);
}

fn thin_project_song(dir: &Dir) -> Song {
    use aqua_pattern_core::slice_features::{Articulation, PercussionRole, RoleLabel};
    let mut song = catalog_project_song(dir);
    let rows = song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    for (&row, columns) in rows.iter_mut() {
        let cell = columns.get_mut(&0).unwrap();
        cell.velocity = Some(0.5);
        cell.pan = Some(0.25);
        cell.delay = Some(if row == 0 { 0.0 } else { 0.23703703703703694 });
    }
    song.set_region_role(
        RegionId(21),
        Some(RoleLabel {
            role: PercussionRole::ClosedHat,
            articulation: Articulation::Normal,
            note: "Explicit manual annotation; source stays unchanged".into(),
        }),
    )
    .unwrap();
    song
}
fn thin_project_config() -> aqua_pattern_core::mutations::ThinConfig {
    use aqua_pattern_core::slice_features::PercussionRole;
    aqua_pattern_core::mutations::ThinConfig {
        version: 1,
        scope: aqua_pattern_core::transforms::TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 2,
            columns: vec![0],
        },
        seed: 0xfeed_beef,
        removal_percent: 100,
        max_removed: 16,
        protected_roles: vec![PercussionRole::Kick, PercussionRole::Snare],
    }
}

#[test]
fn thin_save_reopen_contains_only_baked_cells_and_preserves_catalog_and_original_audio() {
    use aqua_pattern_core::{mutations, song_render};
    let dir = Dir::new();
    let base = thin_project_song(&dir);
    let source = base.samples[&SampleId(11)].path.clone();
    let source_bytes = std::fs::read(&source).unwrap();
    let old_path = dir.path("before-thin.json");
    project::save(&base, &old_path, SaveMode::CreateNew).unwrap();
    let old_bytes = std::fs::read(&old_path).unwrap();
    let config = thin_project_config();
    let preview = mutations::preview(&base, config.clone()).unwrap();
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    let repeated = mutations::preview(&base, config.clone()).unwrap();
    assert_eq!(preview.id(), repeated.id());
    assert_eq!(preview.candidate(), repeated.candidate());
    assert_eq!(preview.record(), repeated.record());
    assert_eq!(preview.changes().len(), 2);
    for change in preview.changes() {
        assert!(
            change.after.is_none(),
            "whole-cell removal, not partial note clearing"
        );
        assert_eq!(
            change.before,
            base.patterns[&PatternId(1)].tracks[&TrackId(1)][&change.row][&change.column]
        );
    }
    assert_eq!(preview.record().config, config);
    assert_eq!(preview.record().change_count, preview.changes().len());
    let mut expected = base.clone();
    let rows = expected
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    rows.get_mut(&0).unwrap().remove(&0);
    rows.get_mut(&1).unwrap().remove(&0);
    assert_eq!(preview.candidate(), &expected);
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    editor.row = 3;
    editor.field = Field::Delay;
    let revision = editor.revision();
    editor.commit_thin_preview(revision, &preview).unwrap();
    assert_eq!(editor.song, expected);
    assert_eq!((editor.row, editor.field), (3, Field::Delay));
    let new_path = dir.path("after-thin.json");
    project::save(&editor.song, &new_path, SaveMode::CreateNew).unwrap();
    let (restored, _) = project::load(&new_path).unwrap();
    assert_eq!(restored, expected);
    assert_eq!(restored.region_catalog, base.region_catalog);
    assert_eq!(
        song_render::evaluate(&restored, None).unwrap(),
        song_render::evaluate(&expected, None).unwrap()
    );
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&new_path).unwrap()).unwrap();
    for field in [
        "mutations",
        "thin",
        "mutation_record",
        "thin_seed",
        "thin_config",
    ] {
        assert!(
            data["song"].get(field).is_none(),
            "{field} is not live or persisted authority"
        );
    }
    let mut restored_editor = Editor::default();
    restored_editor.replace_document(restored.clone()).unwrap();
    assert!(
        !restored_editor.can_undo(),
        "Open does not recreate a hidden mutation history"
    );
    restored_editor.edit_mode = true;
    restored_editor.row = 2;
    restored_editor.field = Field::Note;
    restored_editor.enter_note(Note::Note(75)).unwrap();
    restored_editor.undo();
    assert_eq!(
        restored_editor.song, restored,
        "output is ordinary editable content"
    );
    editor.undo();
    assert_eq!(editor.song, base);
    editor.redo();
    assert_eq!(editor.song, expected);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
}

#[test]
fn thin_identity_review_keeps_old_project_bytes_redo_and_missing_media_independence() {
    use aqua_pattern_core::mutations;
    let dir = Dir::new();
    let base = thin_project_song(&dir);
    let source = base.samples[&SampleId(11)].path.clone();
    let source_bytes = std::fs::read(&source).unwrap();
    let path = dir.path("identity-before.json");
    project::save(&base, &path, SaveMode::CreateNew).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(121.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    let mut config = thin_project_config();
    config.removal_percent = 0;
    std::fs::remove_file(&source).unwrap();
    let preview = mutations::preview(&base, config).unwrap();
    let changed = mutations::preview(&base, thin_project_config()).unwrap();
    assert_ne!(
        changed.candidate(),
        &base,
        "model-evaluable does not claim current external bytes or require audio IO"
    );
    editor.commit_thin_preview(revision, &preview).unwrap();
    assert_eq!(editor.song, base);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    assert!(
        !source.exists(),
        "preview/commit must not restore or publish source files"
    );
    std::fs::write(&source, &source_bytes).unwrap();
    let after_path = dir.path("identity-after.json");
    project::save(&editor.song, &after_path, SaveMode::CreateNew).unwrap();
    assert_eq!(
        std::fs::read(&after_path).unwrap(),
        original,
        "a reviewed identity adds no recipe/schema or float normalization"
    );
    assert_eq!(project::load(&after_path).unwrap().0, base);
}

#[test]
fn thin_role_and_reloaded_document_changes_invalidate_review_without_losing_history() {
    use aqua_pattern_core::{mutations, slice_features::PercussionRole};
    let dir = Dir::new();
    let base = thin_project_song(&dir);
    let path = dir.path("thin-review-base.json");
    project::save(&base, &path, SaveMode::CreateNew).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let revision = editor.revision();
    let preview = mutations::preview(&base, thin_project_config()).unwrap();
    let mut label = base.region_catalog[&RegionId(21)]
        .reviewed_role
        .as_ref()
        .unwrap()
        .label
        .clone();
    label.role = PercussionRole::Kick;
    editor.set_region_role(RegionId(21), Some(label)).unwrap();
    let changed = editor.song.clone();
    let changed_revision = editor.revision();
    assert!(editor
        .commit_thin_preview(changed_revision, &preview)
        .is_err());
    assert_eq!(editor.song, changed);
    assert_eq!(editor.revision(), changed_revision);
    editor.undo();
    assert_eq!(editor.song, base);
    assert!(editor.commit_thin_preview(revision, &preview).is_err());
    assert!(!editor.dirty && editor.can_redo());
    let current_revision = editor.revision();
    let mut session = Session::default();
    session
        .request(&mut editor, Action::Open(path.clone()))
        .unwrap();
    editor.edit_mode = true;
    assert_eq!(editor.song, base);
    assert!(
        editor
            .commit_thin_preview(current_revision, &preview)
            .is_err(),
        "reopening equal persisted bytes invalidates a pending Editor-generation review"
    );
    assert!(!editor.dirty && !editor.can_undo() && !editor.can_redo());
    assert_eq!(session.path, Some(path));
}

#[test]
fn thin_failed_storage_preserves_committed_song_dirty_history_existing_file_and_source() {
    use aqua_pattern_core::mutations;
    let dir = Dir::new();
    let base = thin_project_song(&dir);
    let source = base.samples[&SampleId(11)].path.clone();
    let source_bytes = std::fs::read(&source).unwrap();
    let path = dir.path("protected-thin-save.json");
    project::save(&base, &path, SaveMode::CreateNew).unwrap();
    let saved = std::fs::read(&path).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let preview = mutations::preview(&base, thin_project_config()).unwrap();
    editor
        .commit_thin_preview(editor.revision(), &preview)
        .unwrap();
    let committed = editor.song.clone();
    let revision = editor.revision();
    let mut session = Session {
        path: Some(path.clone()),
        ..Default::default()
    };
    let mut tampered = source_bytes.clone();
    *tampered.last_mut().unwrap() ^= 1;
    std::fs::write(&source, &tampered).unwrap();
    session
        .save_request(&mut editor, path.clone(), false)
        .unwrap();
    assert!(session.confirm_overwrite(&mut editor).is_err());
    assert_eq!(editor.song, committed);
    assert_eq!(editor.revision(), revision);
    assert!(editor.dirty && editor.can_undo());
    assert_eq!(session.path, Some(path.clone()));
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert_eq!(
        std::fs::read(&source).unwrap(),
        tampered,
        "failed storage does not repair or rewrite the caller's source"
    );
    editor.undo();
    assert_eq!(editor.song, base);
    editor.redo();
    assert_eq!(editor.song, committed);
    std::fs::write(&source, &source_bytes).unwrap();
    session
        .save_request(&mut editor, path.clone(), false)
        .unwrap();
    session.confirm_overwrite(&mut editor).unwrap();
    assert!(!editor.dirty);
    assert_eq!(project::load(&path).unwrap().0, committed);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
}
