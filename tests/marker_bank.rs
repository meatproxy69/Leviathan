use aqua_pattern_core::{
    audition,
    marker_bank::{self, MarkerBankConfig, MarkerBankPreview},
    project, sample_asset,
    song::*,
    song_render::{self, Engine},
};
use std::{collections::BTreeMap, path::PathBuf};

fn pattern(rows: u32) -> Pattern {
    Pattern {
        generation: None,
        track_aliases: BTreeMap::new(),
        name: "Existing".into(),
        rows,
        tracks: BTreeMap::new(),
    }
}
fn fixture() -> Song {
    let mut song = example_song();
    song.tracks.retain(|id, _| *id == TrackId(1));
    song.track_order = vec![TrackId(1)];
    song.instruments.clear();
    song.patterns = BTreeMap::from([(PatternId(1), pattern(64))]);
    song.order.clear();
    song.samples.insert(
        SampleId(7),
        SampleAsset {
            path: PathBuf::from("/missing/never-opened-bank-source.wav"),
            sha256: "0".repeat(64),
            sample_rate: 44100,
            channels: 2,
            frames: 3200,
        },
    );
    song.marker_sets.insert(
        MarkerSetId(3),
        MarkerSet {
            name: "Accepted".into(),
            sample: SampleId(7),
            source_bpm: None,
            markers: (0..=32).map(|i| i * 100).collect(),
            locked_frames: vec![100, 300],
        },
    );
    song.validate().unwrap();
    song
}
fn config(indices: &[u16]) -> MarkerBankConfig {
    MarkerBankConfig {
        version: 1,
        marker_set: MarkerSetId(3),
        interval_indices: indices.to_vec(),
        name: "Reviewed bank".into(),
    }
}
fn bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn restore_added(plan: &MarkerBankPreview) -> Song {
    let mut song = plan.candidate().clone();
    song.instruments.remove(&plan.summary().instrument);
    song.drum_kits.remove(&plan.summary().kit);
    for pad in &plan.summary().pads {
        song.regions.remove(&pad.region);
    }
    song
}
fn assert_error(song: &Song, cfg: &MarkerBankConfig, needle: &str) {
    let before = bytes(song);
    let error = marker_bank::preview(song, cfg).unwrap_err();
    assert!(
        error.contains(needle),
        "{error:?} did not contain {needle:?}"
    );
    assert_eq!(bytes(song), before);
}
fn local_cells(song: &mut Song, count: usize) {
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    let p = song.patterns.get_mut(&PatternId(1)).unwrap();
    p.rows = 65536;
    let rows = p.tracks.entry(TrackId(1)).or_default();
    for i in 0..count {
        rows.entry((i / 256) as u32)
            .or_default()
            .insert((i % 256) as u16, Cell::default());
    }
}
fn unassigned() -> Instrument {
    Instrument {
        name: "Unassigned".into(),
        kind: InstrumentKind::Pitched,
        sample: None,
        region: None,
    }
}
fn region() -> SampleRegion {
    SampleRegion {
        name: "Old".into(),
        sample: SampleId(7),
        start_frame: 0,
        end_frame: 1,
        root_note: 12,
        amp_envelope: None,
        forward_loop: None,
    }
}

#[test]
fn exact_sparse_intervals_defaults_and_pure_whole_candidate() {
    let song = fixture();
    let cfg = config(&[0, 2, 31]);
    let plan = marker_bank::preview(&song, &cfg).unwrap();
    assert_eq!(bytes(&restore_added(&plan)), bytes(&song));
    assert_eq!(plan.base(), &song);
    assert_eq!(plan.config(), &cfg);
    assert_eq!(plan.source(), &song.samples[&SampleId(7)]);
    assert!(!plan.summary().source_identity_verified);
    assert_eq!(plan.summary().instrument, InstrumentId(1));
    assert_eq!(plan.summary().kit, DrumKitId(1));
    for (slot, &index) in cfg.interval_indices.iter().enumerate() {
        let row = &plan.summary().pads[slot];
        assert_eq!(
            (row.interval_index, row.start_frame, row.end_frame),
            (index, u64::from(index) * 100, (u64::from(index) + 1) * 100)
        );
        assert_eq!(
            (row.pad, row.region, row.slot, row.selector),
            (
                PadId(slot as u64 + 1),
                RegionId(slot as u64 + 1),
                slot as u8,
                36 + slot as u8
            )
        );
        assert_eq!(row.name, format!("Slice {:03}", index + 1));
        let pad = &plan.candidate().drum_kits[&DrumKitId(1)].pads[&row.pad];
        assert_eq!(
            (pad.root_note, pad.tune_semitones, pad.gain, pad.pan),
            (60, 0, 1.0, 0.0)
        );
        assert_eq!(
            (pad.mode, pad.new_note, pad.choke_group),
            (PadMode::OneShot, PadNewNote::Overlap, None)
        );
        assert!(pad.velocity_zones.is_empty());
        let region = &plan.candidate().regions[&row.region];
        assert_eq!(
            (
                region.sample,
                region.start_frame,
                region.end_frame,
                region.root_note
            ),
            (SampleId(7), row.start_frame, row.end_frame, 60)
        );
        assert_eq!((region.amp_envelope, region.forward_loop), (None, None));
    }
    assert_eq!(
        plan.candidate().instruments[&InstrumentId(1)].kind,
        InstrumentKind::DrumKit { kit: DrumKitId(1) }
    );
    assert!(plan.is_current(&song).unwrap());
    assert_eq!(bytes(&plan.commit(&song).unwrap()), bytes(plan.candidate()));
    let again = marker_bank::preview(&song, &cfg).unwrap();
    assert_eq!(
        (plan.id(), plan.base_sha256(), plan.candidate_sha256()),
        (again.id(), again.base_sha256(), again.candidate_sha256())
    );
}

#[test]
fn one_and_sixteen_intervals_include_single_frame_and_source_end() {
    let mut song = fixture();
    song.marker_sets.get_mut(&MarkerSetId(3)).unwrap().markers[1] = 1;
    song.marker_sets
        .get_mut(&MarkerSetId(3))
        .unwrap()
        .locked_frames
        .clear();
    let one = marker_bank::preview(&song, &config(&[0])).unwrap();
    assert_eq!(one.summary().pads[0].end_frame, 1);
    let cfg = config(&(16..32).collect::<Vec<_>>());
    let full = marker_bank::preview(&song, &cfg).unwrap();
    assert_eq!(full.summary().pads.len(), 16);
    let last = full.summary().pads.last().unwrap();
    assert_eq!(
        (last.pad, last.slot, last.selector, last.end_frame),
        (PadId(16), 15, 51, 3200)
    );
}

#[test]
fn malformed_selection_names_versions_and_unknown_sets_reject_without_normalizing() {
    let song = fixture();
    for indices in [vec![], vec![0, 0], vec![2, 1], (0..17).collect()] {
        assert_error(&song, &config(&indices), "1..16");
    }
    for indices in [vec![32], vec![u16::MAX]] {
        assert_error(&song, &config(&indices), "adjacent");
    }
    let mut cfg = config(&[0]);
    cfg.version = 0;
    assert_error(&song, &cfg, "version 1");
    cfg.version = 1;
    cfg.marker_set = MarkerSetId(99);
    assert_error(&song, &cfg, "Unknown accepted");
    cfg.marker_set = MarkerSetId(3);
    for name in [" \t".into(), "x".repeat(257), "é".repeat(129)] {
        cfg.name = name;
        assert_error(&song, &cfg, "256 UTF-8");
    }
    cfg.name = "é".repeat(128);
    assert!(marker_bank::preview(&song, &cfg).is_ok());
    let mut wire = serde_json::to_value(&cfg).unwrap();
    wire["root_note"] = 60.into();
    assert!(serde_json::from_value::<MarkerBankConfig>(wire).is_err());
}

#[test]
fn source_bpm_and_target_clocks_do_not_affect_region_geometry_or_require_renderability() {
    let mut song = fixture();
    // Existing ordinary notes may have no selected or assigned instrument.
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.insert(
        TrackId(1),
        BTreeMap::from([(
            0,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(60),
                    ..Default::default()
                },
            )]),
        )]),
    );
    for source_bpm in [None, Some(20.0), Some(123.4), Some(400.0)] {
        song.marker_sets
            .get_mut(&MarkerSetId(3))
            .unwrap()
            .source_bpm = source_bpm;
        for (bpm, lpb) in [(20.0, 1), (399.9, 256)] {
            song.bpm = bpm;
            song.lines_per_beat = lpb;
            let p = marker_bank::preview(&song, &config(&[2, 4])).unwrap();
            assert_eq!(bytes(&restore_added(&p)), bytes(&song));
            assert_eq!(
                (
                    p.summary().pads[0].start_frame,
                    p.summary().pads[1].end_frame
                ),
                (200, 500)
            );
            assert_eq!(
                p.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0].instrument,
                None
            );
        }
    }
}

#[test]
fn exact_base_guard_includes_signed_zero_and_every_unrelated_field() {
    let mut song = fixture();
    local_cells(&mut song, 1);
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
        .delay = Some(0.0);
    let p = marker_bank::preview(&song, &config(&[0])).unwrap();
    let mut changed = song.clone();
    changed
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
    assert_eq!(changed, song);
    assert!(!p.is_current(&changed).unwrap());
    assert!(p.commit(&changed).is_err());
    for change in 0..6 {
        let mut changed = song.clone();
        match change {
            0 => changed.title.push('!'),
            1 => changed
                .marker_sets
                .get_mut(&MarkerSetId(3))
                .unwrap()
                .locked_frames
                .clear(),
            2 => {
                changed
                    .marker_sets
                    .get_mut(&MarkerSetId(3))
                    .unwrap()
                    .markers[2] += 1
            }
            3 => changed.samples.get_mut(&SampleId(7)).unwrap().path = PathBuf::from("other.wav"),
            4 => changed.bpm = 121.0,
            _ => changed.samples.get_mut(&SampleId(7)).unwrap().sha256 = "1".repeat(64),
        }
        assert!(!p.is_current(&changed).unwrap());
        assert!(p.commit(&changed).is_err());
    }
    assert!(!p.is_current(p.candidate()).unwrap());
    assert!(p.commit(p.candidate()).is_err());
    let fresh = marker_bank::preview(p.candidate(), &config(&[0])).unwrap();
    assert_eq!(
        (
            fresh.summary().kit,
            fresh.summary().instrument,
            fresh.summary().pads[0].region
        ),
        (DrumKitId(2), InstrumentId(2), RegionId(2))
    );
}

#[test]
fn unresolved_ids_cannot_be_activated_by_fresh_allocations() {
    let mut song = fixture();
    local_cells(&mut song, 1);
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
        .instrument = Some(InstrumentId(1));
    assert_error(&song, &config(&[0]), "unknown instrument");
    let mut song = fixture();
    song.instruments.insert(
        InstrumentId(7),
        Instrument {
            name: "Dangling".into(),
            region: Some(RegionId(1)),
            ..unassigned()
        },
    );
    assert_error(&song, &config(&[0]), "region is unknown");
    let mut song = fixture();
    song.instruments.insert(
        InstrumentId(7),
        Instrument {
            name: "Dangling".into(),
            kind: InstrumentKind::DrumKit { kit: DrumKitId(1) },
            ..unassigned()
        },
    );
    assert_error(&song, &config(&[0]), "unknown kit");
    let mut song = fixture();
    song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Dangling".into(),
            instrument: InstrumentId(1),
            rows: 1,
            lines_per_beat: 4,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::new(),
        },
    );
    assert_error(&song, &config(&[0]), "invalid phrase");
}

#[test]
fn fresh_ids_use_checked_max_not_holes_and_all_namespaces_refuse_exhaustion() {
    let mut song = fixture();
    song.regions.insert(RegionId(8), region());
    song.instruments.insert(InstrumentId(42), unassigned());
    song.drum_kits.insert(
        DrumKitId(9),
        DrumKitInstrument {
            name: "Old".into(),
            pads: BTreeMap::new(),
        },
    );
    let p = marker_bank::preview(&song, &config(&[0, 1])).unwrap();
    assert_eq!(
        (
            p.summary().kit,
            p.summary().instrument,
            p.summary().pads[0].region,
            p.summary().pads[1].region
        ),
        (DrumKitId(10), InstrumentId(43), RegionId(9), RegionId(10))
    );
    for which in 0..3 {
        let mut song = fixture();
        match which {
            0 => {
                song.regions.insert(RegionId(u64::MAX), region());
            }
            1 => {
                song.drum_kits.insert(
                    DrumKitId(u64::MAX),
                    DrumKitInstrument {
                        name: "Old".into(),
                        pads: BTreeMap::new(),
                    },
                );
            }
            _ => {
                song.instruments
                    .insert(InstrumentId(u64::MAX), unassigned());
            }
        }
        assert_error(&song, &config(&[0]), "ID space exhausted");
    }
    let mut song = fixture();
    song.regions.insert(RegionId(u64::MAX - 16), region());
    let cfg = config(&(0..16).collect::<Vec<_>>());
    assert_eq!(
        marker_bank::preview(&song, &cfg).unwrap().summary().pads[15].region,
        RegionId(u64::MAX)
    );
    song.regions.clear();
    song.regions.insert(RegionId(u64::MAX - 15), region());
    assert_error(&song, &cfg, "ID space exhausted");
}

#[test]
fn region_and_kit_capacity_boundaries_do_not_overwrite() {
    let mut song = fixture();
    for id in 1..=4080 {
        song.regions.insert(RegionId(id), region());
    }
    let cfg = config(&(0..16).collect::<Vec<_>>());
    let p = marker_bank::preview(&song, &cfg).unwrap();
    assert_eq!(p.candidate().regions.len(), 4096);
    song.regions.insert(RegionId(4081), region());
    assert_error(&song, &cfg, "4096 regions");
    let mut song = fixture();
    for id in 1..=127 {
        song.drum_kits.insert(
            DrumKitId(id),
            DrumKitInstrument {
                name: "Old".into(),
                pads: BTreeMap::new(),
            },
        );
    }
    assert_eq!(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .candidate()
            .drum_kits
            .len(),
        128
    );
    song.drum_kits.insert(
        DrumKitId(128),
        DrumKitInstrument {
            name: "Full".into(),
            pads: BTreeMap::new(),
        },
    );
    assert_error(&song, &config(&[0]), "128 drum kits");
}

#[test]
fn exact_candidate_pretty_budget_and_base_first_failure() {
    let mut song = fixture();
    song.title.clear();
    let overhead = project::check_serialized_size(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .candidate(),
    )
    .unwrap();
    song.title = "x".repeat(8 * 1024 * 1024 - overhead);
    let p = marker_bank::preview(&song, &config(&[0])).unwrap();
    assert_eq!(
        project::check_serialized_size(p.candidate()).unwrap(),
        8 * 1024 * 1024
    );
    song.title.push('x');
    assert!(project::check_serialized_size(&song).is_ok());
    assert_error(&song, &config(&[0]), "8 MiB");
    song.title = "x".repeat(8 * 1024 * 1024);
    song.schema = 999;
    assert_error(&song, &config(&[0]), "8 MiB");
}

#[test]
fn ordered_empty_blocks_and_rows_have_exact_work_limits() {
    let mut song = fixture();
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        };
        65536
    ];
    let p = marker_bank::preview(&song, &config(&[0])).unwrap();
    assert_eq!(p.summary().work.ordered_track_blocks, 65536);
    song.order.push(song.order[0].clone());
    assert_error(&song, &config(&[0]), "ordered track blocks");
    let mut song = fixture();
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .insert(TrackId(1), (0..64).map(|r| (r, BTreeMap::new())).collect());
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        };
        1024
    ];
    assert_eq!(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .summary()
            .work
            .ordered_rows,
        65536
    );
    song.order.push(song.order[0].clone());
    assert_error(&song, &config(&[0]), "ordered row-container");
}

#[test]
fn ordered_default_cells_and_unused_resolved_definitions_are_bounded() {
    let mut song = fixture();
    local_cells(&mut song, 256);
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        };
        256
    ];
    assert_eq!(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .summary()
            .work
            .ordered_cells,
        65536
    );
    song.order.push(song.order[0].clone());
    assert_error(&song, &config(&[0]), "ordered cell visits");
    let mut song = fixture();
    local_cells(&mut song, 1024);
    for id in 2..=64 {
        let mut p = pattern(65536);
        p.track_aliases.insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
        song.patterns.insert(PatternId(id), p);
    }
    assert_eq!(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .summary()
            .work
            .definition_cells,
        65536
    );
    let mut p = pattern(1);
    p.tracks.insert(
        TrackId(1),
        BTreeMap::from([(0, BTreeMap::from([(0, Cell::default())]))]),
    );
    song.patterns.insert(PatternId(65), p);
    assert_error(&song, &config(&[0]), "resolved definition");
}

#[test]
fn unused_empty_definition_rows_and_alias_cycles_are_checked_before_model() {
    let mut song = fixture();
    song.patterns.get_mut(&PatternId(1)).unwrap().rows = 65536;
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.insert(
        TrackId(1),
        (0..65536).map(|r| (r, BTreeMap::new())).collect(),
    );
    assert_eq!(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .summary()
            .work
            .definition_rows,
        65536
    );
    let mut p = pattern(1);
    p.tracks
        .insert(TrackId(1), BTreeMap::from([(0, BTreeMap::new())]));
    song.patterns.insert(PatternId(2), p);
    assert_error(&song, &config(&[0]), "resolved definition");
    let mut song = fixture();
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
    assert_error(&song, &config(&[0]), "alias cycle");
}

#[test]
fn prospective_instrument_validation_exact_million_boundary_and_phrase_cap() {
    let mut song = fixture();
    local_cells(&mut song, 1024);
    for id in 1..=1023 {
        song.instruments.insert(InstrumentId(id), unassigned());
    }
    let p = marker_bank::preview(&song, &config(&[0])).unwrap();
    assert_eq!(p.summary().work.validation_units, 1_048_576);
    song.instruments.insert(InstrumentId(1024), unassigned());
    assert_error(&song, &config(&[0]), "1048576 units");
    // Exactly limit+1 is also reachable: (1 definition + 61680 repeated
    // default-cell visits) * 17 prospective instruments = 1048577.
    let mut song = fixture();
    local_cells(&mut song, 1);
    for id in 1..=16 {
        song.instruments.insert(InstrumentId(id), unassigned());
    }
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        };
        61_680
    ];
    assert_error(&song, &config(&[0]), "work 1048577 exceeds 1048576");
    let mut song = fixture();
    song.instruments.insert(InstrumentId(1), unassigned());
    let cells: BTreeMap<_, _> = (0..1024)
        .map(|r| (r, (0..16).map(|c| (c, Cell::default())).collect()))
        .collect();
    for id in 1..=2 {
        song.phrases.insert(
            PhraseId(id),
            Phrase {
                name: "Existing".into(),
                instrument: InstrumentId(1),
                rows: 1024,
                lines_per_beat: 4,
                note_columns: 16,
                root_note: 60,
                cells: cells.clone(),
            },
        );
    }
    assert_eq!(
        marker_bank::preview(&song, &config(&[0]))
            .unwrap()
            .summary()
            .work
            .phrase_cells,
        32768
    );
    let mut p = song.phrases[&PhraseId(1)].clone();
    p.cells = BTreeMap::from([(0, BTreeMap::from([(0, Cell::default())]))]);
    song.phrases.insert(PhraseId(3), p);
    assert_error(&song, &config(&[0]), "32768 phrase-definition");
}

#[test]
fn marker_edits_do_not_regenerate_and_new_references_protect_source() {
    let song = fixture();
    let p = marker_bank::preview(&song, &config(&[0, 2])).unwrap();
    let mut candidate = p.commit(&song).unwrap();
    let bank = bytes(&(
        &candidate.regions,
        &candidate.drum_kits,
        &candidate.instruments,
    ));
    assert!(candidate.remove_marker_set(MarkerSetId(3)).is_err());
    candidate
        .set_marker_lock(MarkerSetId(3), 100, false)
        .unwrap();
    candidate
        .set_marker_lock(MarkerSetId(3), 300, false)
        .unwrap();
    candidate.remove_marker_set(MarkerSetId(3)).unwrap();
    assert_eq!(
        bytes(&(
            &candidate.regions,
            &candidate.drum_kits,
            &candidate.instruments
        )),
        bank
    );
    assert!(candidate.remove_sample(SampleId(7)).is_err());
    assert!(candidate.remove_region(p.summary().pads[0].region).is_err());
    assert!(!bytes(&candidate)
        .windows(b"marker_bank".len())
        .any(|w| w == b"marker_bank"));
    let roundtrip: Song = serde_json::from_slice(&bytes(&candidate)).unwrap();
    assert_eq!(bytes(&roundtrip), bytes(&candidate));
}

fn render(song: &Song, block: usize) -> Vec<f32> {
    let mut engine = Engine::new(song, None).unwrap();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut result = Vec::new();
    let mut buf = vec![0.0; block * 2];
    while engine.position < end {
        let n = engine.block(&mut buf, end).unwrap();
        result.extend_from_slice(&buf[..n * 2]);
    }
    result
}
#[test]
fn unused_new_bank_preserves_real_manifest_pcm_and_selected_pad_reuses_exact_source() {
    let dir = std::env::temp_dir().join(format!("leviathan-marker-bank-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("authored.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for f in 0..3200 {
        writer
            .write_sample(0.125 + (f % 31) as f32 * 0.001)
            .unwrap();
        writer.write_sample(-0.0625f32).unwrap();
    }
    writer.finalize().unwrap();
    let mut song = fixture();
    song.samples
        .insert(SampleId(7), sample_asset::import(&path).unwrap().asset);
    song.regions.insert(
        RegionId(5),
        SampleRegion {
            end_frame: 3200,
            ..region()
        },
    );
    song.instruments.insert(
        InstrumentId(4),
        Instrument {
            region: Some(RegionId(5)),
            ..unassigned()
        },
    );
    song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1;
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.insert(
        TrackId(1),
        BTreeMap::from([(
            0,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(12),
                    instrument: Some(InstrumentId(4)),
                    velocity: Some(1.0),
                    pan: Some(0.0),
                    ..Default::default()
                },
            )]),
        )]),
    );
    song.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: Default::default(),
        section: None,
    }];
    let source_before = std::fs::read(&path).unwrap();
    let p = marker_bank::preview(&song, &config(&[0, 2, 31])).unwrap();
    assert_eq!(
        bytes(&song_render::evaluate(&song, None).unwrap()),
        bytes(&song_render::evaluate(p.candidate(), None).unwrap())
    );
    assert_eq!(render(&song, 1), render(p.candidate(), 257));
    for row in &p.summary().pads {
        let pad = audition::pad_preview(p.candidate(), p.summary().instrument, row.pad).unwrap();
        let direct =
            audition::region_preview(p.source(), &p.candidate().regions[&row.region]).unwrap();
        assert_eq!(render(pad.song(), 37), render(direct.song(), 512));
    }
    assert_eq!(std::fs::read(&path).unwrap(), source_before);
    // Pure preview still works when bytes change; the mandatory adapter hash
    // stage, not pure core, refuses external tampering before Create.
    std::fs::write(&path, b"changed").unwrap();
    assert!(marker_bank::preview(&song, &config(&[0])).is_ok());
    assert!(sample_asset::verify_identity(p.source(), &path).is_err());
    std::fs::write(&path, &source_before).unwrap();
    sample_asset::verify_identity(p.source(), &path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
