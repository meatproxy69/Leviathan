use aqua_pattern_core::{
    drum_steps::{self, StepDivision, StepEdit, StepEvent, StepView},
    editor::{Editor, Field},
    song::*,
};

fn fixture() -> Song {
    let mut song = example_song();
    song.patterns.clear();
    song.patterns.insert(
        PatternId(1),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Steps".into(),
            rows: 32,
            tracks: Default::default(),
        },
    );
    song.order = vec![Occurrence {
        muted_tracks: Default::default(),
        section: None,
        pattern: PatternId(1),
    }];
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 4;
    song.samples.insert(
        SampleId(1),
        SampleAsset {
            path: "synthetic.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: 48_000,
            channels: 1,
            frames: 100,
        },
    );
    song.regions.insert(
        RegionId(1),
        SampleRegion {
            name: "Source".into(),
            amp_envelope: None,
            forward_loop: None,
            sample: SampleId(1),
            start_frame: 0,
            end_frame: 100,
            root_note: 60,
        },
    );
    let pads = (0..2)
        .map(|n| {
            (
                PadId(n + 1),
                DrumPad {
                    slot: n as u8,
                    name: format!("Pad {n}"),
                    velocity_zones: Vec::new(),
                    note: 36 + n as u8,
                    region: Some(RegionId(1)),
                    root_note: 60,
                    tune_semitones: 0,
                    gain: 1.0,
                    pan: 0.0,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: None,
                },
            )
        })
        .collect();
    song.upsert_drum_kit(
        DrumKitId(1),
        DrumKitInstrument {
            name: "Drums".into(),
            pads,
        },
    )
    .unwrap();
    song.assign_kit(InstrumentId(1), Some(DrumKitId(1)))
        .unwrap();
    song
}
fn view() -> StepView {
    StepView {
        pattern: PatternId(1),
        track: TrackId(1),
        instrument: InstrumentId(1),
        division: StepDivision::Sixteenth,
        steps: 16,
        page: 0,
    }
}
fn note(note: u8, instrument: Option<InstrumentId>, delay: Option<f64>) -> Cell {
    Cell {
        note: Note::Note(note),
        instrument,
        delay,
        ..Default::default()
    }
}
fn put(song: &mut Song, row: u32, column: u16, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(column, cell);
}
fn at(song: &Song, row: u32, column: u16) -> &Cell {
    &song.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&column]
}
fn event(song: &Song, row: u32, column: u16) -> StepEvent {
    drum_steps::project(song, &view())
        .unwrap()
        .lanes
        .into_iter()
        .flat_map(|lane| lane.steps)
        .flat_map(|slot| slot.events)
        .find(|event| event.row == row && event.column == column)
        .unwrap()
}
fn edit(song: &Song, operation: StepEdit) -> Song {
    drum_steps::apply(
        song,
        &drum_steps::project(song, &view()).unwrap(),
        &operation,
    )
    .unwrap()
}
#[test]
fn rational_grid_uses_lpb_division_page_without_rounding_or_hidden_quantization() {
    let mut song = fixture();
    song.lines_per_beat = 3;
    let before = song.clone();
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert_eq!(song, before);
    assert_eq!((projection.total_steps, projection.total_pages), (43, 3));
    assert_eq!(
        (
            projection.lanes[0].steps[1].row,
            projection.lanes[0].steps[1].delay
        ),
        (0, 0.75)
    );
    assert_eq!(
        (
            projection.lanes[0].steps[2].row,
            projection.lanes[0].steps[2].delay
        ),
        (1, 0.5)
    );
    let inserted = drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 1,
        },
    )
    .unwrap();
    assert_eq!(at(&inserted, 0, 0).delay, Some(0.75));
    let mut later = view();
    later.page = 2;
    let projection = drum_steps::project(&inserted, &later).unwrap();
    assert_eq!(
        (
            projection.lanes[0].steps[0].row,
            projection.lanes[0].steps[0].delay
        ),
        (24, 0.0)
    );
    assert!(!projection.lanes[0].steps[11].in_bounds);
    assert!(drum_steps::apply(
        &inserted,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 11
        }
    )
    .is_err());
    let json = serde_json::to_value(projection).unwrap();
    assert!(json.get("base").is_none() && json.get("base_view").is_none());
}
#[test]
fn offgrid_duplicate_hits_are_visible_and_exact_field_edits_preserve_all_other_cells() {
    let mut song = fixture();
    put(
        &mut song,
        0,
        0,
        note(36, Some(InstrumentId(1)), Some(0.125)),
    );
    put(&mut song, 0, 1, note(36, Some(InstrumentId(1)), Some(0.75)));
    let mut extra = note(37, Some(InstrumentId(1)), None);
    extra.pan = Some(-0.4);
    extra.effects.cut = Some(0.9);
    put(&mut song, 1, 2, extra);
    put(&mut song, 0, 3, note(72, Some(InstrumentId(2)), Some(0.2)));
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert_eq!(projection.lanes[0].steps[0].events.len(), 2);
    assert!(projection.lanes[0].steps[0]
        .events
        .iter()
        .all(|e| !e.on_grid));
    assert!(projection.issues.iter().any(|issue| issue.column == 3));
    assert!(drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 0
        }
    )
    .is_err());
    let mut expected = song.clone();
    expected
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .velocity = Some(0.0);
    let updated = edit(
        &song,
        StepEdit::SetVelocity {
            event: event(&song, 0, 1),
            value: Some(0.0),
        },
    );
    assert_eq!(updated, expected);
    let updated = edit(
        &song,
        StepEdit::SetDelay {
            event: event(&song, 1, 2),
            value: Some(0.25),
        },
    );
    let mut expected_cell = at(&song, 1, 2).clone();
    expected_cell.delay = Some(0.25);
    assert_eq!(at(&updated, 1, 2), &expected_cell);
    assert!(
        drum_steps::project(&updated, &view()).unwrap().lanes[1].steps[1]
            .conflicts
            .len()
            >= 2
    );
}
#[test]
fn one_column_toggle_is_reversible_and_retrigger_children_are_never_separate_hits() {
    let mut song = fixture();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 1;
    let mut hit = note(36, Some(InstrumentId(1)), Some(0.0));
    hit.velocity = Some(0.3);
    hit.pan = Some(0.4);
    hit.effects.retrigger = Some(Retrigger {
        repeats: 3,
        spacing: 0.25,
    });
    put(&mut song, 0, 0, hit.clone());
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert_eq!(projection.lanes[0].steps[0].events.len(), 1);
    let removed = edit(
        &song,
        StepEdit::Toggle {
            pad: PadId(1),
            step: 0,
        },
    );
    let mut residual = hit.clone();
    residual.note = Note::Empty;
    residual.effects.retrigger = None;
    assert_eq!(at(&removed, 0, 0), &residual);
    let reinserted = edit(
        &removed,
        StepEdit::Toggle {
            pad: PadId(1),
            step: 0,
        },
    );
    let mut without_roll = hit;
    without_roll.effects.retrigger = None;
    assert_eq!(at(&reinserted, 0, 0), &without_roll);
    let rolled = edit(
        &reinserted,
        StepEdit::SetRetrigger {
            event: event(&reinserted, 0, 0),
            value: Some(Retrigger {
                repeats: 3,
                spacing: 0.25,
            }),
        },
    );
    assert_eq!(rolled, song);
}
#[test]
fn incoming_selection_aliases_and_phrases_are_not_mistaken_for_owned_steps() {
    let mut song = fixture();
    song.assign_kit(InstrumentId(2), Some(DrumKitId(1)))
        .unwrap();
    put(&mut song, 0, 0, note(36, None, None));
    put(&mut song, 1, 0, note(36, Some(InstrumentId(1)), None));
    put(&mut song, 2, 0, note(37, None, None));
    put(&mut song, 0, 1, note(36, Some(InstrumentId(2)), None));
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(projection.lanes[0].steps[0].events.is_empty());
    assert!(!projection.lanes[0].steps[0].conflicts.is_empty());
    assert_eq!(
        projection.lanes[1].steps[2].events[0].instrument,
        InstrumentId(1)
    );
    assert!(projection.issues.iter().any(|issue| issue.column == 1));
    song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Kit phrase".into(),
            instrument: InstrumentId(1),
            rows: 1,
            lines_per_beat: 4,
            note_columns: 1,
            root_note: 36,
            cells: [(0, [(0, note(36, None, None))].into())].into(),
        },
    );
    let mut trigger = note(36, None, None);
    trigger.phrase = Some(PhraseTrigger {
        phrase: PhraseId(1),
        cycles: 1,
    });
    put(&mut song, 20, 1, trigger);
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(projection
        .issues
        .iter()
        .any(|issue| issue.row == 20 && issue.cell.phrase.is_some()));
    assert!(projection
        .lanes
        .iter()
        .flat_map(|l| &l.steps)
        .all(|s| !s.conflicts.is_empty()));
}
#[test]
fn insertion_never_clobbers_occupied_cells_or_silently_rebinds_incoming_notes() {
    let mut song = fixture();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 1;
    put(&mut song, 0, 0, note(60, Some(InstrumentId(2)), None));
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 0
        }
    )
    .is_err());
    put(&mut song, 2, 0, note(37, None, None));
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 1
        }
    )
    .is_err());
    // Even an otherwise empty source pattern cannot change an incoming prefix
    // in another shared/unused pattern without the user making it explicit.
    let mut song = fixture();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 1;
    let mut other = song.patterns[&PatternId(1)].clone();
    other
        .tracks
        .insert(TrackId(1), [(0, [(0, note(36, None, None))].into())].into());
    song.patterns.insert(PatternId(2), other);
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 0
        }
    )
    .is_err());
}
#[test]
fn projection_bounds_are_explicit_and_never_truncate_content() {
    let song = fixture();
    for bad in [
        StepView {
            steps: 17,
            ..view()
        },
        StepView {
            page: u32::MAX,
            ..view()
        },
        StepView {
            pattern: PatternId(999),
            ..view()
        },
        StepView {
            track: TrackId(999),
            ..view()
        },
        StepView {
            instrument: InstrumentId(2),
            ..view()
        },
    ] {
        assert!(drum_steps::project(&song, &bad).is_err());
    }
    let mut dense = fixture();
    dense.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for row in 0..9 {
        for col in 0..256 {
            put(&mut dense, row, col, note(36, Some(InstrumentId(1)), None));
        }
    }
    assert!(drum_steps::project(&dense, &view())
        .unwrap_err()
        .contains("2048"));
    let mut huge = fixture();
    huge.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    huge.patterns.get_mut(&PatternId(1)).unwrap().rows = 257;
    for row in 0..257 {
        for col in 0..256 {
            put(&mut huge, row, col, Cell::default());
        }
    }
    assert!(drum_steps::project(&huge, &view())
        .unwrap_err()
        .contains("65536"));
}
#[test]
fn guarded_editor_edits_are_one_undo_and_failures_noops_preserve_saved_redo_and_cursor() {
    let mut e = Editor::default();
    e.replace_document(fixture()).unwrap();
    e.row = 7;
    e.field = Field::Pan;
    let cursor = (e.occurrence, e.track, e.column, e.row, e.field);
    let original = e.song.clone();
    let projection = drum_steps::project(&e.song, &view()).unwrap();
    let toggle = StepEdit::Toggle {
        pad: PadId(1),
        step: 0,
    };
    assert!(e
        .edit_drum_step(e.revision(), &projection, &toggle)
        .is_err());
    e.edit_mode = true;
    let revision = e.revision();
    e.edit_drum_step(revision, &projection, &toggle).unwrap();
    let inserted = e.song.clone();
    assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    assert_eq!(e.revision(), revision + 1);
    assert_eq!(
        drum_steps::apply(&original, &projection, &toggle).unwrap(),
        inserted
    );
    e.undo();
    assert_eq!(e.song, original);
    assert!(!e.dirty);
    assert!(
        e.edit_drum_step(revision, &projection, &toggle).is_err(),
        "ABA revision rejected"
    );
    e.redo();
    assert_eq!(e.song, inserted);
    e.mark_saved();
    e.set_timing(140.0, 4).unwrap();
    e.undo();
    let projection = drum_steps::project(&e.song, &view()).unwrap();
    let hit = event(&e.song, 0, 0);
    let revision = e.revision();
    for operation in [
        StepEdit::SetVelocity {
            event: hit.clone(),
            value: Some(f32::NAN),
        },
        StepEdit::SetVelocity {
            event: hit.clone(),
            value: Some(1.1),
        },
        StepEdit::SetDelay {
            event: hit.clone(),
            value: Some(1.0),
        },
        StepEdit::SetRetrigger {
            event: hit.clone(),
            value: Some(Retrigger {
                repeats: 4,
                spacing: 0.25,
            }),
        },
        StepEdit::Toggle {
            pad: PadId(99),
            step: 0,
        },
        StepEdit::Toggle {
            pad: PadId(1),
            step: 16,
        },
    ] {
        assert!(e.edit_drum_step(revision, &projection, &operation).is_err());
        assert_eq!(e.song, inserted);
        assert_eq!(e.revision(), revision);
        assert!(!e.dirty && e.can_redo());
        assert_eq!((e.occurrence, e.track, e.column, e.row, e.field), cursor);
    }
    e.edit_drum_step(
        revision,
        &projection,
        &StepEdit::SetVelocity {
            event: hit.clone(),
            value: None,
        },
    )
    .unwrap();
    assert_eq!(e.revision(), revision);
    assert!(e.can_redo());
    let mut forged = hit;
    forged.column = 3;
    assert!(e
        .edit_drum_step(
            revision,
            &projection,
            &StepEdit::SetVelocity {
                event: forged,
                value: Some(0.5)
            }
        )
        .is_err());
    e.replace_document(inserted).unwrap();
    e.edit_mode = true;
    assert!(e.edit_drum_step(revision, &projection, &toggle).is_err());
}
#[test]
fn shared_patterns_remain_shared_and_cloned_patterns_remain_independent() {
    let mut song = fixture();
    song.append_repeat(PatternId(1)).unwrap();
    song.clone_pattern(PatternId(1), PatternId(2)).unwrap();
    let clone = song.patterns[&PatternId(2)].clone();
    let projection = drum_steps::project(&song, &view()).unwrap();
    let changed = drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(2),
            step: 3,
        },
    )
    .unwrap();
    assert_eq!(changed.order, song.order);
    assert_eq!(changed.patterns[&PatternId(2)], clone);
    for occurrence in &changed.order[..2] {
        assert_eq!(
            changed.patterns[&occurrence.pattern].tracks[&TrackId(1)][&3][&0].note,
            Note::Note(37)
        );
    }
    assert_eq!(
        serde_json::from_str::<Song>(&serde_json::to_string(&changed).unwrap()).unwrap(),
        changed
    );
    let mut stale = projection.clone();
    stale.view.track = TrackId(2);
    assert!(drum_steps::apply(
        &song,
        &stale,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 0
        }
    )
    .is_err());
    assert!(drum_steps::apply(
        &changed,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 0
        }
    )
    .is_err());
}

#[test]
fn implicit_scalar_residue_is_reclaimable_and_default_deletion_stays_sparse() {
    let mut song = fixture();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 1;
    put(&mut song, 0, 0, note(36, Some(InstrumentId(1)), None));
    let mut implicit = note(37, None, None);
    implicit.velocity = Some(0.4);
    implicit.pan = Some(-0.2);
    put(&mut song, 1, 0, implicit.clone());
    put(&mut song, 2, 0, note(37, None, None));
    let removed = edit(
        &song,
        StepEdit::Toggle {
            pad: PadId(2),
            step: 1,
        },
    );
    assert_eq!(at(&removed, 1, 0).instrument, None);
    assert_eq!(
        at(
            &edit(
                &removed,
                StepEdit::Toggle {
                    pad: PadId(2),
                    step: 1
                }
            ),
            1,
            0
        ),
        &implicit
    );
    let removed = edit(
        &song,
        StepEdit::Toggle {
            pad: PadId(2),
            step: 2,
        },
    );
    assert!(!removed.patterns[&PatternId(1)].tracks[&TrackId(1)].contains_key(&2));
    let restored = edit(
        &removed,
        StepEdit::Toggle {
            pad: PadId(2),
            step: 2,
        },
    );
    assert_eq!(
        restored, song,
        "local selection makes an implicit empty cell reclaim safe"
    );
}

#[test]
fn aliases_project_resolved_events_read_only_then_unique_edits_only_the_target() {
    let mut song = fixture();
    put(&mut song, 0, 0, note(36, Some(InstrumentId(1)), None));
    song.tracks.get_mut(&TrackId(2)).unwrap().note_columns = 4;
    song.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            name: "Alias destination".into(),
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
    let aliased_view = StepView {
        pattern: PatternId(2),
        track: TrackId(2),
        ..view()
    };
    let projection = drum_steps::project(&song, &aliased_view).unwrap();
    assert!(projection.read_only);
    assert_eq!(
        projection.source,
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1)
        }
    );
    let event = projection.lanes[0].steps[0].events[0].clone();
    assert_eq!(event.cell, at(&song, 0, 0).clone());
    assert!(projection
        .lanes
        .iter()
        .flat_map(|l| &l.steps)
        .all(|s| !s.conflicts.is_empty()));
    for operation in [
        StepEdit::Toggle {
            pad: PadId(1),
            step: 0,
        },
        StepEdit::Toggle {
            pad: PadId(1),
            step: 1,
        },
        StepEdit::SetVelocity {
            event: event.clone(),
            value: Some(0.2),
        },
        StepEdit::SetDelay {
            event: event.clone(),
            value: Some(0.2),
        },
        StepEdit::SetRetrigger { event, value: None },
    ] {
        assert!(drum_steps::apply(&song, &projection, &operation)
            .unwrap_err()
            .contains("read-only"));
    }
    let source = song.patterns[&PatternId(1)].clone();
    song.make_track_unique(PatternId(2), TrackId(2)).unwrap();
    let local = drum_steps::project(&song, &aliased_view).unwrap();
    assert!(!local.read_only);
    assert_eq!(
        local.source,
        TrackAlias {
            pattern: PatternId(2),
            track: TrackId(2)
        }
    );
    let updated = drum_steps::apply(
        &song,
        &local,
        &StepEdit::SetVelocity {
            event: local.lanes[0].steps[0].events[0].clone(),
            value: Some(0.25),
        },
    )
    .unwrap();
    assert_eq!(updated.patterns[&PatternId(1)], source);
    assert_eq!(
        updated.patterns[&PatternId(2)].tracks[&TrackId(2)][&0][&0].velocity,
        Some(0.25)
    );

    // The model's alias-only expansion budget is distinct from this view's
    // combined stored + expanded scan budget; do not double its promised work.
    let mut bounded = fixture();
    bounded.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    bounded.patterns.get_mut(&PatternId(1)).unwrap().rows = 129;
    for row in 0..129 {
        for column in 0..256 {
            put(&mut bounded, row, column, Cell::default());
        }
    }
    bounded.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            name: "Bounded expansion".into(),
            rows: 129,
            tracks: Default::default(),
            track_aliases: [(
                TrackId(1),
                TrackAlias {
                    pattern: PatternId(1),
                    track: TrackId(1),
                },
            )]
            .into(),
        },
    );
    bounded.validate().unwrap();
    assert!(drum_steps::project(&bounded, &view())
        .unwrap_err()
        .contains("65536 stored/resolved"));
}
#[test]
fn safe_placement_considers_unknown_prefixes_reached_through_other_track_aliases() {
    let mut song = fixture();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 1;
    song.tracks.get_mut(&TrackId(2)).unwrap().note_columns = 1;
    // Target track 1 is empty locally; another pattern borrows a track-2 note
    // with unknown incoming instrument. A new selection must not silently bind it.
    let mut source = Pattern {
        generation: None,
        name: "Implicit source".into(),
        rows: 32,
        tracks: Default::default(),
        track_aliases: Default::default(),
    };
    source
        .tracks
        .insert(TrackId(2), [(0, [(0, note(36, None, None))].into())].into());
    song.patterns.insert(PatternId(2), source);
    song.patterns.insert(
        PatternId(3),
        Pattern {
            generation: None,
            name: "Indirect prefix".into(),
            rows: 32,
            tracks: Default::default(),
            track_aliases: [(
                TrackId(1),
                TrackAlias {
                    pattern: PatternId(2),
                    track: TrackId(2),
                },
            )]
            .into(),
        },
    );
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(drum_steps::apply(
        &song,
        &projection,
        &StepEdit::Toggle {
            pad: PadId(1),
            step: 0
        }
    )
    .is_err());
}

#[test]
fn zoned_pad_projection_and_insertion_keep_ordinary_cells_as_the_only_pattern_authority() {
    let mut song = fixture();
    let zones = vec![VelocityZone {
        min_velocity: 0,
        max_velocity: 127,
        alternatives: vec![
            PadLayer {
                id: PadLayerId(77),
                region: RegionId(1),
            },
            PadLayer {
                id: PadLayerId(88),
                region: RegionId(1),
            },
        ],
    }];
    song.set_pad_sources(DrumKitId(1), PadId(1), None, zones)
        .unwrap();
    let projection = drum_steps::project(&song, &view()).unwrap();
    assert!(
        projection
            .lanes
            .iter()
            .find(|lane| lane.pad == PadId(1))
            .unwrap()
            .mapped
    );
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor
        .edit_drum_step(
            editor.revision(),
            &projection,
            &StepEdit::Toggle {
                pad: PadId(1),
                step: 0,
            },
        )
        .unwrap();
    let toggled = editor.song.clone();
    assert_eq!(at(&toggled, 0, 0).note, Note::Note(36));
    assert_eq!(at(&toggled, 0, 0).instrument, Some(InstrumentId(1)));
    assert_eq!(toggled.drum_kits, song.drum_kits);
    assert_eq!(toggled.samples, song.samples);
    assert_eq!(toggled.regions, song.regions);
    let fresh = drum_steps::project(&toggled, &view()).unwrap();
    let exact = fresh
        .lanes
        .iter()
        .find(|lane| lane.pad == PadId(1))
        .unwrap()
        .steps[0]
        .events[0]
        .clone();
    editor
        .edit_drum_step(
            editor.revision(),
            &fresh,
            &StepEdit::SetVelocity {
                event: exact,
                value: Some(0.0),
            },
        )
        .unwrap();
    assert_eq!(at(&editor.song, 0, 0).velocity, Some(0.0));
    assert_eq!(
        editor.song.drum_kits, song.drum_kits,
        "no round-robin counter or chosen layer is written into the document"
    );
    editor.undo();
    assert_eq!(editor.song, toggled);
    editor.undo();
    assert_eq!(editor.song, song);
    editor.redo();
    assert_eq!(editor.song, toggled);
}
