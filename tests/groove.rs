use aqua_pattern_core::{
    break_compiler::Subdivision,
    groove::{self, ApplyConfig, CaptureConfig},
    song::*,
    song_render::{self, Command},
    transforms::TransformScope,
};
use std::collections::BTreeMap;
fn note() -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.5),
        ..Default::default()
    }
}
fn put(song: &mut Song, row: u32, col: u16, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(col, cell);
}
fn fixture() -> Song {
    let mut s = example_song();
    s.bpm = 120.0;
    s.lines_per_beat = 8;
    s.patterns = BTreeMap::from([(
        PatternId(1),
        Pattern {
            name: "Target".into(),
            rows: 64,
            tracks: Default::default(),
            track_aliases: Default::default(),
            generation: None,
        },
    )]);
    s.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: Default::default(),
        section: None,
    }];
    s.samples.insert(
        SampleId(1),
        SampleAsset {
            path: "/missing-groove/no-io.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: 48000,
            channels: 1,
            frames: 96000,
        },
    );
    s.regions.insert(
        RegionId(1),
        SampleRegion {
            name: "Native".into(),
            sample: SampleId(1),
            start_frame: 0,
            end_frame: 96000,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        },
    );
    s.instruments.get_mut(&InstrumentId(1)).unwrap().region = Some(RegionId(1));
    s.marker_sets.insert(
        MarkerSetId(1),
        MarkerSet {
            name: "Explicit timing".into(),
            sample: SampleId(1),
            source_bpm: Some(120.0),
            markers: vec![
                0, 6600, 12000, 19800, 24000, 30000, 36000, 42000, 48000, 96000,
            ],
            locked_frames: Vec::new(),
        },
    );
    for row in [0, 2, 4, 6, 8, 10, 12, 14] {
        put(&mut s, row, 0, note());
    }
    s.validate().unwrap();
    s
}
fn capture_config() -> CaptureConfig {
    CaptureConfig {
        version: 1,
        name: "Swing accents are NOT captured".into(),
        marker_set: MarkerSetId(1),
        start_marker: 0,
        end_marker: 8,
        include_start_anchor: false,
        division: Subdivision::Sixteenth,
        cycle_steps: 8,
    }
}
fn registered() -> Song {
    let mut s = fixture();
    let t = groove::capture(&s, capture_config()).unwrap();
    s.register_groove(GrooveId(1), t).unwrap();
    s
}
fn apply(strength: u8) -> ApplyConfig {
    ApplyConfig {
        groove: GrooveId(1),
        scope: TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 16,
            columns: vec![0],
        },
        strength_percent: strength,
    }
}
// Clock-only event inspection uses a regular fixture path; evaluate intentionally
// does not decode/hash assets. Pure groove capture/apply still see the missing path.
fn manifest(s: &Song) -> song_render::Manifest {
    let mut inspected = s.clone();
    for source in inspected.samples.values_mut() {
        source.path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/slice-stereo-44100.flac");
    }
    song_render::evaluate(&inspected, None).unwrap()
}
fn ons(s: &Song) -> Vec<u64> {
    manifest(s)
        .events
        .iter()
        .filter_map(|e| matches!(e.command, Command::On { .. }).then_some(e.frame))
        .collect()
}
#[test]
fn accepted_window_endpoints_origin_and_approximate_inspection_are_explicit() {
    let s = fixture();
    let plan = groove::capture_preview(&s, GrooveId(1), capture_config()).unwrap();
    assert_eq!(plan.base(), &s);
    assert!(s.grooves.is_empty());
    assert_eq!(plan.template().anchors.len(), 7);
    assert_eq!(plan.template().anchors[0].source_frame_delta, 6600);
    assert_eq!(plan.template().anchors[0].step, 1);
    assert!((groove::inspect(plan.template()).unwrap()[0].offset_beats - 0.025).abs() < 1e-15);
    let mut c = capture_config();
    c.include_start_anchor = true;
    let t = groove::capture(&s, c).unwrap();
    assert_eq!(t.anchors.len(), 8);
    assert_eq!(t.anchors[0].source_frame_delta, 0);
    let mut c = capture_config();
    c.start_marker = 1;
    c.end_marker = 8;
    let t = groove::capture(&s, c).unwrap();
    assert_eq!(t.provenance.window_start_frame, 6600);
    assert_eq!(t.anchors[0].source_frame_delta, 5400);
    assert_eq!(s, fixture());
}
#[test]
fn capture_rejects_missing_tempo_empty_duplicate_or_excluded_end_without_fallback() {
    let mut s = fixture();
    s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().source_bpm = None;
    assert!(groove::capture(&s, capture_config())
        .unwrap_err()
        .contains("explicit source BPM"));
    let s = fixture();
    let mut c = capture_config();
    c.end_marker = 1;
    assert!(groove::capture(&s, c)
        .unwrap_err()
        .contains("no declared anchors"));
    let mut s = fixture();
    s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers = vec![0, 6000, 6001, 96000];
    let mut c = capture_config();
    c.end_marker = 3;
    assert!(groove::capture(&s, c).unwrap_err().contains("same nearest"));
    let mut c = capture_config();
    c.cycle_steps = 7;
    assert!(groove::capture(&fixture(), c)
        .unwrap_err()
        .contains("outside"));
    for bad in [0, 1025] {
        let mut c = capture_config();
        c.cycle_steps = bad;
        assert!(groove::capture(&fixture(), c).is_err());
    }
    let mut c = capture_config();
    c.version = 2;
    assert!(groove::capture(&fixture(), c).is_err());
}
#[test]
fn captured_templates_are_immutable_historical_and_baked_notes_have_no_live_reference() {
    let mut s = registered();
    let old = s.clone();
    let mut t = s.grooves[&GrooveId(1)].clone();
    t.name = "Changed identity".into();
    assert!(s.register_groove(GrooveId(1), t).is_err());
    assert_eq!(s, old);
    s.marker_sets.clear();
    s.samples.get_mut(&SampleId(1)).unwrap().path = "/another/missing.wav".into();
    s.validate().unwrap();
    let plan = groove::preview(&s, apply(100)).unwrap();
    let mut baked = plan.commit(&s).unwrap();
    baked.remove_groove(GrooveId(1)).unwrap();
    assert_eq!(ons(&baked), ons(plan.candidate()));
    assert!(groove::capture_preview(&old, GrooveId(1), capture_config()).is_err());
    let mut maximum = fixture();
    maximum
        .register_groove(GrooveId(u64::MAX), old.grooves[&GrooveId(1)].clone())
        .unwrap();
    assert!(maximum.next_groove_id().is_err());
}
#[test]
fn stored_template_validation_recomputes_slots_and_rejects_invalid_history() {
    let good = registered().grooves[&GrooveId(1)].clone();
    for mutation in 0..9 {
        let mut t = good.clone();
        match mutation {
            0 => t.anchors[0].step = 2,
            1 => t.anchors[0].source_frame_delta = 0,
            2 => t.source_bpm = f64::NAN,
            3 => t.provenance.window_end_frame = t.provenance.source_frames + 1,
            4 => t.provenance.source_sha256 = "bad".into(),
            5 => t.anchors.swap(0, 1),
            6 => t.provenance.end_marker = 7,
            7 => t.version = 2,
            _ => t.source_rate = 0,
        }
        assert!(t.validate().is_err(), "mutation{mutation}");
    }
    let mut s = fixture();
    for id in 1..=64 {
        s.register_groove(GrooveId(id), good.clone()).unwrap();
    }
    let old = s.clone();
    assert!(s.register_groove(GrooveId(65), good).is_err());
    assert_eq!(s, old);
}
#[test]
fn zero_strength_preserves_none_subnormal_fields_and_serialized_bits() {
    let mut s = registered();
    put(&mut s, 0, 1, Cell::default());
    let mut n = note();
    n.delay = Some(f64::from_bits(1));
    n.effects.offset = Some(3);
    n.pan = Some(-0.75);
    put(&mut s, 2, 0, n);
    let before = serde_json::to_vec(&s).unwrap();
    let p = groove::preview(&s, apply(0)).unwrap();
    assert!(p.changes().is_empty());
    assert_eq!(p.candidate(), &s);
    assert_eq!(serde_json::to_vec(p.candidate()).unwrap(), before);
    assert!(serde_json::to_value(fixture())
        .unwrap()
        .get("grooves")
        .is_none());
    let round: Song = serde_json::from_slice(&before).unwrap();
    assert_eq!(s, round);
}
#[test]
fn strengths_are_phase_blends_and_unmatched_slots_remain_exact() {
    let s = registered();
    for strength in [25, 50, 100] {
        let p = groove::preview(&s, apply(strength)).unwrap();
        let frames = ons(p.candidate());
        assert_eq!(frames[1], 6000 + 6 * u64::from(strength));
        assert_eq!(frames[3], 18000 + 18 * u64::from(strength));
        assert_eq!(p.summary().unmatched_notes, 1);
        assert_eq!(
            p.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0],
            s.patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0]
        );
    }
    let mut s = s;
    let mut n = note();
    n.delay = Some(0.4);
    put(&mut s, 2, 0, n);
    assert_eq!(
        ons(groove::preview(&s, apply(50)).unwrap().candidate())[1],
        6900
    );
    assert_eq!(
        ons(groove::preview(&s, apply(100)).unwrap().candidate())[1],
        6600
    );
    let mut n = note();
    n.delay = Some(f64::from_bits(1));
    put(&mut s, 0, 0, n.clone());
    assert_eq!(
        groove::preview(&s, apply(100))
            .unwrap()
            .candidate()
            .patterns[&PatternId(1)]
            .tracks[&TrackId(1)][&0][&0],
        n
    );
}
#[test]
fn cycles_repeat_and_partial_last_cycle_has_no_excluded_end_wrap() {
    let mut s = registered();
    put(&mut s, 18, 0, note());
    let mut c = apply(100);
    c.scope.row_end = 20;
    let p = groove::preview(&s, c.clone()).unwrap();
    assert_eq!(*ons(p.candidate()).last().unwrap(), 54600);
    let mut n = note();
    n.delay = Some(0.99);
    put(&mut s, 19, 0, n);
    assert!(groove::preview(&s, c)
        .unwrap_err()
        .contains("exclusive selection end"));
}
#[test]
fn two_notes_per_nearest_tick_reject_instead_of_colliding_or_merging() {
    let mut s = registered();
    put(&mut s, 1, 0, note());
    let before = s.clone();
    assert!(groove::preview(&s, apply(50))
        .unwrap_err()
        .contains("Multiple selected notes"));
    assert_eq!(s, before);
}
#[test]
fn aliases_are_read_only_but_shared_source_changes_cover_repeated_alias_uses() {
    let mut s = registered();
    let mut alias = s.patterns[&PatternId(1)].clone();
    alias.tracks.clear();
    alias.track_aliases.insert(
        TrackId(2),
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        },
    );
    s.patterns.insert(PatternId(2), alias);
    s.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: Default::default(),
        section: None,
    });
    s.order.push(s.order[0].clone());
    let p = groove::preview(&s, apply(100)).unwrap();
    assert_eq!(p.summary().affected_occurrences, vec![0, 1, 2]);
    assert_eq!(
        p.summary().affected_aliases,
        vec![TrackAlias {
            pattern: PatternId(2),
            track: TrackId(2)
        }]
    );
    let mut c = apply(100);
    c.scope.pattern = PatternId(2);
    c.scope.track = TrackId(2);
    assert!(groove::preview(&s, c).is_err());
    assert_eq!(
        p.candidate().patterns[&PatternId(2)],
        s.patterns[&PatternId(2)]
    );
}
#[test]
fn whole_cell_fx_shift_exactly_and_invalid_row_span_rejects() {
    let mut s = registered();
    let mut n = note();
    n.effects = StepEffects {
        retrigger: Some(Retrigger {
            repeats: 1,
            spacing: 0.2,
        }),
        release: Some(0.7),
        ..Default::default()
    };
    put(&mut s, 2, 0, n);
    let before = manifest(&s);
    let p = groove::preview(&s, apply(100)).unwrap();
    let after = manifest(p.candidate());
    let before: Vec<_> = before
        .events
        .iter()
        .filter(|e| e.position == Some((0, 2)))
        .map(|e| e.frame)
        .collect();
    let after: Vec<_> = after
        .events
        .iter()
        .filter(|e| e.position == Some((0, 2)))
        .map(|e| e.frame)
        .collect();
    assert_eq!(before.iter().map(|f| f + 600).collect::<Vec<_>>(), after);
    let mut n = note();
    n.effects.retrigger = Some(Retrigger {
        repeats: 1,
        spacing: 0.9,
    });
    put(&mut s, 2, 0, n);
    assert!(groove::preview(&s, apply(100)).is_err());
}
#[test]
fn changed_phrase_triggers_reject_and_unknown_scopes_fail_safely() {
    let mut s = registered();
    s.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Phrase".into(),
            instrument: InstrumentId(1),
            rows: 1,
            lines_per_beat: 8,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([(0, BTreeMap::from([(0, note())]))]),
        },
    );
    let mut n = note();
    n.phrase = Some(PhraseTrigger {
        phrase: PhraseId(1),
        cycles: 1,
    });
    put(&mut s, 2, 0, n);
    assert!(groove::preview(&s, apply(100))
        .unwrap_err()
        .contains("phrase triggers"));
    assert_eq!(groove::preview(&s, apply(0)).unwrap().candidate(), &s);
    let mut c = apply(100);
    c.scope.pattern = PatternId(999);
    assert!(groove::preview(&s, c).is_err());
    let mut c = apply(100);
    c.scope.columns = vec![0, 0];
    assert!(groove::preview(&s, c).is_err());
}
#[test]
fn bounded_work_and_pretty_size_reject_before_publishing_candidate() {
    let mut s = fixture();
    let t = groove::capture(&s, capture_config()).unwrap();
    s.title = "x".repeat(8 * 1024 * 1024);
    assert!(groove::capture_preview(&s, GrooveId(1), capture_config())
        .unwrap_err()
        .contains("8 MiB"));
    let mut s = fixture();
    s.register_groove(GrooveId(1), t).unwrap();
    s.title = "x".repeat(8 * 1024 * 1024);
    assert!(groove::preview(&s, apply(0)).unwrap_err().contains("8 MiB"));
    let mut s = fixture();
    for id in 5..=256 {
        s.tracks.insert(
            TrackId(id),
            Track {
                name: "Empty".into(),
                note_columns: 1,
            },
        );
        s.track_order.push(TrackId(id));
    }
    s.order = vec![s.order[0].clone(); 1024];
    assert!(groove::capture_preview(&s, GrooveId(1), capture_config())
        .unwrap_err()
        .contains("65536"));
}
#[test]
fn default_destinations_and_unselected_scalar_barriers_are_never_overwritten() {
    let mut s = registered();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    put(&mut s, 1, 0, note());
    put(&mut s, 2, 0, Cell::default());
    assert!(groove::preview(&s, apply(100))
        .unwrap_err()
        .contains("collision"));
    let mut s = fixture();
    s.lines_per_beat = 16;
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers[1] = 8999;
    let t = groove::capture(&s, capture_config()).unwrap();
    s.register_groove(GrooveId(1), t).unwrap();
    let mut n = note();
    n.delay = Some(0.1);
    put(&mut s, 2, 0, n);
    put(
        &mut s,
        3,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..Default::default()
        },
    );
    assert!(groove::preview(&s, apply(100))
        .unwrap_err()
        .contains("overtakes"));
}
#[test]
fn retrigger_mute_eligibility_is_checked_even_when_parent_on_stays_unmuted() {
    let mut s = fixture();
    s.lines_per_beat = 4;
    s.patterns.get_mut(&PatternId(1)).unwrap().rows = 16;
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers = vec![0, 90900, 96000];
    let mut c = capture_config();
    c.end_marker = 2;
    c.cycle_steps = 16;
    let t = groove::capture(&s, c).unwrap();
    s.register_groove(GrooveId(1), t).unwrap();
    let mut n = note();
    n.delay = Some(0.2);
    n.effects.retrigger = Some(Retrigger {
        repeats: 1,
        spacing: 0.7999999,
    });
    put(&mut s, 15, 0, n);
    let mut next = s.patterns[&PatternId(1)].clone();
    next.tracks.clear();
    s.patterns.insert(PatternId(2), next);
    s.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: std::collections::BTreeSet::from([TrackId(1)]),
        section: None,
    });
    let old = s.clone();
    let err = groove::preview(&s, apply(100)).unwrap_err();
    assert!(err.contains("mute eligibility"), "{err}");
    assert_eq!(s, old);
}
#[test]
fn drum_selector_and_all_other_fields_are_preserved_by_timing_only_apply() {
    let mut s = registered();
    s.drum_kits.insert(
        DrumKitId(1),
        DrumKitInstrument {
            name: "Kit".into(),
            pads: BTreeMap::from([(
                PadId(1),
                DrumPad {
                    slot: 0,
                    name: "Sound".into(),
                    note: 60,
                    region: Some(RegionId(1)),
                    velocity_zones: Vec::new(),
                    root_note: 72,
                    tune_semitones: -12,
                    gain: 0.5,
                    pan: 0.25,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: Some(2),
                },
            )]),
        },
    );
    s.assign_kit(InstrumentId(1), Some(DrumKitId(1))).unwrap();
    let mut n = note();
    n.effects.offset = Some(12);
    n.effects.direction = Some(Direction::Reverse);
    n.pan = Some(-0.5);
    put(&mut s, 2, 0, n.clone());
    let p = groove::preview(&s, apply(100)).unwrap();
    let after = &p.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)][&2][&0];
    let mut restored = after.clone();
    restored.delay = n.delay;
    assert_eq!(restored, n);
    assert_eq!(after.note, Note::Note(60));
    assert_eq!(p.candidate().drum_kits, s.drum_kits);
}
#[test]
fn checked_partial_strength_precision_rejects_without_weakening_zero_or_full_strength() {
    let mut s = fixture();
    let mut c = capture_config();
    c.include_start_anchor = true;
    let t = groove::capture(&s, c).unwrap();
    s.register_groove(GrooveId(1), t).unwrap();
    let mut n = note();
    n.delay = Some(f64::from_bits(1));
    put(&mut s, 0, 0, n.clone());
    assert_eq!(groove::preview(&s, apply(0)).unwrap().candidate(), &s);
    assert!(groove::preview(&s, apply(50))
        .unwrap_err()
        .contains("bounded integer range"));
    let p = groove::preview(&s, apply(100)).unwrap();
    assert_eq!(
        p.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)][&0][&0],
        n
    );
}
#[test]
fn rational_source_70_to_target_170_changes_timing_not_native_pcm_and_roundtrips() {
    use aqua_pattern_core::{sample_asset, song_render::Engine, RATE};
    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!(
        "leviathan-groove-pcm-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let source = dir.0.join("native.wav");
    let mut writer = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for n in 0..151200 {
        writer
            .write_sample((n as f32 * 0.037).sin() * 0.02)
            .unwrap();
    }
    writer.finalize().unwrap();
    let mut s = fixture();
    s.bpm = 170.0;
    s.samples
        .insert(SampleId(1), sample_asset::import(&source).unwrap().asset);
    s.regions.get_mut(&RegionId(1)).unwrap().end_frame = 151200;
    let markers = s.marker_sets.get_mut(&MarkerSetId(1)).unwrap();
    markers.source_bpm = Some(70.0);
    markers.markers = vec![0, 9891, 151200];
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    let mut n = note();
    n.velocity = Some(1.0);
    put(&mut s, 2, 0, n.clone());
    let mut c = capture_config();
    c.end_marker = 2;
    c.cycle_steps = 4;
    let t = groove::capture(&s, c).unwrap();
    s.register_groove(GrooveId(1), t).unwrap();
    let mut a = apply(100);
    a.scope.row_end = 8;
    let p = groove::preview(&s, a).unwrap();
    let numerator = 9891u128 * 70 * u128::from(RATE);
    let denominator = 44100u128 * 170;
    let expected = ((numerator + denominator / 2) / denominator) as usize;
    let m = song_render::evaluate(p.candidate(), None).unwrap();
    assert_eq!(
        m.events
            .iter()
            .find(|e| matches!(e.command, Command::On { .. }))
            .unwrap()
            .frame,
        expected as u64
    );
    fn render(mut e: Engine, block: usize) -> Vec<f32> {
        let end = e.manifest.musical_frames + e.manifest.tail_frames;
        let mut pcm = Vec::new();
        let mut data = vec![0.0; block * 2];
        while e.position < end {
            let count = e.block(&mut data, end).unwrap();
            pcm.extend_from_slice(&data[..count * 2]);
        }
        pcm
    }
    let actual = render(Engine::new(p.candidate(), None).unwrap(), 37);
    assert!(actual[..expected * 2].iter().all(|x| *x == 0.0));
    let mut native = s.clone();
    native.bpm = 70.0;
    native
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    put(&mut native, 0, 0, n);
    let reference = render(Engine::new(&native, None).unwrap(), 512);
    assert_eq!(
        &actual[expected * 2..(expected + 3000) * 2],
        &reference[..6000]
    );
    assert_eq!(actual, render(Engine::new(p.candidate(), None).unwrap(), 1));
    assert_eq!(
        actual,
        render(Engine::new(p.candidate(), None).unwrap(), 512)
    );
    let reopened: Song =
        serde_json::from_str(&serde_json::to_string(p.candidate()).unwrap()).unwrap();
    assert_eq!(&reopened, p.candidate());
    let out = dir.0.join("grooved.wav");
    song_render::export(&reopened, None, &out).unwrap();
    let decoded: Vec<_> = hound::WavReader::open(&out)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(decoded, actual);
}
#[test]
fn recompiled_cross_track_interleaving_intentionally_changes_rr_and_choke_order() {
    use aqua_pattern_core::song_render::Release;
    let mut s = registered();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    let mut region = s.regions[&RegionId(1)].clone();
    region.start_frame = 32;
    s.regions.insert(RegionId(2), region);
    s.drum_kits.insert(
        DrumKitId(1),
        DrumKitInstrument {
            name: "RR".into(),
            pads: BTreeMap::from([(
                PadId(1),
                DrumPad {
                    slot: 0,
                    name: "Two sources".into(),
                    note: 60,
                    region: None,
                    velocity_zones: vec![VelocityZone {
                        min_velocity: 0,
                        max_velocity: 127,
                        alternatives: vec![
                            PadLayer {
                                id: PadLayerId(1),
                                region: RegionId(1),
                            },
                            PadLayer {
                                id: PadLayerId(2),
                                region: RegionId(2),
                            },
                        ],
                    }],
                    root_note: 60,
                    tune_semitones: 0,
                    gain: 1.0,
                    pan: 0.0,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: Some(1),
                },
            )]),
        },
    );
    s.assign_kit(InstrumentId(1), Some(DrumKitId(1))).unwrap();
    put(&mut s, 2, 0, note());
    let mut other = note();
    other.delay = Some(0.1);
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.insert(
        TrackId(2),
        BTreeMap::from([(2, BTreeMap::from([(0, other)]))]),
    );
    let before = manifest(&s);
    let p = groove::preview(&s, apply(100)).unwrap();
    let after = manifest(p.candidate());
    let choices = |m: &song_render::Manifest| {
        m.events
            .iter()
            .filter_map(|e| {
                if let Command::On { voice, .. } = e.command {
                    Some((
                        e.track,
                        m.pad_voices[&voice].layer_selection.as_ref().unwrap().layer,
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        choices(&before),
        vec![(TrackId(1), PadLayerId(1)), (TrackId(2), PadLayerId(2))]
    );
    assert_eq!(
        choices(&after),
        vec![(TrackId(2), PadLayerId(1)), (TrackId(1), PadLayerId(2))]
    );
    assert!(before.events.iter().any(|e| e.track == TrackId(1)
        && matches!(
            e.command,
            Command::Off {
                reason: Release::Choke,
                ..
            }
        )));
    assert!(after.events.iter().any(|e| e.track == TrackId(2)
        && matches!(
            e.command,
            Command::Off {
                reason: Release::Choke,
                ..
            }
        )));
    assert_eq!(
        p.candidate().patterns[&PatternId(1)].tracks[&TrackId(2)],
        s.patterns[&PatternId(1)].tracks[&TrackId(2)]
    );
    assert!(p
        .summary()
        .warnings
        .iter()
        .any(|w| w.contains("round-robin")));
}
#[test]
fn moving_replacement_note_can_expose_existing_phrase_children_without_new_stored_notes() {
    let mut s = registered();
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    s.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Pending children".into(),
            instrument: InstrumentId(1),
            rows: 8,
            lines_per_beat: 40,
            note_columns: 1,
            root_note: 60,
            cells: (0..8)
                .map(|row| (row, BTreeMap::from([(0, note())])))
                .collect(),
        },
    );
    put(
        &mut s,
        0,
        0,
        Cell {
            note: Note::Note(60),
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 8,
            }),
            ..Default::default()
        },
    );
    put(&mut s, 2, 0, note());
    let old = manifest(&s);
    let plan = groove::preview(&s, apply(100)).unwrap();
    let after = manifest(plan.candidate());
    let children = |m: &song_render::Manifest| {
        m.events
            .iter()
            .filter(|e| e.phrase.is_some() && matches!(e.command, Command::On { .. }))
            .count()
    };
    assert_eq!(children(&old), 10);
    assert_eq!(children(&after), 11);
    assert!(after
        .events
        .iter()
        .any(|e| e.frame == 6000 && e.phrase.is_some() && matches!(e.command, Command::On { .. })));
    let old_rows = &s.patterns[&PatternId(1)].tracks[&TrackId(1)];
    let new_rows = &plan.candidate().patterns[&PatternId(1)].tracks[&TrackId(1)];
    assert_eq!(old_rows.len(), new_rows.len());
    assert_eq!(old_rows[&0], new_rows[&0]);
    assert_eq!(s.phrases, plan.candidate().phrases);
    assert!(plan
        .summary()
        .warnings
        .iter()
        .any(|w| w.contains("not full renderability")));
}
#[test]
fn rounded_frame_encoder_must_not_cross_a_captured_half_slot_boundary() {
    let mut s = fixture();
    s.bpm = 123.0;
    s.lines_per_beat = 4;
    s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    put(&mut s, 1, 0, note());
    s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers = vec![0, 3000, 48000, 96000];
    let mut config = capture_config();
    config.end_marker = 2;
    config.include_start_anchor = true;
    config.cycle_steps = 16;
    let t = groove::capture(&s, config).unwrap();
    assert_eq!(t.anchors[1].step, 1); // Exact half-tie belongs to slot1, not slot0.
    s.register_groove(GrooveId(1), t).unwrap();
    let before = s.clone();
    let error = groove::preview(&s, apply(100)).unwrap_err();
    assert!(error.contains("nearest-slot boundary"), "{error}");
    assert_eq!(s, before);
    assert_eq!(groove::preview(&s, apply(0)).unwrap().candidate(), &s);
    // A clock with an exact representable half-tie preserves the assigned slot;
    // a second full-strength application is then an exact stored-cell no-op.
    s.bpm = 120.0;
    let first = groove::preview(&s, apply(100)).unwrap();
    assert_eq!(ons(first.candidate()), vec![3000]);
    assert_eq!(first.changes()[0].to_row, 0);
    assert_eq!(first.changes()[0].after.delay, Some(0.5));
    let second = groove::preview(first.candidate(), apply(100)).unwrap();
    assert_eq!(second.candidate(), first.candidate());
    assert!(second.changes().is_empty());
}
#[test]
fn accepted_full_strength_reapplication_is_exact_across_half_neighbors_and_prefixes() {
    let mut accepted = 0;
    let mut rejected = 0;
    for division in [
        Subdivision::Sixteenth,
        Subdivision::ThirtySecond,
        Subdivision::SixtyFourth,
        Subdivision::OneTwentyEighth,
    ] {
        let half = 48000 * 60 / 120 * 2 / u64::from(division.denominator());
        for delta in [half - 1, half, half + 1] {
            for bpm in [120.0, 123.0, 137.25] {
                for lpb in [4, 7, 8] {
                    for prefix in [0, 1, 17] {
                        let mut s = fixture();
                        s.bpm = bpm;
                        s.lines_per_beat = lpb;
                        s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
                        s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers =
                            vec![0, delta, 48000, 96000];
                        let mut config = capture_config();
                        config.end_marker = 2;
                        config.division = division;
                        config.cycle_steps = 16;
                        let t = groove::capture(&s, config).unwrap();
                        let pos = f64::from(t.anchors[0].step) * 4.0 * f64::from(lpb)
                            / f64::from(division.denominator());
                        let row = pos.floor() as u32;
                        put(
                            &mut s,
                            row,
                            0,
                            Cell {
                                delay: Some(pos - f64::from(row)),
                                ..note()
                            },
                        );
                        s.register_groove(GrooveId(1), t).unwrap();
                        if prefix != 0 {
                            s.patterns.insert(
                                PatternId(2),
                                Pattern {
                                    name: "Prefix".into(),
                                    rows: prefix,
                                    tracks: Default::default(),
                                    track_aliases: Default::default(),
                                    generation: None,
                                },
                            );
                            s.order.insert(
                                0,
                                Occurrence {
                                    pattern: PatternId(2),
                                    muted_tracks: Default::default(),
                                    section: None,
                                },
                            );
                        }
                        let before = s.clone();
                        match groove::preview(&s, apply(100)) {
                            Ok(first) => {
                                accepted += 1;
                                let second =
                                    groove::preview(first.candidate(), apply(100)).unwrap();
                                assert_eq!(
                                    second.candidate(),
                                    first.candidate(),
                                    "{division:?}/{delta}/{bpm}/{lpb}/{prefix}"
                                );
                                assert!(second.changes().is_empty());
                            }
                            Err(error) => {
                                rejected += 1;
                                assert!(
                                    error.contains("represent")
                                        || error.contains("boundary")
                                        || error.contains("range"),
                                    "{error}"
                                );
                            }
                        }
                        assert_eq!(s, before);
                    }
                }
            }
        }
    }
    assert!(accepted > 100);
    assert!(rejected > 0);
}

#[test]
fn stored_delay_half_tie_ulp_neighbors_never_silently_switch_template_slots() {
    for bpm in [120.0, 123.0] {
        for delay in [
            f64::from_bits(0.5f64.to_bits() - 1),
            0.5,
            f64::from_bits(0.5f64.to_bits() + 1),
        ] {
            let mut s = fixture();
            s.bpm = bpm;
            s.lines_per_beat = 4;
            s.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
            put(
                &mut s,
                0,
                0,
                Cell {
                    delay: Some(delay),
                    ..note()
                },
            );
            s.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers = vec![0, 3000, 48000, 96000];
            let mut config = capture_config();
            config.end_marker = 2;
            config.include_start_anchor = true;
            config.cycle_steps = 16;
            let t = groove::capture(&s, config).unwrap();
            s.register_groove(GrooveId(1), t).unwrap();
            let plan = groove::preview(&s, apply(100));
            if let Ok(first) = plan {
                let second = groove::preview(first.candidate(), apply(100)).unwrap();
                assert_eq!(second.candidate(), first.candidate());
                assert!(second.changes().is_empty());
                assert_eq!(
                    ons(first.candidate()),
                    vec![if delay < 0.5 {
                        0
                    } else if bpm == 120.0 {
                        3000
                    } else {
                        2927
                    }]
                );
            } else {
                let error = plan.unwrap_err();
                // The existing whole-cell encoder can also conservatively reject
                // the next-down value: an integer shift to frame0 has a tiny
                // negative continuous ideal phase. Never weaken its scope guard.
                assert!(
                    error.contains("nearest-slot boundary")
                        || error.contains("selected [start,end)"),
                    "{bpm}/{delay:?}: {error}"
                );
            }
        }
    }
}
