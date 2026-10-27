use aqua_pattern_core::{
    mutations::{self, ThinConfig, ThinReason},
    slice_features::{Articulation, PercussionRole, RoleLabel},
    song::*,
    transforms::TransformScope,
};
use std::collections::BTreeMap;
fn label(role: PercussionRole) -> RoleLabel {
    RoleLabel {
        role,
        articulation: Articulation::Normal,
        note: "Explicit human review".into(),
    }
}
fn note(instrument: u64) -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(instrument)),
        velocity: Some(0.75),
        pan: Some(0.0),
        ..Default::default()
    }
}
fn pattern(rows: u32) -> Pattern {
    Pattern {
        generation: None,
        track_aliases: Default::default(),
        name: "Thin source".into(),
        rows,
        tracks: BTreeMap::new(),
    }
}
fn put(s: &mut Song, p: u64, row: u32, cell: Cell) {
    s.patterns
        .get_mut(&PatternId(p))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(0, cell);
}
fn cell(s: &mut Song, row: u32) -> &mut Cell {
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&row)
        .unwrap()
        .get_mut(&0)
        .unwrap()
}
fn base() -> Song {
    let mut s = example_song();
    s.patterns = BTreeMap::from([(PatternId(1), pattern(8))]);
    s.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: Default::default(),
        section: None,
    }];
    s.tracks.retain(|id, _| *id == TrackId(1));
    s.track_order = vec![TrackId(1)];
    s.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 1;
    s.samples.insert(
        SampleId(1),
        SampleAsset {
            path: "/missing/no-thin-io.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: 48000,
            channels: 1,
            frames: 48000,
        },
    );
    for (id, start, end) in [(1, 0, 24000), (2, 24000, 48000), (3, 0, 12000)] {
        s.regions.insert(
            RegionId(id),
            SampleRegion {
                name: format!("R{id}"),
                sample: SampleId(1),
                start_frame: start,
                end_frame: end,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        );
    }
    for id in 1..=2 {
        let i = s.instruments.get_mut(&InstrumentId(id)).unwrap();
        i.sample = None;
        i.region = Some(RegionId(id));
    }
    for row in 0..8 {
        put(
            &mut s,
            1,
            row,
            note(if row.is_multiple_of(3) { 2 } else { 1 }),
        );
    }
    for (id, role) in [
        (1, PercussionRole::ClosedHat),
        (2, PercussionRole::Kick),
        (3, PercussionRole::ClosedHat),
    ] {
        s.set_region_role(RegionId(id), Some(label(role))).unwrap();
    }
    s.validate().unwrap();
    s
}
fn config() -> ThinConfig {
    ThinConfig {
        version: 1,
        scope: TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 8,
            columns: vec![0],
        },
        seed: 0,
        removal_percent: 100,
        max_removed: 4096,
        protected_roles: vec![PercussionRole::Kick, PercussionRole::Snare],
    }
}
#[test]
fn full_cell_deletion_protects_current_roles_and_preserves_all_other_storage() {
    let s = base();
    let p = mutations::preview(&s, config()).unwrap();
    assert_eq!(p.summary().removed_cells, 5);
    assert_eq!(p.summary().candidate_starts, 3);
    assert_eq!(p.summary().random_draws, 0);
    assert_eq!(p.summary().gate_calls, 5);
    let mut expected = s.clone();
    for row in [1, 2, 4, 5, 7] {
        expected
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&row)
            .unwrap()
            .remove(&0);
    }
    assert_eq!(p.candidate(), &expected);
    assert_eq!(p.commit(&s).unwrap(), expected);
    assert_eq!(p.record().change_count, 5);
    assert!(p.changes().iter().all(|c| c.after.is_none()));
    assert_eq!(
        p.decisions()
            .iter()
            .filter(|d| d.reason == ThinReason::ProtectedRole)
            .count(),
        3
    );
    assert!(!serde_json::to_string(p.candidate())
        .unwrap()
        .contains("mutations"));
    assert_eq!(s.region_catalog, p.candidate().region_catalog);
    let mut silent = s.clone();
    for row in 0..8 {
        cell(&mut silent, row).velocity = Some(0.0);
    }
    let zero_gain = mutations::preview(&silent, config()).unwrap();
    assert_eq!(zero_gain.summary().eligible_cells, 5);
    assert_eq!(zero_gain.summary().removed_starts, 5);
}
#[test]
fn real_pattrns_gate_decisions_are_reproducible_seeded_and_endpoint_exact() {
    use pattrns::{gates::ProbabilityGate, Gate, RhythmEvent};
    let mut s = base();
    s.patterns.get_mut(&PatternId(1)).unwrap().rows = 64;
    for row in 8..64 {
        put(
            &mut s,
            1,
            row,
            note(if row.is_multiple_of(8) { 2 } else { 1 }),
        );
    }
    let mut c = config();
    c.scope.row_end = 64;
    for seed in [0, 1, 42, u64::MAX] {
        for percent in [0, 1, 37, 50, 99, 100] {
            c.seed = seed;
            c.removal_percent = percent;
            let p = mutations::preview(&s, c.clone()).unwrap();
            let q = mutations::preview(&s, c.clone()).unwrap();
            assert_eq!(p.id(), q.id());
            assert_eq!(p.decisions(), q.decisions());
            let probability = (f64::from(percent) / 100.0) as f32;
            let mut gate = ProbabilityGate::new(Some(seed));
            for d in p.decisions() {
                if matches!(d.reason, ThinReason::Removed | ThinReason::GateKept) {
                    let wanted = gate.run(&RhythmEvent {
                        value: probability,
                        step_time: 1.0,
                    });
                    assert_eq!(d.reason == ThinReason::Removed, wanted);
                }
            }
            if percent == 0 {
                assert_eq!(p.candidate(), &s);
                assert_eq!(p.summary().gate_calls, 0);
            }
        }
    }
    c.removal_percent = 50;
    c.seed = 1;
    let a = mutations::preview(&s, c.clone()).unwrap();
    assert!((2..12).any(|seed| {
        c.seed = seed;
        mutations::preview(&s, c.clone()).unwrap().candidate() != a.candidate()
    }));
}
#[test]
fn zero_caps_stale_base_and_model_prerequisites_are_explicit_atomic() {
    let s = base();
    let mut c = config();
    c.removal_percent = 0;
    c.max_removed = 0;
    let p = mutations::preview(&s, c.clone()).unwrap();
    assert_eq!(p.candidate(), &s);
    let mut changed = s.clone();
    changed.title.push('x');
    assert!(p.commit(&changed).unwrap_err().contains("stale"));
    c.removal_percent = 100;
    assert!(mutations::preview(&s, c.clone())
        .unwrap_err()
        .contains("max_removed"));
    c.max_removed = 4097;
    assert!(mutations::preview(&s, c.clone()).is_err());
    c = config();
    c.protected_roles = vec![PercussionRole::Unknown];
    assert!(mutations::preview(&s, c).is_err());
    let mut missing = s.clone();
    missing
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .region = None;
    let mut zero = config();
    zero.removal_percent = 0;
    assert!(mutations::preview(&missing, zero)
        .unwrap_err()
        .contains("sources assigned"));
    assert_eq!(s, base());
}
#[test]
fn unreviewed_stale_unknown_mixed_and_legacy_uses_never_infer_permission() {
    for kind in 0..5 {
        let mut s = base();
        match kind {
            0 => {
                s.set_region_role(RegionId(1), None).unwrap();
            }
            1 => {
                s.regions.get_mut(&RegionId(1)).unwrap().end_frame -= 1;
            }
            2 => {
                s.set_region_role(RegionId(1), Some(label(PercussionRole::Unknown)))
                    .unwrap();
            }
            3 => {
                s.set_region_role(RegionId(1), Some(label(PercussionRole::Mixed)))
                    .unwrap();
            }
            _ => {
                let i = s.instruments.get_mut(&InstrumentId(1)).unwrap();
                i.region = None;
                i.sample = Some("/missing/legacy.wav".into());
            }
        }
        let p = mutations::preview(&s, config()).unwrap();
        assert_eq!(p.candidate(), &s);
        assert_eq!(p.summary().gate_calls, 0);
    }
    let mut s = base();
    let mut l = label(PercussionRole::Kick);
    l.articulation = Articulation::Ghost;
    s.set_region_role(RegionId(1), Some(l)).unwrap();
    assert_eq!(mutations::preview(&s, config()).unwrap().candidate(), &s);
}
#[test]
fn role_protection_is_union_of_all_current_inherited_instrument_uses() {
    let mut s = base();
    s.patterns = BTreeMap::from([
        (PatternId(1), pattern(1)),
        (PatternId(2), pattern(1)),
        (PatternId(3), pattern(1)),
    ]);
    let mut hit = note(1);
    hit.instrument = None;
    put(&mut s, 1, 0, hit);
    put(
        &mut s,
        2,
        0,
        Cell {
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    put(
        &mut s,
        3,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..Default::default()
        },
    );
    s.order = [2, 1, 3, 1]
        .into_iter()
        .map(|id| Occurrence {
            pattern: PatternId(id),
            muted_tracks: Default::default(),
            section: None,
        })
        .collect();
    let mut c = config();
    c.scope.row_end = 1;
    let p = mutations::preview(&s, c).unwrap();
    assert_eq!(p.candidate(), &s);
    assert_eq!(p.decisions()[0].uses, 2);
    assert_eq!(p.decisions()[0].reason, ThinReason::ProtectedRole);
    assert_eq!(
        p.uses()
            .iter()
            .filter(|u| u.retention == Some(ThinReason::ProtectedRole))
            .count(),
        1
    );
}
#[test]
fn alias_repeat_uses_are_affected_but_independent_copies_and_mutes_are_not_guessed() {
    let mut s = base();
    s.tracks.insert(
        TrackId(2),
        Track {
            name: "Alias".into(),
            note_columns: 1,
        },
    );
    s.track_order.push(TrackId(2));
    let original = s.patterns[&PatternId(1)].clone();
    let mut alias = pattern(8);
    alias.track_aliases.insert(
        TrackId(2),
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        },
    );
    s.patterns.insert(PatternId(2), alias);
    s.patterns.insert(PatternId(3), original.clone());
    s.order = [1, 2, 3, 1]
        .into_iter()
        .map(|id| Occurrence {
            pattern: PatternId(id),
            muted_tracks: Default::default(),
            section: None,
        })
        .collect();
    s.order[3].muted_tracks.insert(TrackId(1));
    let p = mutations::preview(&s, config()).unwrap();
    assert_eq!(p.summary().affected_blocks, 3);
    assert_eq!(p.summary().affected_aliases, 1);
    assert_eq!(p.summary().removed_starts, 10);
    assert_eq!(p.candidate().patterns[&PatternId(3)], original);
    assert_eq!(p.contexts().iter().filter(|c| c.muted).count(), 1);
    let mut bad = config();
    bad.scope.pattern = PatternId(2);
    bad.scope.track = TrackId(2);
    assert!(mutations::preview(&s, bad)
        .unwrap_err()
        .to_lowercase()
        .contains("alias"));
}
#[test]
fn retained_on_inherited_gain_changes_reject_not_silently_rebind() {
    let mut s = base();
    cell(&mut s, 0).velocity = Some(0.8);
    cell(&mut s, 1).velocity = Some(0.2);
    *cell(&mut s, 2) = note(2);
    cell(&mut s, 2).velocity = None;
    let mut c = config();
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    let before = s.clone();
    assert!(mutations::preview(&s, c.clone())
        .unwrap_err()
        .contains("retained-ON"));
    assert_eq!(s, before);
    cell(&mut s, 2).velocity = Some(0.2);
    assert_eq!(
        mutations::preview(&s, c).unwrap().summary().removed_cells,
        1
    );
}
#[test]
fn retained_non_lifetime_controls_must_not_retarget_an_older_voice() {
    for kind in 0..6 {
        let mut s = base();
        let mut control = Cell::default();
        match kind {
            0 => control.velocity = Some(0.2),
            1 => control.pan = Some(0.3),
            2 => {
                control.effects.glide = Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                })
            }
            3 => control.effects.offset = Some(5),
            4 => control.effects.direction = Some(Direction::Reverse),
            _ => control.effects.cut = Some(0.5),
        }
        *cell(&mut s, 2) = control;
        let mut c = config();
        c.scope.row_start = 1;
        c.scope.row_end = 2;
        let error = mutations::preview(&s, c).unwrap_err();
        assert!(error.contains("retained-control"), "{kind}: {error}");
    }
    let mut s = base();
    *cell(&mut s, 2) = Cell {
        note: Note::Off,
        ..Default::default()
    };
    let mut c = config();
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    assert_eq!(
        mutations::preview(&s, c).unwrap().summary().removed_cells,
        1
    );
}
fn rr(s: &mut Song) {
    s.drum_kits.insert(
        DrumKitId(1),
        DrumKitInstrument {
            name: "Reviewed RR".into(),
            pads: BTreeMap::from([(
                PadId(1),
                DrumPad {
                    slot: 0,
                    name: "Alternatives".into(),
                    note: 60,
                    region: None,
                    velocity_zones: vec![VelocityZone {
                        min_velocity: 0,
                        max_velocity: 127,
                        alternatives: vec![
                            PadLayer {
                                id: PadLayerId(19),
                                region: RegionId(1),
                            },
                            PadLayer {
                                id: PadLayerId(2),
                                region: RegionId(3),
                            },
                        ],
                    }],
                    root_note: 60,
                    tune_semitones: 0,
                    gain: 1.0,
                    pan: 0.0,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: None,
                },
            )]),
        },
    );
    s.assign_kit(InstrumentId(1), Some(DrumKitId(1))).unwrap();
}
#[test]
fn round_robin_source_reassignment_rejects_but_whole_cycles_may_preserve_sources() {
    let mut s = base();
    rr(&mut s);
    let mut c = config();
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    assert!(mutations::preview(&s, c.clone())
        .unwrap_err()
        .contains("retained-ON"));
    c.scope.row_end = 3;
    let p = mutations::preview(&s, c).unwrap();
    assert_eq!(p.summary().removed_cells, 2);
    assert!(p.uses().iter().any(|u| matches!(
        u.source,
        mutations::ThinSource::DrumPad {
            layer: Some(PadLayerId(19)),
            ..
        }
    )));
}
#[test]
fn unsupported_and_suppressed_cells_are_counted_without_gate_calls() {
    let mut s = base();
    cell(&mut s, 1).effects.release = Some(0.5);
    let mut c = config();
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    let p = mutations::preview(&s, c.clone()).unwrap();
    assert_eq!(p.decisions()[0].reason, ThinReason::Effects);
    assert_eq!(p.candidate(), &s);
    s.patterns.insert(PatternId(2), pattern(1));
    s.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: [TrackId(1)].into_iter().collect(),
        section: None,
    });
    cell(&mut s, 7).delay = Some(f64::from_bits(1.0f64.to_bits() - 1));
    c.scope.row_start = 7;
    c.scope.row_end = 8;
    let p = mutations::preview(&s, c).unwrap();
    assert_eq!(p.decisions()[0].reason, ThinReason::NoAudibleUse);
    assert_eq!(p.summary().gate_calls, 0);
}
#[test]
fn removal_cannot_expose_previously_replaced_phrase_children() {
    let mut s = base();
    let phrase = Phrase {
        name: "Four roots".into(),
        instrument: InstrumentId(2),
        rows: 4,
        lines_per_beat: 4,
        note_columns: 1,
        root_note: 60,
        cells: (0..4)
            .map(|r| {
                (
                    r,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Note(60),
                            ..Default::default()
                        },
                    )]),
                )
            })
            .collect(),
    };
    s.phrases.insert(PhraseId(1), phrase);
    cell(&mut s, 0).phrase = Some(PhraseTrigger {
        phrase: PhraseId(1),
        cycles: 1,
    });
    let mut c = config();
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    assert!(mutations::preview(&s, c)
        .unwrap_err()
        .contains("retained-ON"));
    let mut c = config();
    c.scope.row_end = 1;
    assert_eq!(
        mutations::preview(&s, c).unwrap().decisions()[0].reason,
        ThinReason::Phrase
    );
}
#[test]
fn path_payload_and_report_scan_guards_precede_expensive_evaluation() {
    let mut s = base();
    s.samples.get_mut(&SampleId(1)).unwrap().path = format!("/{}", "p".repeat(8191)).into();
    let mut zero = config();
    zero.removal_percent = 0;
    let p = mutations::preview(&s, zero.clone()).unwrap();
    assert_eq!(
        p.summary().estimated_path_payload,
        mutations::MAX_PATH_PAYLOAD
    );
    s.instruments.insert(
        InstrumentId(99),
        Instrument {
            name: "Unused long path".into(),
            kind: InstrumentKind::Pitched,
            sample: Some(format!("/{}", "p".repeat(8192)).into()),
            region: None,
        },
    );
    assert!(mutations::preview(&s, zero.clone())
        .unwrap_err()
        .contains("path payload"));
    let mut s = base();
    s.patterns.insert(PatternId(2), pattern(5000));
    for r in 0..4097 {
        put(
            &mut s,
            2,
            r,
            Cell {
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        );
    }
    zero.scope.pattern = PatternId(2);
    zero.scope.row_end = 5000;
    assert!(mutations::preview(&s, zero.clone())
        .unwrap_err()
        .contains("4096 nondefault"));
    s.patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .remove(&4096);
    assert_eq!(
        mutations::preview(&s, zero.clone())
            .unwrap()
            .summary()
            .reviewed_cells,
        4096
    );
    s.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    let p = s.patterns.get_mut(&PatternId(2)).unwrap();
    p.rows = 65536;
    p.tracks.insert(
        TrackId(1),
        (0..65536).map(|r| (r, BTreeMap::new())).collect(),
    );
    zero.scope.row_end = 65536;
    assert_eq!(
        mutations::preview(&s, zero.clone())
            .unwrap()
            .summary()
            .reviewed_cells,
        0
    );
    s.patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .insert(255, Cell::default());
    let error = mutations::preview(&s, zero).unwrap_err();
    assert!(error.contains("selected-source scan"), "{error}");
}

#[test]
fn retained_gain_proof_preserves_signed_zero_bits_not_only_float_equality() {
    let mut s = base();
    cell(&mut s, 0).velocity = Some(0.0);
    cell(&mut s, 1).velocity = Some(-0.0);
    *cell(&mut s, 2) = note(2);
    cell(&mut s, 2).velocity = None;
    let mut c = config();
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    assert!(mutations::preview(&s, c.clone())
        .unwrap_err()
        .contains("retained-ON"));
    c.removal_percent = 0;
    let p = mutations::preview(&s, c).unwrap();
    assert_eq!(
        serde_json::to_vec(p.candidate()).unwrap(),
        serde_json::to_vec(&s).unwrap()
    );
    let mut unchecked = s.clone();
    cell(&mut unchecked, 1).velocity = Some(0.0);
    assert_eq!(unchecked, s); // Rust float equality is deliberately weaker.
    assert_ne!(
        serde_json::to_vec(&unchecked).unwrap(),
        serde_json::to_vec(&s).unwrap()
    );
    assert!(p.commit(&unchecked).unwrap_err().contains("stale"));
}
#[test]
fn ordinary_candidate_renders_deterministically_without_touching_its_source() {
    use aqua_pattern_core::{sample_asset, song_render::Engine};
    let path = std::env::temp_dir().join(format!("thin-pcm-{}.wav", std::process::id()));
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..48000 {
        writer.write_sample(0.03 * (i as f32 * 0.2).sin()).unwrap();
    }
    writer.finalize().unwrap();
    let mut s = base();
    let asset = sample_asset::import(&path).unwrap().asset;
    s.samples.insert(SampleId(1), asset.clone());
    for (id, role) in [
        (1, PercussionRole::ClosedHat),
        (2, PercussionRole::Kick),
        (3, PercussionRole::ClosedHat),
    ] {
        s.set_region_role(RegionId(id), Some(label(role))).unwrap();
    }
    let p = mutations::preview(&s, config()).unwrap();
    let restored: Song =
        serde_json::from_slice(&serde_json::to_vec(p.candidate()).unwrap()).unwrap();
    assert_eq!(&restored, p.candidate());
    let render = |song: &Song, block: usize| {
        let mut e = Engine::new(song, None).unwrap();
        let end = e.manifest.musical_frames + e.manifest.tail_frames;
        let mut data = vec![0.; block * 2];
        let mut out = Vec::new();
        while e.position < end {
            let n = e.block(&mut data, end).unwrap();
            out.extend_from_slice(&data[..n * 2]);
        }
        out
    };
    let a = render(p.candidate(), 1);
    assert_eq!(a, render(&restored, 37));
    assert_eq!(a, render(&restored, 512));
    assert!(a.iter().any(|v| *v != 0.));
    sample_asset::verify_identity(&asset, &path).unwrap();
    std::fs::remove_file(path).unwrap();
}
