use aqua_pattern_core::{
    editor::{Editor, Field},
    song::{
        Cell, Direction, Glide, InstrumentId, Note, PatternId, Phrase, PhraseId, PhraseTrigger,
        Retrigger, StepEffects, TrackId,
    },
};
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
    e.move_field(6);
    assert_eq!(e.row, 10);
    assert_eq!(e.column, 1);
    assert_eq!(e.song, before);
    assert!(!e.dirty);
}

#[test]
fn track_and_instrument_expansion_is_atomic_and_undoable_past_new_song_capacity() {
    let mut e = Editor::default();
    e.replace_document(aqua_pattern_core::song::new_song())
        .unwrap();
    e.edit_mode = true;
    let original = e.song.clone();
    let track = e.add_track("Track 25".into()).unwrap();
    assert_eq!(track, TrackId(25));
    assert!(e.song.tracks.contains_key(&track));
    assert_eq!(e.song.track_order.last(), Some(&track));
    assert!(e
        .song
        .patterns
        .values()
        .all(|pattern| !pattern.tracks.contains_key(&track)));
    let instrument = e.add_instrument("Instrument 25".into()).unwrap();
    assert_eq!(instrument, InstrumentId(25));
    assert_eq!(e.song.instruments[&instrument].name, "Instrument 25");
    assert!(e.dirty && e.can_undo());
    e.undo();
    assert!(!e.song.instruments.contains_key(&instrument));
    assert!(e.song.tracks.contains_key(&track));
    e.undo();
    assert_eq!(e.song, original);
    e.redo();
    e.redo();
    assert!(e.song.instruments.contains_key(&instrument));
    assert!(e.song.tracks.contains_key(&track));

    for n in 26..=27 {
        e.add_track(format!("Track {n}")).unwrap();
        e.add_instrument(format!("Instrument {n}")).unwrap();
    }
    assert_eq!(e.song.tracks.len(), 27);
    assert_eq!(e.song.instruments.len(), 27);
    e.song.validate().unwrap();
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
    assert_eq!((e.track, e.column, e.field), (TrackId(4), 1, Field::Delay));
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

#[test]
fn extreme_navigation_and_edit_steps_saturate_without_overflow() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: u32::MAX,
        row: 10,
        ..Editor::default()
    };
    e.move_rows(i64::MAX);
    assert_eq!(e.row, 31);
    e.move_rows(i64::MIN);
    assert_eq!(e.row, 0);
    e.move_field(1);
    e.move_field(i32::MAX);
    assert_eq!((e.track, e.column, e.field), (TrackId(4), 1, Field::Delay));
    e.move_field(i32::MIN);
    assert_eq!((e.track, e.column, e.field), (TrackId(1), 0, Field::Note));
    e.enter_note(Note::Note(60)).unwrap();
    assert_eq!(e.row, 31);
    let before = e.song.clone();
    e.move_occurrence(i32::MIN).unwrap();
    e.move_occurrence(i32::MAX).unwrap();
    assert_eq!(e.occurrence, 0);
    assert_eq!(e.song, before);
}

#[test]
fn empty_and_stale_selections_reject_edits_without_partial_commits() {
    let mut empty = Editor::default().song;
    empty.order.clear();
    let mut e = Editor::default();
    e.replace_document(empty).unwrap();
    e.edit_mode = true;
    let mut invalid = vec![e];
    for (occurrence, track, row, column) in [
        (usize::MAX, TrackId(1), 0, 0),
        (0, TrackId(999), 0, 0),
        (0, TrackId(1), u32::MAX, 0),
        (0, TrackId(1), 0, u16::MAX),
    ] {
        invalid.push(Editor {
            edit_mode: true,
            occurrence,
            track,
            row,
            column,
            ..Editor::default()
        });
    }
    for mut e in invalid {
        let before = e.song.clone();
        let cursor = (e.occurrence, e.track, e.row, e.column);
        assert!(!e.has_grid());
        assert!(e.set_cell(Default::default()).is_err());
        assert!(e.set_effects(StepEffects::default()).is_err());
        assert!(e.set_phrase_trigger(None).is_err());
        assert!(e.enter_note(Note::Off).is_err());
        assert!(e.enter_value("60").is_err());
        assert!(e.clear(true).is_err());
        assert!(e.resize(128).is_err());
        assert!(e.append(false).is_err());
        assert!(e.append(true).is_err());
        assert!(e.move_occurrence(1).is_err());
        assert_eq!(e.song, before);
        assert_eq!((e.occurrence, e.track, e.row, e.column), cursor);
        assert!(!e.can_undo() && !e.can_redo() && !e.dirty);
        // Read-only lookup and navigation must also tolerate absent selections.
        let _ = e.cell();
        e.move_rows(i64::MAX);
        e.move_field(i32::MAX);
        assert_eq!(e.song, before);
    }
}

#[test]
fn invalid_document_selection_and_order_move_are_atomic() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.song.patterns.remove(&PatternId(2));
    let cursor = (e.occurrence, e.track, e.row, e.column);
    let before = e.song.clone();
    assert!(e.select(1, TrackId(2)).is_err());
    assert!(e.move_occurrence(1).is_err());
    assert_eq!((e.occurrence, e.track, e.row, e.column), cursor);
    assert_eq!(e.song, before);
    assert!(!e.can_undo() && !e.dirty);
}

#[test]
fn no_op_and_failed_edits_preserve_redo_and_saved_state() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 0,
        ..Editor::default()
    };
    let initial = e.song.clone();
    e.enter_note(Note::Note(72)).unwrap();
    let changed = e.song.clone();
    e.undo();
    assert!(!e.dirty && e.can_redo());
    e.set_cell(e.cell()).unwrap();
    e.resize(e.pattern().rows).unwrap();
    assert!(e.enter_note(Note::Note(128)).is_err());
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, changed);
    e.mark_saved();
    e.undo();
    assert!(e.dirty);
    e.redo();
    assert!(!e.dirty);
}

#[test]
fn exhausted_clone_id_is_atomic_but_shared_repeat_still_works() {
    let mut e = Editor::default();
    let mut song = e.song.clone();
    song.patterns
        .insert(PatternId(u64::MAX), song.patterns[&PatternId(1)].clone());
    e.replace_document(song.clone()).unwrap();
    e.edit_mode = true;
    assert!(e.append(true).unwrap_err().contains("ID space exhausted"));
    assert_eq!(e.song, song);
    assert!(!e.dirty && !e.can_undo());
    e.append(false).unwrap();
    assert_eq!(e.pattern_id(), PatternId(1));
    e.undo();
    assert_eq!(e.song, song);
    assert!(!e.dirty);
}

#[cfg(feature = "desktop")]
#[test]
fn holding_space_toggles_edit_mode_once_but_navigation_repeats() {
    use aqua_pattern_core::tracker_ui::{grid_id, keyboard};
    use eframe::egui;
    let ctx = egui::Context::default();
    let mut e = Editor::default();
    let initial = e.song.clone();
    ctx.memory_mut(|m| m.request_focus(grid_id()));
    let mut key = |key, pressed, repeat| {
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat,
                    modifiers: egui::Modifiers::NONE,
                }],
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
                keyboard(&mut e, ctx).unwrap();
            },
        );
        e.edit_mode
    };
    assert!(key(egui::Key::Space, true, false));
    assert!(key(egui::Key::Space, true, true));
    assert!(key(egui::Key::Space, true, true));
    assert!(key(egui::Key::Space, false, false));
    assert!(key(egui::Key::ArrowDown, true, false));
    assert!(key(egui::Key::ArrowDown, true, true));
    assert!(!key(egui::Key::Space, true, false));
    assert_eq!(e.row, 2);
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo());
}

fn step_effects() -> StepEffects {
    StepEffects {
        retrigger: Some(Retrigger {
            repeats: 2,
            spacing: 0.2,
        }),
        release: Some(0.75),
        glide: Some(Glide {
            target: 72,
            semitones_per_second: 12.0,
        }),
        ..Default::default()
    }
}

#[test]
fn step_effects_are_guarded_independent_and_one_undoable_transaction() {
    let mut e = Editor::default();
    let initial = e.song.clone();
    let original_cell = e.cell();
    assert!(e.set_effects(step_effects()).is_err());
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.set_effects(step_effects()).unwrap();
    let edited = e.song.clone();
    let mut without_effects = e.cell();
    without_effects.effects = StepEffects::default();
    assert_eq!(without_effects, original_cell);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(e.dirty);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, edited);
    e.mark_saved();
    e.set_effects(step_effects()).unwrap();
    assert!(!e.dirty);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(
        !e.can_undo(),
        "reapplying identical effects must not add history"
    );
    e.redo();
    assert!(!e.dirty);
    e.set_effects(StepEffects::default()).unwrap();
    assert_eq!(e.cell(), original_cell);
    assert!(e.dirty);
    e.undo();
    assert_eq!(e.song, edited);
    assert!(!e.dirty);
}

#[test]
fn invalid_effect_values_and_incompatible_field_changes_are_atomic() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 4,
        ..Editor::default()
    };
    let initial = e.song.clone();
    let mut invalid = Vec::new();
    for (repeats, spacing) in [
        (0, 0.1),
        (17, 0.01),
        (2, 0.0),
        (2, -0.1),
        (2, f64::NAN),
        (2, f64::INFINITY),
        (2, 0.5),
    ] {
        invalid.push(StepEffects {
            retrigger: Some(Retrigger { repeats, spacing }),
            ..Default::default()
        });
    }
    for release in [-0.1, 1.0, f64::NAN, f64::INFINITY] {
        invalid.push(StepEffects {
            release: Some(release),
            ..Default::default()
        });
    }
    for (target, semitones_per_second) in [
        (128, 1.0),
        (60, 0.0),
        (60, 121.0),
        (60, f32::NAN),
        (60, f32::INFINITY),
    ] {
        invalid.push(StepEffects {
            glide: Some(Glide {
                target,
                semitones_per_second,
            }),
            ..Default::default()
        });
    }
    for effects in invalid {
        assert!(e.set_effects(effects).is_err());
        assert_eq!(e.song, initial);
        assert_eq!(e.row, 0);
        assert!(!e.dirty && !e.can_undo());
    }
    e.set_effects(step_effects()).unwrap();
    e.mark_saved();
    let saved = e.song.clone();
    // Retrigger needs a pitched note, and all substeps must fit after row delay.
    assert!(e.clear(false).is_err());
    assert!(e.enter_note(Note::Off).is_err());
    assert_eq!(e.row, 0);
    e.field = Field::Delay;
    assert!(e.enter_value("0.6").is_err());
    let mut too_early = step_effects();
    too_early.release = Some(0.1);
    assert!(e.set_effects(too_early).is_err());
    assert_eq!(e.song, saved);
    assert!(!e.dirty);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.can_undo() && e.can_redo());
    e.field = Field::Note;
    e.set_effects(StepEffects::default()).unwrap();
    assert!(e.can_redo(), "no-op effects edit must retain redo");
    e.redo();
    assert_eq!(e.song, saved);
    assert!(!e.dirty);
    e.clear(true).unwrap();
    assert_eq!(e.cell(), Default::default());
    e.undo();
    assert_eq!(e.song, saved);
}

#[test]
fn effects_only_cells_survive_field_clear_and_protect_shrink() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.clear(true).unwrap();
    e.row = 31;
    e.clear(true).unwrap();
    // Remove the other tracks' last-row OFF events so only effects guard shrinking.
    for track in 2..=4 {
        e.select(0, TrackId(track)).unwrap();
        e.clear(true).unwrap();
    }
    e.select(0, TrackId(1)).unwrap();
    e.set_effects(StepEffects {
        release: Some(0.5),
        glide: Some(Glide {
            target: 72,
            semitones_per_second: 12.0,
        }),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(e.cell().note, Note::Empty);
    let effect_cell = e.cell();
    e.clear(false).unwrap();
    assert_eq!(e.cell(), effect_cell);
    assert!(e.resize(31).is_err());
    e.set_effects(StepEffects::default()).unwrap();
    assert_eq!(e.cell(), Default::default());
    assert!(!e.pattern().tracks.contains_key(&TrackId(1)));
    e.resize(31).unwrap();
    assert_eq!(e.row, 30);
    e.undo();
    e.undo();
    assert_eq!(e.row, 31);
    assert_eq!(e.cell(), effect_cell);
}

#[test]
fn repeated_effects_share_content_but_cloned_effects_are_independent() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.set_effects(step_effects()).unwrap();
    e.select(2, TrackId(1)).unwrap();
    assert_eq!(e.cell().effects, step_effects());
    e.append(true).unwrap();
    let clone = e.pattern_id();
    assert_ne!(clone, PatternId(1));
    assert_eq!(e.cell().effects, step_effects());
    e.set_effects(StepEffects::default()).unwrap();
    assert_eq!(e.cell().effects, StepEffects::default());
    e.select(0, TrackId(1)).unwrap();
    assert_eq!(e.cell().effects, step_effects());
    e.undo();
    assert_eq!(e.pattern_id(), clone);
    assert_eq!(e.cell().effects, step_effects());
    e.redo();
    assert_eq!(
        e.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0].effects,
        step_effects()
    );
    assert_eq!(
        e.song.patterns[&clone].tracks[&TrackId(1)][&0][&0].effects,
        StepEffects::default()
    );
}

#[test]
fn document_batch_is_protected_validated_and_one_undoable_transaction() {
    let mut e = Editor::default();
    e.select(1, TrackId(3)).unwrap();
    e.row = 50;
    e.column = 1;
    e.field = Field::Pan;
    let initial = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut candidate = initial.clone();
    candidate.title = "Atomic batch".into();
    candidate.bpm = 137.0;
    candidate.lines_per_beat = 8;
    candidate.tracks.get_mut(&TrackId(1)).unwrap().name = "Renamed".into();
    assert!(e.apply_document_edit(candidate.clone()).is_err());
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    let mut invalid = candidate.clone();
    invalid.patterns.get_mut(&PatternId(2)).unwrap().rows = 0;
    assert!(e.apply_document_edit(invalid).is_err());
    assert_eq!(e.song, initial);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo());
    e.apply_document_edit(candidate.clone()).unwrap();
    assert_eq!(e.song, candidate);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(e.dirty && e.edit_mode);
    e.undo();
    assert_eq!(e.song, initial);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, candidate);
    e.mark_saved();
    e.apply_document_edit(candidate).unwrap();
    assert!(!e.dirty);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.can_undo(), "no-op batch must not add an undo entry");
}

#[test]
fn document_batch_clamps_changed_geometry_and_handles_empty_documents() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.select(2, TrackId(4)).unwrap();
    e.row = 31;
    e.column = 1;
    e.field = Field::Delay;
    let initial = e.song.clone();
    let mut candidate = initial.clone();
    candidate.tracks.retain(|id, _| *id == TrackId(2));
    candidate.track_order = vec![TrackId(2)];
    candidate.tracks.get_mut(&TrackId(2)).unwrap().note_columns = 1;
    candidate.patterns.retain(|id, _| *id == PatternId(2));
    let pattern = candidate.patterns.get_mut(&PatternId(2)).unwrap();
    pattern.tracks.clear();
    pattern.rows = 8;
    candidate.order.truncate(1);
    candidate.order[0].pattern = PatternId(2);
    e.apply_document_edit(candidate.clone()).unwrap();
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(2), 0, 7)
    );
    assert_eq!(e.field, Field::Delay);
    assert!(e.has_grid());
    e.undo();
    assert_eq!(e.song, initial);
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (2, TrackId(4), 1, 31)
    );
    e.redo();
    assert_eq!(e.song, candidate);
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(2), 0, 7)
    );
    let mut empty = candidate.clone();
    empty.tracks.clear();
    empty.track_order.clear();
    empty.patterns.clear();
    empty.order.clear();
    e.apply_document_edit(empty.clone()).unwrap();
    assert!(!e.has_grid());
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(0), 0, 0)
    );
    e.undo();
    assert_eq!(e.song, candidate);
    assert!(e.has_grid());
    e.redo();
    assert_eq!(e.song, empty);
    e.apply_document_edit(initial.clone()).unwrap();
    assert_eq!(e.song, initial);
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(1), 0, 0)
    );
    assert!(e.has_grid() && !e.dirty);
}

#[test]
fn invalid_and_no_op_batches_preserve_redo_until_a_real_change() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.enter_note(Note::Note(72)).unwrap();
    e.undo();
    let initial = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut invalid = initial.clone();
    invalid.bpm = f64::NAN;
    assert!(e.apply_document_edit(invalid).is_err());
    e.apply_document_edit(initial.clone()).unwrap();
    assert_eq!(e.song, initial);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let mut changed = initial.clone();
    changed.title = "Divergent batch".into();
    e.apply_document_edit(changed).unwrap();
    assert!(e.dirty && e.can_undo() && !e.can_redo());
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.dirty);
}

#[test]
fn advanced_effect_edits_undo_clear_and_reject_incompatible_commands_atomically() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 0,
        ..Editor::default()
    };
    let initial = e.song.clone();
    let original_cell = e.cell();
    let advanced = StepEffects {
        retrigger: Some(Retrigger {
            repeats: 2,
            spacing: 0.2,
        }),
        cut: Some(0.75),
        offset: Some(42),
        direction: Some(Direction::Reverse),
        glide: Some(Glide {
            target: 72,
            semitones_per_second: 12.0,
        }),
        ..Default::default()
    };
    e.set_effects(advanced.clone()).unwrap();
    let changed = e.song.clone();
    let mut musical_fields = e.cell();
    musical_fields.effects = StepEffects::default();
    assert_eq!(musical_fields, original_cell);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo());
    e.redo();
    assert_eq!(e.song, changed);
    e.mark_saved();
    e.set_effects(StepEffects::default()).unwrap();
    assert_eq!(e.cell(), original_cell);
    e.undo();
    assert_eq!(e.song, changed);
    assert!(!e.dirty && e.can_redo());
    let mut conflicting_release = advanced.clone();
    conflicting_release.release = Some(0.9);
    let mut excessive_offset = advanced.clone();
    excessive_offset.offset = Some(u64::from(u32::MAX) + 1);
    let mut premature_cut = advanced.clone();
    premature_cut.cut = Some(0.1);
    for effects in [conflicting_release, excessive_offset, premature_cut] {
        assert!(e.set_effects(effects).is_err());
        assert_eq!(e.song, changed);
        assert!(!e.dirty && e.can_redo());
        assert_eq!(e.row, 0);
    }
    assert!(e.enter_note(Note::Off).is_err());
    assert_eq!(e.song, changed);
    assert!(!e.dirty && e.can_redo());
    e.set_effects(StepEffects::default()).unwrap();
    e.enter_note(Note::Off).unwrap();
    let off = e.song.clone();
    assert!(e.set_effects(advanced).is_err());
    assert_eq!(e.song, off);
    assert_eq!(e.row, 0);
}

fn phrase_fixture() -> Phrase {
    use std::collections::BTreeMap;
    Phrase {
        name: "Motif".into(),
        instrument: InstrumentId(1),
        rows: 4,
        lines_per_beat: 4,
        note_columns: 2,
        root_note: 60,
        cells: BTreeMap::from([
            (
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        ..Default::default()
                    },
                )]),
            ),
            (
                3,
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
fn phrase_creation_editing_and_removal_are_guarded_individual_transactions() {
    let mut e = Editor::default();
    let initial = e.song.clone();
    assert!(e.upsert_phrase(PhraseId(10), phrase_fixture()).is_err());
    assert!(e.remove_phrase(PhraseId(10)).is_err());
    assert!(e
        .set_phrase_cell(PhraseId(10), 0, 0, Cell::default())
        .is_err());
    assert!(e
        .set_phrase_trigger(Some(PhraseTrigger {
            phrase: PhraseId(10),
            cycles: 1
        }))
        .is_err());
    assert_eq!(e.song, initial);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    e.select(1, TrackId(3)).unwrap();
    e.row = 12;
    e.column = 1;
    e.field = Field::Pan;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.upsert_phrase(PhraseId(10), phrase_fixture()).unwrap();
    let created = e.song.clone();
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.can_undo() && !e.dirty);
    e.redo();
    e.mark_saved();
    e.upsert_phrase(PhraseId(10), phrase_fixture()).unwrap();
    assert!(!e.dirty);
    e.set_phrase_cell(
        PhraseId(10),
        1,
        0,
        Cell {
            note: Note::Note(64),
            ..Default::default()
        },
    )
    .unwrap();
    let edited = e.song.clone();
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, created);
    assert!(!e.dirty && e.can_redo());
    e.set_phrase_cell(PhraseId(10), 0, 0, phrase_fixture().cells[&0][&0].clone())
        .unwrap();
    assert!(e.can_redo());
    e.redo();
    assert_eq!(e.song, edited);
    e.set_phrase_cell(PhraseId(10), 1, 0, Cell::default())
        .unwrap();
    assert!(!e.song.phrases[&PhraseId(10)].cells.contains_key(&1));
    e.undo();
    assert_eq!(e.song, edited);
    e.remove_phrase(PhraseId(10)).unwrap();
    assert!(!e.song.phrases.contains_key(&PhraseId(10)));
    e.undo();
    assert_eq!(e.song, edited);
    e.undo();
    assert_eq!(e.song, created);
    e.undo();
    assert_eq!(e.song, initial);
    assert!(!e.can_undo(), "same-content upsert must not add history");
}

#[test]
fn phrase_trigger_edits_preserve_other_fields_and_references_block_deletion() {
    let mut e = Editor {
        edit_mode: true,
        edit_step: 0,
        ..Editor::default()
    };
    e.upsert_phrase(PhraseId(10), phrase_fixture()).unwrap();
    let original_cell = e.cell();
    let trigger = PhraseTrigger {
        phrase: PhraseId(10),
        cycles: 2,
    };
    e.set_phrase_trigger(Some(trigger.clone())).unwrap();
    let referenced = e.song.clone();
    let mut musical_fields = e.cell();
    musical_fields.phrase = None;
    assert_eq!(musical_fields, original_cell);
    e.mark_saved();
    e.select(2, TrackId(1)).unwrap();
    assert_eq!(
        e.cell().phrase,
        Some(trigger.clone()),
        "repeated pattern shares phrase reference"
    );
    assert!(e.remove_phrase(PhraseId(10)).is_err());
    let mut invalid_transposition = phrase_fixture();
    invalid_transposition
        .cells
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .note = Note::Note(0);
    assert!(e
        .upsert_phrase(PhraseId(10), invalid_transposition)
        .is_err());
    let mut incompatible_root = phrase_fixture();
    incompatible_root.root_note = 127;
    assert!(e.upsert_phrase(PhraseId(10), incompatible_root).is_err());
    let mut incompatible_owner = phrase_fixture();
    incompatible_owner.instrument = InstrumentId(2);
    assert!(e.upsert_phrase(PhraseId(10), incompatible_owner).is_err());
    assert!(e.remove_phrase(PhraseId(999)).is_err());
    assert!(e
        .set_phrase_trigger(Some(PhraseTrigger {
            phrase: PhraseId(999),
            cycles: 1
        }))
        .is_err());
    assert!(e
        .set_phrase_trigger(Some(PhraseTrigger {
            phrase: PhraseId(10),
            cycles: 0
        }))
        .is_err());
    assert!(e
        .set_phrase_trigger(Some(PhraseTrigger {
            phrase: PhraseId(10),
            cycles: 65
        }))
        .is_err());
    assert!(
        e.clear(false).is_err(),
        "a trigger cannot remain on an empty note"
    );
    assert!(e.enter_note(Note::Off).is_err());
    assert!(
        e.set_effects(step_effects()).is_err(),
        "trigger cannot also retrigger/glide"
    );
    assert_eq!(e.song, referenced);
    assert!(!e.dirty);
    e.set_phrase_trigger(None).unwrap();
    assert_eq!(e.cell(), original_cell);
    e.remove_phrase(PhraseId(10)).unwrap();
    e.undo();
    e.undo();
    assert_eq!(e.song, referenced);
    assert!(!e.dirty);
    // References in unused patterns are just as persistent as ordered ones.
    let mut unused = e.song.clone();
    unused.order.retain(|o| o.pattern != PatternId(1));
    e.apply_document_edit(unused.clone()).unwrap();
    assert!(e.remove_phrase(PhraseId(10)).is_err());
    assert_eq!(e.song, unused);
}

#[test]
fn phrase_validation_and_shrink_failures_preserve_content_history_and_cursor() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.upsert_phrase(PhraseId(10), phrase_fixture()).unwrap();
    e.mark_saved();
    e.set_phrase_cell(
        PhraseId(10),
        1,
        0,
        Cell {
            note: Note::Note(62),
            ..Default::default()
        },
    )
    .unwrap();
    e.undo();
    let saved = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut invalid = Vec::new();
    for (rows, columns, lpb, root) in [
        (0, 2, 4, 60),
        (1025, 2, 4, 60),
        (3, 2, 4, 60),
        (4, 1, 4, 60),
        (4, 0, 4, 60),
        (4, 17, 4, 60),
        (4, 2, 0, 60),
        (4, 2, 257, 60),
        (4, 2, 4, 128),
    ] {
        let mut phrase = phrase_fixture();
        phrase.rows = rows;
        phrase.note_columns = columns;
        phrase.lines_per_beat = lpb;
        phrase.root_note = root;
        invalid.push(phrase);
    }
    let mut wrong_owner = phrase_fixture();
    wrong_owner.instrument = InstrumentId(999);
    invalid.push(wrong_owner);
    for phrase in invalid {
        assert!(e.upsert_phrase(PhraseId(10), phrase).is_err());
    }
    assert!(e.upsert_phrase(PhraseId(0), phrase_fixture()).is_err());
    assert!(e
        .set_phrase_cell(PhraseId(999), 0, 0, Cell::default())
        .is_err());
    assert!(e
        .set_phrase_cell(PhraseId(10), 4, 0, Cell::default())
        .is_err());
    assert!(e
        .set_phrase_cell(PhraseId(10), 0, 2, Cell::default())
        .is_err());
    assert!(e
        .set_phrase_cell(
            PhraseId(10),
            0,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                ..Default::default()
            }
        )
        .is_err());
    assert!(e
        .set_phrase_cell(
            PhraseId(10),
            0,
            0,
            Cell {
                note: Note::Note(60),
                phrase: Some(PhraseTrigger {
                    phrase: PhraseId(10),
                    cycles: 1
                }),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(e.song, saved);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(
        e.song.phrases[&PhraseId(10)].cells[&1][&0].note,
        Note::Note(62)
    );
}

#[test]
fn copied_phrase_ids_are_independent_and_explicit_reference_rewrites_are_atomic() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.upsert_phrase(PhraseId(10), phrase_fixture()).unwrap();
    e.set_phrase_trigger(Some(PhraseTrigger {
        phrase: PhraseId(10),
        cycles: 1,
    }))
    .unwrap();
    e.append(true).unwrap();
    assert_eq!(
        e.cell().phrase.as_ref().unwrap().phrase,
        PhraseId(10),
        "pattern clone preserves the stable phrase reference"
    );
    let copied = e.song.phrases[&PhraseId(10)].clone();
    e.upsert_phrase(PhraseId(11), copied).unwrap();
    e.set_phrase_trigger(Some(PhraseTrigger {
        phrase: PhraseId(11),
        cycles: 1,
    }))
    .unwrap();
    e.set_phrase_cell(
        PhraseId(11),
        0,
        0,
        Cell {
            note: Note::Note(72),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        e.song.phrases[&PhraseId(10)].cells[&0][&0].note,
        Note::Note(60)
    );
    assert_eq!(
        e.song.phrases[&PhraseId(11)].cells[&0][&0].note,
        Note::Note(72)
    );
    let before = e.song.clone();
    let mut replacement = before.clone();
    for pattern in replacement.patterns.values_mut() {
        for track in pattern.tracks.values_mut() {
            for row in track.values_mut() {
                for cell in row.values_mut() {
                    if let Some(trigger) = &mut cell.phrase {
                        if trigger.phrase == PhraseId(10) {
                            trigger.phrase = PhraseId(11);
                        }
                    }
                }
            }
        }
    }
    replacement.phrases.remove(&PhraseId(10));
    e.apply_document_edit(replacement.clone()).unwrap();
    assert_eq!(e.song, replacement);
    e.undo();
    assert_eq!(e.song, before);
    e.redo();
    assert_eq!(e.song, replacement);
}

fn source_fixture() -> aqua_pattern_core::song::SampleAsset {
    aqua_pattern_core::song::SampleAsset {
        path: std::path::PathBuf::from("/imported/source.wav"),
        sha256: "a".repeat(64),
        sample_rate: 48_000,
        channels: 2,
        frames: 480,
    }
}
fn region_fixture() -> aqua_pattern_core::song::SampleRegion {
    aqua_pattern_core::song::SampleRegion {
        name: "Kick slice".into(),
        amp_envelope: None,
        forward_loop: None,
        sample: aqua_pattern_core::song::SampleId(11),
        start_frame: 12,
        end_frame: 240,
        root_note: 60,
    }
}

#[test]
fn source_and_region_operations_are_protected_undoable_and_noops_keep_redo() {
    use aqua_pattern_core::song::{RegionId, SampleId};
    let mut e = Editor::default();
    let before = e.song.clone();
    assert!(e.register_sample(SampleId(11), source_fixture()).is_err());
    assert!(e.remove_sample(SampleId(11)).is_err());
    assert!(e.upsert_region(RegionId(21), region_fixture()).is_err());
    assert!(e.remove_region(RegionId(21)).is_err());
    assert!(e.assign_region(InstrumentId(1), None).is_err());
    assert_eq!(e.song, before);
    assert!(!e.can_undo() && !e.dirty);
    e.edit_mode = true;
    e.row = 3;
    e.field = Field::Pan;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.register_sample(SampleId(11), source_fixture()).unwrap();
    let registered = e.song.clone();
    e.mark_saved();
    e.upsert_region(RegionId(21), region_fixture()).unwrap();
    let defined = e.song.clone();
    e.undo();
    assert_eq!(e.song, registered);
    assert!(!e.dirty && e.can_redo());
    e.register_sample(SampleId(11), source_fixture()).unwrap();
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(e.song, defined);
    e.assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    e.undo();
    assert_eq!(e.song, defined);
    e.upsert_region(RegionId(21), region_fixture()).unwrap();
    assert!(e.can_redo());
    e.redo();
    assert_eq!(
        e.song.instruments[&InstrumentId(1)].region,
        Some(RegionId(21))
    );
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
}

#[test]
fn invalid_source_region_edits_and_referenced_removal_preserve_saved_history() {
    use aqua_pattern_core::song::{RegionId, SampleId};
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.register_sample(SampleId(11), source_fixture()).unwrap();
    e.upsert_region(RegionId(21), region_fixture()).unwrap();
    e.assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    e.mark_saved();
    e.assign_region(InstrumentId(2), Some(RegionId(21)))
        .unwrap();
    e.undo();
    let saved = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    assert!(e.remove_sample(SampleId(11)).is_err());
    assert!(e.remove_sample(SampleId(999)).is_err());
    assert!(e.remove_region(RegionId(21)).is_err());
    assert!(e.remove_region(RegionId(999)).is_err());
    assert!(e
        .assign_region(InstrumentId(1), Some(RegionId(999)))
        .is_err());
    assert!(e.assign_region(InstrumentId(999), None).is_err());
    let mut changed_metadata = source_fixture();
    changed_metadata.frames += 1;
    assert!(
        e.register_sample(SampleId(11), changed_metadata).is_err(),
        "immutable identity"
    );
    let mut changed_path = source_fixture();
    changed_path.path = "/other/source.wav".into();
    assert!(e.register_sample(SampleId(11), changed_path).is_err());
    for (start, end, root, source) in [
        (240, 240, 60, 11),
        (241, 240, 60, 11),
        (0, 481, 60, 11),
        (0, 100, 128, 11),
        (0, 100, 60, 999),
    ] {
        let mut region = region_fixture();
        region.start_frame = start;
        region.end_frame = end;
        region.root_note = root;
        region.sample = SampleId(source);
        assert!(e.upsert_region(RegionId(21), region).is_err());
    }
    assert!(e.upsert_region(RegionId(0), region_fixture()).is_err());
    for mutate in [
        |s: &mut aqua_pattern_core::song::SampleAsset| s.sha256 = "A".repeat(64),
        |s: &mut aqua_pattern_core::song::SampleAsset| s.sample_rate = 0,
        |s: &mut aqua_pattern_core::song::SampleAsset| s.channels = 3,
        |s: &mut aqua_pattern_core::song::SampleAsset| s.frames = 0,
        |s: &mut aqua_pattern_core::song::SampleAsset| s.path.clear(),
    ] {
        let mut sample = source_fixture();
        mutate(&mut sample);
        assert!(e.register_sample(SampleId(12), sample).is_err());
    }
    assert_eq!(e.song, saved);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(
        e.song.instruments[&InstrumentId(2)].region,
        Some(RegionId(21))
    );
}

#[test]
fn region_copy_is_independent_and_source_region_mapping_batch_is_one_undo() {
    use aqua_pattern_core::song::{RegionId, SampleId};
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.upsert_phrase(PhraseId(17), phrase_fixture()).unwrap();
    e.set_phrase_trigger(Some(PhraseTrigger {
        phrase: PhraseId(17),
        cycles: 1,
    }))
    .unwrap();
    e.mark_saved();
    let before = e.song.clone();
    let mut batch = before.clone();
    batch
        .register_sample(SampleId(11), source_fixture())
        .unwrap();
    batch.upsert_region(RegionId(21), region_fixture()).unwrap();
    batch.upsert_region(RegionId(22), region_fixture()).unwrap();
    batch
        .assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    batch
        .assign_region(InstrumentId(2), Some(RegionId(21)))
        .unwrap();
    e.apply_document_edit(batch.clone()).unwrap();
    e.undo();
    assert_eq!(e.song, before);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(e.song, batch);
    let mut edited = region_fixture();
    edited.start_frame = 120;
    e.upsert_region(RegionId(21), edited.clone()).unwrap();
    assert_eq!(e.song.regions[&RegionId(22)], region_fixture());
    assert_eq!(e.song.regions[&RegionId(21)], edited);
    assert_eq!(e.song.phrases, before.phrases);
    assert_eq!(e.song.patterns, before.patterns);
    let before_delete = e.song.clone();
    let mut deletion = before_delete.clone();
    deletion.assign_region(InstrumentId(1), None).unwrap();
    deletion.assign_region(InstrumentId(2), None).unwrap();
    deletion.remove_region(RegionId(21)).unwrap();
    assert!(
        deletion.remove_sample(SampleId(11)).is_err(),
        "unmapped regions still reference source"
    );
    deletion.remove_region(RegionId(22)).unwrap();
    deletion.remove_sample(SampleId(11)).unwrap();
    e.apply_document_edit(deletion.clone()).unwrap();
    e.undo();
    assert_eq!(e.song, before_delete);
    e.redo();
    assert_eq!(e.song, deletion);
}

#[test]
fn document_batches_cannot_rewrite_registered_source_identity_and_fail_atomically() {
    use aqua_pattern_core::song::SampleId;
    let mut editor = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    editor
        .register_sample(SampleId(11), source_fixture())
        .unwrap();
    editor.mark_saved();
    editor.enter_note(Note::Note(71)).unwrap();
    editor.undo();
    let saved = editor.song.clone();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    for mutate in [
        |s: &mut aqua_pattern_core::song::SampleAsset| s.sha256 = "b".repeat(64),
        |s: &mut aqua_pattern_core::song::SampleAsset| s.sample_rate += 1,
        |s: &mut aqua_pattern_core::song::SampleAsset| s.channels = 1,
        |s: &mut aqua_pattern_core::song::SampleAsset| s.frames += 1,
        |s: &mut aqua_pattern_core::song::SampleAsset| s.path = "/relinked/source.wav".into(),
    ] {
        let mut candidate = saved.clone();
        let mut source = candidate.samples.remove(&SampleId(11)).unwrap();
        mutate(&mut source);
        candidate.register_sample(SampleId(11), source).unwrap();
        candidate.bpm += 1.0;
        assert!(editor
            .apply_document_edit(candidate)
            .unwrap_err()
            .contains("immutable"));
        assert_eq!(editor.song, saved);
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
        assert!(!editor.dirty && editor.can_redo());
    }
    let mut replacement = saved.clone();
    replacement.samples.get_mut(&SampleId(11)).unwrap().frames += 1;
    editor.replace_document(replacement.clone()).unwrap();
    assert_eq!(
        editor.song, replacement,
        "opening a different project has independent IDs"
    );
    assert!(!editor.dirty && !editor.can_undo() && !editor.can_redo());
}

#[test]
fn real_song_timing_is_atomic_undoable_and_preserves_content_and_cursor() {
    let mut e = Editor::default();
    let before = e.song.clone();
    assert!(e.set_timing(175.0, 8).is_err());
    assert_eq!(e.revision(), 0);
    e.edit_mode = true;
    e.row = 7;
    e.field = Field::Delay;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for (bpm, lpb) in [
        (f64::NAN, 4),
        (f64::INFINITY, 4),
        (19.9, 4),
        (400.1, 4),
        (120.0, 0),
        (120.0, 257),
    ] {
        assert!(e.set_timing(bpm, lpb).is_err());
        assert_eq!(e.song, before);
        assert_eq!(e.revision(), 0);
        assert!(!e.dirty && !e.can_undo());
    }
    e.set_timing(175.0, 8).unwrap();
    let mut timed = before.clone();
    timed.bpm = 175.0;
    timed.lines_per_beat = 8;
    assert_eq!(e.song, timed);
    assert_eq!(e.revision(), 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, before);
    assert_eq!(e.revision(), 2);
    assert!(!e.dirty && e.can_redo());
    e.set_timing(before.bpm, before.lines_per_beat).unwrap();
    assert_eq!(e.revision(), 2);
    assert!(e.can_redo());
    e.redo();
    assert_eq!(e.song, timed);
    assert_eq!(e.revision(), 3);
    e.mark_saved();
    e.move_rows(1);
    assert_eq!(e.revision(), 3);
    assert!(!e.dirty);
}

#[test]
fn preview_commits_require_original_revision_and_content_and_are_one_transaction() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let revision = e.revision();
    let base = e.song.clone();
    let mut candidate = base.clone();
    candidate.bpm = 180.0;
    candidate.lines_per_beat = 8;
    candidate.title = "Compiler candidate".into();
    assert_eq!(
        e.song, base,
        "preparing a candidate does not mutate the editor"
    );
    let mut invalid = candidate.clone();
    invalid.lines_per_beat = 0;
    assert!(e
        .apply_document_edit_if_current(revision, &base, invalid)
        .is_err());
    assert_eq!(e.revision(), revision);
    assert_eq!(e.song, base);
    e.edit_mode = false;
    assert!(e
        .apply_document_edit_if_current(revision, &base, candidate.clone())
        .is_err());
    e.edit_mode = true;
    e.apply_document_edit_if_current(revision, &base, candidate.clone())
        .unwrap();
    assert_eq!(e.song, candidate);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && e.can_redo());
    assert!(e
        .apply_document_edit_if_current(revision, &base, candidate.clone())
        .unwrap_err()
        .contains("stale"));
    assert_eq!(e.song, base);
    assert!(e.can_redo());
    let same_content_revision = e.revision();
    e.apply_document_edit_if_current(same_content_revision, &base, base.clone())
        .unwrap();
    assert_eq!(e.revision(), same_content_revision);
    assert!(
        e.can_redo(),
        "a current no-op preview preserves the redo branch"
    );
    let mut invalid_document = base.clone();
    invalid_document.lines_per_beat = 0;
    assert!(e.replace_document(invalid_document).is_err());
    assert_eq!(e.revision(), same_content_revision);
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    assert!(
        e.apply_document_edit_if_current(same_content_revision, &base, candidate.clone())
            .is_err(),
        "New/Open invalidates even identical content"
    );
    let current_revision = e.revision();
    e.song.title = "Unchecked external edit".into();
    let unchecked = e.song.clone();
    assert!(e
        .apply_document_edit_if_current(current_revision, &base, candidate)
        .is_err());
    assert_eq!(
        e.song, unchecked,
        "content guard catches direct public draft mutation"
    );
    assert!(!e.can_undo());
}

fn marker_fixture() -> aqua_pattern_core::song::MarkerSet {
    aqua_pattern_core::song::MarkerSet {
        name: "Accepted manual boundaries".into(),
        locked_frames: Vec::new(),
        sample: aqua_pattern_core::song::SampleId(11),
        source_bpm: None,
        markers: vec![0, 120, 480],
    }
}

#[test]
fn manual_markers_are_guarded_atomic_and_preserve_source_and_analysis_independence() {
    use aqua_pattern_core::song::{MarkerSetId, SampleId};
    let mut e = Editor::default();
    assert!(e
        .upsert_marker_set(MarkerSetId(31), marker_fixture())
        .is_err());
    assert!(e.remove_marker_set(MarkerSetId(31)).is_err());
    e.edit_mode = true;
    assert!(e
        .upsert_marker_set(MarkerSetId(31), marker_fixture())
        .is_err());
    e.register_sample(SampleId(11), source_fixture()).unwrap();
    e.mark_saved();
    let source_only = e.song.clone();
    e.upsert_marker_set(MarkerSetId(31), marker_fixture())
        .unwrap();
    let accepted = e.song.clone();
    assert!(accepted.regions.is_empty());
    assert!(
        e.remove_sample(SampleId(11)).is_err(),
        "manual markers reference the source without any regions"
    );
    e.undo();
    assert_eq!(e.song, source_only);
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(e.song, accepted);
    e.upsert_marker_set(MarkerSetId(32), marker_fixture())
        .unwrap();
    let copied = e.song.clone();
    let mut edited = marker_fixture();
    edited.markers[1] = 128;
    edited.source_bpm = Some(120.0);
    e.upsert_marker_set(MarkerSetId(31), edited.clone())
        .unwrap();
    assert_eq!(e.song.marker_sets[&MarkerSetId(32)], marker_fixture());
    let saved = e.song.clone();
    let revision = e.revision();
    let analysis = aqua_pattern_core::break_analysis::analyze_transients(
        &vec![0.0; 960],
        2,
        48_000,
        &Default::default(),
    )
    .unwrap();
    assert!(analysis.suggestions.is_empty());
    assert_eq!(
        e.song, saved,
        "re-running analysis cannot erase saved manual boundaries"
    );
    assert_eq!(e.revision(), revision);
    e.undo();
    assert_eq!(e.song, copied);
    e.upsert_marker_set(MarkerSetId(31), marker_fixture())
        .unwrap();
    assert!(e.can_redo(), "no-op marker save preserves redo");
    e.redo();
    assert_eq!(e.song.marker_sets[&MarkerSetId(31)], edited);
    e.remove_marker_set(MarkerSetId(31)).unwrap();
    assert!(!e.song.marker_sets.contains_key(&MarkerSetId(31)));
    e.undo();
    assert_eq!(e.song, saved);
}

#[test]
fn invalid_manual_marker_boundaries_tempo_and_caps_preserve_redo_revision_and_cursor() {
    use aqua_pattern_core::song::{MarkerSetId, SampleId};
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.register_sample(SampleId(11), source_fixture()).unwrap();
    e.upsert_marker_set(MarkerSetId(31), marker_fixture())
        .unwrap();
    e.mark_saved();
    e.set_timing(170.0, 8).unwrap();
    e.undo();
    let saved = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for markers in [
        vec![],
        vec![0],
        vec![1, 480],
        vec![0, 479],
        vec![0, 480, 481],
        vec![0, 120, 120, 480],
        vec![0, 240, 120, 480],
        (0..=256).chain([480]).collect(),
    ] {
        let mut set = marker_fixture();
        set.markers = markers;
        assert!(e.upsert_marker_set(MarkerSetId(31), set).is_err());
    }
    for bpm in [f64::NAN, f64::INFINITY, 19.9, 400.1] {
        let mut set = marker_fixture();
        set.source_bpm = Some(bpm);
        assert!(e.upsert_marker_set(MarkerSetId(31), set).is_err());
    }
    let mut missing = marker_fixture();
    missing.sample = SampleId(999);
    assert!(e.upsert_marker_set(MarkerSetId(31), missing).is_err());
    assert!(e
        .upsert_marker_set(MarkerSetId(0), marker_fixture())
        .is_err());
    assert!(e.remove_marker_set(MarkerSetId(999)).is_err());
    assert_eq!(e.song, saved);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    let mut maximal = marker_fixture();
    maximal.markers = (0..=255).chain([480]).collect();
    e.upsert_marker_set(MarkerSetId(31), maximal.clone())
        .unwrap();
    assert_eq!(e.song.marker_sets[&MarkerSetId(31)].markers.len(), 257);
    e.undo();
    assert_eq!(e.song, saved);
    assert!(!e.dirty);
    let mut maximum_sets = saved.clone();
    for id in 100..227 {
        maximum_sets
            .marker_sets
            .insert(MarkerSetId(id), marker_fixture());
    }
    e.apply_document_edit(maximum_sets.clone()).unwrap();
    assert_eq!(e.song.marker_sets.len(), 128);
    let at_capacity = e.revision();
    assert!(e
        .upsert_marker_set(MarkerSetId(999), marker_fixture())
        .is_err());
    assert_eq!(e.song, maximum_sets);
    assert_eq!(e.revision(), at_capacity);
    e.undo();
    assert_eq!(e.song, saved);
}

fn compiler_editor() -> Editor {
    use aqua_pattern_core::song::{MarkerSetId, SampleId};
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    let mut source = source_fixture();
    source.sample_rate = 8_000;
    source.channels = 1;
    source.frames = 8_000;
    e.register_sample(SampleId(11), source).unwrap();
    let mut set = marker_fixture();
    set.markers = vec![0, 2_000, 4_000, 8_000];
    set.source_bpm = Some(120.0);
    e.upsert_marker_set(MarkerSetId(31), set).unwrap();
    e.mark_saved();
    e
}
fn compiler_config() -> aqua_pattern_core::break_compiler::CompileConfig {
    use aqua_pattern_core::break_compiler::{CompileConfig, CompileTiming, TailPolicy};
    CompileConfig {
        marker_set: aqua_pattern_core::song::MarkerSetId(31),
        target_bpm: 180.0,
        timing: CompileTiming::SourceRhythm,
        cycles: 1,
        roll: None,
        tail: TailPolicy::Overlap,
        minimum_slice_frames: 1,
        root_note: 60,
        pad_tails: false,
        name: "Accepted compiler output".into(),
    }
}

#[test]
fn compile_preview_is_nonmutating_and_all_generated_ids_tempo_and_order_undo_together() {
    use aqua_pattern_core::break_compiler;
    let mut e = compiler_editor();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let preview = break_compiler::preview(&base, &compiler_config()).unwrap();
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty);
    let candidate = preview.candidate().clone();
    assert_eq!(candidate.bpm, 180.0);
    assert_eq!(candidate.lines_per_beat, base.lines_per_beat);
    assert_eq!(candidate.marker_sets, base.marker_sets);
    assert_eq!(candidate.samples, base.samples);
    assert_eq!(candidate.regions.len(), base.regions.len() + 3);
    assert_eq!(candidate.instruments.len(), base.instruments.len() + 3);
    assert_eq!(candidate.tracks.len(), base.tracks.len() + 1);
    assert_eq!(candidate.patterns.len(), base.patterns.len() + 1);
    assert_eq!(candidate.order.len(), base.order.len() + 1);
    assert_eq!(&candidate.order[..base.order.len()], base.order);
    for (id, region) in &base.regions {
        assert_eq!(candidate.regions.get(id), Some(region));
    }
    for (id, pattern) in &base.patterns {
        assert_eq!(candidate.patterns.get(id), Some(pattern));
    }
    e.edit_mode = false;
    assert!(e.commit_preview(revision, &preview).is_err());
    assert_eq!(e.revision(), revision);
    e.edit_mode = true;
    e.commit_preview(revision, &preview).unwrap();
    assert_eq!(e.song, candidate);
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && e.can_redo());
    assert!(
        e.commit_preview(revision, &preview).is_err(),
        "undoing to the same base does not revive an old preview"
    );
    assert!(e.can_redo());
    e.redo();
    assert_eq!(e.song, candidate);
    let committed = e.song.clone();
    let current_revision = e.revision();
    assert!(
        e.commit_preview(current_revision, &preview).is_err(),
        "changing the token cannot bypass private preview base equality"
    );
    assert_eq!(e.song, committed);
    assert_eq!(e.revision(), current_revision);
}

#[test]
fn invalid_compile_requests_and_replaced_document_leave_no_allocated_ids_or_history() {
    use aqua_pattern_core::{break_compiler, song::MarkerSetId};
    let mut e = compiler_editor();
    let base = e.song.clone();
    let revision = e.revision();
    for mutate in [
        |c: &mut break_compiler::CompileConfig| c.marker_set = MarkerSetId(999),
        |c: &mut break_compiler::CompileConfig| c.target_bpm = f64::NAN,
        |c: &mut break_compiler::CompileConfig| c.cycles = 0,
        |c: &mut break_compiler::CompileConfig| c.minimum_slice_frames = 4_001,
        |c: &mut break_compiler::CompileConfig| c.root_note = 128,
    ] {
        let mut config = compiler_config();
        mutate(&mut config);
        assert!(break_compiler::preview(&e.song, &config).is_err());
        assert_eq!(e.song, base);
        assert_eq!(e.revision(), revision);
        assert!(!e.dirty);
    }
    for exhausted in 0..4 {
        use aqua_pattern_core::song::{RegionId, SampleId, SampleRegion};
        let mut no_ids = base.clone();
        match exhausted {
            0 => {
                no_ids.regions.insert(
                    RegionId(u64::MAX),
                    SampleRegion {
                        name: "Last region ID".into(),
                        amp_envelope: None,
                        forward_loop: None,
                        sample: SampleId(11),
                        start_frame: 0,
                        end_frame: 8_000,
                        root_note: 60,
                    },
                );
            }
            1 => {
                no_ids.instruments.insert(
                    InstrumentId(u64::MAX),
                    no_ids.instruments[&InstrumentId(1)].clone(),
                );
            }
            2 => {
                no_ids
                    .patterns
                    .insert(PatternId(u64::MAX), no_ids.patterns[&PatternId(1)].clone());
            }
            _ => {
                no_ids
                    .tracks
                    .insert(TrackId(u64::MAX), no_ids.tracks[&TrackId(1)].clone());
                no_ids.track_order.push(TrackId(u64::MAX));
            }
        }
        no_ids.validate().unwrap();
        assert!(break_compiler::preview(&no_ids, &compiler_config())
            .unwrap_err()
            .contains("fresh IDs"));
        assert_eq!(e.song, base);
        assert_eq!(e.revision(), revision);
    }
    let mut unknown_tempo = base.clone();
    unknown_tempo
        .marker_sets
        .get_mut(&MarkerSetId(31))
        .unwrap()
        .source_bpm = None;
    assert!(
        break_compiler::preview(&unknown_tempo, &compiler_config()).is_err(),
        "source tempo must not be inferred from target tempo"
    );
    let preview = break_compiler::preview(&base, &compiler_config()).unwrap();
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    assert!(e.commit_preview(revision, &preview).is_err());
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo());
}

#[test]
fn removing_order_occurrence_preserves_shared_patterns_and_undo_restores_cursor() {
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.select(1, TrackId(2)).unwrap();
    e.row = 63;
    e.column = 1;
    e.field = Field::Pan;
    e.mark_saved();
    let before = e.song.clone();
    let before_cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let revision = e.revision();
    e.remove_occurrence().unwrap();
    assert_eq!(
        e.song.order.iter().map(|o| o.pattern).collect::<Vec<_>>(),
        vec![PatternId(1), PatternId(1)]
    );
    assert_eq!(
        e.song.patterns, before.patterns,
        "unused B and repeated A data remain in the pool"
    );
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row, e.field),
        (1, TrackId(2), 1, 31, Field::Pan)
    );
    assert_eq!(e.revision(), revision + 1);
    assert!(e.dirty);
    let removed = e.song.clone();
    e.undo();
    assert_eq!(e.song, before);
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row, e.field),
        before_cursor
    );
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(e.song, removed);
    e.remove_occurrence().unwrap();
    assert_eq!(e.song.order.len(), 1);
    assert_eq!(e.occurrence, 0);
    assert_eq!(e.song.patterns, before.patterns);
    e.song.validate().unwrap();
    e.undo();
    assert_eq!(e.song, removed);
    assert_eq!(e.occurrence, 1);
}

#[test]
fn occurrence_removal_guards_final_empty_stale_and_protected_states_atomically() {
    let mut e = Editor::default();
    let before = e.song.clone();
    assert!(e.remove_occurrence().is_err());
    assert_eq!(e.song, before);
    assert_eq!(e.revision(), 0);
    e.edit_mode = true;
    e.remove_occurrence().unwrap();
    assert_eq!(e.occurrence, 0);
    assert_eq!(
        e.pattern_id(),
        PatternId(2),
        "the next occurrence takes the removed index"
    );
    assert_eq!(e.song.patterns, before.patterns);
    e.remove_occurrence().unwrap();
    e.mark_saved();
    e.set_timing(175.0, 8).unwrap();
    e.undo();
    let final_song = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    assert!(e
        .remove_occurrence()
        .unwrap_err()
        .contains("final occurrence"));
    assert_eq!(e.song, final_song);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    e.occurrence = usize::MAX;
    assert!(e.remove_occurrence().is_err());
    assert_eq!(e.song, final_song);
    assert_eq!(e.revision(), revision);
    assert!(e.can_redo());
    let mut empty = final_song;
    empty.order.clear();
    e.replace_document(empty.clone()).unwrap();
    e.edit_mode = true;
    let revision = e.revision();
    assert!(e.remove_occurrence().is_err());
    assert_eq!(e.song, empty);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo());
}

#[test]
fn persistent_marker_locks_are_protected_undoable_and_failed_edits_preserve_redo() {
    use aqua_pattern_core::song::MarkerSetId;
    let mut e = compiler_editor();
    e.edit_mode = false;
    assert!(e.set_marker_lock(MarkerSetId(31), 2_000, true).is_err());
    assert!(e.add_marker(MarkerSetId(31), 1_000).is_err());
    assert!(e.move_marker(MarkerSetId(31), 2_000, 2_100).is_err());
    assert!(e.remove_marker(MarkerSetId(31), 2_000).is_err());
    e.edit_mode = true;
    let unlocked = e.song.clone();
    e.set_marker_lock(MarkerSetId(31), 2_000, true).unwrap();
    let locked = e.song.clone();
    assert_eq!(
        locked.marker_sets[&MarkerSetId(31)].locked_frames,
        vec![2_000]
    );
    e.undo();
    assert_eq!(e.song, unlocked);
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(e.song, locked);
    e.mark_saved();
    e.add_marker(MarkerSetId(31), 1_000).unwrap();
    e.undo();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.set_marker_lock(MarkerSetId(31), 2_000, true).unwrap();
    assert_eq!(e.revision(), revision, "relocking is a no-op");
    assert!(e.can_redo());
    assert!(e.move_marker(MarkerSetId(31), 2_000, 2_100).is_err());
    assert!(e.remove_marker(MarkerSetId(31), 2_000).is_err());
    assert!(e.remove_marker_set(MarkerSetId(31)).is_err());
    for endpoint in [0, 8_000] {
        assert!(e.set_marker_lock(MarkerSetId(31), endpoint, true).is_err());
        assert!(e.set_marker_lock(MarkerSetId(31), endpoint, false).is_err());
        assert!(e.move_marker(MarkerSetId(31), endpoint, 1_000).is_err());
        assert!(e.remove_marker(MarkerSetId(31), endpoint).is_err());
    }
    assert!(e.set_marker_lock(MarkerSetId(31), 999, true).is_err());
    assert!(e.set_marker_lock(MarkerSetId(999), 2_000, true).is_err());
    assert!(e.add_marker(MarkerSetId(31), 2_000).is_err());
    assert!(e.add_marker(MarkerSetId(31), 8_001).is_err());
    assert!(e.move_marker(MarkerSetId(31), 4_000, 8_001).is_err());
    assert_eq!(e.song, locked);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert!(e.song.marker_sets[&MarkerSetId(31)]
        .markers
        .contains(&1_000));
    assert_eq!(
        e.song.marker_sets[&MarkerSetId(31)].locked_frames,
        vec![2_000]
    );
}

#[test]
fn marker_upsert_cleanup_and_source_rebinding_cannot_silently_drop_saved_locks() {
    use aqua_pattern_core::{
        break_compiler,
        song::{MarkerSetId, SampleId},
    };
    let mut e = compiler_editor();
    let before_preview = break_compiler::preview(&e.song, &compiler_config()).unwrap();
    e.set_marker_lock(MarkerSetId(31), 2_000, true).unwrap();
    assert!(e.commit_preview(e.revision(), &before_preview).is_err(),
        "new protection metadata invalidates an older compiler preview even with a fresh revision token");
    let second_source = e.song.samples[&SampleId(11)].clone();
    e.register_sample(SampleId(12), second_source).unwrap();
    e.mark_saved();
    let saved = e.song.clone();
    let revision = e.revision();
    let mut dropped = saved.marker_sets[&MarkerSetId(31)].clone();
    dropped.locked_frames.clear();
    assert!(e.upsert_marker_set(MarkerSetId(31), dropped).is_err());
    let mut rebound = saved.marker_sets[&MarkerSetId(31)].clone();
    rebound.sample = SampleId(12);
    assert!(e.upsert_marker_set(MarkerSetId(31), rebound).is_err());
    let cleanup = break_compiler::suggest_marker_cleanup(
        &saved.samples[&SampleId(11)],
        &[0, 2_000, 4_000, 8_000],
        3_000,
    )
    .unwrap();
    assert!(!cleanup.markers.contains(&2_000));
    let mut replacement = saved.marker_sets[&MarkerSetId(31)].clone();
    replacement.markers = cleanup.markers;
    assert!(e.upsert_marker_set(MarkerSetId(31), replacement).is_err());
    for invalid in [
        vec![0],
        vec![8_000],
        vec![1_999],
        vec![2_000, 2_000],
        vec![4_000, 2_000],
    ] {
        let mut set = saved.marker_sets[&MarkerSetId(31)].clone();
        set.locked_frames = invalid;
        assert!(e.upsert_marker_set(MarkerSetId(99), set).is_err());
    }
    assert_eq!(e.song, saved);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty);
    let locked_preview = break_compiler::preview(&e.song, &compiler_config()).unwrap();
    assert_eq!(locked_preview.candidate().marker_sets, saved.marker_sets);
    let mut candidate = locked_preview.candidate().clone();
    candidate.samples.remove(&SampleId(12));
    candidate
        .marker_sets
        .get_mut(&MarkerSetId(31))
        .unwrap()
        .locked_frames
        .clear();
    assert_eq!(
        &candidate,
        before_preview.candidate(),
        "locks change protection metadata, not compiler output"
    );
}

#[test]
fn explicit_unlock_move_relock_batch_is_one_transaction_and_undo_restores_frame_identity() {
    use aqua_pattern_core::song::MarkerSetId;
    let mut e = compiler_editor();
    e.set_marker_lock(MarkerSetId(31), 2_000, true).unwrap();
    e.mark_saved();
    let original = e.song.clone();
    let mut rejected = original.clone();
    rejected
        .set_marker_lock(MarkerSetId(31), 2_000, false)
        .unwrap();
    let before_failed_move = rejected.clone();
    assert!(rejected.move_marker(MarkerSetId(31), 2_000, 4_000).is_err());
    assert_eq!(
        rejected, before_failed_move,
        "a failed shared operation is atomic"
    );
    assert_eq!(
        e.song, original,
        "an uncommitted batch has no editor side effects"
    );
    let mut candidate = original.clone();
    candidate
        .set_marker_lock(MarkerSetId(31), 2_000, false)
        .unwrap();
    candidate
        .move_marker(MarkerSetId(31), 2_000, 2_100)
        .unwrap();
    candidate
        .set_marker_lock(MarkerSetId(31), 2_100, true)
        .unwrap();
    let revision = e.revision();
    e.apply_document_edit(candidate.clone()).unwrap();
    assert_eq!(e.song, candidate);
    assert_eq!(e.revision(), revision + 1);
    assert_eq!(
        e.song.marker_sets[&MarkerSetId(31)].locked_frames,
        vec![2_100]
    );
    e.undo();
    assert_eq!(e.song, original);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(e.song, candidate);
    e.set_marker_lock(MarkerSetId(31), 2_100, false).unwrap();
    e.move_marker(MarkerSetId(31), 2_100, 2_200).unwrap();
    let revision = e.revision();
    e.move_marker(MarkerSetId(31), 2_200, 2_200).unwrap();
    assert_eq!(e.revision(), revision);
    e.remove_marker(MarkerSetId(31), 2_200).unwrap();
    assert_eq!(
        e.song.marker_sets[&MarkerSetId(31)].markers,
        vec![0, 4_000, 8_000]
    );
    e.remove_marker_set(MarkerSetId(31)).unwrap();
    e.undo();
    assert!(e.song.marker_sets.contains_key(&MarkerSetId(31)));
}

fn drum_pad_fixture() -> aqua_pattern_core::song::DrumPad {
    use aqua_pattern_core::song::{DrumPad, PadMode, PadNewNote, RegionId};
    DrumPad {
        slot: 0,
        name: "Kick".into(),
        velocity_zones: Vec::new(),
        note: 36,
        region: Some(RegionId(21)),
        root_note: 60,
        tune_semitones: 0,
        gain: 0.75,
        pan: -0.2,
        mode: PadMode::OneShot,
        new_note: PadNewNote::Overlap,
        choke_group: None,
    }
}
fn drum_kit_fixture() -> aqua_pattern_core::song::DrumKitInstrument {
    use aqua_pattern_core::song::{DrumKitInstrument, PadId};
    DrumKitInstrument {
        name: "Shared kit".into(),
        pads: [(PadId(501), drum_pad_fixture())].into(),
    }
}
fn drum_kit_editor() -> Editor {
    use aqua_pattern_core::song::{Instrument, InstrumentKind, RegionId, SampleId};
    let mut e = Editor {
        edit_mode: true,
        ..Editor::default()
    };
    e.register_sample(SampleId(11), source_fixture()).unwrap();
    e.upsert_region(RegionId(21), region_fixture()).unwrap();
    let mut candidate = e.song.clone();
    candidate.instruments.insert(
        InstrumentId(99),
        Instrument {
            name: "Kit instrument".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: None,
        },
    );
    e.replace_document(candidate).unwrap();
    e.edit_mode = true;
    e
}

#[test]
fn kit_and_pad_operations_are_guarded_individual_transactions_with_stable_ids() {
    use aqua_pattern_core::song::{DrumKitId, InstrumentKind, PadId};
    let mut e = drum_kit_editor();
    e.edit_mode = false;
    let before = e.song.clone();
    assert!(e
        .upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .is_err());
    assert!(e.remove_drum_kit(DrumKitId(41)).is_err());
    assert!(e
        .upsert_drum_pad(DrumKitId(41), PadId(501), drum_pad_fixture())
        .is_err());
    assert!(e.remove_drum_pad(DrumKitId(41), PadId(501)).is_err());
    assert!(e.assign_kit(InstrumentId(99), None).is_err());
    assert!(e.insert_pad_note(InstrumentId(99), PadId(501)).is_err());
    assert_eq!(e.song, before);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    let defined = e.song.clone();
    e.undo();
    assert_eq!(e.song, before);
    e.redo();
    assert_eq!(e.song, defined);
    e.assign_kit(InstrumentId(99), Some(DrumKitId(41))).unwrap();
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].kind,
        InstrumentKind::DrumKit { kit: DrumKitId(41) }
    );
    e.undo();
    assert_eq!(e.song, defined);
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    assert!(e.can_redo());
    e.redo();
    let mapped = e.song.clone();
    let mut pad = drum_pad_fixture();
    pad.slot = 15;
    pad.name = "Moved visual slot".into();
    e.upsert_drum_pad(DrumKitId(41), PadId(501), pad).unwrap();
    assert!(e.song.drum_kits[&DrumKitId(41)]
        .pads
        .contains_key(&PadId(501)));
    e.undo();
    assert_eq!(e.song, mapped);
    e.upsert_drum_pad(DrumKitId(41), PadId(501), drum_pad_fixture())
        .unwrap();
    assert!(e.can_redo());
    assert!(
        e.remove_drum_kit(DrumKitId(41)).is_err(),
        "mapped kit cannot dangle"
    );
    e.remove_drum_pad(DrumKitId(41), PadId(501)).unwrap();
    assert!(
        e.song.drum_kits[&DrumKitId(41)].pads.is_empty(),
        "unused empty kit is allowed"
    );
    e.undo();
    assert_eq!(e.song, mapped);
}

#[test]
fn invalid_kit_pad_ranges_duplicates_and_overflow_preserve_saved_redo_and_cursor() {
    use aqua_pattern_core::song::{DrumKitId, DrumPad, PadId, RegionId};
    let mut e = drum_kit_editor();
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    e.mark_saved();
    e.set_timing(175.0, 8).unwrap();
    e.undo();
    let before = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for mutate in [
        |p: &mut DrumPad| p.slot = 16,
        |p: &mut DrumPad| p.note = 128,
        |p: &mut DrumPad| p.root_note = 128,
        |p: &mut DrumPad| p.tune_semitones = 49,
        |p: &mut DrumPad| {
            p.root_note = 100;
            p.tune_semitones = 48;
        },
        |p: &mut DrumPad| p.region = Some(RegionId(999)),
        |p: &mut DrumPad| p.gain = f32::NAN,
        |p: &mut DrumPad| p.gain = 1.01,
        |p: &mut DrumPad| p.pan = f32::INFINITY,
        |p: &mut DrumPad| p.pan = -1.01,
        |p: &mut DrumPad| p.choke_group = Some(0),
        |p: &mut DrumPad| p.choke_group = Some(17),
        |p: &mut DrumPad| p.name.clear(),
    ] {
        let mut pad = drum_pad_fixture();
        mutate(&mut pad);
        assert!(e.upsert_drum_pad(DrumKitId(41), PadId(501), pad).is_err());
    }
    let mut duplicate_note = drum_pad_fixture();
    duplicate_note.slot = 1;
    assert!(e
        .upsert_drum_pad(DrumKitId(41), PadId(502), duplicate_note)
        .is_err());
    let mut duplicate_slot = drum_pad_fixture();
    duplicate_slot.note = 37;
    assert!(e
        .upsert_drum_pad(DrumKitId(41), PadId(502), duplicate_slot)
        .is_err());
    assert!(e
        .upsert_drum_pad(DrumKitId(41), PadId(0), drum_pad_fixture())
        .is_err());
    assert!(e
        .upsert_drum_pad(DrumKitId(999), PadId(501), drum_pad_fixture())
        .is_err());
    assert!(e.upsert_drum_kit(DrumKitId(0), drum_kit_fixture()).is_err());
    assert!(e.remove_drum_pad(DrumKitId(41), PadId(999)).is_err());
    assert!(e.remove_drum_kit(DrumKitId(999)).is_err());
    assert!(e
        .assign_kit(InstrumentId(999), Some(DrumKitId(41)))
        .is_err());
    assert!(e
        .assign_kit(InstrumentId(99), Some(DrumKitId(999)))
        .is_err());
    let mut full = drum_kit_fixture();
    full.pads.clear();
    for n in 0..17 {
        let mut pad = drum_pad_fixture();
        pad.slot = n;
        pad.note = 36 + n;
        full.pads.insert(PadId(501 + u64::from(n)), pad);
    }
    assert!(
        e.upsert_drum_kit(DrumKitId(41), full.clone()).is_err(),
        "never truncate a seventeenth pad"
    );
    assert_eq!(e.song, before);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    full.pads.remove(&PadId(517));
    e.upsert_drum_kit(DrumKitId(41), full).unwrap();
    assert_eq!(e.song.drum_kits[&DrumKitId(41)].pads.len(), 16);
    e.undo();
    assert_eq!(e.song, before);
}

#[test]
fn pad_insertion_writes_one_ordinary_cell_and_preserves_independent_fields_and_history() {
    use aqua_pattern_core::song::{DrumKitId, PadId};
    let mut e = drum_kit_editor();
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    e.assign_kit(InstrumentId(99), Some(DrumKitId(41))).unwrap();
    e.edit_step = 4;
    let old_cell = Cell {
        note: Note::Note(49),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.25),
        pan: Some(0.3),
        delay: Some(0.1),
        effects: StepEffects {
            cut: Some(0.8),
            ..Default::default()
        },
        ..Default::default()
    };
    e.set_cell(old_cell.clone()).unwrap();
    e.mark_saved();
    let before = e.song.clone();
    let revision = e.revision();
    e.insert_pad_note(InstrumentId(99), PadId(501)).unwrap();
    assert_eq!(e.row, 4);
    assert_eq!(e.revision(), revision + 1);
    let mut expected = old_cell.clone();
    expected.note = Note::Note(36);
    expected.instrument = Some(InstrumentId(99));
    assert_eq!(e.pattern().tracks[&TrackId(1)][&0][&0], expected);
    assert_eq!(e.song.drum_kits, before.drum_kits);
    e.undo();
    assert_eq!(e.song, before);
    assert_eq!(e.row, 0);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(e.row, 4);
    e.row = 0;
    let revision = e.revision();
    e.insert_pad_note(InstrumentId(99), PadId(501)).unwrap();
    assert_eq!(e.revision(), revision, "same cell is a content no-op");
    assert_eq!(
        e.row, 4,
        "successful note entry still uses the normal edit step"
    );
    let current = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    assert!(e.insert_pad_note(InstrumentId(1), PadId(501)).is_err());
    assert!(e.insert_pad_note(InstrumentId(99), PadId(999)).is_err());
    assert!(e.insert_pad_note(InstrumentId(999), PadId(501)).is_err());
    let mut unmapped = drum_pad_fixture();
    unmapped.slot = 1;
    unmapped.note = 37;
    unmapped.region = None;
    e.upsert_drum_pad(DrumKitId(41), PadId(502), unmapped)
        .unwrap();
    let with_unmapped = e.song.clone();
    assert!(e.insert_pad_note(InstrumentId(99), PadId(502)).is_err());
    assert_eq!(e.song, with_unmapped);
    e.undo();
    assert_eq!(e.song, current);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(e.revision() > revision);
    e.upsert_phrase(PhraseId(17), phrase_fixture()).unwrap();
    e.set_cell(Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        phrase: Some(PhraseTrigger {
            phrase: PhraseId(17),
            cycles: 1,
        }),
        ..Default::default()
    })
    .unwrap();
    let phrase_song = e.song.clone();
    let revision = e.revision();
    assert!(e
        .insert_pad_note(InstrumentId(99), PadId(501))
        .unwrap_err()
        .contains("phrase"));
    assert_eq!(e.song, phrase_song);
    assert_eq!(e.revision(), revision);
    assert_eq!(e.row, 4);
    e.occurrence = usize::MAX;
    assert!(e.insert_pad_note(InstrumentId(99), PadId(501)).is_err());
    assert_eq!(e.song, phrase_song);
}

#[test]
fn kit_mapping_references_include_latched_unused_pattern_and_phrase_notes() {
    use aqua_pattern_core::song::{DrumKitId, PadId, RegionId, SampleId};
    let mut e = drum_kit_editor();
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    let mut second = drum_pad_fixture();
    second.slot = 1;
    second.note = 37;
    e.upsert_drum_pad(DrumKitId(41), PadId(502), second.clone())
        .unwrap();
    e.assign_kit(InstrumentId(99), Some(DrumKitId(41))).unwrap();
    e.edit_step = 1;
    e.insert_pad_note(InstrumentId(99), PadId(501)).unwrap();
    e.set_cell(Cell {
        note: Note::Note(37),
        ..Default::default()
    })
    .unwrap();
    let referenced = e.song.clone();
    assert!(
        e.remove_drum_pad(DrumKitId(41), PadId(502)).is_err(),
        "latched kit selector remains a reference"
    );
    let mut rebound = second.clone();
    rebound.note = 38;
    assert!(e
        .upsert_drum_pad(DrumKitId(41), PadId(502), rebound)
        .is_err());
    let mut unmapped = second;
    unmapped.region = None;
    assert!(e
        .upsert_drum_pad(DrumKitId(41), PadId(502), unmapped)
        .is_err());
    assert!(e.remove_drum_kit(DrumKitId(41)).is_err());
    assert!(e.remove_region(RegionId(21)).is_err());
    assert!(e.remove_sample(SampleId(11)).is_err());
    assert_eq!(e.song, referenced);
    let mut unused = referenced.clone();
    unused.order.clear();
    e.apply_document_edit(unused.clone()).unwrap();
    assert!(
        e.remove_drum_pad(DrumKitId(41), PadId(502)).is_err(),
        "unused pattern still holds its selector references"
    );
    assert_eq!(e.song, unused);
    let mut phrase_only = unused;
    for pattern in phrase_only.patterns.values_mut() {
        pattern.tracks.clear();
    }
    phrase_only.phrases.insert(
        PhraseId(77),
        Phrase {
            name: "Kit phrase".into(),
            instrument: InstrumentId(99),
            rows: 1,
            lines_per_beat: 4,
            note_columns: 1,
            root_note: 36,
            cells: [(
                0,
                [(
                    0,
                    Cell {
                        note: Note::Note(37),
                        ..Default::default()
                    },
                )]
                .into(),
            )]
            .into(),
        },
    );
    e.apply_document_edit(phrase_only.clone()).unwrap();
    assert!(
        e.remove_drum_pad(DrumKitId(41), PadId(502)).is_err(),
        "unused kit-owned phrase holds its pad reference"
    );
    assert_eq!(e.song, phrase_only);
}

#[test]
fn kit_mapping_batch_and_independent_kit_clone_are_one_undo_without_second_pattern_storage() {
    use aqua_pattern_core::song::{DrumKitId, PadId};
    let mut e = drum_kit_editor();
    let before = e.song.clone();
    let mut candidate = before.clone();
    candidate
        .upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    candidate
        .assign_kit(InstrumentId(99), Some(DrumKitId(41)))
        .unwrap();
    e.apply_document_edit(candidate.clone()).unwrap();
    assert_eq!(e.song.patterns, before.patterns);
    assert_eq!(e.song.order, before.order);
    e.undo();
    assert_eq!(e.song, before);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(e.song, candidate);
    e.upsert_drum_kit(DrumKitId(42), drum_kit_fixture())
        .unwrap();
    let mut copied_pad = drum_pad_fixture();
    copied_pad.gain = 0.3;
    copied_pad.slot = 3;
    e.upsert_drum_pad(DrumKitId(42), PadId(501), copied_pad.clone())
        .unwrap();
    assert_eq!(
        e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)],
        drum_pad_fixture()
    );
    assert_eq!(
        e.song.drum_kits[&DrumKitId(42)].pads[&PadId(501)],
        copied_pad
    );
    let before_delete = e.song.clone();
    let mut deletion = before_delete.clone();
    deletion.assign_kit(InstrumentId(99), None).unwrap();
    deletion.remove_drum_kit(DrumKitId(41)).unwrap();
    e.apply_document_edit(deletion.clone()).unwrap();
    e.undo();
    assert_eq!(e.song, before_delete);
    e.redo();
    assert_eq!(e.song, deletion);
}

#[test]
fn create_kit_on_fresh_instrument_preserves_populated_demo_and_is_one_undo() {
    use aqua_pattern_core::song::{DrumKitId, InstrumentKind};
    let mut e = Editor {
        row: 12,
        field: Field::Pan,
        ..Editor::default()
    };
    let before = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    assert!(e.create_drum_kit_instrument("New drums".into()).is_err());
    assert_eq!(e.song, before);
    assert!(!e.can_undo() && !e.dirty);
    e.edit_mode = true;
    let revision = e.revision();
    let (instrument, kit) = e.create_drum_kit_instrument("New drums".into()).unwrap();
    assert_eq!((instrument, kit), (InstrumentId(5), DrumKitId(1)));
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    let created = e.song.clone();
    let mut old_content = created.clone();
    let new_instrument = old_content.instruments.remove(&instrument).unwrap();
    let new_kit = old_content.drum_kits.remove(&kit).unwrap();
    assert_eq!(new_instrument.name, "New drums");
    assert_eq!(new_instrument.kind, InstrumentKind::DrumKit { kit });
    assert!(new_instrument.sample.is_none() && new_instrument.region.is_none());
    assert_eq!(new_kit.name, "New drums");
    assert!(new_kit.pads.is_empty());
    assert_eq!(
        old_content, before,
        "all prior notes, mappings and assets survive"
    );
    let mut direct = before.clone();
    assert_eq!(
        direct
            .create_drum_kit_instrument("New drums".into())
            .unwrap(),
        (instrument, kit)
    );
    assert_eq!(direct, created, "shared model and Editor outcomes match");
    assert!(
        e.assign_kit(InstrumentId(1), Some(kit)).is_err(),
        "empty-kit referenced-note validation is not weakened"
    );
    assert_eq!(e.song, created);
    e.undo();
    assert_eq!(e.song, before);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.can_undo() && !e.dirty && e.can_redo());
    e.redo();
    assert_eq!(e.song, created);
    assert!(e.dirty);
}

#[test]
fn create_kit_uses_fresh_namespaces_and_failures_preserve_redo_and_document() {
    use aqua_pattern_core::song::{DrumKitId, DrumKitInstrument};
    let mut e = drum_kit_editor();
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    e.mark_saved();
    let original = e.song.clone();
    assert_eq!(
        e.create_drum_kit_instrument("Fresh kit".into()).unwrap(),
        (InstrumentId(100), DrumKitId(42)),
        "gaps are not reused and occupied definitions are not overwritten"
    );
    e.undo();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for name in [String::new(), " \n\t".into(), "x".repeat(257)] {
        assert!(e.create_drum_kit_instrument(name).is_err());
        assert_eq!(e.song, original);
        assert_eq!(e.revision(), revision);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(!e.dirty && e.can_redo());
    }
    for exhausted_namespace in ["instrument", "kit", "kit cap"] {
        let mut limited = original.clone();
        match exhausted_namespace {
            "instrument" => {
                limited.instruments.insert(
                    InstrumentId(u64::MAX),
                    limited.instruments[&InstrumentId(1)].clone(),
                );
            }
            "kit" => {
                limited.drum_kits.insert(
                    DrumKitId(u64::MAX),
                    DrumKitInstrument {
                        name: "Last ID".into(),
                        pads: Default::default(),
                    },
                );
            }
            _ => {
                for id in 1..=128 {
                    limited.drum_kits.insert(
                        DrumKitId(id),
                        DrumKitInstrument {
                            name: format!("Kit {id}"),
                            pads: Default::default(),
                        },
                    );
                }
            }
        }
        limited.validate().unwrap();
        e.replace_document(limited.clone()).unwrap();
        e.edit_mode = true;
        e.set_timing(123.0, 4).unwrap();
        e.undo();
        let revision = e.revision();
        let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
        assert!(
            e.create_drum_kit_instrument("Cannot create".into())
                .is_err(),
            "{exhausted_namespace}"
        );
        assert_eq!(e.song, limited);
        assert_eq!(e.revision(), revision);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(!e.dirty && e.can_redo());
        let mut direct = limited.clone();
        assert!(direct
            .create_drum_kit_instrument("Cannot create".into())
            .is_err());
        assert_eq!(
            direct, limited,
            "shared helper publishes neither partial ID"
        );
    }
}

fn alias_editor() -> Editor {
    use aqua_pattern_core::song::{Occurrence, Pattern};
    let mut e = Editor::default();
    let mut song = e.song.clone();
    song.patterns.insert(
        PatternId(3),
        Pattern {
            generation: None,
            name: "Alias target".into(),
            rows: 32,
            tracks: Default::default(),
            track_aliases: Default::default(),
        },
    );
    song.order.push(Occurrence {
        pattern: PatternId(3),
        muted_tracks: Default::default(),
        section: None,
    });
    e.replace_document(song).unwrap();
    e.edit_mode = true;
    e.select(3, TrackId(2)).unwrap();
    e
}
#[test]
fn alias_reads_terminal_content_but_all_cell_edits_require_explicit_unique_or_source() {
    use aqua_pattern_core::song::TrackAlias;
    let mut e = alias_editor();
    let before = e.song.clone();
    let source = before.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0].clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let link = TrackAlias {
        pattern: PatternId(1),
        track: TrackId(1),
    };
    e.edit_mode = false;
    assert!(e
        .set_track_alias(PatternId(3), TrackId(2), Some(link))
        .is_err());
    assert!(e.make_track_unique(PatternId(3), TrackId(2)).is_err());
    assert!(e.set_occurrence_track_mute(3, TrackId(2), true).is_err());
    assert_eq!(e.song, before);
    e.edit_mode = true;
    let revision = e.revision();
    e.set_track_alias(PatternId(3), TrackId(2), Some(link))
        .unwrap();
    assert_eq!(e.revision(), revision + 1);
    assert_eq!(e.content_source().unwrap(), (PatternId(1), TrackId(1)));
    assert_eq!(e.pattern_id(), PatternId(3));
    assert_eq!(e.cell(), source);
    assert!(!e.pattern().tracks.contains_key(&TrackId(2)));
    let aliased = e.song.clone();
    e.undo();
    assert_eq!(e.song, before);
    assert!(!e.dirty);
    e.redo();
    assert_eq!(e.song, aliased);
    e.mark_saved();
    e.set_timing(130.0, 4).unwrap();
    e.undo();
    let revision = e.revision();
    assert!(
        e.set_cell(Cell::default()).is_err(),
        "clear cannot create a shadow row"
    );
    assert!(
        e.set_cell(source.clone()).is_err(),
        "even an identical alias write is not local authority"
    );
    assert!(e.enter_note(Note::Note(65)).is_err());
    assert!(e.enter_value("65").is_err());
    assert!(e.clear(false).is_err());
    assert!(e.clear(true).is_err());
    assert!(e.set_effects(StepEffects::default()).is_err());
    assert!(e.set_phrase_trigger(None).is_err());
    assert_eq!(e.song, aliased);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && e.can_redo());
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.make_track_unique(PatternId(3), TrackId(2)).unwrap();
    assert_eq!(e.content_source().unwrap(), (PatternId(3), TrackId(2)));
    assert_eq!(e.cell(), source);
    assert_eq!(
        e.song.patterns[&PatternId(1)],
        aliased.patterns[&PatternId(1)]
    );
    let unique = e.song.clone();
    e.undo();
    assert_eq!(e.song, aliased);
    e.redo();
    assert_eq!(e.song, unique);
    e.enter_note(Note::Note(65)).unwrap();
    assert_eq!(
        e.song.patterns[&PatternId(1)],
        aliased.patterns[&PatternId(1)]
    );
    assert_eq!(e.pattern().tracks[&TrackId(2)][&0][&0].note, Note::Note(65));
}
#[test]
fn alias_graph_failures_preserve_history_cursor_and_hidden_content() {
    use aqua_pattern_core::song::TrackAlias;
    let mut e = alias_editor();
    let source = TrackAlias {
        pattern: PatternId(1),
        track: TrackId(1),
    };
    e.set_track_alias(PatternId(3), TrackId(2), Some(source))
        .unwrap();
    e.mark_saved();
    e.set_timing(130.0, 4).unwrap();
    e.undo();
    let before = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for alias in [
        TrackAlias {
            pattern: PatternId(3),
            track: TrackId(2),
        },
        TrackAlias {
            pattern: PatternId(999),
            track: TrackId(1),
        },
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(999),
        },
        TrackAlias {
            pattern: PatternId(2),
            track: TrackId(1),
        },
    ] {
        assert!(e
            .set_track_alias(PatternId(3), TrackId(2), Some(alias))
            .is_err());
    }
    assert!(
        e.set_track_alias(PatternId(1), TrackId(1), Some(source))
            .is_err(),
        "occupied local data is not overwritten"
    );
    assert!(e
        .set_track_alias(PatternId(999), TrackId(2), Some(source))
        .is_err());
    assert!(e.make_track_unique(PatternId(3), TrackId(999)).is_err());
    assert!(e
        .set_occurrence_track_mute(usize::MAX, TrackId(2), true)
        .is_err());
    assert!(e.set_occurrence_track_mute(3, TrackId(999), true).is_err());
    assert!(
        e.resize(64).is_err(),
        "target alias row count must remain equal"
    );
    for change in 0..5 {
        let mut candidate = before.clone();
        match change {
            0 => candidate.patterns.get_mut(&PatternId(1)).unwrap().rows = 64,
            1 => candidate.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 3,
            2 => {
                candidate
                    .patterns
                    .get_mut(&PatternId(3))
                    .unwrap()
                    .tracks
                    .insert(TrackId(2), Default::default());
            }
            3 => {
                candidate.order.retain(|o| o.pattern != PatternId(1));
                candidate.patterns.remove(&PatternId(1));
            }
            _ => {
                for pattern in candidate.patterns.values_mut() {
                    pattern.tracks.remove(&TrackId(1));
                }
                candidate.tracks.remove(&TrackId(1));
                candidate.track_order.retain(|t| *t != TrackId(1));
            }
        }
        assert!(
            e.apply_document_edit(candidate).is_err(),
            "invalid graph change {change}"
        );
    }
    assert_eq!(e.song, before);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && e.can_redo());
    e.set_track_alias(PatternId(3), TrackId(2), Some(source))
        .unwrap();
    e.make_track_unique(PatternId(1), TrackId(1)).unwrap();
    e.set_track_alias(PatternId(1), TrackId(1), None).unwrap();
    assert_eq!(e.song, before);
    assert_eq!(e.revision(), revision);
    assert!(e.can_redo());
    e.set_track_alias(PatternId(3), TrackId(2), None).unwrap();
    assert_eq!(e.cell(), Cell::default());
    assert_eq!(
        e.song.patterns[&PatternId(1)],
        before.patterns[&PatternId(1)]
    );
    e.undo();
    assert_eq!(e.song, before);
}
#[test]
fn alias_chains_resolve_live_sources_and_complete_pattern_clone_materializes_independently() {
    use aqua_pattern_core::song::{Occurrence, Pattern, TrackAlias};
    let mut e = alias_editor();
    e.set_track_alias(
        PatternId(3),
        TrackId(2),
        Some(TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        }),
    )
    .unwrap();
    let mut chain = e.song.clone();
    chain.patterns.insert(
        PatternId(4),
        Pattern {
            generation: None,
            name: "Chained".into(),
            rows: 32,
            tracks: Default::default(),
            track_aliases: [(
                TrackId(3),
                TrackAlias {
                    pattern: PatternId(3),
                    track: TrackId(2),
                },
            )]
            .into(),
        },
    );
    chain.order.push(Occurrence {
        pattern: PatternId(4),
        muted_tracks: Default::default(),
        section: None,
    });
    e.apply_document_edit(chain).unwrap();
    e.select(0, TrackId(1)).unwrap();
    e.enter_note(Note::Note(66)).unwrap();
    e.select(4, TrackId(3)).unwrap();
    e.row = 0;
    assert_eq!(e.content_source().unwrap(), (PatternId(1), TrackId(1)));
    assert_eq!(e.cell().note, Note::Note(66));
    e.set_occurrence_track_mute(4, TrackId(3), true).unwrap();
    let shared = e.song.clone();
    e.append(true).unwrap();
    let cloned = e.pattern_id();
    assert_eq!(cloned, PatternId(5));
    assert!(e.pattern().track_aliases.is_empty());
    assert_eq!(e.cell().note, Note::Note(66));
    assert!(e.song.order[e.occurrence].muted_tracks.is_empty());
    let copied = e.pattern().clone();
    e.select(0, TrackId(1)).unwrap();
    e.row = 0;
    e.enter_note(Note::Note(67)).unwrap();
    assert_eq!(e.song.patterns[&cloned], copied);
    e.undo();
    e.undo();
    assert_eq!(e.song, shared);
    assert_eq!((e.occurrence, e.track), (4, TrackId(3)));
    e.append(false).unwrap();
    assert_eq!(e.pattern_id(), PatternId(4));
    assert!(e.song.order[e.occurrence].muted_tracks.is_empty());
    assert!(e.song.order[4].muted_tracks.contains(&TrackId(3)));
    let mut exhausted = e.song.clone();
    exhausted.patterns.insert(PatternId(u64::MAX), copied);
    e.apply_document_edit(exhausted.clone()).unwrap();
    let revision = e.revision();
    assert!(e.append(true).is_err());
    assert_eq!(e.song, exhausted);
    assert_eq!(e.revision(), revision);
}
#[test]
fn occurrence_mutes_are_independent_undoable_and_move_with_the_whole_occurrence() {
    let mut e = alias_editor();
    let before = e.song.clone();
    e.set_occurrence_track_mute(0, TrackId(1), true).unwrap();
    assert!(
        !e.song.order[2].muted_tracks.contains(&TrackId(1)),
        "shared pattern does not share occurrence mute"
    );
    assert_eq!(e.song.patterns, before.patterns);
    e.undo();
    assert_eq!(e.song, before);
    e.redo();
    e.set_occurrence_track_mute(3, TrackId(2), true).unwrap();
    e.mark_saved();
    e.set_timing(130.0, 4).unwrap();
    e.undo();
    let marked = e.song.clone();
    let revision = e.revision();
    e.set_occurrence_track_mute(3, TrackId(2), true).unwrap();
    e.set_occurrence_track_mute(2, TrackId(1), false).unwrap();
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && e.can_redo());
    e.row = 4;
    e.column = 1;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.move_occurrence(-2).unwrap();
    let mut swapped = marked.clone();
    swapped.order.swap(3, 1);
    assert_eq!(e.song, swapped);
    assert_eq!(e.occurrence, 1);
    e.undo();
    assert_eq!(e.song, marked);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.redo();
    assert_eq!(e.song, swapped);
    e.remove_occurrence().unwrap();
    assert!(!e.song.order.iter().any(|o| o.pattern == PatternId(3)));
    assert_eq!(e.song.patterns, marked.patterns);
    e.undo();
    assert_eq!(e.song, swapped);
}

fn transform_config() -> aqua_pattern_core::transforms::TransformConfig {
    use aqua_pattern_core::transforms::{TransformConfig, TransformFields, TransformScope};
    TransformConfig {
        scope: TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 2,
            columns: vec![0],
        },
        fields: TransformFields {
            notes: true,
            glide_targets: false,
            velocity: false,
            timing: false,
        },
        transpose_semitones: 12,
        velocity_scale: 1.0,
        nudge_frames: 0,
        swing: None,
    }
}
fn transform_editor() -> Editor {
    use aqua_pattern_core::song::{Pattern, TrackAlias};
    let mut e = Editor::default();
    let mut song = e.song.clone();
    let rows = song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    rows.get_mut(&0).unwrap().insert(
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            velocity: Some(0.8),
            pan: Some(-0.2),
            delay: Some(0.125),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.25,
                }),
                release: Some(0.8),
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    rows.get_mut(&0).unwrap().insert(
        1,
        Cell {
            note: Note::Note(55),
            instrument: Some(InstrumentId(2)),
            velocity: Some(0.3),
            delay: Some(0.5),
            ..Default::default()
        },
    );
    rows.insert(
        1,
        [(
            0,
            Cell {
                note: Note::Note(64),
                ..Default::default()
            },
        )]
        .into(),
    );
    rows.insert(
        2,
        [(
            0,
            Cell {
                note: Note::Note(67),
                instrument: Some(InstrumentId(1)),
                velocity: Some(0.0),
                delay: Some(0.0),
                ..Default::default()
            },
        )]
        .into(),
    );
    song.patterns.insert(
        PatternId(3),
        Pattern {
            generation: None,
            name: "Source alias".into(),
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
    song.clone_pattern(PatternId(1), PatternId(4)).unwrap();
    e.replace_document(song).unwrap();
    e.edit_mode = true;
    e.row = 7;
    e.field = Field::Pan;
    e
}
#[test]
fn transform_masks_and_scope_change_only_reviewed_fields_as_one_cursor_preserving_edit() {
    use aqua_pattern_core::transforms;
    let mut e = transform_editor();
    let base = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let revision = e.revision();
    let mut config = transform_config();
    config.fields.velocity = true;
    config.velocity_scale = 0.5;
    let plan = transforms::preview(&e.song, config.clone()).unwrap();
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo());
    assert_eq!(plan.config(), &config);
    assert_eq!(plan.base(), &base);
    assert_eq!(plan.summary().selected_cells, 2);
    assert_eq!(plan.summary().skipped_unspecified_velocity, 1);
    assert_eq!(plan.summary().affected_occurrences, vec![0, 2]);
    assert_eq!(plan.summary().affected_aliases.len(), 1);
    let mut expected = base.clone();
    let rows = expected
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    rows.get_mut(&0).unwrap().get_mut(&0).unwrap().note = Note::Note(72);
    rows.get_mut(&0).unwrap().get_mut(&0).unwrap().velocity = Some(0.4);
    rows.get_mut(&1).unwrap().get_mut(&0).unwrap().note = Note::Note(76);
    assert_eq!(
        plan.candidate(),
        &expected,
        "unmasked glide/pan/delay/FX and all off-selection cells remain byte-equivalent"
    );
    assert_eq!(
        transforms::preview(&base, config).unwrap().candidate(),
        &expected
    );
    e.edit_mode = false;
    assert!(e.commit_transform_preview(revision, &plan).is_err());
    assert_eq!(e.song, base);
    e.edit_mode = true;
    e.commit_transform_preview(revision, &plan).unwrap();
    assert_eq!(e.song, expected);
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert_eq!(
        e.song.patterns[&PatternId(4)],
        base.patterns[&PatternId(4)],
        "independent clone is not transformed"
    );
    assert_eq!(
        e.song.resolved_track(PatternId(3), TrackId(2)).unwrap(),
        e.song.resolved_track(PatternId(1), TrackId(1)).unwrap()
    );
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo());
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.redo();
    assert_eq!(e.song, expected);
    let mut glide_only = transform_config();
    glide_only.fields.notes = false;
    glide_only.fields.glide_targets = true;
    let plan = transforms::preview(&base, glide_only).unwrap();
    let mut expected = base.clone();
    expected
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
        .effects
        .glide
        .as_mut()
        .unwrap()
        .target = 84;
    assert_eq!(
        plan.candidate(),
        &expected,
        "glide-target mask does not transpose note selectors"
    );
}
#[test]
fn identity_transform_preserves_optional_bits_saved_state_redo_and_current_cursor() {
    use aqua_pattern_core::{
        break_compiler::Subdivision,
        transforms::{self, TransformSwing},
    };
    let mut e = transform_editor();
    e.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .insert(
            3,
            [(
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    delay: Some(1e-15),
                    velocity: Some(0.0),
                    ..Default::default()
                },
            )]
            .into(),
        );
    e.mark_saved();
    e.set_timing(130.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let bytes = serde_json::to_vec(&base).unwrap();
    let revision = e.revision();
    let mut config = transform_config();
    config.scope.row_end = 4;
    config.fields.glide_targets = true;
    config.fields.velocity = true;
    config.fields.timing = true;
    config.transpose_semitones = 0;
    config.velocity_scale = 1.0;
    config.nudge_frames = 0;
    config.swing = Some(TransformSwing {
        division: Subdivision::Sixteenth,
        amount_percent: 0,
    });
    let plan = transforms::preview(&e.song, config).unwrap();
    assert_eq!(serde_json::to_vec(plan.candidate()).unwrap(), bytes);
    assert!(plan.changes().is_empty());
    // Navigation is not a document mutation. The explicit reviewed scope stays
    // fixed, and the commit wrapper must not move the current tracker cursor.
    e.row = 9;
    e.column = 1;
    e.field = Field::Delay;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.commit_transform_preview(revision, &plan).unwrap();
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), bytes);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && e.can_redo());
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert_eq!(
        e.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&3][&0]
            .delay
            .unwrap()
            .to_bits(),
        1e-15f64.to_bits()
    );
    let mut empty_scope = transform_config();
    empty_scope.scope.row_start = 4;
    empty_scope.scope.row_end = 5;
    let empty = transforms::preview(&e.song, empty_scope).unwrap();
    assert_eq!(empty.summary().selected_cells, 0);
    e.commit_transform_preview(revision, &empty).unwrap();
    assert_eq!(e.revision(), revision);
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), bytes);
    assert!(!e.dirty && e.can_redo());
}
#[test]
fn transform_preview_rejects_stale_unrelated_edits_aba_replacement_and_unchecked_drafts() {
    use aqua_pattern_core::transforms;
    let mut e = transform_editor();
    let base = e.song.clone();
    let revision = e.revision();
    let plan = transforms::preview(&base, transform_config()).unwrap();
    e.set_occurrence_track_mute(2, TrackId(4), true).unwrap();
    let changed = e.song.clone();
    let changed_revision = e.revision();
    assert!(e.commit_transform_preview(revision, &plan).is_err());
    assert!(
        e.commit_transform_preview(changed_revision, &plan).is_err(),
        "correct new generation cannot override stale full base"
    );
    assert_eq!(e.song, changed);
    assert_eq!(e.revision(), changed_revision);
    e.undo();
    assert_eq!(e.song, base);
    assert!(
        e.commit_transform_preview(revision, &plan).is_err(),
        "Undo ABA rejected"
    );
    let revision = e.revision();
    let mut unrelated_scope = transform_config();
    unrelated_scope.scope.pattern = PatternId(2);
    let unaffected = e.song.patterns[&PatternId(2)].clone();
    let plan = transforms::preview(&e.song, unrelated_scope).unwrap();
    e.row = 0;
    let mut source_cell = e.cell();
    source_cell.pan = Some(0.4);
    e.set_cell(source_cell).unwrap();
    assert_eq!(e.song.patterns[&PatternId(2)], unaffected);
    assert!(
        e.commit_transform_preview(revision, &plan).is_err(),
        "editing another alias source still invalidates full document preview"
    );
    assert!(e.commit_transform_preview(e.revision(), &plan).is_err());
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    let revision = e.revision();
    let plan = transforms::preview(&e.song, transform_config()).unwrap();
    e.replace_document(base).unwrap();
    e.edit_mode = true;
    assert!(
        e.commit_transform_preview(revision, &plan).is_err(),
        "identical replacement is a different document generation"
    );
    let revision = e.revision();
    let plan = transforms::preview(&e.song, transform_config()).unwrap();
    e.song.title.push_str(" unchecked");
    let changed = e.song.clone();
    assert!(
        e.commit_transform_preview(revision, &plan).is_err(),
        "base guard catches unchecked draft writes without revision bump"
    );
    assert_eq!(e.song, changed);
    assert_eq!(e.revision(), revision);
    assert!(!e.can_undo());
}
#[test]
fn invalid_transform_scopes_masks_and_ranges_never_mutate_editor_or_materialize_aliases() {
    use aqua_pattern_core::{
        break_compiler::Subdivision,
        transforms::{self, TransformSwing},
    };
    let mut e = transform_editor();
    e.set_timing(130.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for case in 0..13 {
        let mut config = transform_config();
        match case {
            0 => config.scope.pattern = PatternId(999),
            1 => config.scope.track = TrackId(999),
            2 => config.scope.row_end = 33,
            3 => config.scope.row_start = config.scope.row_end,
            4 => config.scope.columns.clear(),
            5 => config.scope.columns = vec![0, 0],
            6 => config.scope.columns = vec![1, 0],
            7 => config.scope.columns = vec![2],
            8 => {
                config.scope.pattern = PatternId(3);
                config.scope.track = TrackId(2);
                config.transpose_semitones = 0;
            }
            9 => config.transpose_semitones = 49,
            10 => config.velocity_scale = f32::NAN,
            11 => config.nudge_frames = 48_001,
            _ => {
                config.swing = Some(TransformSwing {
                    division: Subdivision::Sixteenth,
                    amount_percent: 51,
                })
            }
        }
        assert!(
            transforms::preview(&e.song, config).is_err(),
            "invalid case {case}"
        );
        assert_eq!(e.song, base);
        assert_eq!(e.revision(), revision);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(!e.dirty && e.can_redo());
    }
    assert!(!e.song.patterns[&PatternId(3)]
        .tracks
        .contains_key(&TrackId(2)));
}

#[test]
fn transform_identity_and_velocity_masks_do_not_collapse_near_one_delays_at_low_or_high_rows() {
    use aqua_pattern_core::transforms::{self, TransformFields};
    for row in [1, 10_000] {
        let mut e = transform_editor();
        let mut base = e.song.clone();
        let rows = 32.max(row + 2);
        base.patterns.get_mut(&PatternId(1)).unwrap().rows = rows;
        base.patterns.get_mut(&PatternId(3)).unwrap().rows = rows;
        let cells = base
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap();
        for (at, delay, velocity) in [(row, Some(1.0f64.next_down()), 0.6), (row + 1, None, 0.4)] {
            cells.insert(
                at,
                [(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        delay,
                        velocity: Some(velocity),
                        ..Default::default()
                    },
                )]
                .into(),
            );
        }
        e.replace_document(base.clone()).unwrap();
        e.edit_mode = true;
        let mut config = transform_config();
        config.scope.row_start = row;
        config.scope.row_end = row + 2;
        config.fields = TransformFields::default();
        // All disabled fields ignore valid nonidentity parameters; no cell is encoded anew.
        config.velocity_scale = 2.0;
        config.nudge_frames = 37;
        let identity = transforms::preview(&base, config.clone()).unwrap();
        assert_eq!(
            serde_json::to_vec(identity.candidate()).unwrap(),
            serde_json::to_vec(&base).unwrap()
        );
        e.commit_transform_preview(e.revision(), &identity).unwrap();
        assert!(!e.dirty && !e.can_undo());
        config.fields.velocity = true;
        config.velocity_scale = 0.5;
        let plan = transforms::preview(&base, config).unwrap();
        let mut expected = base.clone();
        for (at, velocity) in [(row, 0.3), (row + 1, 0.2)] {
            expected
                .patterns
                .get_mut(&PatternId(1))
                .unwrap()
                .tracks
                .get_mut(&TrackId(1))
                .unwrap()
                .get_mut(&at)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .velocity = Some(velocity);
        }
        assert_eq!(plan.candidate(), &expected);
        assert_eq!(
            plan.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0]
                .delay
                .unwrap()
                .to_bits(),
            1.0f64.next_down().to_bits()
        );
        e.commit_transform_preview(e.revision(), &plan).unwrap();
        assert_eq!(e.song, expected);
        e.undo();
        assert_eq!(e.song, base);
        assert!(!e.dirty);
    }
}
#[test]
fn timing_transform_moves_whole_cells_atomically_and_swing_shifts_all_row_local_commands() {
    use aqua_pattern_core::{
        break_compiler::Subdivision,
        transforms::{self, TransformFields, TransformSwing},
    };
    let mut e = transform_editor();
    let base = e.song.clone();
    let mut config = transform_config();
    config.scope.row_end = 4;
    config.fields = TransformFields {
        timing: true,
        ..Default::default()
    };
    config.nudge_frames = 6_000;
    config.transpose_semitones = 0;
    let plan = transforms::preview(&base, config).unwrap();
    assert_eq!(plan.summary().moved_cells, 3);
    let mut expected = base.clone();
    let target = expected
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    let mut shifted = Vec::new();
    for row in 0..3 {
        let mut cell = target.get_mut(&row).unwrap().remove(&0).unwrap();
        if target[&row].is_empty() {
            target.remove(&row);
        }
        cell.delay = Some(cell.delay.unwrap_or(0.0));
        shifted.push((row + 1, cell));
    }
    for (row, cell) in shifted {
        target.entry(row).or_default().insert(0, cell);
    }
    assert_eq!(
        plan.candidate(),
        &expected,
        "whole-cell moves do not change any off-selection content"
    );
    assert_eq!(
        expected.patterns[&PatternId(1)].tracks[&TrackId(1)][&1][&0],
        base.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0]
    );
    assert_eq!(
        expected.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&1],
        base.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&1]
    );
    assert!(!expected.patterns[&PatternId(1)].tracks[&TrackId(1)][&0].contains_key(&0));
    assert_eq!(
        expected.patterns[&PatternId(1)].tracks[&TrackId(1)][&31],
        base.patterns[&PatternId(1)].tracks[&TrackId(1)][&31]
    );
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    e.commit_transform_preview(e.revision(), &plan).unwrap();
    e.undo();
    assert_eq!(e.song, base);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.redo();
    assert_eq!(e.song, expected);
    // One odd sixteenth bin: swing moves parent note, retrigger and release by
    // the same integer frames, rather than shortening the note accidentally.
    let mut swung_base = base.clone();
    let cell = Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.5),
        pan: Some(-0.3),
        delay: Some(0.1),
        effects: StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.25,
            }),
            release: Some(0.7),
            ..Default::default()
        },
        ..Default::default()
    };
    swung_base
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .insert(1, [(0, cell.clone())].into());
    let mut swing = transform_config();
    swing.scope.row_start = 1;
    swing.scope.row_end = 2;
    swing.fields = TransformFields {
        timing: true,
        ..Default::default()
    };
    swing.transpose_semitones = 0;
    swing.swing = Some(TransformSwing {
        division: Subdivision::Sixteenth,
        amount_percent: 25,
    });
    let plan = transforms::preview(&swung_base, swing).unwrap();
    assert_eq!(plan.summary().swing_frames, 1_500);
    let after = &plan.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)][&1][&0];
    for prefix in [0u32, 96] {
        for (old, new) in [
            (cell.delay.unwrap(), after.delay.unwrap()),
            (
                cell.delay.unwrap() + cell.effects.retrigger.as_ref().unwrap().spacing,
                after.delay.unwrap() + after.effects.retrigger.as_ref().unwrap().spacing,
            ),
            (
                cell.effects.release.unwrap(),
                after.effects.release.unwrap(),
            ),
        ] {
            assert_eq!(
                ((f64::from(prefix + 1) + new) * 6_000.0).round() as u64,
                ((f64::from(prefix + 1) + old) * 6_000.0).round() as u64 + 1_500
            );
        }
    }
    assert_eq!(after.note, cell.note);
    assert_eq!(after.instrument, cell.instrument);
    assert_eq!(after.velocity, cell.velocity);
    assert_eq!(after.pan, cell.pan);
    // Two distinct starts becoming one row/column address must never be merged.
    let mut collision = swung_base.clone();
    let rows = collision
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    rows.insert(
        0,
        [(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                delay: Some(0.75),
                ..Default::default()
            },
        )]
        .into(),
    );
    rows.insert(
        1,
        [(
            0,
            Cell {
                note: Note::Note(62),
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        )]
        .into(),
    );
    let unchanged = collision.clone();
    let mut bad = transform_config();
    bad.fields = TransformFields {
        timing: true,
        ..Default::default()
    };
    bad.transpose_semitones = 0;
    bad.scope.row_end = 3;
    bad.nudge_frames = -1_500;
    assert!(transforms::preview(&collision, bad).is_err());
    assert_eq!(collision, unchanged);
    let mut out = transform_config();
    out.fields = TransformFields {
        timing: true,
        ..Default::default()
    };
    out.transpose_semitones = 0;
    out.nudge_frames = -6_000;
    assert!(transforms::preview(&base, out).is_err());
    assert_eq!(e.song, expected);
}

fn generation_config() -> aqua_pattern_core::generation::GenerationConfig {
    use aqua_pattern_core::generation::{GenerationConfig, GenerationNote, GenerationPreset};
    GenerationConfig {
        version: 1,
        name: "Reviewed Euclidean take".into(),
        bars: 2,
        instrument: InstrumentId(1),
        seed: 0xA11CE,
        preset: GenerationPreset::Euclidean {
            steps: 7,
            pulses: 5,
            rotation: 2,
            probability: 0.75,
        },
        sequence: vec![
            GenerationNote {
                note: 60,
                velocity: 0.4,
            },
            GenerationNote {
                note: 67,
                velocity: 0.8,
            },
        ],
    }
}
fn generation_editor() -> Editor {
    let mut e = transform_editor();
    let mut song = e.song.clone();
    // Preview is pure musical work: a resolved source need not be opened here.
    for instrument in song.instruments.values_mut() {
        instrument.sample = Some("/no-generation-io/assigned-source.wav".into());
    }
    song.order[1].muted_tracks.insert(TrackId(2));
    e.replace_document(song).unwrap();
    e.edit_mode = true;
    e.row = 7;
    e.field = Field::Pan;
    e
}
#[test]
fn generation_preview_appends_only_fresh_content_as_one_cursor_preserving_undo() {
    use aqua_pattern_core::generation;
    let mut e = generation_editor();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let plan = generation::preview(&base, generation_config()).unwrap();
    let repeated = generation::preview(&base, generation_config()).unwrap();
    assert_eq!(plan.candidate(), repeated.candidate());
    assert_eq!(plan.cells(), repeated.cells());
    assert_eq!(plan.provenance(), repeated.provenance());
    assert_eq!(plan.id(), repeated.id());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && !e.can_redo());
    let summary = plan.summary();
    assert_eq!(
        summary.pattern.0,
        base.patterns.keys().next_back().unwrap().0 + 1
    );
    assert_eq!(
        summary.track.0,
        base.tracks.keys().next_back().unwrap().0 + 1
    );
    assert_eq!(summary.occurrence, base.order.len());
    assert!(summary.hits > 0);
    assert_eq!(summary.hits, plan.cells().len());
    let candidate = plan.candidate().clone();
    assert_eq!(
        candidate.patterns[&summary.pattern].generation.as_ref(),
        Some(plan.provenance())
    );
    assert_eq!(
        generation::content_matches(&candidate.patterns[&summary.pattern]).unwrap(),
        Some(true)
    );
    assert_eq!(candidate.order[summary.occurrence].pattern, summary.pattern);
    assert!(candidate.order[summary.occurrence].muted_tracks.is_empty());
    // Removing only the advertised append restores every existing map, cell,
    // alias, mute, clock value and order record exactly; no holes were reused.
    let mut restored = candidate.clone();
    restored.patterns.remove(&summary.pattern).unwrap();
    restored.tracks.remove(&summary.track).unwrap();
    assert_eq!(restored.track_order.pop(), Some(summary.track));
    restored.order.pop();
    assert_eq!(restored, base);
    e.edit_mode = false;
    assert!(e.commit_generation_preview(revision, &plan).is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    e.edit_mode = true;
    e.commit_generation_preview(revision, &plan).unwrap();
    assert_eq!(e.song, candidate);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert_eq!(e.revision(), revision + 1);
    assert!(e.dirty && e.can_undo() && !e.can_redo());
    assert!(e.commit_generation_preview(e.revision(), &plan).is_err());
    assert_eq!(e.song, candidate);
    e.undo();
    assert_eq!(e.song, base);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, candidate);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
}
#[test]
fn generation_commit_rejects_unrelated_edits_aba_replacement_and_unchecked_drafts() {
    use aqua_pattern_core::generation;
    let mut e = generation_editor();
    let base = e.song.clone();
    let revision = e.revision();
    let plan = generation::preview(&base, generation_config()).unwrap();
    e.set_occurrence_track_mute(0, TrackId(1), true).unwrap();
    let changed = e.song.clone();
    let changed_revision = e.revision();
    assert!(e.commit_generation_preview(revision, &plan).is_err());
    assert!(e
        .commit_generation_preview(changed_revision, &plan)
        .is_err());
    assert_eq!(e.song, changed);
    assert_eq!(e.revision(), changed_revision);
    e.undo();
    assert_eq!(e.song, base);
    let aba_revision = e.revision();
    assert!(e.commit_generation_preview(revision, &plan).is_err());
    assert_eq!(e.revision(), aba_revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    let replaced_revision = e.revision();
    assert!(e.commit_generation_preview(revision, &plan).is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), replaced_revision);
    // Public draft changes cannot bypass the opaque plan's full-base guard.
    e.song.title.push_str(" unchecked draft");
    let draft = e.song.clone();
    assert!(e
        .commit_generation_preview(replaced_revision, &plan)
        .is_err());
    assert_eq!(e.song, draft);
    assert_eq!(e.revision(), replaced_revision);
    assert!(!e.can_undo() && !e.can_redo());
}
#[test]
fn generation_failures_and_checked_id_exhaustion_preserve_saved_redo_state() {
    use aqua_pattern_core::{
        generation::{self, GenerationPreset},
        song::Track,
    };
    let mut e = generation_editor();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut invalid = Vec::new();
    let mut config = generation_config();
    config.version = 2;
    invalid.push(config);
    let mut config = generation_config();
    config.bars = 0;
    invalid.push(config);
    let mut config = generation_config();
    config.instrument = InstrumentId(999);
    invalid.push(config);
    let mut config = generation_config();
    config.sequence.clear();
    invalid.push(config);
    let mut config = generation_config();
    config.sequence[0].velocity = f32::NAN;
    invalid.push(config);
    let mut config = generation_config();
    config.preset = GenerationPreset::Euclidean {
        steps: 7,
        pulses: 5,
        rotation: 2,
        probability: 0.0,
    };
    invalid.push(config);
    for config in invalid {
        assert!(generation::preview(&e.song, config).is_err());
        assert_eq!(e.song, base);
        assert_eq!(e.revision(), revision);
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
        assert!(!e.dirty && !e.can_undo() && e.can_redo());
    }
    for exhaust_track in [false, true] {
        let mut full = base.clone();
        if exhaust_track {
            full.tracks.insert(
                TrackId(u64::MAX),
                Track {
                    name: "Last track ID".into(),
                    note_columns: 1,
                },
            );
            full.track_order.push(TrackId(u64::MAX));
        } else {
            full.patterns
                .insert(PatternId(u64::MAX), full.patterns[&PatternId(1)].clone());
        }
        full.validate().unwrap();
        let bytes = serde_json::to_vec(&full).unwrap();
        let error = generation::preview(&full, generation_config()).unwrap_err();
        assert!(error.contains("ID space exhausted"), "{error}");
        assert_eq!(serde_json::to_vec(&full).unwrap(), bytes);
    }
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(e.can_redo());
}
#[test]
fn generated_notes_remain_ordinary_editable_and_historical_metadata_never_recreates_them() {
    use aqua_pattern_core::generation;
    let mut e = generation_editor();
    let base = e.song.clone();
    let plan = generation::preview(&base, generation_config()).unwrap();
    let summary = plan.summary().clone();
    let provenance = plan.provenance().clone();
    let first = plan.cells()[0].clone();
    e.commit_generation_preview(e.revision(), &plan).unwrap();
    let generated = e.song.clone();
    let mut independent = generated.clone();
    let clone_id = PatternId(summary.pattern.0 + 1);
    independent
        .clone_pattern(summary.pattern, clone_id)
        .unwrap();
    independent
        .patterns
        .get_mut(&clone_id)
        .unwrap()
        .name
        .push_str(" renamed");
    assert_eq!(
        independent.patterns[&clone_id].generation.as_ref(),
        Some(&provenance)
    );
    assert_eq!(
        generation::content_matches(&independent.patterns[&clone_id]).unwrap(),
        Some(true)
    );
    e.select(summary.occurrence, summary.track).unwrap();
    e.row = first.row;
    e.column = first.column;
    e.edit_step = 0;
    let mut edited_cell = e.cell();
    edited_cell.note = Note::Note(81);
    e.set_cell(edited_cell.clone()).unwrap();
    assert_eq!(e.cell(), edited_cell);
    assert_eq!(e.pattern().generation.as_ref(), Some(&provenance));
    assert_eq!(
        generation::content_matches(e.pattern()).unwrap(),
        Some(false)
    );
    e.undo();
    assert_eq!(e.song, generated);
    assert_eq!(
        generation::content_matches(e.pattern()).unwrap(),
        Some(true)
    );
    e.redo();
    assert_eq!(e.cell(), edited_cell);
    e.append(true).unwrap();
    let cloned = e.pattern_id();
    assert_ne!(cloned, summary.pattern);
    assert_eq!(e.pattern().generation.as_ref(), Some(&provenance));
    assert_eq!(e.pattern().tracks, e.song.patterns[&summary.pattern].tracks);
    assert_eq!(
        generation::content_matches(e.pattern()).unwrap(),
        Some(false)
    );
    e.resize(summary.rows + 1).unwrap();
    assert_eq!(e.pattern().generation.as_ref(), Some(&provenance));
    assert_eq!(e.song.patterns[&summary.pattern].rows, summary.rows);
    // Historical instrument IDs are informational. Once real cells are cleared,
    // removing their instrument must not be prohibited by the recipe alone.
    let mut cleared = e.song.clone();
    for pattern in cleared.patterns.values_mut() {
        pattern.tracks.clear();
        pattern.track_aliases.clear();
    }
    cleared.instruments.remove(&InstrumentId(1));
    e.apply_document_edit(cleared.clone()).unwrap();
    assert_eq!(e.song, cleared);
    assert_eq!(e.pattern().generation.as_ref(), Some(&provenance));
    assert_eq!(
        generation::content_matches(e.pattern()).unwrap(),
        Some(false)
    );
    e.undo();
    assert!(!e.pattern().tracks.is_empty());
    assert!(e.song.instruments.contains_key(&InstrumentId(1)));
}

#[test]
fn generation_first_append_initializes_empty_selection_and_undo_restores_empty_document() {
    use aqua_pattern_core::generation;
    let mut e = generation_editor();
    let mut empty = e.song.clone();
    empty.tracks.clear();
    empty.track_order.clear();
    empty.patterns.clear();
    empty.order.clear();
    e.replace_document(empty.clone()).unwrap();
    e.edit_mode = true;
    assert!(!e.has_grid());
    assert_eq!(e.track, TrackId(0));
    let plan = generation::preview(&empty, generation_config()).unwrap();
    assert_eq!(plan.summary().pattern, PatternId(1));
    assert_eq!(plan.summary().track, TrackId(1));
    assert_eq!(plan.summary().occurrence, 0);
    e.commit_generation_preview(e.revision(), &plan).unwrap();
    assert_eq!(e.song, *plan.candidate());
    assert!(e.has_grid());
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(1), 0, 0)
    );
    e.undo();
    assert_eq!(e.song, empty);
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(0), 0, 0)
    );
    assert!(!e.has_grid() && !e.dirty && !e.can_undo());
    e.redo();
    assert_eq!(e.song, *plan.candidate());
    assert!(e.has_grid());
    assert_eq!(
        (e.occurrence, e.track, e.column, e.row),
        (0, TrackId(1), 0, 0)
    );
}

#[test]
fn generation_context_budget_rejection_preserves_valid_large_arrangement_and_redo() {
    use aqua_pattern_core::{
        generation,
        song::{DrumKitId, DrumKitInstrument, Occurrence, Track},
    };
    let mut e = generation_editor();
    let mut song = e.song.clone();
    song.patterns.retain(|id, _| *id == PatternId(1));
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 1;
    pattern.tracks.clear();
    pattern.track_aliases.clear();
    for id in 5..=256 {
        song.tracks.insert(
            TrackId(id),
            Track {
                name: format!("Track {id}"),
                note_columns: 1,
            },
        );
        song.track_order.push(TrackId(id));
    }
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        };
        1024
    ];
    song.drum_kits.insert(
        DrumKitId(99),
        DrumKitInstrument {
            name: "Unused context fixture".into(),
            pads: Default::default(),
        },
    );
    song.assign_kit(InstrumentId(4), Some(DrumKitId(99)))
        .unwrap();
    song.validate().unwrap();
    e.replace_document(song.clone()).unwrap();
    e.edit_mode = true;
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let error = generation::preview(&e.song, generation_config()).unwrap_err();
    assert!(error.contains("65536"), "{error}");
    assert_eq!(e.song, song);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
}

fn region_playback_editor() -> Editor {
    use aqua_pattern_core::song::{RegionId, SampleId};
    let mut song = aqua_pattern_core::song::example_song();
    song.register_sample(SampleId(11), source_fixture())
        .unwrap();
    song.upsert_region(RegionId(21), region_fixture()).unwrap();
    for instrument in [InstrumentId(1), InstrumentId(2)] {
        song.assign_region(instrument, Some(RegionId(21))).unwrap();
    }
    let mut e = Editor::default();
    e.replace_document(song).unwrap();
    e.row = 3;
    e.field = Field::Pan;
    e
}

#[test]
fn shared_region_playback_is_protected_one_undo_and_explicit_none_is_not_default_envelope() {
    use aqua_pattern_core::song::{AmpEnvelope, ForwardLoop, RegionId};
    let mut e = region_playback_editor();
    let base = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let revision = e.revision();
    let envelope = Some(AmpEnvelope::default());
    let forward_loop = Some(ForwardLoop {
        start_frame: 24,
        end_frame: 120,
    });
    assert!(e
        .set_region_playback(RegionId(21), envelope, forward_loop)
        .is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    e.set_region_playback(RegionId(21), envelope, forward_loop)
        .unwrap();
    let mut expected = base.clone();
    let region = expected.regions.get_mut(&RegionId(21)).unwrap();
    region.amp_envelope = envelope;
    region.forward_loop = forward_loop;
    assert_eq!(
        e.song, expected,
        "only the shared region metadata changes, not its users"
    );
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let undone_revision = e.revision();
    e.set_region_playback(RegionId(21), None, None).unwrap();
    assert_eq!(e.revision(), undone_revision);
    assert!(
        e.can_redo(),
        "a no-op must not consume the prior playback edit"
    );
    e.redo();
    assert_eq!(e.song, expected);
    e.set_region_playback(RegionId(21), None, None).unwrap();
    assert_eq!(
        e.song, base,
        "Some(default) is opt-in; None explicitly restores legacy metadata"
    );
    e.undo();
    assert_eq!(e.song, expected);
}

#[test]
fn invalid_region_playback_ranges_and_unused_regions_preserve_clean_redo_and_cursor() {
    use aqua_pattern_core::song::{AmpEnvelope, ForwardLoop, RegionId};
    let mut e = region_playback_editor();
    e.edit_mode = true;
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    for envelope in [
        AmpEnvelope {
            attack_ms: 10_001,
            ..Default::default()
        },
        AmpEnvelope {
            decay_ms: 10_001,
            ..Default::default()
        },
        AmpEnvelope {
            sustain_percent: 101,
            ..Default::default()
        },
        AmpEnvelope {
            release_ms: 501,
            ..Default::default()
        },
    ] {
        assert!(e
            .set_region_playback(RegionId(21), Some(envelope), None)
            .is_err());
    }
    for (start_frame, end_frame) in [
        (11, 100),
        (12, 241),
        (12, 12),
        (12, 13),
        (120, 24),
        (u64::MAX, 0),
    ] {
        assert!(e
            .set_region_playback(
                RegionId(21),
                None,
                Some(ForwardLoop {
                    start_frame,
                    end_frame
                })
            )
            .is_err());
    }
    assert!(e.set_region_playback(RegionId(999), None, None).is_err());
    let mut invalid_unused = region_fixture();
    invalid_unused.amp_envelope = Some(AmpEnvelope {
        sustain_percent: 101,
        ..Default::default()
    });
    assert!(e.upsert_region(RegionId(22), invalid_unused).is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    // Inclusive envelope limits and an exact two-frame loop are valid, not clamped.
    e.set_region_playback(
        RegionId(21),
        Some(AmpEnvelope {
            attack_ms: 10_000,
            decay_ms: 10_000,
            sustain_percent: 0,
            release_ms: 500,
        }),
        Some(ForwardLoop {
            start_frame: 238,
            end_frame: 240,
        }),
    )
    .unwrap();
    assert_eq!(
        e.song.regions[&RegionId(21)]
            .amp_envelope
            .unwrap()
            .attack_ms,
        10_000
    );
    e.undo();
    assert_eq!(e.song, base);
}

#[test]
fn region_metadata_drafts_preserve_playback_and_incompatible_shrink_does_not_clear_or_clip_loop() {
    use aqua_pattern_core::song::{AmpEnvelope, ForwardLoop, RegionId};
    let mut e = region_playback_editor();
    e.edit_mode = true;
    let envelope = Some(AmpEnvelope {
        attack_ms: 7,
        decay_ms: 19,
        sustain_percent: 43,
        release_ms: 111,
    });
    let forward_loop = Some(ForwardLoop {
        start_frame: 24,
        end_frame: 120,
    });
    e.set_region_playback(RegionId(21), envelope, forward_loop)
        .unwrap();
    e.mark_saved();
    let base = e.song.clone();
    let mut draft = base.regions[&RegionId(21)].clone();
    draft.name = "Renamed shared region".into();
    draft.root_note = 61;
    draft.start_frame = 20;
    draft.end_frame = 200;
    e.upsert_region(RegionId(21), draft.clone()).unwrap();
    assert_eq!(e.song.regions[&RegionId(21)], draft);
    assert_eq!(draft.amp_envelope, envelope);
    assert_eq!(draft.forward_loop, forward_loop);
    let changed = e.song.clone();
    e.undo();
    assert_eq!(e.song, base);
    let revision = e.revision();
    for (start, end) in [(25, 240), (12, 119), (121, 240)] {
        let mut bad = base.regions[&RegionId(21)].clone();
        bad.start_frame = start;
        bad.end_frame = end;
        assert!(e.upsert_region(RegionId(21), bad).is_err());
    }
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && e.can_redo());
    e.redo();
    assert_eq!(e.song, changed);
}

#[test]
fn combined_region_playback_transaction_is_atomic_and_stale_draft_cannot_resurrect_settings() {
    use aqua_pattern_core::song::{AmpEnvelope, ForwardLoop, RegionId};
    let mut e = region_playback_editor();
    e.edit_mode = true;
    let base = e.song.clone();
    let revision = e.revision();
    let mut candidate = base.clone();
    let mut region = candidate.regions[&RegionId(21)].clone();
    region.name = "Combined boundaries and playback".into();
    region.start_frame = 30;
    region.end_frame = 180;
    region.root_note = 64;
    region.amp_envelope = Some(AmpEnvelope::default());
    region.forward_loop = Some(ForwardLoop {
        start_frame: 32,
        end_frame: 178,
    });
    candidate.upsert_region(RegionId(21), region).unwrap();
    let mut late_invalid = candidate.clone();
    late_invalid
        .regions
        .get_mut(&RegionId(21))
        .unwrap()
        .end_frame = 100;
    assert!(e
        .apply_document_edit_if_current(revision, &base, late_invalid)
        .is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo());
    e.apply_document_edit_if_current(revision, &base, candidate.clone())
        .unwrap();
    assert_eq!(e.song, candidate);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.can_undo() && !e.dirty && e.can_redo());
    let undone_revision = e.revision();
    assert!(
        e.apply_document_edit_if_current(revision, &base, candidate.clone())
            .is_err(),
        "Undo ABA must not revive an old inspector draft"
    );
    assert_eq!(e.revision(), undone_revision);
    assert_eq!(e.song, base);
    e.redo();
    assert_eq!(e.song, candidate);
}

fn velocity_zones_fixture() -> Vec<aqua_pattern_core::song::VelocityZone> {
    use aqua_pattern_core::song::{PadLayer, PadLayerId, RegionId, VelocityZone};
    vec![
        VelocityZone {
            min_velocity: 0,
            max_velocity: 63,
            alternatives: vec![
                PadLayer {
                    id: PadLayerId(11),
                    region: RegionId(21),
                },
                PadLayer {
                    id: PadLayerId(12),
                    region: RegionId(22),
                },
            ],
        },
        VelocityZone {
            min_velocity: 64,
            max_velocity: 127,
            alternatives: vec![PadLayer {
                id: PadLayerId(21),
                region: RegionId(21),
            }],
        },
    ]
}
fn velocity_zone_editor() -> Editor {
    use aqua_pattern_core::song::{DrumKitId, RegionId, SampleId};
    let mut e = drum_kit_editor();
    e.register_sample(SampleId(12), source_fixture()).unwrap();
    let mut alternate = region_fixture();
    alternate.sample = SampleId(12);
    alternate.name = "Quiet alternate".into();
    e.upsert_region(RegionId(22), alternate).unwrap();
    e.upsert_drum_kit(DrumKitId(41), drum_kit_fixture())
        .unwrap();
    e.assign_kit(InstrumentId(99), Some(DrumKitId(41))).unwrap();
    e.replace_document(e.song.clone()).unwrap();
    e.edit_mode = true;
    e.row = 4;
    e.field = Field::Velocity;
    e
}

#[test]
fn pad_source_modes_are_explicit_protected_single_undo_and_noop_preserves_saved_redo() {
    use aqua_pattern_core::song::{DrumKitId, PadId, RegionId};
    let mut e = velocity_zone_editor();
    let base = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let revision = e.revision();
    e.edit_mode = false;
    assert!(e
        .set_pad_sources(DrumKitId(41), PadId(501), None, velocity_zones_fixture())
        .is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    e.edit_mode = true;
    e.set_pad_sources(DrumKitId(41), PadId(501), None, velocity_zones_fixture())
        .unwrap();
    let mut expected = base.clone();
    let pad = expected
        .drum_kits
        .get_mut(&DrumKitId(41))
        .unwrap()
        .pads
        .get_mut(&PadId(501))
        .unwrap();
    pad.region = None;
    pad.velocity_zones = velocity_zones_fixture();
    assert_eq!(
        e.song, expected,
        "no other pad metadata, source or ordinary cell changes"
    );
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, expected);
    e.mark_saved();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let revision = e.revision();
    e.set_pad_sources(DrumKitId(41), PadId(501), None, velocity_zones_fixture())
        .unwrap();
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && e.can_redo());
    e.set_pad_sources(DrumKitId(41), PadId(501), Some(RegionId(21)), Vec::new())
        .unwrap();
    assert_eq!(
        e.song, base,
        "explicit single-region switch clears the zoned mapping only"
    );
    e.undo();
    assert_eq!(e.song, expected);
    e.set_pad_sources(DrumKitId(41), PadId(501), None, Vec::new())
        .unwrap();
    let pad = &e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)];
    assert!(
        !pad.has_source(),
        "explicit None plus empty zones unmaps an unused pad"
    );
    e.undo();
    assert_eq!(e.song, expected);
}

#[test]
fn malformed_velocity_zones_layers_and_limits_reject_atomically_even_when_unused() {
    use aqua_pattern_core::song::{DrumKitId, PadId, PadLayer, PadLayerId, RegionId, VelocityZone};
    let mut e = velocity_zone_editor();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut invalid = Vec::new();
    let valid = velocity_zones_fixture();
    for change in [
        |z: &mut Vec<VelocityZone>| z[0].min_velocity = 1,
        |z: &mut Vec<VelocityZone>| z[1].max_velocity = 126,
        |z: &mut Vec<VelocityZone>| z[1].max_velocity = 128,
        |z: &mut Vec<VelocityZone>| z[1].min_velocity = 65,
        |z: &mut Vec<VelocityZone>| z[1].min_velocity = 63,
        |z: &mut Vec<VelocityZone>| z.swap(0, 1),
        |z: &mut Vec<VelocityZone>| z[0].alternatives.clear(),
        |z: &mut Vec<VelocityZone>| z[0].alternatives[1].id = PadLayerId(0),
        |z: &mut Vec<VelocityZone>| z[1].alternatives[0].id = PadLayerId(11),
        |z: &mut Vec<VelocityZone>| z[0].alternatives[1].region = RegionId(999),
    ] {
        let mut zones = valid.clone();
        change(&mut zones);
        invalid.push(zones);
    }
    invalid.push(vec![VelocityZone {
        min_velocity: 0,
        max_velocity: 127,
        alternatives: (1..=9)
            .map(|id| PadLayer {
                id: PadLayerId(id),
                region: RegionId(21),
            })
            .collect(),
    }]);
    invalid.push(
        (0..16)
            .map(|i| VelocityZone {
                min_velocity: i * 8,
                max_velocity: i * 8 + 7,
                alternatives: vec![PadLayer {
                    id: PadLayerId(u64::from(i) + 1),
                    region: RegionId(21),
                }],
            })
            .collect(),
    );
    invalid.push(
        (0..8)
            .map(|i| VelocityZone {
                min_velocity: i * 16,
                max_velocity: i * 16 + 15,
                alternatives: (0..5)
                    .map(|j| PadLayer {
                        id: PadLayerId(u64::from(i) * 5 + j + 1),
                        region: RegionId(21),
                    })
                    .collect(),
            })
            .collect(),
    );
    for zones in invalid {
        assert!(e
            .set_pad_sources(DrumKitId(41), PadId(501), None, zones)
            .is_err());
        assert_eq!(e.song, base);
        assert_eq!(e.revision(), revision);
    }
    assert!(e
        .set_pad_sources(DrumKitId(41), PadId(501), Some(RegionId(21)), valid.clone())
        .is_err());
    assert!(e
        .set_pad_sources(DrumKitId(999), PadId(501), None, valid.clone())
        .is_err());
    assert!(e
        .set_pad_sources(DrumKitId(41), PadId(999), None, valid)
        .is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    // All three capacity boundaries are accepted without truncation: 8 zones,
    // 32 alternatives in total, including 8 in the first zone.
    let lengths = [8, 4, 4, 4, 3, 3, 3, 3];
    let mut next_id = 1;
    let zones = lengths
        .into_iter()
        .enumerate()
        .map(|(i, length)| VelocityZone {
            min_velocity: (i * 16) as u8,
            max_velocity: (i * 16 + 15) as u8,
            alternatives: (0..length)
                .map(|_| {
                    let id = next_id;
                    next_id += 1;
                    PadLayer {
                        id: PadLayerId(id),
                        region: RegionId(21),
                    }
                })
                .collect(),
        })
        .collect();
    e.set_pad_sources(DrumKitId(41), PadId(501), None, zones)
        .unwrap();
    assert_eq!(
        e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)]
            .velocity_zones
            .iter()
            .map(|z| z.alternatives.len())
            .sum::<usize>(),
        32
    );
    let mut independent = e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)].clone();
    independent.slot = 1;
    independent.note = 37;
    e.upsert_drum_pad(DrumKitId(41), PadId(502), independent)
        .unwrap();
    assert_eq!(
        e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)].velocity_zones,
        e.song.drum_kits[&DrumKitId(41)].pads[&PadId(502)].velocity_zones,
        "layer IDs are scoped to one pad, not a global uniqueness constraint"
    );
}

#[test]
fn zoned_pad_metadata_and_every_alternative_reference_survive_ordinary_edits_and_undo() {
    use aqua_pattern_core::song::{DrumKitId, PadId, RegionId, SampleId};
    let mut e = velocity_zone_editor();
    e.set_pad_sources(DrumKitId(41), PadId(501), None, velocity_zones_fixture())
        .unwrap();
    e.mark_saved();
    let zoned = e.song.clone();
    assert!(
        e.remove_region(RegionId(22)).is_err(),
        "an unplayed, nonfirst alternative still holds a reference"
    );
    assert!(e.remove_sample(SampleId(12)).is_err());
    let mut metadata = e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)].clone();
    metadata.name = "Renamed zoned pad".into();
    metadata.slot = 7;
    metadata.tune_semitones = 3;
    metadata.pan = 0.25;
    e.upsert_drum_pad(DrumKitId(41), PadId(501), metadata.clone())
        .unwrap();
    assert_eq!(e.song.drum_kits[&DrumKitId(41)].pads[&PadId(501)], metadata);
    assert_eq!(metadata.velocity_zones, velocity_zones_fixture());
    e.undo();
    assert_eq!(e.song, zoned);
    e.edit_step = 1;
    let previous_row = e.row;
    let before_cell = e.cell();
    e.insert_pad_note(InstrumentId(99), PadId(501)).unwrap();
    assert_eq!(e.row, previous_row + 1);
    e.row = previous_row;
    let cell = e.cell();
    assert_eq!(cell.note, Note::Note(36));
    assert_eq!(cell.instrument, Some(InstrumentId(99)));
    assert_eq!(cell.velocity, before_cell.velocity);
    assert_eq!(cell.pan, before_cell.pan);
    assert_eq!(cell.delay, before_cell.delay);
    assert_eq!(cell.effects, before_cell.effects);
    let played = e.song.clone();
    let revision = e.revision();
    assert!(e
        .set_pad_sources(DrumKitId(41), PadId(501), None, Vec::new())
        .is_err());
    assert!(e.remove_drum_pad(DrumKitId(41), PadId(501)).is_err());
    assert!(e.remove_region(RegionId(22)).is_err());
    assert_eq!(e.song, played);
    assert_eq!(e.revision(), revision);
    e.undo();
    assert_eq!(e.song, zoned);
    // Explicitly rewriting every mapping and deleting the now-unreferenced
    // alternate source is one complete candidate, never partial live deletion.
    let revision = e.revision();
    let mut candidate = zoned.clone();
    candidate
        .set_pad_sources(DrumKitId(41), PadId(501), Some(RegionId(21)), Vec::new())
        .unwrap();
    candidate.remove_region(RegionId(22)).unwrap();
    candidate.remove_sample(SampleId(12)).unwrap();
    e.apply_document_edit_if_current(revision, &zoned, candidate.clone())
        .unwrap();
    assert_eq!(e.song, candidate);
    e.undo();
    assert_eq!(e.song, zoned);
    assert!(
        e.apply_document_edit_if_current(revision, &zoned, candidate.clone())
            .is_err(),
        "stale mapping drafts cannot resurrect deleted sources after ABA"
    );
    e.redo();
    assert_eq!(e.song, candidate);
}

fn groove_capture_config() -> aqua_pattern_core::groove::CaptureConfig {
    aqua_pattern_core::groove::CaptureConfig {
        version: 1,
        name: "Explicit accepted marker timing".into(),
        marker_set: aqua_pattern_core::song::MarkerSetId(31),
        start_marker: 0,
        end_marker: 4,
        include_start_anchor: true,
        division: aqua_pattern_core::break_compiler::Subdivision::Sixteenth,
        cycle_steps: 4,
    }
}
fn groove_apply_config(strength_percent: u8) -> aqua_pattern_core::groove::ApplyConfig {
    aqua_pattern_core::groove::ApplyConfig {
        groove: aqua_pattern_core::song::GrooveId(9),
        scope: aqua_pattern_core::transforms::TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 4,
            columns: vec![0],
        },
        strength_percent,
    }
}
fn groove_editor() -> Editor {
    use aqua_pattern_core::song::{MarkerSet, MarkerSetId, Occurrence, SampleId};
    let mut song = aqua_pattern_core::song::example_song();
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
                        pan: Some(-0.25),
                        delay: if row == 0 { Some(0.0) } else { None },
                        effects: if row == 1 {
                            StepEffects {
                                release: Some(0.5),
                                ..Default::default()
                            }
                        } else {
                            StepEffects::default()
                        },
                        ..Default::default()
                    },
                )]
                .into(),
            );
    }
    let mut source = source_fixture();
    source.path = "/__groove_capture_model_only_missing__.wav".into();
    source.sample_rate = 8_000;
    source.channels = 1;
    source.frames = 8_000;
    song.register_sample(SampleId(11), source).unwrap();
    song.upsert_marker_set(
        MarkerSetId(31),
        MarkerSet {
            name: "Reviewed user anchors".into(),
            sample: SampleId(11),
            source_bpm: Some(120.0),
            markers: vec![0, 1_100, 2_000, 3_100, 4_000, 8_000],
            locked_frames: vec![1_100],
        },
    )
    .unwrap();
    let mut e = Editor::default();
    e.replace_document(song).unwrap();
    e.row = 6;
    e.field = Field::Pan;
    e
}
fn editor_with_groove() -> Editor {
    use aqua_pattern_core::{groove, song::GrooveId};
    let mut e = groove_editor();
    let capture = groove::capture_preview(&e.song, GrooveId(9), groove_capture_config()).unwrap();
    e.edit_mode = true;
    e.commit_groove_capture_preview(e.revision(), &capture)
        .unwrap();
    e.replace_document(e.song.clone()).unwrap();
    e.edit_mode = true;
    e
}

#[test]
fn groove_capture_registers_only_reviewed_metadata_as_one_protected_cursor_preserving_undo() {
    use aqua_pattern_core::{groove, song::GrooveId};
    let mut e = groove_editor();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let preview = groove::capture_preview(&base, GrooveId(9), groove_capture_config()).unwrap();
    assert_eq!(
        e.song, base,
        "capture is pure and requires no access to the missing source path"
    );
    assert!(e.commit_groove_capture_preview(revision, &preview).is_err());
    assert!(e.remove_groove(GrooveId(9)).is_err());
    assert_eq!(e.revision(), revision);
    assert!(!e.can_undo() && !e.dirty);
    e.edit_mode = true;
    e.commit_groove_capture_preview(revision, &preview).unwrap();
    let mut expected = base.clone();
    expected
        .grooves
        .insert(GrooveId(9), preview.template().clone());
    assert_eq!(e.song, expected);
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let undone_revision = e.revision();
    assert!(
        e.commit_groove_capture_preview(revision, &preview).is_err(),
        "ABA cannot revive an earlier reviewed capture"
    );
    assert_eq!(e.revision(), undone_revision);
    e.redo();
    assert_eq!(e.song, expected);
}

#[test]
fn groove_ids_are_immutable_across_batches_and_noops_preserve_saved_redo() {
    use aqua_pattern_core::song::GrooveId;
    let mut e = editor_with_groove();
    let base = e.song.clone();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let revision = e.revision();
    let mut changed = base.clone();
    changed.grooves.get_mut(&GrooveId(9)).unwrap().name = "Different captured authority".into();
    changed.validate().unwrap();
    assert!(e
        .apply_document_edit(changed.clone())
        .unwrap_err()
        .contains("immutable"));
    let mut reregistered = base.clone();
    reregistered.remove_groove(GrooveId(9)).unwrap();
    reregistered
        .register_groove(GrooveId(9), changed.grooves[&GrooveId(9)].clone())
        .unwrap();
    assert!(e.apply_document_edit(reregistered).is_err());
    assert!(e.remove_groove(GrooveId(999)).is_err());
    let mut same = base.clone();
    same.register_groove(GrooveId(9), base.grooves[&GrooveId(9)].clone())
        .unwrap();
    e.apply_document_edit(same).unwrap();
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    // Opening a different valid document is not an in-document identity edit.
    e.replace_document(changed.clone()).unwrap();
    assert_eq!(e.song, changed);
    assert!(!e.dirty && !e.can_undo() && !e.can_redo());
}

#[test]
fn groove_apply_bakes_only_reviewed_timing_and_zero_strength_keeps_bits_and_redo() {
    use aqua_pattern_core::{groove, song::GrooveId};
    let mut e = editor_with_groove();
    let base = e.song.clone();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let revision = e.revision();
    let preview = groove::preview(&base, groove_apply_config(100)).unwrap();
    assert_eq!(e.song, base);
    let expected = preview.candidate().clone();
    assert_ne!(expected, base);
    assert_eq!(expected.grooves, base.grooves);
    assert_eq!(expected.marker_sets, base.marker_sets);
    assert_eq!(expected.samples, base.samples);
    assert_eq!(expected.order, base.order);
    assert_eq!(
        expected.patterns[&PatternId(2)],
        base.patterns[&PatternId(2)]
    );
    for row in 0..4 {
        let old = &base.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0];
        let new = &expected.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0];
        assert_eq!(
            (new.note, new.instrument, new.velocity, new.pan),
            (old.note, old.instrument, old.velocity, old.pan)
        );
    }
    let moved = &expected.patterns[&PatternId(1)].tracks[&TrackId(1)][&1][&0];
    assert_eq!((moved.delay.unwrap() * 6000.0).round() as u64, 600);
    assert_eq!(
        (moved.effects.release.unwrap() * 6000.0).round() as u64,
        3600
    );
    e.edit_mode = false;
    assert!(e.commit_groove_preview(revision, &preview).is_err());
    e.edit_mode = true;
    e.commit_groove_preview(revision, &preview).unwrap();
    assert_eq!(e.song, expected);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let noop = groove::preview(&e.song, groove_apply_config(0)).unwrap();
    let revision = e.revision();
    e.commit_groove_preview(revision, &noop).unwrap();
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, expected);
    e.remove_groove(GrooveId(9)).unwrap();
    assert_eq!(
        e.song.patterns, expected.patterns,
        "baked cells do not retain a live template reference"
    );
    e.undo();
    assert_eq!(e.song, expected);
    let mut tiny = base.clone();
    tiny.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .delay = Some(1e-300);
    e.replace_document(tiny.clone()).unwrap();
    e.edit_mode = true;
    let revision = e.revision();
    let noop = groove::preview(&tiny, groove_apply_config(0)).unwrap();
    e.commit_groove_preview(revision, &noop).unwrap();
    assert_eq!(e.song, tiny);
    assert_eq!(e.revision(), revision);
    assert_eq!(
        e.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0]
            .delay
            .unwrap()
            .to_bits(),
        1e-300f64.to_bits()
    );
    assert!(!e.dirty && !e.can_undo());
}

#[test]
fn groove_previews_reject_changed_full_base_replacement_and_undo_aba() {
    use aqua_pattern_core::{groove, song::GrooveId};
    let mut e = groove_editor();
    e.edit_mode = true;
    let base = e.song.clone();
    let revision = e.revision();
    let capture = groove::capture_preview(&base, GrooveId(9), groove_capture_config()).unwrap();
    e.set_timing(121.0, 4).unwrap();
    let changed = e.song.clone();
    assert!(
        e.commit_groove_capture_preview(e.revision(), &capture)
            .is_err(),
        "full base must reject even with a current supplied revision"
    );
    assert_eq!(e.song, changed);
    e.undo();
    assert!(e.commit_groove_capture_preview(revision, &capture).is_err());
    e.replace_document(base).unwrap();
    e.edit_mode = true;
    assert!(e.commit_groove_capture_preview(revision, &capture).is_err());
    let mut e = editor_with_groove();
    let base = e.song.clone();
    let revision = e.revision();
    let preview = groove::preview(&base, groove_apply_config(100)).unwrap();
    e.set_occurrence_track_mute(0, TrackId(2), true).unwrap();
    let changed = e.song.clone();
    assert!(
        e.commit_groove_preview(e.revision(), &preview).is_err(),
        "unrelated mute change invalidates the whole reviewed plan"
    );
    assert_eq!(e.song, changed);
    e.undo();
    let current_revision = e.revision();
    assert!(e.commit_groove_preview(revision, &preview).is_err());
    assert_eq!(e.revision(), current_revision);
    assert_eq!(e.song, base);
    assert!(!e.dirty && e.can_redo());
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    assert!(e.commit_groove_preview(revision, &preview).is_err());
    assert_eq!(e.song, base);
}

#[test]
fn invalid_groove_capture_and_apply_never_mutate_or_materialize_alias_targets() {
    use aqua_pattern_core::{
        groove,
        song::{GrooveId, MarkerSetId, TrackAlias},
    };
    let mut e = editor_with_groove();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let revision = e.revision();
    for change in [
        |c: &mut groove::CaptureConfig| c.version = 2,
        |c: &mut groove::CaptureConfig| c.marker_set = MarkerSetId(999),
        |c: &mut groove::CaptureConfig| c.end_marker = 0,
        |c: &mut groove::CaptureConfig| c.cycle_steps = 0,
        |c: &mut groove::CaptureConfig| c.cycle_steps = 1025,
    ] {
        let mut config = groove_capture_config();
        change(&mut config);
        assert!(groove::capture_preview(&base, GrooveId(10), config).is_err());
    }
    for change in [
        |c: &mut groove::ApplyConfig| c.groove = GrooveId(999),
        |c: &mut groove::ApplyConfig| c.strength_percent = 101,
        |c: &mut groove::ApplyConfig| c.scope.row_end = 0,
        |c: &mut groove::ApplyConfig| c.scope.columns = vec![0, 0],
    ] {
        let mut config = groove_apply_config(100);
        change(&mut config);
        assert!(groove::preview(&base, config).is_err());
    }
    assert!(groove::capture_preview(&base, GrooveId(0), groove_capture_config()).is_err());
    assert!(groove::capture_preview(&base, GrooveId(9), groove_capture_config()).is_err());
    let mut exhausted = base.clone();
    exhausted
        .grooves
        .insert(GrooveId(u64::MAX), base.grooves[&GrooveId(9)].clone());
    assert!(
        exhausted.next_groove_id().is_err(),
        "fresh IDs do not reuse holes after overflow"
    );
    let mut full = base.clone();
    for id in 1..=64 {
        full.grooves
            .insert(GrooveId(id), base.grooves[&GrooveId(9)].clone());
    }
    full.validate().unwrap();
    assert!(groove::capture_preview(&full, GrooveId(65), groove_capture_config()).is_err());
    let mut unknown_tempo = base.clone();
    unknown_tempo
        .marker_sets
        .get_mut(&MarkerSetId(31))
        .unwrap()
        .source_bpm = None;
    assert!(
        groove::capture_preview(&unknown_tempo, GrooveId(10), groove_capture_config()).is_err()
    );
    let mut duplicate_tick = base.clone();
    duplicate_tick
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
        .delay = Some(0.6);
    assert!(groove::preview(&duplicate_tick, groove_apply_config(100)).is_err());
    let mut excluded_end = base.clone();
    excluded_end
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&3)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .delay = Some(0.9);
    assert!(groove::preview(&excluded_end, groove_apply_config(100)).is_err());
    let mut alias = base.clone();
    alias.patterns.get_mut(&PatternId(2)).unwrap().rows = 8;
    alias
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .remove(&TrackId(1));
    alias
        .set_track_alias(
            PatternId(1),
            TrackId(1),
            Some(TrackAlias {
                pattern: PatternId(2),
                track: TrackId(1),
            }),
        )
        .unwrap();
    assert!(groove::preview(&alias, groove_apply_config(0)).is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
}

fn melodic_map_fixture() -> aqua_pattern_core::song::MultisampleInstrument {
    use aqua_pattern_core::song::{KeyZone, KeyZoneId, MultisampleInstrument, RegionId};
    MultisampleInstrument {
        name: "Shared melodic map".into(),
        zones: [
            (
                KeyZoneId(41),
                KeyZone {
                    name: "Soft".into(),
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
                    name: "Hard".into(),
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
fn melodic_editor() -> Editor {
    use aqua_pattern_core::song::{MultisampleId, RegionId};
    let mut e = drum_kit_editor();
    let mut candidate = e.song.clone();
    let mut second = region_fixture();
    second.name = "Second region".into();
    second.start_frame = 240;
    second.end_frame = 480;
    candidate.upsert_region(RegionId(22), second).unwrap();
    candidate
        .upsert_multisample(MultisampleId(51), melodic_map_fixture())
        .unwrap();
    e.replace_document(candidate).unwrap();
    e.edit_mode = true;
    e.row = 7;
    e.field = Field::Pan;
    e
}

#[test]
fn fresh_multisample_creation_preserves_occupied_song_and_cursor_as_one_undo() {
    use aqua_pattern_core::song::{InstrumentKind, MultisampleId};
    let mut e = Editor::default();
    let before = e.song.clone();
    assert!(e
        .create_multisample_instrument("New melodic map".into())
        .is_err());
    assert_eq!(e.song, before);
    assert!(!e.dirty && !e.can_undo());
    e.edit_mode = true;
    e.row = 9;
    e.field = Field::Delay;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let revision = e.revision();
    let (instrument, map) = e
        .create_multisample_instrument("New melodic map".into())
        .unwrap();
    assert_eq!((instrument, map), (InstrumentId(5), MultisampleId(1)));
    assert_eq!(
        e.song.instruments[&instrument].kind,
        InstrumentKind::Multisample { map }
    );
    assert!(e.song.instruments[&instrument].sample.is_none());
    assert!(e.song.instruments[&instrument].region.is_none());
    assert!(e.song.multisamples[&map].zones.is_empty());
    let created = e.song.clone();
    let mut removed = created.clone();
    removed.instruments.remove(&instrument);
    removed.multisamples.remove(&map);
    assert_eq!(
        removed, before,
        "creation cannot replace occupied notes/assets/mappings"
    );
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, before);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.redo();
    assert_eq!(e.song, created);
}

#[test]
fn whole_keyzone_edits_are_shared_protected_atomic_and_noop_preserves_redo() {
    use aqua_pattern_core::song::{InstrumentKind, KeyZoneId, MultisampleId};
    let mut e = melodic_editor();
    e.assign_multisample(InstrumentId(1), Some(MultisampleId(51)))
        .unwrap();
    e.assign_multisample(InstrumentId(99), Some(MultisampleId(51)))
        .unwrap();
    let base = e.song.clone();
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    e.row = 7;
    e.field = Field::Pan;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut zones = base.multisamples[&MultisampleId(51)].zones.clone();
    zones.get_mut(&KeyZoneId(41)).unwrap().velocity_max = 79;
    assert!(
        e.set_keyzones(MultisampleId(51), zones.clone()).is_err(),
        "intermediate overlap must not publish"
    );
    assert_eq!(e.song, base);
    zones.get_mut(&KeyZoneId(17)).unwrap().velocity_min = 80;
    e.edit_mode = false;
    assert!(e.set_keyzones(MultisampleId(51), zones.clone()).is_err());
    assert!(e
        .upsert_multisample(MultisampleId(52), melodic_map_fixture())
        .is_err());
    assert!(e.remove_multisample(MultisampleId(51)).is_err());
    assert!(e.assign_multisample(InstrumentId(99), None).is_err());
    e.edit_mode = true;
    e.set_keyzones(MultisampleId(51), zones.clone()).unwrap();
    let after = e.song.clone();
    let mut expected = base.clone();
    expected
        .multisamples
        .get_mut(&MultisampleId(51))
        .unwrap()
        .zones = zones;
    assert_eq!(after, expected);
    for instrument in [InstrumentId(1), InstrumentId(99)] {
        assert_eq!(
            e.song.instruments[&instrument].kind,
            InstrumentKind::Multisample {
                map: MultisampleId(51)
            }
        );
    }
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.undo();
    assert_eq!(e.song, base);
    let revision = e.revision();
    e.set_keyzones(
        MultisampleId(51),
        base.multisamples[&MultisampleId(51)].zones.clone(),
    )
    .unwrap();
    e.upsert_multisample(
        MultisampleId(51),
        base.multisamples[&MultisampleId(51)].clone(),
    )
    .unwrap();
    e.assign_multisample(InstrumentId(1), Some(MultisampleId(51)))
        .unwrap();
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(e.song, after);
}

#[test]
fn malformed_keyzones_and_creation_limits_reject_without_history_or_id_reuse() {
    use aqua_pattern_core::song::{InstrumentKind, KeyZoneId, MultisampleId, RegionId};
    let mut e = melodic_editor();
    e.set_timing(123.0, 4).unwrap();
    e.undo();
    let base = e.song.clone();
    let revision = e.revision();
    let changes: [fn(&mut aqua_pattern_core::song::MultisampleInstrument); 14] = [
        |m: &mut aqua_pattern_core::song::MultisampleInstrument| m.name = " ".into(),
        |m| m.name = "é".repeat(129),
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().name = " ".into(),
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().name = "é".repeat(65),
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().note_min = 128,
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().note_max = 128,
        |m| {
            let z = m.zones.get_mut(&KeyZoneId(41)).unwrap();
            z.note_min = 70;
            z.note_max = 69;
        },
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().velocity_min = 128,
        |m| m.zones.get_mut(&KeyZoneId(17)).unwrap().velocity_max = 128,
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().velocity_min = 64,
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().root_note = 128,
        |m| m.zones.get_mut(&KeyZoneId(41)).unwrap().region = RegionId(999),
        |m| m.zones.get_mut(&KeyZoneId(17)).unwrap().velocity_min = 63,
        |m| {
            let zone = m.zones.remove(&KeyZoneId(41)).unwrap();
            m.zones.insert(KeyZoneId(0), zone);
        },
    ];
    for change in changes {
        let mut map = melodic_map_fixture();
        change(&mut map);
        assert!(e.upsert_multisample(MultisampleId(51), map).is_err());
        assert_eq!(e.song, base);
    }
    assert!(e
        .upsert_multisample(MultisampleId(0), melodic_map_fixture())
        .is_err());
    assert!(e
        .set_keyzones(MultisampleId(999), Default::default())
        .is_err());
    assert!(e
        .assign_multisample(InstrumentId(999), Some(MultisampleId(51)))
        .is_err());
    assert!(e
        .assign_multisample(InstrumentId(99), Some(MultisampleId(999)))
        .is_err());
    assert!(e.remove_multisample(MultisampleId(999)).is_err());
    for name in ["".to_owned(), "  ".to_owned(), "é".repeat(129)] {
        assert!(e.create_multisample_instrument(name).is_err());
    }
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let mut full = melodic_map_fixture();
    full.zones.clear();
    let template = melodic_map_fixture().zones[&KeyZoneId(41)].clone();
    for note in 0..128u8 {
        let mut zone = template.clone();
        zone.note_min = note;
        zone.note_max = note;
        zone.velocity_max = 127;
        full.zones.insert(KeyZoneId(u64::from(note) + 1), zone);
    }
    e.set_keyzones(MultisampleId(51), full.zones.clone())
        .unwrap();
    full.zones.insert(KeyZoneId(129), template);
    assert!(e.set_keyzones(MultisampleId(51), full.zones).is_err());
    for exhaust_map in [false, true] {
        let mut candidate = base.clone();
        if exhaust_map {
            candidate
                .multisamples
                .insert(MultisampleId(u64::MAX), melodic_map_fixture());
        } else {
            let mut instrument = candidate.instruments[&InstrumentId(99)].clone();
            instrument.kind = InstrumentKind::Pitched;
            candidate
                .instruments
                .insert(InstrumentId(u64::MAX), instrument);
        }
        e.replace_document(candidate.clone()).unwrap();
        e.edit_mode = true;
        assert!(e
            .create_multisample_instrument("No hole reuse".into())
            .is_err());
        assert_eq!(e.song, candidate);
        assert!(!e.dirty && !e.can_undo());
    }
    let mut cap = base;
    cap.multisamples.clear();
    for id in 1..=64 {
        let mut map = melodic_map_fixture();
        map.zones.clear();
        cap.multisamples.insert(MultisampleId(id), map);
    }
    e.replace_document(cap.clone()).unwrap();
    e.edit_mode = true;
    assert!(e
        .create_multisample_instrument("Beyond cap".into())
        .is_err());
    assert!(e
        .upsert_multisample(MultisampleId(65), melodic_map_fixture())
        .is_err());
    assert_eq!(e.song, cap);
    // Aggregate count is independent of the per-map/map-count limits.
    cap.multisamples.clear();
    let mut full = melodic_map_fixture();
    full.zones.clear();
    for note in 0..128u8 {
        let mut zone = melodic_map_fixture().zones[&KeyZoneId(41)].clone();
        zone.note_min = note;
        zone.note_max = note;
        zone.velocity_max = 127;
        full.zones.insert(KeyZoneId(u64::from(note) + 1), zone);
    }
    for id in 1..=32 {
        cap.multisamples.insert(MultisampleId(id), full.clone());
    }
    e.replace_document(cap.clone()).unwrap();
    e.edit_mode = true;
    let revision = e.revision();
    assert!(e
        .upsert_multisample(MultisampleId(33), melodic_map_fixture())
        .is_err());
    assert_eq!(e.song, cap);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo());
}

#[test]
fn unused_keyzone_references_are_protected_and_explicit_kind_switches_keep_legacy_policy() {
    use aqua_pattern_core::song::{DrumKitId, InstrumentKind, MultisampleId, RegionId, SampleId};
    let mut e = melodic_editor();
    let base = e.song.clone();
    assert!(
        e.remove_region(RegionId(22)).is_err(),
        "unused hard zone still owns its source reference"
    );
    assert!(e.remove_sample(SampleId(11)).is_err());
    assert_eq!(e.song, base);
    e.assign_multisample(InstrumentId(99), Some(MultisampleId(51)))
        .unwrap();
    assert!(e.remove_multisample(MultisampleId(51)).is_err());
    let assigned = e.song.clone();
    e.assign_sample(InstrumentId(99), None).unwrap();
    e.assign_region(InstrumentId(99), None).unwrap();
    assert_eq!(
        e.song, assigned,
        "old nullable path/region edits must not erase the new kind"
    );
    e.set_keyzones(MultisampleId(51), Default::default())
        .unwrap();
    assert!(
        e.song.multisamples[&MultisampleId(51)].zones.is_empty(),
        "storage permits explicit uncovered maps; playback reports coverage"
    );
    e.undo();
    assert_eq!(e.song, assigned);
    e.assign_multisample(InstrumentId(99), None).unwrap();
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].kind,
        InstrumentKind::Pitched
    );
    e.assign_region(InstrumentId(99), Some(RegionId(21)))
        .unwrap();
    let pitched = e.song.clone();
    let revision = e.revision();
    e.assign_multisample(InstrumentId(99), None).unwrap();
    assert_eq!(e.song, pitched);
    assert_eq!(e.revision(), revision);
    e.upsert_drum_kit(DrumKitId(61), drum_kit_fixture())
        .unwrap();
    e.assign_kit(InstrumentId(99), Some(DrumKitId(61))).unwrap();
    let kit = e.song.clone();
    e.assign_multisample(InstrumentId(99), None).unwrap();
    assert_eq!(
        e.song, kit,
        "new None detaches only Multisample, never an unrelated kit"
    );
    e.assign_multisample(InstrumentId(99), Some(MultisampleId(51)))
        .unwrap();
    assert!(e.song.instruments[&InstrumentId(99)].sample.is_none());
    assert!(e.song.instruments[&InstrumentId(99)].region.is_none());
    e.assign_kit(InstrumentId(99), None).unwrap();
    assert_eq!(
        e.song.instruments[&InstrumentId(99)].kind,
        InstrumentKind::Pitched,
        "legacy assign_kit(None) reset remains unchanged"
    );
    e.remove_multisample(MultisampleId(51)).unwrap();
    e.remove_region(RegionId(22)).unwrap();
}

#[test]
fn independent_melodic_copy_batch_and_stale_drafts_preserve_exact_undo_and_refs() {
    use aqua_pattern_core::song::{KeyZoneId, MultisampleId};
    let mut e = melodic_editor();
    e.assign_multisample(InstrumentId(99), Some(MultisampleId(51)))
        .unwrap();
    let base = e.song.clone();
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    e.row = 8;
    e.field = Field::Velocity;
    let revision = e.revision();
    let mut candidate = base.clone();
    candidate
        .upsert_multisample(
            MultisampleId(52),
            base.multisamples[&MultisampleId(51)].clone(),
        )
        .unwrap();
    candidate
        .assign_multisample(InstrumentId(99), Some(MultisampleId(52)))
        .unwrap();
    let map = candidate.multisamples.get_mut(&MultisampleId(52)).unwrap();
    map.name = "Independent map metadata".into();
    map.zones.get_mut(&KeyZoneId(17)).unwrap().root_note = 65;
    assert_eq!(
        candidate.multisamples[&MultisampleId(51)],
        base.multisamples[&MultisampleId(51)]
    );
    assert_eq!(candidate.samples, base.samples);
    assert_eq!(candidate.regions, base.regions);
    assert_eq!(candidate.patterns, base.patterns);
    e.apply_document_edit_if_current(revision, &base, candidate.clone())
        .unwrap();
    assert_eq!(e.row, 8);
    assert_eq!(e.field, Field::Velocity);
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let current_revision = e.revision();
    assert!(
        e.apply_document_edit_if_current(revision, &base, candidate.clone())
            .is_err(),
        "ABA must not accept an old full-value map draft"
    );
    let mut invalid = candidate.clone();
    invalid.regions.clear();
    assert!(e
        .apply_document_edit_if_current(current_revision, &base, invalid)
        .is_err());
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), current_revision);
    assert!(e.can_redo() && !e.dirty);
    e.redo();
    assert_eq!(e.song, candidate);
    let stale = e.song.clone();
    let stale_revision = e.revision();
    e.assign_multisample(InstrumentId(99), None).unwrap();
    e.remove_multisample(MultisampleId(52)).unwrap();
    let removed = e.song.clone();
    assert!(e
        .apply_document_edit_if_current(stale_revision, &stale, candidate)
        .is_err());
    assert_eq!(
        e.song, removed,
        "old map draft cannot resurrect a removed ID"
    );
}

fn tempo_marker_editor() -> Editor {
    use aqua_pattern_core::song::{MarkerSet, MarkerSetId, SampleId};
    let mut song = groove_editor().song;
    song.bpm = 123.5;
    song.lines_per_beat = 7;
    let asset = song.samples.get_mut(&SampleId(11)).unwrap();
    asset.sample_rate = 48_000;
    asset.frames = 192_000;
    asset.path = "/__tempo_marker_metadata_only_missing__.wav".into();
    song.marker_sets.clear();
    song.upsert_marker_set(
        MarkerSetId(31),
        MarkerSet {
            name: "Reviewed periodic boundaries".into(),
            sample: SampleId(11),
            source_bpm: None,
            markers: (0..=8).map(|i| i * 24_000).collect(),
            locked_frames: vec![48_000, 144_000],
        },
    )
    .unwrap();
    song.upsert_marker_set(
        MarkerSetId(32),
        MarkerSet {
            name: "Separate same-source target".into(),
            sample: SampleId(11),
            source_bpm: Some(91.25),
            markers: vec![0, 48_000, 96_000, 144_000, 192_000],
            locked_frames: vec![96_000],
        },
    )
    .unwrap();
    let mut other = song.samples[&SampleId(11)].clone();
    other.path = "/__tempo_other_metadata_only_missing__.wav".into();
    song.register_sample(SampleId(12), other).unwrap();
    let mut marker = song.marker_sets[&MarkerSetId(32)].clone();
    marker.sample = SampleId(12);
    song.upsert_marker_set(MarkerSetId(33), marker).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song).unwrap();
    editor.row = 6;
    editor.column = 0;
    editor.field = Field::Pan;
    editor
}
fn tempo_marker_request() -> aqua_pattern_core::tempo::MarkerRequest {
    aqua_pattern_core::tempo::MarkerRequest {
        marker_set: aqua_pattern_core::song::MarkerSetId(31),
        start_index: 0,
        end_index: 8,
        include_start: true,
    }
}
fn tempo_marker_config() -> aqua_pattern_core::tempo::TempoConfig {
    aqua_pattern_core::tempo::TempoConfig {
        version: 1,
        min_bpm_tenths: 1190,
        max_bpm_tenths: 1210,
    }
}

#[test]
fn reviewed_tempo_use_is_protected_pure_bpm_only_and_one_cursor_preserving_undo() {
    use aqua_pattern_core::{song::MarkerSetId, tempo};
    let mut editor = tempo_marker_editor();
    let base = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    // No file exists at either source path. Accepted-marker analysis and BPM-only
    // metadata use must not pretend to decode/reverify audio or require source IO.
    let analysis =
        tempo::analyze_markers(&base, tempo_marker_request(), tempo_marker_config()).unwrap();
    let preview = analysis.use_bpm(MarkerSetId(31), 0).unwrap();
    let bpm = preview.candidate().marker_sets[&MarkerSetId(31)]
        .source_bpm
        .unwrap();
    let mut expected = base.clone();
    expected
        .marker_sets
        .get_mut(&MarkerSetId(31))
        .unwrap()
        .source_bpm = Some(bpm);
    assert_eq!(preview.candidate(), &expected);
    assert_eq!(analysis.base(), &base);
    assert_eq!(
        editor.song, base,
        "analysis/review never commits automatically"
    );
    assert!(!editor.dirty && !editor.can_undo());
    assert!(editor.commit_tempo_preview(revision, &preview).is_err());
    assert_eq!(editor.revision(), revision);
    editor.edit_mode = true;
    editor.commit_tempo_preview(revision, &preview).unwrap();
    assert_eq!(editor.song, expected);
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
    assert_eq!(editor.song.bpm.to_bits(), 123.5f64.to_bits());
    assert_eq!(editor.song.lines_per_beat, 7);
    assert!(editor.dirty && editor.can_undo() && !editor.can_redo());
    editor.undo();
    assert_eq!(editor.song, base);
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
    editor.redo();
    assert_eq!(editor.song, expected);
}

#[test]
fn reviewed_tempo_targets_only_explicit_same_source_set_and_invalid_choices_are_atomic() {
    use aqua_pattern_core::{song::MarkerSetId, tempo};
    let mut editor = tempo_marker_editor();
    editor.edit_mode = true;
    editor.set_timing(124.0, 7).unwrap();
    editor.undo();
    let base = editor.song.clone();
    let revision = editor.revision();
    let analysis =
        tempo::analyze_markers(&base, tempo_marker_request(), tempo_marker_config()).unwrap();
    assert!(analysis.use_bpm(MarkerSetId(999), 0).is_err());
    assert!(
        analysis.use_bpm(MarkerSetId(33), 0).is_err(),
        "different sample ID is not equivalent evidence"
    );
    assert!(analysis.use_bpm(MarkerSetId(31), usize::MAX).is_err());
    let mut invalid_request = tempo_marker_request();
    invalid_request.end_index = 2;
    let insufficient =
        tempo::analyze_markers(&base, invalid_request, tempo_marker_config()).unwrap();
    assert!(insufficient.analysis().insufficient_reason.is_some());
    assert!(insufficient.analysis().candidates.is_empty());
    assert!(insufficient.use_bpm(MarkerSetId(31), 0).is_err());
    for change in [
        |config: &mut tempo::TempoConfig| config.version = 2,
        |config: &mut tempo::TempoConfig| config.min_bpm_tenths = 199,
        |config: &mut tempo::TempoConfig| config.max_bpm_tenths = 4001,
        |config: &mut tempo::TempoConfig| config.min_bpm_tenths = 1211,
    ] {
        let mut config = tempo_marker_config();
        change(&mut config);
        assert!(tempo::analyze_markers(&base, tempo_marker_request(), config).is_err());
    }
    assert_eq!(editor.song, base);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    let preview = analysis.use_bpm(MarkerSetId(32), 0).unwrap();
    let mut expected = base.clone();
    expected
        .marker_sets
        .get_mut(&MarkerSetId(32))
        .unwrap()
        .source_bpm = preview.candidate().marker_sets[&MarkerSetId(32)].source_bpm;
    editor.commit_tempo_preview(revision, &preview).unwrap();
    assert_eq!(editor.song, expected);
    assert_eq!(
        editor.song.marker_sets[&MarkerSetId(31)],
        base.marker_sets[&MarkerSetId(31)]
    );
    editor.undo();
    assert_eq!(editor.song, base);
}

#[test]
fn same_reviewed_bpm_is_a_true_noop_preserving_saved_redo_and_optional_bits() {
    use aqua_pattern_core::{song::MarkerSetId, tempo};
    let mut editor = tempo_marker_editor();
    let first = tempo::analyze_markers(&editor.song, tempo_marker_request(), tempo_marker_config())
        .unwrap();
    let candidate = first
        .use_bpm(MarkerSetId(31), 0)
        .unwrap()
        .candidate()
        .clone();
    editor.replace_document(candidate.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(124.0, 7).unwrap();
    editor.undo();
    let revision = editor.revision();
    let snapshot = serde_json::to_vec(&editor.song).unwrap();
    let analysis =
        tempo::analyze_markers(&editor.song, tempo_marker_request(), tempo_marker_config())
            .unwrap();
    let noop = analysis.use_bpm(MarkerSetId(31), 0).unwrap();
    editor.commit_tempo_preview(revision, &noop).unwrap();
    assert_eq!(editor.song, candidate);
    assert_eq!(serde_json::to_vec(&editor.song).unwrap(), snapshot);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && !editor.can_undo() && editor.can_redo());
    editor.redo();
    assert_eq!(editor.song.bpm, 124.0);
}

#[test]
fn tempo_reviews_reject_full_base_changes_removed_targets_replacement_and_undo_aba() {
    use aqua_pattern_core::{
        song::{MarkerSetId, SampleId},
        tempo,
    };
    for case in 0..6 {
        let mut editor = tempo_marker_editor();
        editor.edit_mode = true;
        let base = editor.song.clone();
        let revision = editor.revision();
        let analysis =
            tempo::analyze_markers(&base, tempo_marker_request(), tempo_marker_config()).unwrap();
        let preview = analysis.use_bpm(MarkerSetId(32), 0).unwrap();
        match case {
            0 => editor.set_timing(124.0, 7).unwrap(),
            1 => editor
                .set_marker_lock(MarkerSetId(31), 24_000, true)
                .unwrap(),
            2 => editor.move_marker(MarkerSetId(31), 24_000, 24_001).unwrap(),
            3 => {
                let mut candidate = editor.song.clone();
                candidate
                    .set_marker_lock(MarkerSetId(32), 96_000, false)
                    .unwrap();
                candidate.remove_marker_set(MarkerSetId(32)).unwrap();
                editor.apply_document_edit(candidate).unwrap();
            }
            4 => {
                let mut marker = editor.song.marker_sets[&MarkerSetId(32)].clone();
                marker.name.push_str(" edited");
                editor.upsert_marker_set(MarkerSetId(32), marker).unwrap();
            }
            _ => {
                editor.song.samples.get_mut(&SampleId(11)).unwrap().path =
                    "/__changed_tempo_source__.wav".into()
            }
        }
        let changed = editor.song.clone();
        let changed_revision = editor.revision();
        assert!(
            editor
                .commit_tempo_preview(changed_revision, &preview)
                .is_err(),
            "full-base guard case {case}"
        );
        assert_eq!(editor.song, changed);
        assert_eq!(editor.revision(), changed_revision);
        if case != 5 {
            editor.undo();
            assert_eq!(editor.song, base);
            let current = editor.revision();
            assert!(
                editor.commit_tempo_preview(revision, &preview).is_err(),
                "ABA case {case}"
            );
            assert_eq!(editor.revision(), current);
            assert!(!editor.dirty && editor.can_redo());
        }
        editor.replace_document(base.clone()).unwrap();
        editor.edit_mode = true;
        let replacement_revision = editor.revision();
        assert!(editor.commit_tempo_preview(revision, &preview).is_err());
        assert_eq!(editor.revision(), replacement_revision);
        assert_eq!(editor.song, base);
        assert!(!editor.dirty && !editor.can_undo());
    }
}

fn reviewed_region_role() -> aqua_pattern_core::slice_features::RoleLabel {
    use aqua_pattern_core::slice_features::{Articulation, PercussionRole, RoleLabel};
    RoleLabel {
        role: PercussionRole::Unknown,
        articulation: Articulation::Unspecified,
        note: "Explicitly reviewed; no role inferred from level".into(),
    }
}

#[test]
fn manual_region_role_is_separate_protected_metadata_and_none_differs_from_unknown() {
    use aqua_pattern_core::{slice_features, song::RegionId};
    let mut editor = region_playback_editor();
    let base = editor.song.clone();
    let revision = editor.revision();
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.column,
        editor.row,
        editor.field,
    );
    let label = reviewed_region_role();
    assert!(editor
        .set_region_role(RegionId(21), Some(label.clone()))
        .is_err());
    assert!(editor.clear_region_features(RegionId(21)).is_err());
    assert_eq!(editor.song, base);
    assert_eq!(editor.revision(), revision);
    editor.edit_mode = true;
    editor
        .set_region_role(RegionId(21), Some(label.clone()))
        .unwrap();
    let annotated = editor.song.clone();
    let entry = &annotated.region_catalog[&RegionId(21)];
    assert!(
        entry.features.is_none(),
        "manual review must not invent acoustic measurements"
    );
    assert_eq!(entry.reviewed_role.as_ref().unwrap().label, label);
    assert!(entry
        .reviewed_role
        .as_ref()
        .unwrap()
        .based_on_feature_sha256
        .is_none());
    let status = slice_features::inspect_catalog(&annotated, RegionId(21)).unwrap();
    assert_eq!(status.features_current_match, None);
    assert_eq!(status.reviewed_role_current_match, Some(true));
    let mut stripped = annotated.clone();
    stripped.region_catalog.clear();
    assert_eq!(
        stripped, base,
        "annotation is not playback, source or timing authority"
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
    assert_eq!(editor.song, base);
    assert!(!editor.dirty && editor.can_redo());
    let revision = editor.revision();
    editor.set_region_role(RegionId(21), None).unwrap();
    editor.clear_region_features(RegionId(21)).unwrap();
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && editor.can_redo());
    editor.redo();
    assert_eq!(editor.song, annotated);
    editor.mark_saved();
    editor.set_timing(123.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    editor.set_region_role(RegionId(21), Some(label)).unwrap();
    editor.clear_region_features(RegionId(21)).unwrap();
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && editor.can_redo());
    editor.set_region_role(RegionId(21), None).unwrap();
    assert!(
        editor.song.region_catalog.is_empty(),
        "empty entries are pruned rather than persisting fake reviews"
    );
    editor.undo();
    assert_eq!(editor.song, annotated);
}

#[test]
fn catalog_native_binding_ignores_playback_metadata_but_bounds_and_source_edits_are_stale() {
    use aqua_pattern_core::{
        slice_features,
        song::{AmpEnvelope, ForwardLoop, RegionId, SampleId},
    };
    let mut editor = region_playback_editor();
    editor.edit_mode = true;
    editor
        .set_region_role(RegionId(21), Some(reviewed_region_role()))
        .unwrap();
    editor.mark_saved();
    let annotated = editor.song.clone();
    let original_entry = annotated.region_catalog[&RegionId(21)].clone();
    let mut region = editor.song.regions[&RegionId(21)].clone();
    region.name = "Renamed region, same native audio".into();
    region.root_note = 72;
    region.amp_envelope = Some(AmpEnvelope::default());
    region.forward_loop = Some(ForwardLoop {
        start_frame: 40,
        end_frame: 80,
    });
    editor.upsert_region(RegionId(21), region.clone()).unwrap();
    assert_eq!(editor.song.region_catalog[&RegionId(21)], original_entry);
    assert_eq!(
        slice_features::inspect_catalog(&editor.song, RegionId(21))
            .unwrap()
            .reviewed_role_current_match,
        Some(true)
    );
    region.start_frame += 1;
    editor.upsert_region(RegionId(21), region.clone()).unwrap();
    assert_eq!(editor.song.region_catalog[&RegionId(21)], original_entry);
    assert_eq!(
        slice_features::inspect_catalog(&editor.song, RegionId(21))
            .unwrap()
            .reviewed_role_current_match,
        Some(false)
    );
    editor.undo();
    assert_eq!(
        slice_features::inspect_catalog(&editor.song, RegionId(21))
            .unwrap()
            .reviewed_role_current_match,
        Some(true)
    );
    editor.undo();
    assert_eq!(editor.song, annotated);
    let mut other = source_fixture();
    other.path = "/__new_catalog_source__.wav".into();
    other.sha256 = "b".repeat(64);
    let mut changed = editor.song.clone();
    changed.register_sample(SampleId(12), other).unwrap();
    region = changed.regions[&RegionId(21)].clone();
    region.sample = SampleId(12);
    changed.upsert_region(RegionId(21), region).unwrap();
    changed.remove_sample(SampleId(11)).unwrap();
    editor.apply_document_edit(changed.clone()).unwrap();
    assert_eq!(editor.song.region_catalog[&RegionId(21)], original_entry);
    assert_eq!(
        slice_features::inspect_catalog(&editor.song, RegionId(21))
            .unwrap()
            .reviewed_role_current_match,
        Some(false)
    );
    assert!(
        !editor.song.samples.contains_key(&SampleId(11)),
        "historical bindings are not live source references"
    );
    editor.undo();
    assert_eq!(editor.song, annotated);
    editor.redo();
    assert_eq!(editor.song, changed);
}

#[test]
fn referenced_region_removal_keeps_catalog_atomic_and_explicit_unmap_remove_undo_restores_it() {
    use aqua_pattern_core::song::RegionId;
    let mut editor = region_playback_editor();
    editor.edit_mode = true;
    editor
        .set_region_role(RegionId(21), Some(reviewed_region_role()))
        .unwrap();
    let base = editor.song.clone();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    assert!(editor.remove_region(RegionId(21)).is_err());
    assert_eq!(editor.song, base);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && editor.can_redo());
    let mut candidate = base.clone();
    for instrument in [InstrumentId(1), InstrumentId(2)] {
        candidate.assign_region(instrument, None).unwrap();
    }
    candidate.remove_region(RegionId(21)).unwrap();
    assert!(!candidate.region_catalog.contains_key(&RegionId(21)));
    editor.apply_document_edit(candidate.clone()).unwrap();
    editor.undo();
    assert_eq!(editor.song, base);
    editor.redo();
    assert_eq!(editor.song, candidate);
    let removed_revision = editor.revision();
    assert!(editor
        .set_region_role(RegionId(21), Some(reviewed_region_role()))
        .is_err());
    assert!(editor.clear_region_features(RegionId(21)).is_err());
    assert_eq!(editor.revision(), removed_revision);
    assert_eq!(editor.song, candidate);
}

#[test]
fn catalog_role_bounds_capacity_and_invalid_targets_preserve_clean_redo() {
    use aqua_pattern_core::song::RegionId;
    let mut editor = region_playback_editor();
    editor.edit_mode = true;
    let mut label = reviewed_region_role();
    label.note = "é".repeat(128);
    editor
        .set_region_role(RegionId(21), Some(label.clone()))
        .unwrap();
    let valid = editor.song.clone();
    editor.replace_document(valid.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    label.note.push('x');
    assert!(editor.set_region_role(RegionId(21), Some(label)).is_err());
    assert!(editor
        .set_region_role(RegionId(999), Some(reviewed_region_role()))
        .is_err());
    assert!(editor.clear_region_features(RegionId(999)).is_err());
    assert_eq!(editor.song, valid);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && editor.can_redo());
    let mut full = region_playback_editor().song;
    for id in 100..=356 {
        full.upsert_region(RegionId(id), region_fixture()).unwrap();
    }
    for id in 100..356 {
        full.set_region_role(RegionId(id), Some(reviewed_region_role()))
            .unwrap();
    }
    assert_eq!(full.region_catalog.len(), 256);
    editor.replace_document(full.clone()).unwrap();
    editor.edit_mode = true;
    editor.set_timing(123.0, 4).unwrap();
    editor.undo();
    let revision = editor.revision();
    assert!(editor
        .set_region_role(RegionId(356), Some(reviewed_region_role()))
        .is_err());
    editor
        .set_region_role(RegionId(100), Some(reviewed_region_role()))
        .unwrap();
    assert_eq!(editor.song, full);
    assert_eq!(editor.revision(), revision);
    assert!(!editor.dirty && editor.can_redo());
    let mut replacement = full.clone();
    replacement.set_region_role(RegionId(100), None).unwrap();
    replacement
        .set_region_role(RegionId(356), Some(reviewed_region_role()))
        .unwrap();
    editor.apply_document_edit(replacement.clone()).unwrap();
    assert_eq!(editor.song.region_catalog.len(), 256);
    editor.undo();
    assert_eq!(editor.song, full);
    editor.redo();
    assert_eq!(editor.song, replacement);
}

fn thin_config() -> aqua_pattern_core::mutations::ThinConfig {
    use aqua_pattern_core::slice_features::PercussionRole;
    aqua_pattern_core::mutations::ThinConfig {
        version: 1,
        scope: aqua_pattern_core::transforms::TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 4,
            columns: vec![0],
        },
        seed: 0x1234_5678,
        removal_percent: 100,
        max_removed: 16,
        protected_roles: vec![PercussionRole::Kick, PercussionRole::Snare],
    }
}
fn thin_editor() -> Editor {
    use aqua_pattern_core::{
        slice_features::{Articulation, PercussionRole, RoleLabel},
        song::{Occurrence, RegionId, SampleId, TrackAlias},
    };
    let mut song = aqua_pattern_core::song::example_song();
    for pattern in song.patterns.values_mut() {
        pattern.rows = 8;
        pattern.tracks.clear();
    }
    song.register_sample(SampleId(11), source_fixture())
        .unwrap();
    song.upsert_region(RegionId(21), region_fixture()).unwrap();
    song.assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    song.set_region_role(
        RegionId(21),
        Some(RoleLabel {
            role: PercussionRole::ClosedHat,
            articulation: Articulation::Normal,
            note: "Explicit manual role, no source-file verification claim".into(),
        }),
    )
    .unwrap();
    let rows = song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default();
    for row in 0..6 {
        rows.insert(
            row,
            [(
                0,
                Cell {
                    note: Note::Note(60 + row as u8),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(0.25 + row as f32 * 0.0625),
                    pan: Some(-0.25),
                    delay: if row == 0 { Some(0.0) } else { Some(0.125) },
                    ..Default::default()
                },
            )]
            .into(),
        );
    }
    rows.get_mut(&2).unwrap().insert(
        1,
        Cell {
            note: Note::Note(72),
            instrument: Some(InstrumentId(1)),
            velocity: Some(0.75),
            pan: Some(0.25),
            delay: Some(0.375),
            ..Default::default()
        },
    );
    rows.insert(
        7,
        [(
            0,
            Cell {
                note: Note::Off,
                ..Default::default()
            },
        )]
        .into(),
    );
    song.set_track_alias(
        PatternId(2),
        TrackId(2),
        Some(TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        }),
    )
    .unwrap();
    song.clone_pattern(PatternId(1), PatternId(3)).unwrap();
    song.order = [1, 2, 1]
        .map(|id| Occurrence {
            pattern: PatternId(id),
            muted_tracks: Default::default(),
            section: None,
        })
        .into();
    song.validate().unwrap();
    let mut e = Editor::default();
    e.replace_document(song).unwrap();
    e.occurrence = 2;
    e.row = 5;
    e.column = 1;
    e.field = Field::Pan;
    e
}

#[test]
fn thin_preview_is_protected_whole_cell_scoped_and_one_undo_across_shared_uses() {
    use aqua_pattern_core::mutations;
    let mut e = thin_editor();
    let base = e.song.clone();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let preview = mutations::preview(&base, thin_config()).unwrap();
    assert_eq!(preview.base(), &base);
    assert_eq!(preview.config(), &thin_config());
    assert_eq!(e.song, base, "preview performs no edit or source IO");
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo());
    let mut expected = base.clone();
    let rows = expected
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    for row in 0..4 {
        let cells = rows.get_mut(&row).unwrap();
        cells.remove(&0);
    }
    assert_eq!(preview.candidate(), &expected,
        "only whole selected cells disappear; other columns, scalars and independent clones stay exact");
    assert!(e.commit_thin_preview(revision, &preview).is_err());
    assert_eq!(e.song, base);
    e.edit_mode = true;
    e.commit_thin_preview(revision, &preview).unwrap();
    assert_eq!(e.song, expected);
    assert_eq!(e.revision(), revision + 1);
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert_eq!(e.song.patterns[&PatternId(3)], base.patterns[&PatternId(3)]);
    assert_eq!(
        e.song.resolved_track(PatternId(2), TrackId(2)).unwrap(),
        e.song.resolved_track(PatternId(1), TrackId(1)).unwrap()
    );
    e.undo();
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    e.redo();
    assert_eq!(e.song, expected);
    e.undo();
    assert_eq!(
        mutations::preview(&e.song, thin_config())
            .unwrap()
            .candidate(),
        &expected
    );
}

#[test]
fn thin_zero_percent_and_protected_or_uncertain_roles_preserve_clean_redo_exactly() {
    use aqua_pattern_core::{mutations, slice_features::PercussionRole, song::RegionId};
    let mut e = thin_editor();
    e.edit_mode = true;
    let base = e.song.clone();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let revision = e.revision();
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let mut config = thin_config();
    config.removal_percent = 0;
    let preview = mutations::preview(&base, config.clone()).unwrap();
    assert_eq!(preview.candidate(), &base);
    assert_eq!(
        preview.id(),
        mutations::preview(&base, config).unwrap().id()
    );
    e.commit_thin_preview(revision, &preview).unwrap();
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    for role in [
        Some(PercussionRole::Kick),
        Some(PercussionRole::Snare),
        Some(PercussionRole::Unknown),
        Some(PercussionRole::Mixed),
        None,
    ] {
        let mut protected = base.clone();
        let label = role.map(|role| {
            let mut label = protected.region_catalog[&RegionId(21)]
                .reviewed_role
                .as_ref()
                .unwrap()
                .label
                .clone();
            label.role = role;
            label
        });
        protected.set_region_role(RegionId(21), label).unwrap();
        e.replace_document(protected.clone()).unwrap();
        e.edit_mode = true;
        e.set_timing(121.0, 4).unwrap();
        e.undo();
        let revision = e.revision();
        let preview = mutations::preview(&e.song, thin_config()).unwrap();
        assert_eq!(preview.candidate(), &protected, "{role:?}");
        e.commit_thin_preview(revision, &preview).unwrap();
        assert_eq!(e.revision(), revision);
        assert!(!e.dirty && !e.can_undo() && e.can_redo());
    }
    let mut stale = base;
    let mut region = stale.regions[&RegionId(21)].clone();
    region.start_frame += 1;
    stale.upsert_region(RegionId(21), region).unwrap();
    let preview = mutations::preview(&stale, thin_config()).unwrap();
    assert_eq!(
        preview.candidate(),
        &stale,
        "historical current-label semantics are not fresh authority after bounds change"
    );
}

#[test]
fn thin_review_rejects_full_base_changes_aba_and_replacement_without_consuming_history() {
    use aqua_pattern_core::mutations;
    let mut e = thin_editor();
    e.edit_mode = true;
    let base = e.song.clone();
    let revision = e.revision();
    let preview = mutations::preview(&base, thin_config()).unwrap();
    e.song
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
        .delay = Some(-0.0);
    assert_eq!(
        e.song, base,
        "numeric equality alone does not detect signed-zero byte changes"
    );
    assert_eq!(e.revision(), revision);
    assert!(
        e.commit_thin_preview(revision, &preview).is_err(),
        "the opaque base digest must reject unchecked signed-zero ABA without history mutation"
    );
    assert!(!e.can_undo() && !e.can_redo());
    e.song = base.clone();
    e.song.title.push_str(" public draft changed");
    assert_eq!(e.revision(), revision);
    assert!(e.commit_thin_preview(revision, &preview).is_err());
    assert!(!e.can_undo() && !e.can_redo());
    e.song = base.clone();
    e.set_timing(121.0, 4).unwrap();
    let changed = e.song.clone();
    let changed_revision = e.revision();
    assert!(e.commit_thin_preview(changed_revision, &preview).is_err());
    assert_eq!(e.song, changed);
    assert_eq!(e.revision(), changed_revision);
    e.undo();
    assert_eq!(e.song, base);
    assert!(
        e.commit_thin_preview(revision, &preview).is_err(),
        "Undo ABA is stale"
    );
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let replacement_revision = e.revision();
    let replacement_plan = mutations::preview(&e.song, thin_config()).unwrap();
    e.replace_document(base.clone()).unwrap();
    e.edit_mode = true;
    assert!(
        e.commit_thin_preview(replacement_revision, &replacement_plan)
            .is_err(),
        "New/Open of byte-identical content still invalidates a generation-bound review"
    );
    assert_eq!(e.song, base);
    assert!(!e.dirty && !e.can_undo() && !e.can_redo());
    let mut changed = base.clone();
    changed
        .set_occurrence_track_mute(0, TrackId(2), true)
        .unwrap();
    e.apply_document_edit(changed.clone()).unwrap();
    let revision = e.revision();
    assert!(
        e.commit_thin_preview(revision, &preview).is_err(),
        "an unrelated occurrence/mute change is still a different full base"
    );
    assert_eq!(e.song, changed);
    assert_eq!(e.revision(), revision);
}

#[test]
fn thin_invalid_scopes_and_non_evaluable_bases_fail_without_mutating_or_materializing() {
    use aqua_pattern_core::mutations;
    let mut e = thin_editor();
    e.edit_mode = true;
    let base = e.song.clone();
    e.set_timing(121.0, 4).unwrap();
    e.undo();
    let revision = e.revision();
    let mut invalid = Vec::new();
    let mut c = thin_config();
    c.version = 2;
    invalid.push(c);
    let mut c = thin_config();
    c.removal_percent = 101;
    invalid.push(c);
    let mut c = thin_config();
    c.max_removed = 4097;
    invalid.push(c);
    let mut c = thin_config();
    c.max_removed = 0;
    invalid.push(c);
    let mut c = thin_config();
    c.protected_roles
        .push(aqua_pattern_core::slice_features::PercussionRole::Unknown);
    invalid.push(c);
    let mut c = thin_config();
    c.protected_roles
        .push(aqua_pattern_core::slice_features::PercussionRole::Kick);
    invalid.push(c);
    let mut c = thin_config();
    c.scope.row_end = 9;
    invalid.push(c);
    let mut c = thin_config();
    c.scope.row_start = c.scope.row_end;
    invalid.push(c);
    let mut c = thin_config();
    c.scope.columns = vec![1, 0];
    invalid.push(c);
    let mut c = thin_config();
    c.scope.columns = vec![0, 0];
    invalid.push(c);
    let mut c = thin_config();
    c.scope.pattern = PatternId(2);
    c.scope.track = TrackId(2);
    invalid.push(c);
    for config in invalid {
        assert!(mutations::preview(&e.song, config).is_err());
    }
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    let mut inherited = base.clone();
    let rows = inherited
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap();
    for row in 1..6 {
        rows.get_mut(&row).unwrap().get_mut(&0).unwrap().velocity = None;
    }
    assert!(
        mutations::preview(&inherited, thin_config()).is_err(),
        "deleting stored scalar memory must not silently change retained ON gain"
    );
    let mut non_evaluable = base.clone();
    non_evaluable.assign_region(InstrumentId(1), None).unwrap();
    let mut config = thin_config();
    config.removal_percent = 0;
    assert!(
        mutations::preview(&non_evaluable, config).is_err(),
        "zero percent is not a bypass around the full model-evaluable prerequisite"
    );
    assert_eq!(e.song, base);
    assert_eq!(e.revision(), revision);
}
