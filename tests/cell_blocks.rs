use aqua_pattern_core::{
    cell_blocks::{self, CellBlockSnapshot, PasteDestination, RectScope},
    project,
    song::*,
};
use std::collections::BTreeMap;

fn pattern(rows: u32) -> Pattern {
    Pattern {
        generation: None,
        track_aliases: BTreeMap::new(),
        name: "Existing".into(),
        rows,
        tracks: BTreeMap::new(),
    }
}
fn instrument() -> Instrument {
    Instrument {
        name: "Unassigned is legal for editing".into(),
        kind: InstrumentKind::Pitched,
        sample: None,
        region: None,
    }
}
fn song() -> Song {
    let mut s = example_song();
    s.instruments = BTreeMap::from([(InstrumentId(1), instrument())]);
    s.tracks = BTreeMap::from([
        (
            TrackId(1),
            Track {
                name: "A".into(),
                note_columns: 8,
            },
        ),
        (
            TrackId(5),
            Track {
                name: "B".into(),
                note_columns: 8,
            },
        ),
    ]);
    s.track_order = vec![TrackId(1), TrackId(5)];
    s.patterns = BTreeMap::from([
        (PatternId(1), pattern(32)),
        (PatternId(2), pattern(32)),
        (PatternId(3), pattern(32)),
    ]);
    s.order.clear();
    s
}
fn scope(p: u64, t: u64, r0: u32, r1: u32, c0: u16, c1: u16) -> RectScope {
    RectScope {
        pattern: PatternId(p),
        track: TrackId(t),
        row_start: r0,
        row_end: r1,
        column_start: c0,
        column_end: c1,
    }
}
fn dest(p: u64, t: u64, r: u32, c: u16) -> PasteDestination {
    PasteDestination {
        pattern: PatternId(p),
        track: TrackId(t),
        row_start: r,
        column_start: c,
    }
}
fn put(s: &mut Song, p: u64, t: u64, r: u32, c: u16, cell: Cell) {
    s.patterns
        .get_mut(&PatternId(p))
        .unwrap()
        .tracks
        .entry(TrackId(t))
        .or_default()
        .entry(r)
        .or_default()
        .insert(c, cell);
}
fn get(s: &Song, p: u64, t: u64, r: u32, c: u16) -> Option<&Cell> {
    s.patterns[&PatternId(p)]
        .tracks
        .get(&TrackId(t))
        .and_then(|rows| rows.get(&r))
        .and_then(|row| row.get(&c))
}
fn note(n: u8) -> Cell {
    Cell {
        note: Note::Note(n),
        instrument: Some(InstrumentId(1)),
        ..Default::default()
    }
}
fn bytes(v: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn rich() -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.12345679),
        pan: Some(-0.2345679),
        delay: Some(0.123456789012345),
        effects: StepEffects {
            retrigger: Some(Retrigger {
                repeats: 2,
                spacing: 0.123456789012345,
            }),
            release: Some(0.987654321098765),
            glide: Some(Glide {
                target: 73,
                semitones_per_second: 0.012345679,
            }),
            cut: None,
            offset: Some(u64::from(u32::MAX)),
            direction: Some(Direction::Reverse),
        },
        phrase: None,
    }
}
fn phrase() -> Phrase {
    Phrase {
        name: "Reference".into(),
        instrument: InstrumentId(1),
        rows: 2,
        lines_per_beat: 4,
        note_columns: 1,
        root_note: 60,
        cells: BTreeMap::from([(0, BTreeMap::from([(0, note(61))]))]),
    }
}
fn err_contains<T>(r: Result<T, String>, needle: &str) {
    let e = match r {
        Err(error) => error,
        Ok(_) => panic!("Expected refusal containing {needle:?}"),
    };
    assert!(e.contains(needle), "{e:?} missing {needle:?}");
}
fn manual_paste(s: &Song, clip: &CellBlockSnapshot, d: &PasteDestination) -> Song {
    let mut expected = s.clone();
    for (i, cell) in clip.cells().iter().enumerate() {
        let r = d.row_start + (i / usize::from(clip.columns())) as u32;
        let c = d.column_start + (i % usize::from(clip.columns())) as u16;
        if let Some(cell) = cell {
            put(&mut expected, d.pattern.0, d.track.0, r, c, cell.clone());
        } else if let Some(row) = expected
            .patterns
            .get_mut(&d.pattern)
            .unwrap()
            .tracks
            .get_mut(&d.track)
            .and_then(|rows| rows.get_mut(&r))
        {
            row.remove(&c);
        }
    }
    expected
}

#[test]
fn copy_preserves_every_cell_field_holes_defaults_and_source_without_io() {
    let mut s = song();
    s.phrases.insert(PhraseId(7), phrase());
    put(&mut s, 1, 1, 0, 0, rich());
    put(
        &mut s,
        1,
        1,
        0,
        1,
        Cell {
            note: Note::Off,
            velocity: Some(0.0),
            pan: Some(-0.0),
            delay: Some(-0.0),
            ..Default::default()
        },
    );
    put(&mut s, 1, 1, 0, 2, Cell::default());
    put(
        &mut s,
        1,
        1,
        1,
        1,
        Cell {
            note: Note::Empty,
            instrument: Some(InstrumentId(1)),
            velocity: Some(f32::from_bits(1)),
            pan: Some(-0.0),
            delay: Some(f64::from_bits(1)),
            effects: StepEffects {
                cut: Some(0.25),
                ..Default::default()
            },
            phrase: None,
        },
    );
    put(
        &mut s,
        1,
        1,
        1,
        2,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(7),
                cycles: 3,
            }),
            effects: StepEffects {
                release: Some(0.5),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let before = bytes(&s);
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 2, 0, 4)).unwrap();
    assert_eq!(
        (
            cp.rows(),
            cp.columns(),
            cp.summary().positions,
            cp.summary().stored_cells,
            cp.summary().holes,
            cp.summary().explicit_default_cells
        ),
        (2, 4, 8, 5, 3, 1)
    );
    assert_eq!(cp.cells()[0], Some(rich()));
    assert_eq!(cp.cells()[2], Some(Cell::default()));
    assert_eq!(cp.cells()[3], None);
    assert_eq!(
        cp.cells()[1].as_ref().unwrap().pan.unwrap().to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(
        cp.cells()[1].as_ref().unwrap().delay.unwrap().to_bits(),
        (-0.0f64).to_bits()
    );
    assert_eq!(cp.summary().serialized_bytes, bytes(&cp).len());
    assert_eq!(bytes(&s), before);
    assert_eq!(
        cp.source_document_sha256(),
        cell_blocks::document_sha256(&s).unwrap()
    );
    assert_eq!(
        cp.id(),
        cell_blocks::copy(&s, &scope(1, 1, 0, 2, 0, 4))
            .unwrap()
            .id()
    );
}

#[test]
fn pages_are_bounded_exact_row_major_and_preserve_missing_vs_default() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 1, Cell::default());
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 16, 0, 8)).unwrap();
    let first = cp.page(0, 64).unwrap();
    let second = cp.page(64, 64).unwrap();
    assert_eq!(
        (first.total, first.cells.len(), second.cells.len()),
        (128, 64, 64)
    );
    assert_eq!(first.cells[0].cell, None);
    assert_eq!(first.cells[1].cell, Some(Cell::default()));
    assert_eq!(
        (second.cells[0].row_offset, second.cells[0].column_offset),
        (8, 0)
    );
    assert!(cp.page(128, 1).unwrap().cells.is_empty());
    for (start, limit) in [(0, 0), (0, 65), (129, 1), (usize::MAX, 1)] {
        assert!(cp.page(start, limit).is_err());
    }
}

#[test]
fn paste_replace_erases_holes_but_stores_explicit_defaults_and_preserves_containers() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 0, note(61));
    put(&mut s, 1, 1, 0, 2, Cell::default());
    for r in 4..6 {
        for c in 1..5 {
            put(&mut s, 2, 1, r, c, note(80));
        }
    }
    put(&mut s, 2, 1, 10, 7, note(90));
    s.patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .insert(15, BTreeMap::new());
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 2, 0, 4)).unwrap();
    let d = dest(2, 1, 4, 1);
    let expected = manual_paste(&s, &cp, &d);
    let p = cell_blocks::paste_preview(&s, &cp, &d).unwrap();
    assert_eq!(bytes(p.candidate()), bytes(&expected));
    assert_eq!(
        (
            p.summary().positions,
            p.summary().changed_cells,
            p.summary().removed_cells,
            p.summary().replaced_cells
        ),
        (8, 8, 6, 2)
    );
    assert!(p.candidate().patterns[&PatternId(2)].tracks[&TrackId(1)][&5].is_empty());
    assert!(p.candidate().patterns[&PatternId(2)].tracks[&TrackId(1)].contains_key(&15));
    assert_eq!(get(p.candidate(), 2, 1, 4, 3), Some(&Cell::default()));
    assert_eq!(get(p.candidate(), 2, 1, 4, 2), None);
    assert_eq!(get(p.candidate(), 2, 1, 10, 7), Some(&note(90)));
    assert_eq!(
        bytes(&s.patterns[&PatternId(1)]),
        bytes(&p.candidate().patterns[&PatternId(1)])
    );
    assert_eq!(p.summary().diff_bytes, bytes(&p.changes()).len());
    assert_eq!(
        p.summary().projected_work,
        cell_blocks::check_document_work(p.candidate()).unwrap()
    );
}

#[test]
fn overlap_both_directions_uses_immutable_source_and_reusable_snapshot() {
    let mut s = song();
    for r in 0..5 {
        for c in 0..4 {
            put(&mut s, 1, 1, r, c, note(40 + (r * 4 + u32::from(c)) as u8));
        }
    }
    for (src, d) in [
        (scope(1, 1, 0, 3, 0, 3), dest(1, 1, 1, 1)),
        (scope(1, 1, 1, 4, 1, 4), dest(1, 1, 0, 0)),
    ] {
        let cp = cell_blocks::copy(&s, &src).unwrap();
        let original = bytes(&cp);
        let p = cell_blocks::paste_preview(&s, &cp, &d).unwrap();
        assert_eq!(bytes(p.candidate()), bytes(&manual_paste(&s, &cp, &d)));
        let repeat = cell_blocks::paste_preview(p.candidate(), &cp, &dest(2, 1, 0, 0)).unwrap();
        assert_eq!(
            bytes(repeat.candidate()),
            bytes(&manual_paste(p.candidate(), &cp, &dest(2, 1, 0, 0)))
        );
        assert_eq!(bytes(&cp), original);
    }
}

#[test]
fn same_location_and_all_hole_paste_are_bit_exact_noops_without_new_containers() {
    let mut s = song();
    put(
        &mut s,
        1,
        1,
        0,
        0,
        Cell {
            pan: Some(-0.0),
            delay: Some(-0.0),
            ..Default::default()
        },
    );
    put(&mut s, 1, 1, 0, 1, Cell::default());
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 2, 0, 3)).unwrap();
    let p = cell_blocks::paste_preview(&s, &cp, &dest(1, 1, 0, 0)).unwrap();
    assert!(p.changes().is_empty());
    assert_eq!(p.base_sha256(), p.candidate_sha256());
    assert_eq!(bytes(p.candidate()), bytes(&s));
    let holes = cell_blocks::copy(&s, &scope(3, 1, 0, 2, 0, 3)).unwrap();
    let p = cell_blocks::paste_preview(&s, &holes, &dest(2, 1, 0, 0)).unwrap();
    assert!(p.changes().is_empty());
    assert!(!p.candidate().patterns[&PatternId(2)]
        .tracks
        .contains_key(&TrackId(1)));
    let p = cell_blocks::clear_preview(&s, &scope(3, 1, 0, 2, 0, 3)).unwrap();
    assert!(p.changes().is_empty());
    assert_eq!(bytes(p.candidate()), bytes(&s));
}

#[test]
fn clear_removes_off_scalar_fx_phrase_and_defaults_without_pruning_rows_tracks() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 0, note(60));
    put(&mut s, 1, 1, 0, 1, Cell::default());
    put(
        &mut s,
        1,
        1,
        1,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    put(&mut s, 1, 1, 2, 0, rich());
    s.phrases.insert(PhraseId(1), phrase());
    put(
        &mut s,
        1,
        1,
        3,
        0,
        Cell {
            note: Note::Note(60),
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 1,
            }),
            ..Default::default()
        },
    );
    put(
        &mut s,
        1,
        1,
        4,
        0,
        Cell {
            velocity: Some(0.5),
            ..Default::default()
        },
    );
    let p = cell_blocks::clear_preview(&s, &scope(1, 1, 0, 5, 0, 2)).unwrap();
    assert_eq!(p.summary().removed_cells, 6);
    assert!(p.changes().iter().all(|c| c.after.is_none()));
    assert_eq!(
        p.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)].len(),
        5
    );
    assert!(p.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)]
        .values()
        .all(BTreeMap::is_empty));
    assert_eq!(bytes(&s.phrases), bytes(&p.candidate().phrases));
}

#[test]
fn source_alias_is_readonly_frozen_and_track_constraint_is_logical_not_terminal() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 0, note(60));
    s.patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(5),
            TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
    let cp = cell_blocks::copy(&s, &scope(2, 5, 0, 1, 0, 1)).unwrap();
    assert_eq!(
        cp.source_chain(),
        &[
            TrackAlias {
                pattern: PatternId(2),
                track: TrackId(5)
            },
            TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1)
            }
        ]
    );
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(3, 1, 0, 0)),
        "same logical TrackId",
    );
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 5, 0, 0)),
        "Aliased track",
    );
    err_contains(
        cell_blocks::clear_preview(&s, &scope(2, 5, 0, 1, 0, 1)),
        "Aliased track",
    );
    put(&mut s, 1, 1, 0, 0, note(70));
    let p = cell_blocks::paste_preview(&s, &cp, &dest(3, 5, 0, 0)).unwrap();
    assert_eq!(get(p.candidate(), 3, 5, 0, 0), Some(&note(60)));
    assert_eq!(get(p.candidate(), 1, 1, 0, 0), Some(&note(70)));
}

#[test]
fn bounds_unknown_ids_overflow_and_position_limit_reject_without_cropping() {
    let mut s = song();
    s.patterns.get_mut(&PatternId(1)).unwrap().rows = 5000;
    assert_eq!(
        cell_blocks::copy(&s, &scope(1, 1, 0, 512, 0, 8))
            .unwrap()
            .cells()
            .len(),
        4096
    );
    let empty = cell_blocks::clear_preview(&s, &scope(1, 1, 0, 512, 0, 8)).unwrap();
    assert_eq!(empty.summary().positions, 4096);
    assert!(empty.changes().is_empty());
    assert_eq!(bytes(empty.candidate()), bytes(&s));
    for bad in [
        scope(1, 1, 0, 4097, 0, 1),
        scope(1, 1, 4, 4, 0, 1),
        scope(1, 1, 4, 3, 0, 1),
        scope(1, 1, 0, 1, 2, 2),
        scope(1, 1, 0, 1, 0, 9),
        scope(1, 1, 4999, 5001, 0, 1),
        scope(99, 1, 0, 1, 0, 1),
        scope(1, 99, 0, 1, 0, 1),
    ] {
        assert!(cell_blocks::copy(&s, &bad).is_err());
    }
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 2, 0, 2)).unwrap();
    let before = bytes(&s);
    for d in [
        dest(2, 1, 31, 0),
        dest(2, 1, 0, 7),
        dest(2, 1, u32::MAX, 0),
        dest(2, 1, 0, u16::MAX),
        dest(99, 1, 0, 0),
        dest(2, 5, 0, 0),
    ] {
        assert!(cell_blocks::paste_preview(&s, &cp, &d).is_err());
    }
    assert_eq!(bytes(&s), before);
}

#[test]
fn signed_zero_only_paste_is_a_real_change_and_stale_base_guards_are_exact() {
    let mut s = song();
    let plus = Cell {
        velocity: Some(0.0),
        pan: Some(0.0),
        delay: Some(0.0),
        ..Default::default()
    };
    let minus = Cell {
        velocity: Some(-0.0),
        pan: Some(-0.0),
        delay: Some(-0.0),
        ..Default::default()
    };
    put(&mut s, 1, 1, 0, 0, minus.clone());
    put(&mut s, 2, 1, 0, 0, plus.clone());
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    let p = cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)).unwrap();
    assert_eq!(p.candidate(), &s);
    assert_eq!(p.summary().changed_cells, 1);
    assert_ne!(p.base_sha256(), p.candidate_sha256());
    assert_eq!(
        bytes(get(p.candidate(), 2, 1, 0, 0).unwrap()),
        bytes(&minus)
    );
    let mut changed = s.clone();
    put(&mut changed, 2, 1, 0, 0, minus);
    assert_eq!(changed, s);
    assert!(!p.is_current(&changed).unwrap());
    assert!(p.commit(&changed).is_err());
    assert_eq!(bytes(&p.commit(&s).unwrap()), bytes(p.candidate()));
    changed = s.clone();
    changed.title.push('!');
    assert!(p.commit(&changed).is_err());
    changed = s.clone();
    put(
        &mut changed,
        2,
        1,
        0,
        0,
        Cell {
            delay: Some(f64::from_bits(1)),
            ..plus
        },
    );
    assert!(p.commit(&changed).is_err());
}

#[test]
fn clipboard_ids_resolve_current_document_and_dangling_or_invalid_phrase_refs_refuse() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 0, note(60));
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    s.instruments.clear();
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "unknown instrument",
    );
    s.instruments.insert(
        InstrumentId(1),
        Instrument {
            name: "Reused current mapping".into(),
            ..instrument()
        },
    );
    let p = cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)).unwrap();
    assert_eq!(
        p.candidate().instruments[&InstrumentId(1)].name,
        "Reused current mapping"
    );
    s.phrases.insert(PhraseId(1), phrase());
    put(
        &mut s,
        1,
        1,
        0,
        0,
        Cell {
            note: Note::Note(60),
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 1,
            }),
            ..Default::default()
        },
    );
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    s.phrases.clear();
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "unknown phrase",
    );
    let mut changed = phrase();
    changed.root_note = 0;
    changed.cells.get_mut(&0).unwrap().get_mut(&0).unwrap().note = Note::Note(127);
    s.phrases.insert(PhraseId(1), changed);
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "transposition",
    );
}

#[test]
fn projected_new_local_block_counts_every_alias_repeated_and_muted_use() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 0, note(60));
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    s.patterns
        .get_mut(&PatternId(3))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(2),
                track: TrackId(1),
            },
        );
    let mut p = pattern(32);
    p.track_aliases.insert(
        TrackId(5),
        TrackAlias {
            pattern: PatternId(3),
            track: TrackId(1),
        },
    );
    s.patterns.insert(PatternId(4), p);
    s.order = [2, 3, 4, 4]
        .map(|p| Occurrence {
            pattern: PatternId(p),
            muted_tracks: if p == 4 {
                [TrackId(5)].into_iter().collect()
            } else {
                Default::default()
            },
            section: None,
        })
        .to_vec();
    let p = cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 7, 3)).unwrap();
    assert_eq!(
        (
            p.summary().base_work.definition_cells,
            p.summary().projected_work.definition_cells,
            p.summary().projected_work.ordered_cells
        ),
        (1, 4, 4)
    );
    assert_eq!(
        p.summary().projected_work,
        cell_blocks::check_document_work(p.candidate()).unwrap()
    );
    assert_eq!(p.aliases().len(), 2);
    assert_eq!(p.contexts().len(), 4);
    assert_eq!(p.contexts().iter().filter(|c| c.muted).count(), 2);
    assert!(!s.patterns[&PatternId(2)].tracks.contains_key(&TrackId(1)));
}

#[test]
fn projected_ordered_boundary_refuses_before_invalid_model_validation() {
    let mut s = song();
    s.tracks.remove(&TrackId(5));
    s.track_order = vec![TrackId(1)];
    put(&mut s, 1, 1, 0, 0, Cell::default());
    put(&mut s, 1, 1, 0, 1, Cell::default());
    let one = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    let two = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 2)).unwrap();
    s.order = vec![
        Occurrence {
            pattern: PatternId(2),
            muted_tracks: Default::default(),
            section: None,
        };
        65536
    ];
    let p = cell_blocks::paste_preview(&s, &one, &dest(2, 1, 0, 0)).unwrap();
    assert_eq!(p.summary().projected_work.ordered_cells, 65536);
    s.schema = 999;
    err_contains(
        cell_blocks::paste_preview(&s, &two, &dest(2, 1, 0, 0)),
        "65536 ordered row/cell",
    );
}

#[test]
fn projected_definition_boundary_includes_unused_aliases_and_new_rows() {
    let mut s = song();
    s.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    s.patterns.get_mut(&PatternId(1)).unwrap().rows = 65536;
    s.patterns.get_mut(&PatternId(2)).unwrap().rows = 65536;
    for i in 0..1024 {
        put(&mut s, 1, 1, i / 256, (i % 256) as u16, Cell::default());
    }
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 4, 0, 256)).unwrap();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    for id in 3..=65 {
        let mut p = pattern(65536);
        p.track_aliases.insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(2),
                track: TrackId(1),
            },
        );
        s.patterns.insert(PatternId(id), p);
    }
    let p = cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)).unwrap();
    assert_eq!(p.summary().projected_work.definition_cells, 65536);
    assert_eq!(
        p.summary().projected_work,
        cell_blocks::check_document_work(p.candidate()).unwrap()
    );
    let mut extra = pattern(65536);
    extra.track_aliases.insert(
        TrackId(1),
        TrackAlias {
            pattern: PatternId(2),
            track: TrackId(1),
        },
    );
    s.patterns.insert(PatternId(66), extra);
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "resolved definition",
    );
    let mut s = song();
    s.patterns.get_mut(&PatternId(2)).unwrap().rows = 65536;
    put(&mut s, 1, 1, 0, 0, Cell::default());
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    s.patterns.get_mut(&PatternId(2)).unwrap().tracks.insert(
        TrackId(1),
        (0..65535).map(|r| (r, BTreeMap::new())).collect(),
    );
    assert_eq!(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 65535, 0))
            .unwrap()
            .summary()
            .projected_work
            .definition_rows,
        65536
    );
    s.patterns
        .get_mut(&PatternId(3))
        .unwrap()
        .tracks
        .insert(TrackId(1), BTreeMap::from([(0, BTreeMap::new())]));
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 65535, 0)),
        "resolved definition",
    );
}

#[test]
fn projected_validation_product_exact_limit_and_literal_plus_one() {
    let mut s = song();
    s.tracks.remove(&TrackId(5));
    s.track_order = vec![TrackId(1)];
    put(&mut s, 1, 1, 0, 0, Cell::default());
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    for id in 2..=16 {
        s.instruments.insert(InstrumentId(id), instrument());
    }
    s.order = vec![
        Occurrence {
            pattern: PatternId(2),
            muted_tracks: Default::default(),
            section: None,
        };
        65534
    ];
    assert_eq!(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0))
            .unwrap()
            .summary()
            .projected_work
            .validation_units,
        1048576
    );
    s.instruments.insert(InstrumentId(17), instrument());
    s.order.truncate(61679);
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "work 1048577 exceeds 1048576",
    );
}

#[test]
fn clipboard_one_mib_and_diff_two_mib_are_whole_request_refusals() {
    let mut s = song();
    for p in [1, 2] {
        s.patterns.get_mut(&PatternId(p)).unwrap().rows = 4096;
    }
    for r in 0..4096 {
        put(&mut s, 1, 1, r, 0, rich());
    }
    err_contains(
        cell_blocks::copy(&s, &scope(1, 1, 0, 4096, 0, 1)),
        "Cell clipboard exceeds",
    );
    let (mut lo, mut hi) = (1, 4096);
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if cell_blocks::copy(&s, &scope(1, 1, 0, mid, 0, 1)).is_ok() {
            lo = mid
        } else {
            hi = mid
        }
    }
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, lo, 0, 1)).unwrap();
    assert_eq!(cp.summary().serialized_bytes, bytes(&cp).len());
    assert!(bytes(&cp).len() <= cell_blocks::MAX_CLIPBOARD_BYTES);
    assert!(cell_blocks::copy(&s, &scope(1, 1, 0, lo + 1, 0, 1)).is_err());
    // Drop unselected source rows to keep the separate base-document budget
    // below8MiB while probing a rich before+after diff at clipboard capacity.
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .retain(|&r, _| r < lo);
    for r in 0..lo {
        let mut c = rich();
        c.note = Note::Note(61);
        put(&mut s, 2, 1, r, 0, c);
    }
    assert!(project::check_serialized_size(&s).is_ok());
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "Cell-block diff exceeds",
    );
    let half = cell_blocks::copy(&s, &scope(1, 1, 0, lo / 2, 0, 1)).unwrap();
    let p = cell_blocks::paste_preview(&s, &half, &dest(2, 1, 0, 0)).unwrap();
    assert_eq!(p.summary().diff_bytes, bytes(&p.changes()).len());
}

#[test]
fn exact_eight_mib_candidate_and_base_budget_precedence() {
    let mut s = song();
    s.title.clear();
    put(&mut s, 1, 1, 0, 0, note(60));
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    let overhead = project::check_serialized_size(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0))
            .unwrap()
            .candidate(),
    )
    .unwrap();
    s.title = "x".repeat(8 * 1024 * 1024 - overhead);
    let p = cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)).unwrap();
    assert_eq!(
        project::check_serialized_size(p.candidate()).unwrap(),
        8 * 1024 * 1024
    );
    s.title.push('x');
    assert!(project::check_serialized_size(&s).is_ok());
    err_contains(
        cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)),
        "8 MiB",
    );
    s.title = "x".repeat(8 * 1024 * 1024);
    s.schema = 999;
    err_contains(cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)), "8 MiB");
}

#[test]
fn copy_and_plan_are_transient_no_schema_or_source_metadata_changes() {
    let mut s = song();
    put(&mut s, 1, 1, 0, 0, note(60));
    let cp = cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).unwrap();
    let p = cell_blocks::paste_preview(&s, &cp, &dest(2, 1, 0, 0)).unwrap();
    let candidate = p.commit(&s).unwrap();
    let mut recovered = candidate.clone();
    recovered
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .clear();
    assert_eq!(bytes(&recovered), bytes(&s));
    let encoded = bytes(&candidate);
    let decoded: Song = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(encoded, bytes(&decoded));
    assert!(!String::from_utf8(encoded).unwrap().contains("cell_blocks"));
    assert_eq!(bytes(&s), bytes(p.base()));
}

#[test]
fn immutable_registered_source_rejects_lexically_changed_equal_paths() {
    let mut s = song();
    let source = SampleAsset {
        path: "audio//source.wav".into(),
        sha256: "1".repeat(64),
        sample_rate: 48000,
        channels: 1,
        frames: 100,
    };
    s.register_sample(SampleId(1), source.clone()).unwrap();
    let before = bytes(&s);
    s.register_sample(SampleId(1), source.clone()).unwrap();
    assert_eq!(bytes(&s), before);
    let mut altered = source;
    altered.path = "audio/source.wav".into();
    assert_eq!(s.samples[&SampleId(1)], altered);
    assert_ne!(
        s.samples[&SampleId(1)].path.as_os_str(),
        altered.path.as_os_str()
    );
    assert!(s.register_sample(SampleId(1), altered.clone()).is_err());
    assert_eq!(bytes(&s), before);
    // New source ID is the existing explicit way to change immutable metadata.
    s.register_sample(SampleId(2), altered).unwrap();
    assert_eq!(s.samples.len(), 2);
}

#[test]
fn phrase_work_cap_includes_unused_default_cells_before_model_validation() {
    let mut s = song();
    let cells: BTreeMap<_, _> = (0..1024)
        .map(|r| (r, (0..16).map(|c| (c, Cell::default())).collect()))
        .collect();
    for id in 1..=2 {
        s.phrases.insert(
            PhraseId(id),
            Phrase {
                rows: 1024,
                note_columns: 16,
                cells: cells.clone(),
                ..phrase()
            },
        );
    }
    assert_eq!(
        cell_blocks::check_document_work(&s).unwrap().phrase_cells,
        32768
    );
    assert!(cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)).is_ok());
    s.phrases.insert(
        PhraseId(3),
        Phrase {
            cells: BTreeMap::from([(0, BTreeMap::from([(0, Cell::default())]))]),
            ..phrase()
        },
    );
    s.schema = 999;
    err_contains(
        cell_blocks::copy(&s, &scope(1, 1, 0, 1, 0, 1)),
        "32768 phrase-definition",
    );
}
