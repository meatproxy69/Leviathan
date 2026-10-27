use aqua_pattern_core::{
    editor::{Editor, Field},
    song::{Note, PatternId, TrackId},
};
#[test]
fn tempo_edit_is_guarded_validated_and_undoable() {
    let mut editor = Editor::default();
    let original = editor.song.bpm;
    assert!(editor.set_bpm(140.0).is_err());
    editor.edit_mode = true;
    assert!(editor.set_bpm(401.0).is_err());
    assert_eq!(editor.song.bpm, original);
    editor.set_bpm(140.0).unwrap();
    assert!(editor.dirty);
    assert_eq!(editor.song.bpm, 140.0);
    editor.undo();
    assert_eq!(editor.song.bpm, original);
    editor.redo();
    assert_eq!(editor.song.bpm, 140.0);
}

#[test]
fn default_has_saved_24_slots_and_existing_song_expansion_is_undoable() {
    let mut fresh = Editor::default();
    assert_eq!(fresh.song.tracks.len(), 24);
    assert_eq!(fresh.song.instruments.len(), 24);
    assert!(!fresh.dirty);
    assert!(!fresh.can_undo());
    fresh.edit_mode = true;
    fresh.ensure_24_slots().unwrap();
    assert!(!fresh.dirty);
    assert!(!fresh.can_undo());

    let mut existing = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    existing
        .replace_document(aqua_pattern_core::song::example_song())
        .unwrap();
    existing.edit_mode = true;
    let before = existing.song.clone();
    existing.ensure_24_slots().unwrap();
    assert_eq!(existing.song.tracks.len(), 24);
    assert_eq!(existing.song.instruments.len(), 24);
    assert_eq!(existing.song.tracks[&TrackId(1)].note_columns, 2);
    assert_eq!(
        existing.song.patterns[&PatternId(1)],
        before.patterns[&PatternId(1)]
    );
    assert!(existing.dirty && existing.can_undo());
    existing.undo();
    assert_eq!(existing.song, before);
    assert!(!existing.dirty);
    existing.redo();
    assert_eq!(existing.song.tracks.len(), 24);
}
#[test]
fn edit_mode_protects_all_mutations_and_navigation_remains_available() {
    let mut e = Editor::default();
    let before = e.song.clone();
    assert!(e.enter_note(Note::Note(60)).is_err());
    assert!(e.enter_value("72").is_err());
    assert!(e.clear(true).is_err());
    assert!(e.resize(128).is_err());
    assert!(e.append(false).is_err());
    assert!(e.append(true).is_err());
    e.move_rows(10);
    e.move_field(Field::ALL.len() as i32);
    assert_eq!(e.row, 10);
    assert_eq!(e.column, 1);
    assert_eq!(e.song, before);
    assert!(!e.dirty);
}
#[test]
fn optional_fields_and_clear_do_not_overwrite_other_fields() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    for (field, value) in [
        (Field::Instrument, ""),
        (Field::Velocity, "0"),
        (Field::Pan, "-0.5"),
        (Field::Delay, "0.25"),
    ] {
        e.field = field;
        e.enter_value(value).unwrap();
    }
    e.field = Field::Note;
    e.enter_value("OFF").unwrap();
    let c = e.cell();
    assert_eq!(c.note, Note::Off);
    assert_eq!(c.instrument, None);
    assert_eq!(c.velocity, Some(0.0));
    assert_eq!(c.pan, Some(-0.5));
    assert_eq!(c.delay, Some(0.25));
    e.clear(false).unwrap();
    assert_eq!(e.cell().note, Note::Empty);
    assert_eq!(e.cell().delay, Some(0.25));
    e.clear(true).unwrap();
    assert_eq!(e.cell(), Default::default());
    e.song.validate().unwrap();
}
#[test]
fn invalid_edits_are_atomic_and_resize_preserves_content() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    for (field, value) in [
        (Field::Note, "128"),
        (Field::Instrument, "99"),
        (Field::Velocity, "NaN"),
        (Field::Pan, "2"),
        (Field::Delay, "1"),
    ] {
        e.field = field;
        let before = e.song.clone();
        assert!(e.enter_value(value).is_err());
        assert_eq!(e.song, before);
    }
    let before = e.song.clone();
    assert!(e.resize(31).is_err());
    assert_eq!(e.song, before);
    e.resize(128).unwrap();
    assert_eq!(e.pattern().rows, 128);
    assert!(e.resize(0).is_err());
    assert!(e.resize(65537).is_err());
}
#[test]
fn order_block_selection_repeat_and_clone_link_correct_content() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 0,
        ..Editor::default()
    };
    e.select(2, TrackId(3)).unwrap();
    e.enter_note(Note::Note(80)).unwrap();
    e.select(0, TrackId(3)).unwrap();
    assert_eq!(e.cell().note, Note::Note(80));
    e.append(true).unwrap();
    assert_eq!(e.pattern_id(), PatternId(3));
    assert_eq!(e.occurrence, 3);
    e.enter_note(Note::Note(81)).unwrap();
    e.select(0, TrackId(3)).unwrap();
    assert_eq!(e.cell().note, Note::Note(80));
    e.append(false).unwrap();
    assert_eq!(e.pattern_id(), PatternId(1));
    assert_eq!(e.occurrence, 4);
    let before = (e.occurrence, e.track);
    assert!(e.select(999, TrackId(1)).is_err());
    assert!(e.select(1, TrackId(999)).is_err());
    assert_eq!((e.occurrence, e.track), before);
}
#[test]
fn navigation_and_edit_step_stay_inside_variable_patterns() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 4,
        ..Editor::default()
    };
    e.replace_document(aqua_pattern_core::song::example_song())
        .unwrap();
    e.edit_mode = true;
    e.enter_note(Note::Note(60)).unwrap();
    assert_eq!(e.row, 4);
    e.move_rows(99999);
    assert_eq!(e.row, 31);
    e.enter_note(Note::Off).unwrap();
    assert_eq!(e.row, 31);
    e.select(1, TrackId(4)).unwrap();
    e.move_rows(999);
    assert_eq!(e.row, 63);
    e.select(0, TrackId(1)).unwrap();
    assert_eq!(e.row, 31);
    e.move_rows(-999);
    assert_eq!(e.row, 0);
    e.move_field(999);
    assert_eq!((e.track, e.column, e.field), (TrackId(4), 1, Field::Phrase));
    e.move_field(-999);
    assert_eq!((e.track, e.column, e.field), (TrackId(1), 0, Field::Note));
}

#[cfg(feature = "desktop")]
#[test]
fn egui_keyboard_focus_keeps_path_typing_out_of_song() {
    use aqua_pattern_core::tracker_ui::{grid_id, keyboard};
    use eframe::egui;
    let ctx = egui::Context::default();
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let before = editor.song.clone();
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut path = String::new();
    let path_id = egui::Id::new("sample-path-test");
    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut path).id(path_id))
                .request_focus();
        });
    });
    let _ = ctx.run(
        egui::RawInput {
            events: vec![key(egui::Key::Z), egui::Event::Text("z/sample.wav".into())],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut path).id(path_id));
            });
            keyboard(&mut editor, ctx).unwrap();
        },
    );
    assert_eq!(path, "z/sample.wav");
    assert_eq!(editor.song, before);
    ctx.memory_mut(|m| m.request_focus(grid_id()));
    let _ = ctx.run(
        egui::RawInput {
            events: vec![key(egui::Key::Z)],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.interact(
                    ui.max_rect(),
                    grid_id(),
                    egui::Sense::focusable_noninteractive(),
                );
            });
            keyboard(&mut editor, ctx).unwrap();
        },
    );
    assert_eq!(editor.row, 1);
    editor.row = 0;
    assert_eq!(editor.cell().note, Note::Note(60));
    editor.edit_mode = false;
    let before = editor.song.clone();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![key(egui::Key::X)],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.interact(
                    ui.max_rect(),
                    grid_id(),
                    egui::Sense::focusable_noninteractive(),
                );
            });
            keyboard(&mut editor, ctx).unwrap();
        },
    );
    assert_eq!(editor.song, before);
}

#[cfg(feature = "desktop")]
#[test]
fn undo_shortcuts_only_affect_song_with_grid_focus() {
    use aqua_pattern_core::tracker_ui::{grid_id, keyboard};
    use eframe::egui;
    let ctx = egui::Context::default();
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let initial = e.song.clone();
    e.enter_note(Note::Note(70)).unwrap();
    let edited = e.song.clone();
    let mut text = String::new();
    let path_id = egui::Id::new("project-path");
    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(path_id))
                .request_focus();
        });
    });
    let input = || egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let _ = ctx.run(input(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(path_id));
        });
        keyboard(&mut e, ctx).unwrap();
    });
    assert_eq!(e.song, edited);
    ctx.memory_mut(|m| m.request_focus(grid_id()));
    let _ = ctx.run(input(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.interact(
                ui.max_rect(),
                grid_id(),
                egui::Sense::focusable_noninteractive(),
            );
        });
        keyboard(&mut e, ctx).unwrap();
    });
    assert_eq!(e.song, initial);
}
