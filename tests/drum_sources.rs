use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine, RenderLayerSelection},
    RATE,
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
    rate: u32,
}
fn signal(frame: u32, channel: u16) -> f32 {
    let window = frame / 32;
    let x = frame % 32;
    if x >= 16 {
        0.9
    } else {
        (0.02 * (window + 1) as f32 + x as f32 * 0.0001) * if channel == 0 { 1.0 } else { -0.5 }
    }
}
fn note(velocity: Option<f32>) -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity,
        ..Default::default()
    }
}
fn zone(lo: u8, hi: u8, alternatives: &[(u64, u64)]) -> VelocityZone {
    VelocityZone {
        min_velocity: lo,
        max_velocity: hi,
        alternatives: alternatives
            .iter()
            .map(|&(id, region)| PadLayer {
                id: PadLayerId(id),
                region: RegionId(region),
            })
            .collect(),
    }
}
impl Fixture {
    fn new(rate: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-pad-sources-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("source.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for f in 0..128 {
            for c in 0..2 {
                writer.write_sample(signal(f, c)).unwrap();
            }
        }
        writer.finalize().unwrap();
        let mut song = example_song();
        song.bpm = 120.0;
        song.lines_per_beat = 100;
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 16;
        song.tracks.get_mut(&TrackId(2)).unwrap().note_columns = 16;
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "RR".into(),
                rows: 16,
                tracks: Default::default(),
            },
        )]);
        song.order = vec![Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        }];
        song.register_sample(SampleId(1), sample_asset::import(&path).unwrap().asset)
            .unwrap();
        for id in 1..=4 {
            song.upsert_region(
                RegionId(id),
                SampleRegion {
                    name: format!("Region{id}"),
                    sample: SampleId(1),
                    start_frame: (id - 1) * 32,
                    end_frame: (id - 1) * 32 + 16,
                    root_note: 72 + id as u8,
                    amp_envelope: None,
                    forward_loop: None,
                },
            )
            .unwrap();
        }
        song.upsert_drum_kit(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Kit".into(),
                pads: BTreeMap::from([(
                    PadId(1),
                    DrumPad {
                        slot: 0,
                        name: "Pad".into(),
                        note: 60,
                        region: Some(RegionId(1)),
                        velocity_zones: Vec::new(),
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
        )
        .unwrap();
        song.assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
        let mut f = Self { dir, song, rate };
        f.put(0, 0, note(Some(1.0)));
        f
    }
    fn put(&mut self, row: u32, col: u16, cell: Cell) {
        self.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(row)
            .or_default()
            .insert(col, cell);
    }
    fn zones(&mut self, zones: Vec<VelocityZone>) {
        self.song
            .set_pad_sources(DrumKitId(1), PadId(1), None, zones)
            .unwrap();
    }
    fn pad(&mut self) -> &mut DrumPad {
        self.song
            .drum_kits
            .get_mut(&DrumKitId(1))
            .unwrap()
            .pads
            .get_mut(&PadId(1))
            .unwrap()
    }
    fn manifest(&self) -> song_render::Manifest {
        song_render::evaluate(&self.song, None).unwrap()
    }
    fn pcm(&self, block: usize) -> Vec<f32> {
        render_engine(Engine::new(&self.song, None).unwrap(), block)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn render_engine(mut engine: Engine, block: usize) -> Vec<f32> {
    let mut pcm = Vec::new();
    let mut data = vec![0.0; block * 2];
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    while engine.position < end {
        let n = engine.block(&mut data, end).unwrap();
        pcm.extend_from_slice(&data[..n * 2]);
    }
    pcm
}
fn selected(f: &Fixture) -> Vec<(u64, RegionId, RenderLayerSelection)> {
    let m = f.manifest();
    m.events
        .iter()
        .filter_map(|e| {
            if let Command::On { voice, .. } = e.command {
                let p = &m.pad_voices[&voice];
                Some((e.frame, p.region.region, p.layer_selection.clone().unwrap()))
            } else {
                None
            }
        })
        .collect()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
#[test]
fn old_defaults_and_unused_zone_banks_keep_bytes_and_audio_exact() {
    let mut f = Fixture::new(RATE);
    let json = serde_json::to_string(&f.song).unwrap();
    assert!(!json.contains("velocity_zones"));
    let old = f.manifest();
    let pcm = f.pcm(37);
    let mut unused = f.song.drum_kits[&DrumKitId(1)].clone();
    let pad = unused.pads.get_mut(&PadId(1)).unwrap();
    pad.region = None;
    pad.velocity_zones = vec![zone(0, 127, &[(1, 1), (2, 2)])];
    f.song.upsert_drum_kit(DrumKitId(2), unused).unwrap();
    assert_eq!(old, f.manifest());
    assert_eq!(pcm, f.pcm(37));
    f.zones(vec![zone(0, 127, &[(7, 1)])]);
    assert_eq!(f.manifest().schema, 9);
    f.song
        .set_pad_sources(DrumKitId(1), PadId(1), Some(RegionId(1)), Vec::new())
        .unwrap();
    assert_eq!(old, f.manifest());
    assert_eq!(pcm, f.pcm(37));
}
#[test]
fn model_rejects_invalid_partitions_ids_refs_and_budget_atomically() {
    let mut f = Fixture::new(RATE);
    let old = f.song.clone();
    let mut bad = vec![
        vec![zone(1, 127, &[(1, 1)])],
        vec![zone(0, 126, &[(1, 1)])],
        vec![zone(0, 63, &[(1, 1)]), zone(63, 127, &[(2, 2)])],
        vec![zone(0, 62, &[(1, 1)]), zone(64, 127, &[(2, 2)])],
        vec![zone(0, 127, &[])],
        vec![zone(0, 127, &[(0, 1)])],
        vec![zone(0, 63, &[(1, 1)]), zone(64, 127, &[(1, 2)])],
        vec![zone(0, 127, &[(1, 99)])],
    ];
    bad.push(vec![zone(
        0,
        127,
        &(1..=9).map(|id| (id, 1)).collect::<Vec<_>>(),
    )]);
    bad.push(
        (0..16)
            .map(|n| zone(n * 8, n * 8 + 7, &[(u64::from(n) + 1, 1)]))
            .collect(),
    );
    bad.push(
        (0..8)
            .map(|n| {
                zone(
                    n * 16,
                    n * 16 + 15,
                    &(0..5)
                        .map(|i| (u64::from(n) * 5 + i + 1, 1))
                        .collect::<Vec<_>>(),
                )
            })
            .collect(),
    );
    for zones in bad {
        assert!(f
            .song
            .set_pad_sources(DrumKitId(1), PadId(1), None, zones)
            .is_err());
        assert_eq!(f.song, old);
    }
    assert!(f
        .song
        .set_pad_sources(
            DrumKitId(1),
            PadId(1),
            Some(RegionId(1)),
            vec![zone(0, 127, &[(1, 1)])]
        )
        .is_err());
    assert_eq!(f.song, old);
    assert!(f
        .song
        .set_pad_sources(DrumKitId(1), PadId(1), None, vec![])
        .is_err());
    assert_eq!(f.song, old);
    let zones = (0..8)
        .map(|n| {
            zone(
                n * 16,
                n * 16 + 15,
                &(0..4)
                    .map(|i| (u64::from(n) * 4 + i + 1, 1))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    f.zones(zones);
    assert_eq!(f.pad().source_regions().count(), 32);
}
#[test]
fn velocity_boundaries_quantize_selection_only_not_audio_gain() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 63, &[(11, 1)]), zone(64, 127, &[(22, 2)])]);
    let values = [
        0.0,
        f32::from_bits(0.5f32.to_bits() - 1),
        0.5,
        f32::from_bits(0.5f32.to_bits() + 1),
        1.0,
    ];
    for (row, value) in values.iter().copied().enumerate() {
        f.put(row as u32, 0, note(Some(value)));
    }
    let hits = selected(&f);
    let pcm = f.pcm(37);
    for (i, (at, region, s)) in hits.iter().enumerate() {
        let expected = if values[i] < 0.5 { 1 } else { 2 };
        assert_eq!(region.0, expected);
        assert_eq!(s.input_velocity.to_bits(), values[i].to_bits());
        assert_eq!(s.velocity_bin, (f64::from(values[i]) * 127.0).round() as u8);
        near(
            pcm[*at as usize * 2],
            signal(((expected - 1) * 32) as u32, 0) * values[i] * 0.5,
        );
    }
    assert_eq!(hits[2].2.velocity_bin, 64);
    assert_eq!(hits[1].2.velocity_bin, 63);
}
#[test]
fn independent_zone_counters_follow_vector_order_and_ignore_pad_gain() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![
        zone(0, 63, &[(50, 2), (10, 1)]),
        zone(64, 127, &[(90, 4), (20, 3)]),
    ]);
    for (row, v) in [0.1, 0.9, 0.1, 0.1, 0.9, 0.9].into_iter().enumerate() {
        f.put(row as u32, 0, note(Some(v)));
    }
    f.pad().gain = 0.0;
    let h = selected(&f);
    assert_eq!(
        h.iter().map(|x| x.2.layer.0).collect::<Vec<_>>(),
        [50, 90, 10, 50, 20, 90]
    );
    assert_eq!(
        h.iter()
            .map(|x| x.2.round_robin_ordinal)
            .collect::<Vec<_>>(),
        [0, 0, 1, 2, 1, 2]
    );
    assert!(f.pcm(37).iter().all(|x| *x == 0.0));
}
#[test]
fn zero_hits_advance_but_later_volume_controls_do_not_reselect() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    f.put(0, 0, note(Some(0.0)));
    f.put(
        1,
        0,
        Cell {
            velocity: Some(0.75),
            ..Default::default()
        },
    );
    f.put(2, 0, note(None));
    let h = selected(&f);
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].1, RegionId(1));
    assert_eq!(h[1].1, RegionId(2));
    assert_eq!(h[1].2.input_velocity, 0.75);
    let pcm = f.pcm(37);
    assert!(pcm[..480 * 2].iter().all(|x| *x == 0.0));
    near(pcm[480 * 2], signal(32, 0) * 0.75 * 0.5);
}
#[test]
fn native_rate_source_choices_and_fixed_tuning_have_real_pcm_oracle() {
    for rate in [24000, 44100, 48000, 96000] {
        for tune in [-12, 0, 12] {
            let mut f = Fixture::new(rate);
            f.zones(vec![zone(0, 127, &[(1, 1), (2, 2), (3, 3), (4, 4)])]);
            f.pad().tune_semitones = tune;
            for row in 0..4 {
                f.put(row, 0, note(Some(0.75)));
            }
            let pcm = f.pcm(37);
            let speed = 2.0_f64.powf(f64::from(tune) / 12.0);
            for (at, r, _) in selected(&f) {
                let start = (r.0 - 1) * 32;
                let duration = (16.0 * 48000.0 / (f64::from(f.rate) * speed)).ceil() as usize;
                for n in 0..duration {
                    let p = (start as f64 * 48000.0 + n as f64 * speed * f64::from(rate)) / 48000.0;
                    let lo = p.floor() as u32;
                    for c in 0..2 {
                        let a = signal(lo, c);
                        let b = if u64::from(lo) + 1 < start + 16 {
                            signal(lo + 1, c)
                        } else {
                            0.0
                        };
                        near(
                            pcm[(at as usize + n) * 2 + usize::from(c)],
                            (a + (b - a) * (p - p.floor()) as f32) * 0.75 * 0.5,
                        );
                    }
                }
                near(pcm[(at as usize + duration) * 2], 0.0);
            }
            assert_eq!(pcm, f.pcm(1));
            assert_eq!(pcm, f.pcm(512));
        }
    }
}
#[test]
fn retriggers_repeats_and_alias_uses_share_full_arrangement_counter() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    let mut on = note(Some(1.0));
    on.effects.retrigger = Some(Retrigger {
        repeats: 3,
        spacing: 0.25,
    });
    f.put(0, 0, on);
    f.put(1, 0, note(None));
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 2;
    f.song.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            track_aliases: BTreeMap::from([(
                TrackId(2),
                TrackAlias {
                    pattern: PatternId(1),
                    track: TrackId(1),
                },
            )]),
            name: "Alias".into(),
            rows: 2,
            tracks: Default::default(),
        },
    );
    f.song.order.extend([
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        },
        Occurrence {
            pattern: PatternId(2),
            muted_tracks: Default::default(),
            section: None,
        },
    ]);
    let h = selected(&f);
    assert_eq!(h.len(), 15);
    for (i, (_, region, s)) in h.iter().enumerate() {
        assert_eq!(*region, RegionId(i as u64 % 2 + 1));
        assert_eq!(s.round_robin_ordinal, i as u32);
        assert_eq!(s.round_robin_index, (i % 2) as u8);
    }
}
#[test]
fn phrase_instances_share_instrument_counter_and_snapshot_velocity() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![
        zone(0, 63, &[(1, 1), (2, 2)]),
        zone(64, 127, &[(3, 3), (4, 4)]),
    ]);
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Phrase".into(),
            instrument: InstrumentId(1),
            rows: 2,
            lines_per_beat: 100,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([
                (0, BTreeMap::from([(0, note(Some(0.5)))])),
                (1, BTreeMap::from([(0, note(None))])),
            ]),
        },
    );
    for col in 0..2 {
        f.put(
            0,
            col,
            Cell {
                note: Note::Note(60),
                velocity: Some(0.5),
                phrase: Some(PhraseTrigger {
                    phrase: PhraseId(1),
                    cycles: 2,
                }),
                ..Default::default()
            },
        );
    }
    let h = selected(&f);
    assert_eq!(h.len(), 8);
    for (i, (_, r, s)) in h.iter().enumerate() {
        assert_eq!(*r, RegionId(i as u64 % 2 + 1));
        assert_eq!(s.input_velocity, 0.25);
        assert_eq!(s.round_robin_ordinal, i as u32);
    }
}
#[test]
fn same_kit_on_distinct_instruments_has_separate_counters() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    f.song
        .assign_kit(InstrumentId(2), Some(DrumKitId(1)))
        .unwrap();
    for row in 0..3 {
        f.put(row, 0, note(None));
        f.put(
            row,
            1,
            Cell {
                instrument: Some(InstrumentId(2)),
                ..note(None)
            },
        );
    }
    let h = selected(&f);
    assert_eq!(
        h.iter().map(|x| x.1 .0).collect::<Vec<_>>(),
        [1, 1, 2, 2, 1, 1]
    );
    assert_eq!(
        h.iter()
            .map(|x| x.2.round_robin_ordinal)
            .collect::<Vec<_>>(),
        [0, 0, 1, 1, 2, 2]
    );
}
#[test]
fn muted_rows_and_rounding_spill_do_not_consume_choices() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 2;
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.9999999),
            ..note(None)
        },
    );
    let mut muted = f.song.patterns[&PatternId(1)].clone();
    muted.tracks.clear();
    muted.tracks.insert(
        TrackId(1),
        BTreeMap::from([(0, BTreeMap::from([(0, note(None))]))]),
    );
    f.song.patterns.insert(PatternId(2), muted.clone());
    f.song.patterns.insert(PatternId(3), muted);
    f.song.order.extend([
        Occurrence {
            pattern: PatternId(2),
            muted_tracks: std::collections::BTreeSet::from([TrackId(1)]),
            section: None,
        },
        Occurrence {
            pattern: PatternId(3),
            muted_tracks: Default::default(),
            section: None,
        },
    ]);
    let h = selected(&f);
    assert_eq!(h.len(), 2);
    assert_eq!((h[0].0, h[1].0), (0, 960));
    assert_eq!((h[0].1, h[1].1), (RegionId(1), RegionId(2)));
    assert_eq!(h[1].2.round_robin_ordinal, 1);
}
#[test]
fn only_selected_alternatives_determine_loop_fx_compatibility() {
    let mut f = Fixture::new(RATE);
    f.song
        .set_region_playback(
            RegionId(2),
            None,
            Some(ForwardLoop {
                start_frame: 32,
                end_frame: 40,
            }),
        )
        .unwrap();
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    let mut on = note(Some(1.0));
    on.effects.direction = Some(Direction::Reverse);
    f.put(0, 0, on.clone());
    assert!(Engine::new(&f.song, None).is_ok());
    f.put(1, 0, on);
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("forward loop does not support"));
    let out = f.dir.join("invalid.wav");
    assert!(song_render::export(&f.song, None, &out).is_err());
    assert!(!out.exists());
    assert!(!out.with_extension("events.json").exists());
}
#[test]
fn unused_alternative_regions_cannot_be_deleted_and_all_ids_roundtrip() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(u64::MAX, 1), (13, 2)])]);
    let old = f.song.clone();
    assert!(f.song.remove_region(RegionId(2)).is_err());
    assert_eq!(f.song, old);
    let json = serde_json::to_string(&f.song).unwrap();
    assert_eq!(serde_json::from_str::<Song>(&json).unwrap(), old);
    assert!(!f.pad().velocity_zones.is_empty());
    // Unused kits protect their configured alternatives too.
    let kit = f.song.drum_kits[&DrumKitId(1)].clone();
    f.song.upsert_drum_kit(DrumKitId(2), kit).unwrap();
    f.song
        .set_pad_sources(DrumKitId(1), PadId(1), Some(RegionId(1)), vec![])
        .unwrap();
    assert!(f.song.remove_region(RegionId(2)).is_err());
}
#[test]
fn cold_seek_stream_loop_and_export_reopen_keep_compiled_choices() {
    use aqua_pattern_core::transport::{Producer, Shared};
    use std::sync::Arc;
    let mut f = Fixture::new(44100);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    for row in 1..5 {
        f.put(row, 0, note(None));
    }
    let expected = f.pcm(512);
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(240).unwrap();
    let cold = render_engine(engine, 37);
    assert_eq!(cold, &expected[480..]);
    near(cold[0], signal(32, 0) * 0.5);
    let json = serde_json::to_string(&f.song).unwrap();
    let reopened: Song = serde_json::from_str(&json).unwrap();
    let a = f.dir.join("a.wav");
    let b = f.dir.join("b.wav");
    song_render::export(&f.song, None, &a).unwrap();
    song_render::export(&reopened, None, &b).unwrap();
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
    let shared = Arc::new(Shared::default());
    shared.set_loop(true, 3840);
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut looped = Vec::new();
    while looped.len() < 3840 * 4 {
        producer.pump_until_buffered().unwrap();
        let mut data = [0.0; 512];
        shared.fill(&mut data);
        looped.extend(data);
    }
    assert_eq!(&looped[..7680], &expected[..7680]);
    assert_eq!(&looped[7680..15360], &expected[..7680]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut data = [1.0; 512];
    shared.fill(&mut data);
    assert!(data.iter().all(|x| *x == 0.0));
}
#[test]
fn simultaneous_alternatives_keep_independent_loops_envelopes_and_release() {
    let mut f = Fixture::new(RATE);
    f.song
        .set_region_playback(
            RegionId(1),
            Some(AmpEnvelope {
                attack_ms: 0,
                decay_ms: 0,
                sustain_percent: 100,
                release_ms: 100,
            }),
            Some(ForwardLoop {
                start_frame: 0,
                end_frame: 8,
            }),
        )
        .unwrap();
    f.song
        .set_region_playback(
            RegionId(2),
            Some(AmpEnvelope {
                attack_ms: 1,
                decay_ms: 1,
                sustain_percent: 50,
                release_ms: 0,
            }),
            Some(ForwardLoop {
                start_frame: 32,
                end_frame: 36,
            }),
        )
        .unwrap();
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    f.put(1, 0, note(None));
    f.put(
        3,
        0,
        Cell {
            effects: StepEffects {
                release: Some(0.0),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let pcm = f.pcm(37);
    for n in 0..1000 {
        let old = signal((n % 8) as u32, 0);
        let new = if (240..720).contains(&n) {
            let age = n - 240;
            let gain = if age < 48 {
                age as f32 / 48.0
            } else if age < 96 {
                1.0 - 0.5 * (age - 48) as f32 / 48.0
            } else {
                0.5
            };
            signal(32 + (age % 4) as u32, 0) * gain
        } else {
            0.0
        };
        near(pcm[n * 2], (old + new) * 0.5);
    }
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
}
#[test]
fn new_note_and_choke_follow_pad_identity_across_alternative_switches() {
    use aqua_pattern_core::song_render::Release;
    for choke in [false, true] {
        let mut f = Fixture::new(RATE);
        for id in 1..=2 {
            let start = (id - 1) * 32;
            f.song
                .set_region_playback(
                    RegionId(id),
                    Some(AmpEnvelope {
                        release_ms: 500,
                        ..Default::default()
                    }),
                    Some(ForwardLoop {
                        start_frame: start,
                        end_frame: start + 8,
                    }),
                )
                .unwrap();
        }
        f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
        f.pad().new_note = if choke {
            PadNewNote::Overlap
        } else {
            PadNewNote::ReleasePrevious
        };
        f.pad().choke_group = choke.then_some(1);
        f.put(1, 0, note(Some(0.0)));
        let m = f.manifest();
        let expected = if choke {
            Release::Choke
        } else {
            Release::PadReplaced
        };
        assert!(m
            .events
            .iter()
            .any(|e| e.frame == 240
                && matches!(&e.command,Command::Off{reason,..}if *reason==expected)));
        let h = selected(&f);
        assert_eq!(h[1].1, RegionId(2));
        let pcm = f.pcm(37);
        for n in 240..2640 {
            near(
                pcm[n * 2],
                signal((n % 8) as u32, 0) * (1.0 - (n - 240) as f32 / 2400.0) * 0.5,
            );
        }
        assert!(pcm[2640 * 2..].iter().all(|x| *x == 0.0));
    }
}
#[test]
fn generation_accepts_zoned_assignment_and_continues_existing_counter() {
    use aqua_pattern_core::generation::{self, GenerationConfig, GenerationNote, GenerationPreset};
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    let plan = generation::preview(
        &f.song,
        GenerationConfig {
            version: 1,
            name: "RR generated".into(),
            bars: 1,
            instrument: InstrumentId(1),
            seed: 0,
            preset: GenerationPreset::Euclidean {
                steps: 4,
                pulses: 4,
                rotation: 0,
                probability: 1.0,
            },
            sequence: vec![GenerationNote {
                note: 60,
                velocity: 1.0,
            }],
        },
    )
    .unwrap();
    let m = song_render::evaluate(plan.candidate(), None).unwrap();
    let selections: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On { voice, .. } = e.command {
                Some(m.pad_voices[&voice].layer_selection.as_ref().unwrap())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(selections.len(), 5);
    for (i, s) in selections.iter().enumerate() {
        assert_eq!(s.round_robin_ordinal, i as u32);
        assert_eq!(s.layer, PadLayerId(i as u64 % 2 + 1));
    }
    assert_eq!(plan.base(), &f.song);
}
#[test]
fn cancelled_phrase_children_do_not_consume_but_started_cut_hits_do() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2), (3, 3)])]);
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Cancelled future starts".into(),
            instrument: InstrumentId(1),
            rows: 2,
            lines_per_beat: 100,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([
                (0, BTreeMap::from([(0, note(Some(1.0)))])),
                (1, BTreeMap::from([(0, note(None))])),
            ]),
        },
    );
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(60),
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 8,
            }),
            effects: StepEffects {
                cut: Some(0.5),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(
        2,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.0),
                ..Default::default()
            },
            ..note(Some(1.0))
        },
    );
    f.put(3, 0, note(None));
    let hits = selected(&f);
    assert_eq!(
        hits.iter()
            .map(|(frame, region, selection)| (*frame, *region, selection.round_robin_ordinal))
            .collect::<Vec<_>>(),
        vec![
            (0, RegionId(1), 0),
            (480, RegionId(2), 1),
            (720, RegionId(3), 2)
        ]
    );
    let pcm = f.pcm(37);
    assert!(pcm[480 * 2..720 * 2].iter().all(|x| *x == 0.0));
    near(pcm[720 * 2], signal(64, 0) * 0.5);
}
#[test]
fn zoned_hits_keep_existing_start_clipping_and_export_cleanup_limits() {
    let mut f = Fixture::new(RATE);
    f.zones(vec![zone(0, 127, &[(1, 1), (2, 2)])]);
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for col in 0..256 {
        f.put(0, col, note(Some(1.0)));
    }
    let clipped = f.dir.join("clipped.wav");
    let error = song_render::export(&f.song, None, &clipped).unwrap_err();
    assert!(error.contains("clipping"), "{error}");
    assert!(!clipped.exists());
    assert!(!clipped.with_extension("events.json").exists());
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 129;
    for row in 0..129 {
        f.put(
            row,
            0,
            Cell {
                effects: StepEffects {
                    retrigger: Some(Retrigger {
                        repeats: 16,
                        spacing: 1.0 / 17.0,
                    }),
                    ..Default::default()
                },
                ..note(Some(0.01))
            },
        );
    }
    let limited = f.dir.join("limited.wav");
    let error = song_render::export(&f.song, None, &limited).unwrap_err();
    assert!(error.contains("2048"), "{error}");
    assert!(!limited.exists());
    assert!(!limited.with_extension("events.json").exists());
}
