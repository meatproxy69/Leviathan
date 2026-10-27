use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    song: Song,
}
fn note(pitch: u8, velocity: f32) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(1)),
        velocity: Some(velocity),
        ..Default::default()
    }
}
fn zone(region: u64, lo: u8, hi: u8, vlo: u8, vhi: u8, root: u8) -> KeyZone {
    KeyZone {
        name: format!("Zone{region}"),
        note_min: lo,
        note_max: hi,
        velocity_min: vlo,
        velocity_max: vhi,
        region: RegionId(region),
        root_note: root,
    }
}
fn put(s: &mut Song, row: u32, col: u16, cell: Cell) {
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(col, cell);
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-keyzones-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut song = example_song();
        song.bpm = 120.;
        song.lines_per_beat = 100;
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 16;
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                name: "Keys".into(),
                rows: 32,
                tracks: Default::default(),
                track_aliases: Default::default(),
                generation: None,
            },
        )]);
        song.order = vec![Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        }];
        for (id, rate, channels) in [(1, 44100, 2), (2, 24000, 1), (3, 96000, 2)] {
            let path = dir.join(format!("source{id}.wav"));
            let mut w = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels,
                    sample_rate: rate,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            for frame in 0..4096 {
                for ch in 0..channels {
                    w.write_sample(
                        (id as f32 * 0.02 + (frame % 31) as f32 * 0.0001)
                            * if ch == 0 { 1. } else { -0.4 },
                    )
                    .unwrap();
                }
            }
            w.finalize().unwrap();
            song.register_sample(SampleId(id), sample_asset::import(&path).unwrap().asset)
                .unwrap();
            song.upsert_region(
                RegionId(id),
                SampleRegion {
                    name: format!("Region{id}"),
                    sample: SampleId(id),
                    start_frame: 7,
                    end_frame: 4093,
                    root_note: 100,
                    amp_envelope: None,
                    forward_loop: None,
                },
            )
            .unwrap();
        }
        song.upsert_multisample(
            MultisampleId(1),
            MultisampleInstrument {
                name: "Melodic".into(),
                zones: BTreeMap::from([
                    (KeyZoneId(1), zone(1, 0, 63, 0, 127, 60)),
                    (KeyZoneId(2), zone(2, 64, 127, 0, 63, 72)),
                    (KeyZoneId(3), zone(3, 64, 127, 64, 127, 72)),
                ]),
            },
        )
        .unwrap();
        song.assign_multisample(InstrumentId(1), Some(MultisampleId(1)))
            .unwrap();
        put(&mut song, 0, 0, note(60, 1.));
        Self { dir, song }
    }
    fn manifest(&self) -> song_render::Manifest {
        song_render::evaluate(&self.song, None).unwrap()
    }
    fn pcm(&self, block: usize) -> Vec<f32> {
        render(&self.song, block)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn render(song: &Song, block: usize) -> Vec<f32> {
    let mut e = Engine::new(song, None).unwrap();
    let end = e.manifest.musical_frames + e.manifest.tail_frames;
    let mut out = Vec::new();
    let mut b = vec![0.; block * 2];
    while e.position < end {
        let n = e.block(&mut b, end).unwrap();
        out.extend_from_slice(&b[..2 * n]);
    }
    out
}
fn ordinary_reference(s: &Song) -> Song {
    let mut r = s.clone();
    let map = &s.multisamples[&MultisampleId(1)];
    for (&id, z) in &map.zones {
        let rid = RegionId(100 + id.0);
        let mut region = s.regions[&z.region].clone();
        region.root_note = z.root_note;
        r.regions.insert(rid, region);
        r.instruments.insert(
            InstrumentId(100 + id.0),
            Instrument {
                name: z.name.clone(),
                kind: InstrumentKind::Pitched,
                sample: None,
                region: Some(rid),
            },
        );
    }
    for rows in r.patterns.values_mut().flat_map(|p| p.tracks.values_mut()) {
        for cell in rows.values_mut().flat_map(|c| c.values_mut()) {
            if let Note::Note(pitch) = cell.note {
                let (id, _, _) = s
                    .select_keyzone(MultisampleId(1), pitch, cell.velocity.unwrap_or(1.))
                    .unwrap();
                cell.instrument = Some(InstrumentId(100 + id.0));
            }
        }
    }
    r
}
#[test]
fn inclusive_rectangles_overlap_only_in_both_dimensions_and_edits_are_atomic() {
    let mut f = Fixture::new();
    let before = f.song.clone();
    let mut zones = f.song.multisamples[&MultisampleId(1)].zones.clone();
    zones.get_mut(&KeyZoneId(2)).unwrap().note_min = 63;
    assert!(f
        .song
        .set_keyzones(MultisampleId(1), zones)
        .unwrap_err()
        .contains("overlap"));
    assert_eq!(f.song, before);
    let mut zones = before.multisamples[&MultisampleId(1)].zones.clone();
    zones.get_mut(&KeyZoneId(2)).unwrap().velocity_max = 64;
    assert!(f.song.set_keyzones(MultisampleId(1), zones).is_err());
    assert_eq!(f.song, before);
    assert_eq!(
        f.song.select_keyzone(MultisampleId(1), 63, 1.).unwrap().0,
        KeyZoneId(1)
    );
    assert_eq!(
        f.song.select_keyzone(MultisampleId(1), 64, 0.).unwrap().0,
        KeyZoneId(2)
    );
    assert_eq!(
        f.song.select_keyzone(MultisampleId(1), 127, 1.).unwrap().0,
        KeyZoneId(3)
    );
    let mut zones = before.multisamples[&MultisampleId(1)].zones.clone();
    zones.get_mut(&KeyZoneId(1)).unwrap().note_max = 62;
    zones.get_mut(&KeyZoneId(2)).unwrap().note_min = 63;
    zones.get_mut(&KeyZoneId(3)).unwrap().note_min = 63;
    f.song.set_keyzones(MultisampleId(1), zones).unwrap();
}
#[test]
fn all_velocity_half_neighbors_use_exact_pre_source_f32_and_order_has_no_priority() {
    let f = Fixture::new();
    for bin in 0..127 {
        let half = (bin as f64 + 0.5) / 127.;
        let x = half as f32;
        for v in [
            f32::from_bits(x.to_bits() - 1),
            x,
            f32::from_bits(x.to_bits() + 1),
        ] {
            let (_, z, b) = f.song.select_keyzone(MultisampleId(1), 72, v).unwrap();
            let expected = (f64::from(v) * 127.).round() as u8;
            assert_eq!(b, expected);
            assert_eq!(z.region, RegionId(if expected <= 63 { 2 } else { 3 }));
        }
    }
    for v in [0., f32::from_bits(1), 1.] {
        let (_, _, b) = f.song.select_keyzone(MultisampleId(1), 72, v).unwrap();
        assert_eq!(b, (f64::from(v) * 127.).round() as u8);
    }
    let mut permuted = f.song.clone();
    permuted
        .multisamples
        .get_mut(&MultisampleId(1))
        .unwrap()
        .zones = permuted.multisamples[&MultisampleId(1)]
        .zones
        .values()
        .cloned()
        .zip([KeyZoneId(99), KeyZoneId(4), KeyZoneId(1)])
        .map(|(z, id)| (id, z))
        .collect();
    for n in 0..=127 {
        for v in [0., 0.5, 1.] {
            assert_eq!(
                f.song.select_keyzone(MultisampleId(1), n, v).unwrap().1,
                permuted.select_keyzone(MultisampleId(1), n, v).unwrap().1
            );
        }
    }
}
#[test]
fn native_root_override_mixed_rates_polyphony_and_tuning_match_ordinary_regions() {
    let mut f = Fixture::new();
    for (row, col, n, v) in [
        (0, 0, 48, 1.),
        (0, 1, 60, 0.5),
        (1, 2, 72, 0.25),
        (2, 3, 84, 1.),
    ] {
        put(&mut f.song, row, col, note(n, v));
    }
    let m = f.manifest();
    assert_eq!(m.schema, 10);
    assert_eq!(m.multisample_voices.len(), 4);
    assert!(m
        .multisample_voices
        .values()
        .all(|v| v.region.root_note != 100));
    let expected = render(&ordinary_reference(&f.song), 37);
    assert_eq!(f.pcm(37), expected);
    assert_eq!(f.pcm(1), expected);
    assert_eq!(f.pcm(512), expected);
}
#[test]
fn maps_can_be_empty_or_holey_but_actual_zero_gain_gap_rejects_before_decode_or_outputs() {
    let mut f = Fixture::new();
    f.song
        .set_keyzones(MultisampleId(1), BTreeMap::new())
        .unwrap();
    f.song.validate().unwrap();
    put(&mut f.song, 0, 0, note(60, 0.));
    std::fs::write(f.song.samples[&SampleId(1)].path.clone(), b"not audio").unwrap();
    let error = Engine::new(&f.song, None).err().unwrap();
    assert!(error.contains("no keyzone"), "{error}");
    let out = f.dir.join("reject.wav");
    assert!(song_render::export(&f.song, None, &out).is_err());
    assert!(!out.exists());
    assert!(!out.with_extension("events.json").exists());
    f.song.order[0].muted_tracks.insert(TrackId(1));
    let pcm = f.pcm(37);
    assert!(pcm.iter().all(|x| *x == 0.));
    assert!(f.manifest().multisample_voices.is_empty());
}
#[test]
fn unused_regions_maps_and_sources_are_validated_without_decoding_or_changing_old_output() {
    let mut f = Fixture::new();
    let mut ordinary = ordinary_reference(&f.song);
    ordinary.assign_multisample(InstrumentId(1), None).unwrap();
    ordinary.multisamples.clear();
    let json = serde_json::to_string(&ordinary).unwrap();
    assert!(!json.contains("multisamples"));
    let old = song_render::evaluate(&ordinary, None).unwrap();
    let pcm = render(&ordinary, 37);
    ordinary
        .upsert_multisample(
            MultisampleId(99),
            f.song.multisamples[&MultisampleId(1)].clone(),
        )
        .unwrap();
    assert_eq!(song_render::evaluate(&ordinary, None).unwrap(), old);
    assert_eq!(render(&ordinary, 37), pcm);
    assert!(ordinary.remove_region(RegionId(3)).is_err());
    assert!(f.song.remove_multisample(MultisampleId(1)).is_err());
    let before = f.song.clone();
    let mut bad = before.multisamples[&MultisampleId(1)].clone();
    bad.zones.get_mut(&KeyZoneId(3)).unwrap().region = RegionId(999);
    assert!(f.song.upsert_multisample(MultisampleId(1), bad).is_err());
    assert_eq!(f.song, before);
}
#[test]
fn gain_and_absolute_glide_never_remap_an_active_source_after_future_instrument_selection() {
    let mut f = Fixture::new();
    f.song.regions.get_mut(&RegionId(1)).unwrap().forward_loop = Some(ForwardLoop {
        start_frame: 8,
        end_frame: 40,
    });
    put(&mut f.song, 0, 0, note(60, 0.2));
    put(
        &mut f.song,
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            velocity: Some(1.),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 84,
                    semitones_per_second: 120.,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let m = f.manifest();
    assert_eq!(m.multisample_voices.len(), 1);
    let voice = m.multisample_voices.values().next().unwrap();
    assert_eq!(voice.zone, KeyZoneId(1));
    assert_eq!(voice.input_velocity, 0.2);
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::Glide { target: 84, .. })
            && e.instrument == InstrumentId(1)));
    let mut reference = ordinary_reference(&f.song);
    assert_eq!(f.pcm(37), render(&reference, 37));
    // Root-reference MIDI pitch in glide remains absolute, never another zone lookup.
    reference
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&2)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .effects
        .direction = Some(Direction::Reverse);
    assert!(Engine::new(&reference, None).is_err());
}
#[test]
fn selected_loop_controls_reject_but_unused_loop_zones_and_reverse_one_shots_work() {
    let mut f = Fixture::new();
    f.song.regions.get_mut(&RegionId(3)).unwrap().forward_loop = Some(ForwardLoop {
        start_frame: 8,
        end_frame: 40,
    });
    let mut cell = note(60, 1.);
    cell.effects.direction = Some(Direction::Reverse);
    cell.effects.offset = Some(100);
    put(&mut f.song, 0, 0, cell.clone());
    assert_eq!(f.pcm(37), render(&ordinary_reference(&f.song), 37));
    cell.note = Note::Note(72);
    put(&mut f.song, 0, 0, cell);
    std::fs::write(&f.song.samples[&SampleId(3)].path, b"bad").unwrap();
    assert!(Engine::new(&f.song, None)
        .err()
        .unwrap()
        .contains("forward loop does not support"));
}
#[test]
fn phrase_transpose_velocity_snapshot_and_retrigger_resolve_actual_melodic_notes() {
    let mut f = Fixture::new();
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Melodic".into(),
            instrument: InstrumentId(1),
            rows: 2,
            lines_per_beat: 100,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([
                (
                    0,
                    BTreeMap::from([(
                        0,
                        Cell {
                            effects: StepEffects {
                                retrigger: Some(Retrigger {
                                    repeats: 1,
                                    spacing: 0.5,
                                }),
                                ..Default::default()
                            },
                            ..note(60, 0.5)
                        },
                    )]),
                ),
                (1, BTreeMap::from([(0, note(72, 1.))])),
            ]),
        },
    );
    put(
        &mut f.song,
        0,
        0,
        Cell {
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 2,
            }),
            ..note(72, 0.5)
        },
    );
    let m = f.manifest();
    let selected: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On { voice, .. } = e.command {
                let z = &m.multisample_voices[&voice];
                Some((z.note, z.input_velocity, z.zone))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        selected,
        vec![
            (72, 0.25, KeyZoneId(2)),
            (72, 0.25, KeyZoneId(2)),
            (84, 0.5, KeyZoneId(3)),
            (72, 0.25, KeyZoneId(2)),
            (72, 0.25, KeyZoneId(2)),
            (84, 0.5, KeyZoneId(3))
        ]
    );
    assert_eq!(f.pcm(1), f.pcm(512));
}
#[test]
fn source_choice_is_precompiled_through_seek_replay_and_serialization() {
    let mut f = Fixture::new();
    put(&mut f.song, 1, 1, note(72, 0.2));
    put(&mut f.song, 2, 2, note(72, 1.));
    let m = f.manifest();
    let reopened: Song = serde_json::from_slice(&serde_json::to_vec(&f.song).unwrap()).unwrap();
    assert_eq!(reopened, f.song);
    assert_eq!(render(&reopened, 37), f.pcm(37));
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(240).unwrap();
    assert_eq!(engine.manifest, m);
    let mut data = [0.; 74];
    engine.block(&mut data, 500).unwrap();
    engine.cold_seek(0).unwrap();
    assert_eq!(engine.manifest, m);
    let out = f.dir.join("one.wav");
    let out2 = f.dir.join("two.wav");
    song_render::export(&f.song, None, &out).unwrap();
    song_render::export(&reopened, None, &out2).unwrap();
    assert_eq!(std::fs::read(out).unwrap(), std::fs::read(out2).unwrap());
}
#[test]
fn structural_limits_root_bounds_and_fresh_id_overflow_reject_atomically() {
    let mut f = Fixture::new();
    let before = f.song.clone();
    for zone in [
        KeyZone {
            root_note: 128,
            ..zone(1, 0, 127, 0, 127, 60)
        },
        KeyZone {
            velocity_min: 128,
            velocity_max: 128,
            ..zone(1, 0, 127, 0, 127, 60)
        },
    ] {
        assert!(f
            .song
            .set_keyzones(MultisampleId(1), BTreeMap::from([(KeyZoneId(1), zone)]))
            .is_err());
        assert_eq!(f.song, before);
    }
    let zones = (1..=129)
        .map(|id| (KeyZoneId(id), zone(1, 0, 127, 0, 127, 60)))
        .collect();
    assert!(f
        .song
        .set_keyzones(MultisampleId(1), zones)
        .unwrap_err()
        .contains("128"));
    f.song.instruments.insert(
        InstrumentId(u64::MAX),
        Instrument {
            name: "Last".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: None,
        },
    );
    let before = f.song.clone();
    assert!(f
        .song
        .create_multisample_instrument("Fresh".into())
        .is_err());
    assert_eq!(f.song, before);
}
#[test]
fn transpose_selects_another_zone_but_glide_target_does_not_and_generator_uses_same_mapping() {
    use aqua_pattern_core::{
        generation::{self, GenerationConfig, GenerationNote, GenerationPreset},
        transforms::{self, TransformConfig, TransformFields, TransformScope},
    };
    let f = Fixture::new();
    let config = TransformConfig {
        scope: TransformScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 1,
            columns: vec![0],
        },
        fields: TransformFields {
            notes: true,
            ..Default::default()
        },
        transpose_semitones: 12,
        velocity_scale: 1.,
        nudge_frames: 0,
        swing: None,
    };
    let p = transforms::preview(&f.song, config).unwrap();
    let m = song_render::evaluate(p.candidate(), None).unwrap();
    let z = m.multisample_voices.values().next().unwrap();
    assert_eq!((z.note, z.zone), (72, KeyZoneId(3)));
    let mut c = GenerationConfig {
        version: 1,
        name: "Mapped melody".into(),
        bars: 1,
        instrument: InstrumentId(1),
        seed: 1,
        preset: GenerationPreset::Euclidean {
            steps: 4,
            pulses: 4,
            rotation: 0,
            probability: 1.,
        },
        sequence: vec![
            GenerationNote {
                note: 60,
                velocity: 0.5,
            },
            GenerationNote {
                note: 72,
                velocity: 0.2,
            },
            GenerationNote {
                note: 84,
                velocity: 1.,
            },
        ],
    };
    let p = generation::preview(&f.song, c.clone()).unwrap();
    let m = song_render::evaluate(p.candidate(), None).unwrap();
    assert_eq!(m.multisample_voices.len(), 5);
    let mut gap = f.song.clone();
    gap.multisamples
        .get_mut(&MultisampleId(1))
        .unwrap()
        .zones
        .remove(&KeyZoneId(3));
    c.sequence = vec![GenerationNote {
        note: 72,
        velocity: 1.,
    }];
    assert!(generation::preview(&gap, c)
        .unwrap_err()
        .contains("no keyzone"));
}
#[test]
fn independent_region_envelopes_loops_off_cut_and_mute_match_pitched_reference() {
    let mut f = Fixture::new();
    f.song
        .set_region_playback(
            RegionId(1),
            Some(AmpEnvelope {
                attack_ms: 1,
                decay_ms: 2,
                sustain_percent: 30,
                release_ms: 100,
            }),
            Some(ForwardLoop {
                start_frame: 8,
                end_frame: 40,
            }),
        )
        .unwrap();
    f.song
        .set_region_playback(
            RegionId(3),
            Some(AmpEnvelope {
                attack_ms: 0,
                decay_ms: 0,
                sustain_percent: 100,
                release_ms: 500,
            }),
            Some(ForwardLoop {
                start_frame: 10,
                end_frame: 50,
            }),
        )
        .unwrap();
    put(&mut f.song, 0, 0, note(60, 1.));
    put(&mut f.song, 0, 1, note(72, 1.));
    put(
        &mut f.song,
        1,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        1,
        Cell {
            effects: StepEffects {
                cut: Some(0.),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let reference = ordinary_reference(&f.song);
    let pcm = f.pcm(37);
    assert_eq!(pcm, render(&reference, 37));
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
    assert!(pcm[2 * 5040..].iter().all(|x| *x == 0.));
    let mut alias = f.song.clone();
    alias.patterns.insert(
        PatternId(2),
        Pattern {
            name: "Alias".into(),
            rows: 32,
            tracks: Default::default(),
            track_aliases: BTreeMap::from([(
                TrackId(1),
                TrackAlias {
                    pattern: PatternId(1),
                    track: TrackId(1),
                },
            )]),
            generation: None,
        },
    );
    alias.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: Default::default(),
        section: None,
    });
    assert_eq!(
        song_render::evaluate(&alias, None)
            .unwrap()
            .multisample_voices
            .len(),
        4
    );
    alias.order[1].muted_tracks.insert(TrackId(1));
    let muted = song_render::evaluate(&alias, None).unwrap();
    assert_eq!(muted.multisample_voices.len(), 2);
}
#[test]
fn cold_seek_does_not_revive_old_sources_and_streaming_loops_repeat_compiled_choices() {
    use aqua_pattern_core::transport::{Producer, Shared};
    use std::sync::Arc;
    let mut f = Fixture::new();
    put(&mut f.song, 1, 1, note(72, 0.2));
    put(&mut f.song, 2, 2, note(72, 1.));
    let expected = f.pcm(37);
    let mut e = Engine::new(&f.song, None).unwrap();
    e.cold_seek(240).unwrap();
    let mut b = [0.; 74];
    e.block(&mut b, 314).unwrap();
    // A cold seek omits old column0; render a matching source-free prefix oracle.
    let mut cold = f.song.clone();
    cold.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .remove(&0);
    let pcm = render(&cold, 37);
    assert_eq!(&b[..], &pcm[480..554]);
    let end = f.manifest().musical_frames as usize;
    let shared = Arc::new(Shared::default());
    shared.set_loop(true, end as u64);
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut looped = Vec::new();
    while looped.len() < end * 4 {
        producer.pump_until_buffered().unwrap();
        let mut b = [0.; 512];
        shared.fill(&mut b);
        looped.extend(b);
    }
    assert_eq!(&looped[..end * 2], &expected[..end * 2]);
    assert_eq!(&looped[end * 2..end * 4], &expected[..end * 2]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut b = [1.; 128];
    shared.fill(&mut b);
    assert!(b.iter().all(|v| *v == 0.));
}
#[test]
fn later_offset_and_reverse_validate_the_original_zone_not_future_selected_instrument() {
    let mut f = Fixture::new();
    f.song.regions.get_mut(&RegionId(1)).unwrap().end_frame = 1000;
    f.song
        .assign_region(InstrumentId(2), Some(RegionId(3)))
        .unwrap();
    put(
        &mut f.song,
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(2000),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let error = Engine::new(&f.song, None).err().unwrap();
    assert!(error.contains("[7..1000)"), "{error}");
    f.song
        .set_region_playback(
            RegionId(1),
            None,
            Some(ForwardLoop {
                start_frame: 8,
                end_frame: 40,
            }),
        )
        .unwrap();
    put(
        &mut f.song,
        2,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    std::fs::write(&f.song.samples[&SampleId(1)].path, b"invalid audio").unwrap();
    let error = Engine::new(&f.song, None).err().unwrap();
    assert!(error.contains("forward loop does not support"), "{error}");
}
