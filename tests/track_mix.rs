use aqua_pattern_core::{
    audition, sample_asset,
    song::*,
    song_render::{self, Command, Engine},
    track_mix::{self, MixLayoutStatus, TrackMix, TrackMixConfig, TrackMixSetting},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    asset: SampleAsset,
}
impl Fixture {
    fn new(rate: u32, channels: u16, frames: usize) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-track-mix-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("authored.wav");
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
        for i in 0..frames {
            for channel in 0..channels {
                w.write_sample(
                    (0.025 + (i % 7) as f32 * 0.001) * if channel == 0 { 1.0 } else { -0.4 },
                )
                .unwrap();
            }
        }
        w.finalize().unwrap();
        let asset = sample_asset::import(&path).unwrap().asset;
        Self { dir, asset }
    }
    fn song(&self) -> Song {
        let region = SampleRegion {
            name: "Authored".into(),
            sample: SampleId(1),
            start_frame: 0,
            end_frame: self.asset.frames,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        };
        audition::region_preview(&self.asset, &region)
            .unwrap()
            .song()
            .clone()
    }
    fn legacy(&self) -> Song {
        let mut s = self.song();
        let i = s.instruments.get_mut(&InstrumentId(1)).unwrap();
        i.region = None;
        i.sample = Some(self.asset.path.clone());
        s.samples.clear();
        s.regions.clear();
        s
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn bytes(v: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn config(values: &[(u64, f64, f64, bool, bool)]) -> TrackMixConfig {
    TrackMixConfig {
        version: 1,
        settings: values
            .iter()
            .map(|&(t, g, b, m, s)| TrackMixSetting {
                track: TrackId(t),
                gain: g,
                balance: b,
                mute: m,
                solo: s,
            })
            .collect(),
    }
}
fn add_track(s: &mut Song, id: u64) {
    s.tracks.insert(
        TrackId(id),
        Track {
            name: format!("Track{id}"),
            note_columns: 1,
        },
    );
    s.track_order.push(TrackId(id));
}
fn cell(s: &mut Song, track: u64, row: u32, velocity: f32) {
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(track))
        .or_default()
        .entry(row)
        .or_default()
        .insert(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                velocity: Some(velocity),
                pan: Some(0.0),
                ..Default::default()
            },
        );
}
fn render(s: &Song, block: usize) -> Vec<f32> {
    let mut engine = Engine::new(s, None).unwrap();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    read(&mut engine, end as usize, block)
}
fn read(engine: &mut Engine, frames: usize, block: usize) -> Vec<f32> {
    let end = engine.position + frames as u64;
    let mut out = Vec::new();
    let mut buffer = vec![0.0; block * 2];
    while engine.position < end {
        let n = engine.block(&mut buffer, end).unwrap();
        out.extend_from_slice(&buffer[..n * 2]);
    }
    out
}
fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|v| v.to_bits()).collect()
}
fn ramp(a: f32, b: f32, k: usize) -> f32 {
    if k >= 128 {
        b
    } else {
        (f64::from(a) + (f64::from(b) - f64::from(a)) * (k as f64 / 128.0)) as f32
    }
}

#[test]
fn pure_full_set_unavailable_model_save_and_strict_input_boundaries() {
    let mut s = example_song();
    s.order.clear();
    let before = bytes(&s);
    let p = track_mix::preview(&s, config(&[(1, 0.5, -0.25, false, false)])).unwrap();
    assert!(matches!(
        p.summary().layout_status,
        MixLayoutStatus::Unavailable { .. }
    ));
    assert_eq!(bytes(&s), before);
    assert_eq!(p.candidate().track_mix[&TrackId(1)].gain, 0.5);
    assert_eq!(p.summary().changed_tracks, 1);
    s.set_track_mix(config(&[(1, 0.5, -0.25, false, false)]))
        .unwrap();
    assert_eq!(bytes(&s), bytes(p.candidate()));
    for value in [f64::NAN, f64::INFINITY, -0.00000001, 1.0000000000000002] {
        assert!(track_mix::preview(&s, config(&[(1, value, 0.0, false, false)])).is_err());
    }
    for value in [
        f64::NAN,
        f64::NEG_INFINITY,
        -1.0000000000000002,
        1.0000000000000002,
    ] {
        assert!(track_mix::preview(&s, config(&[(1, 1.0, value, false, false)])).is_err());
    }
    assert!(track_mix::preview(
        &s,
        config(&[(1, 1.0, 0.0, false, false), (1, 1.0, 0.0, false, false)])
    )
    .is_err());
    assert!(track_mix::preview(&s, config(&[(99, 1.0, 0.0, false, false)])).is_err());
    let mut cfg = config(&[]);
    cfg.version = 2;
    assert!(track_mix::preview(&s, cfg).is_err());
    s.set_track_mix(config(&[])).unwrap();
    assert!(!String::from_utf8(bytes(&s)).unwrap().contains("track_mix"));
}
#[test]
fn exact_preview_base_and_only_mix_excluded_content_authority() {
    let f = Fixture::new(48000, 1, 400);
    let mut s = f.song();
    s.patterns
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
    let p = track_mix::preview(&s, config(&[(1, 0.5, -0.0, false, false)])).unwrap();
    assert!(track_mix::same_playing_content(&s, p.candidate()).unwrap());
    assert_eq!(
        track_mix::content_sha256(&s).unwrap(),
        p.summary().content_sha256
    );
    let mut edited = s.clone();
    edited
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
    assert_eq!(edited, s);
    assert!(!p.is_current(&edited).unwrap());
    assert!(p.commit(&edited).is_err());
    assert!(!track_mix::same_playing_content(&s, &edited).unwrap());
    edited = s.clone();
    let old = edited.samples[&SampleId(1)].path.clone();
    edited.samples.get_mut(&SampleId(1)).unwrap().path = old
        .parent()
        .unwrap()
        .join(".")
        .join(old.file_name().unwrap());
    assert_eq!(edited, s);
    assert!(!track_mix::same_playing_content(&s, &edited).unwrap());
    let p = track_mix::preview(p.candidate(), config(&[(1, 0.5, 0.0, false, false)])).unwrap();
    assert_eq!(p.summary().changed_tracks, 1);
    assert_eq!(p.base(), p.candidate());
    assert_ne!(bytes(p.base()), bytes(p.candidate()));
}
#[test]
fn old_json_and_effective_neutral_maps_keep_manifest_and_pcm_exact() {
    for (rate, channels) in [(8000, 1), (44100, 2), (48000, 2), (96000, 1)] {
        let f = Fixture::new(rate, channels, 400);
        let s = f.song();
        let original = render(&s, 37);
        let manifest = bytes(&song_render::evaluate(&s, None).unwrap());
        for cfg in [
            config(&[(1, 1.0, -0.0, false, false)]),
            config(&[(1, 1.0, 0.0, false, true)]),
        ] {
            let p = track_mix::preview(&s, cfg).unwrap();
            assert_eq!(
                manifest,
                bytes(&song_render::evaluate(p.candidate(), None).unwrap())
            );
            for block in [1, 512] {
                assert_eq!(bits(&original), bits(&render(p.candidate(), block)));
            }
            let parsed: Song = serde_json::from_slice(&bytes(p.candidate())).unwrap();
            assert_eq!(bytes(&parsed), bytes(p.candidate()));
        }
        let mut unused = s.clone();
        add_track(&mut unused, 2);
        unused.track_mix.insert(
            TrackId(2),
            TrackMix {
                gain: 0.0,
                ..Default::default()
            },
        );
        assert_eq!(bits(&original), bits(&render(&unused, 257)));
    }
}
#[test]
fn static_gain_balance_mute_solo_scale_exact_original_channels() {
    let f = Fixture::new(48000, 2, 400);
    let mut s = f.song();
    add_track(&mut s, 2);
    let baseline = render(&s, 37);
    for (cfg, l, r) in [
        (config(&[(1, 0.5, 0.0, false, false)]), 0.5, 0.5),
        (config(&[(1, 1.0, 0.5, false, false)]), 0.5, 1.0),
        (config(&[(1, 0.5, -1.0, false, false)]), 0.5, 0.0),
        (config(&[(1, 1.0, 0.0, true, true)]), 0.0, 0.0),
        (config(&[(2, 1.0, 0.0, false, true)]), 0.0, 0.0),
    ] {
        let p = track_mix::preview(&s, cfg).unwrap();
        let out = render(p.candidate(), 511);
        for (i, (&a, &b)) in baseline.iter().zip(&out).enumerate() {
            assert_eq!(
                (0.0 + a * if i % 2 == 0 { l } else { r }).to_bits(),
                b.to_bits()
            );
        }
        let base = song_render::evaluate(&s, None).unwrap();
        let changed = song_render::evaluate(p.candidate(), None).unwrap();
        assert_eq!(base.events, changed.events);
        assert_eq!(changed.schema, 11);
        assert!(!changed.track_mix.is_empty());
    }
}
#[test]
fn initial_static_settings_do_not_ramp_and_runtime_zero_is_canonical() {
    let f = Fixture::new(48000, 1, 400);
    let s = f.song();
    let p = track_mix::preview(&s, config(&[(1, -0.0, -0.0, false, false)])).unwrap();
    assert_eq!(
        p.candidate().track_mix[&TrackId(1)].gain.to_bits(),
        (-0.0f32).to_bits()
    );
    let snapshot = p
        .layout()
        .unwrap()
        .snapshot(&p.candidate().track_mix)
        .unwrap();
    assert_eq!(snapshot.coefficients()[0].left.to_bits(), 0);
    let out = render(p.candidate(), 1);
    assert!(out.iter().all(|&v| v == 0.0));
    let p = track_mix::preview(&s, config(&[(1, 0.5, 0.0, false, false)])).unwrap();
    let base = render(&s, 1);
    let out = render(p.candidate(), 1);
    assert_eq!(out[0].to_bits(), (base[0] * 0.5).to_bits());
}
#[test]
fn live_ramp_is_exact_128_emitted_frames_and_partition_independent() {
    let f = Fixture::new(48000, 2, 4096);
    let s = f.song();
    let baseline = render(&s, 1);
    let target = track_mix::preview(&s, config(&[(1, 0.0, 0.0, false, false)])).unwrap();
    let mut outputs = Vec::new();
    for block in [1, 37, 512] {
        let mut e = Engine::new(&s, None).unwrap();
        let mut out = read(&mut e, 64, block);
        e.apply_mix_snapshot(
            &e.mix_layout()
                .unwrap()
                .snapshot(&target.candidate().track_mix)
                .unwrap(),
        )
        .unwrap();
        out.extend(read(&mut e, 192, block));
        for frame in 0..256 {
            for c in 0..2 {
                let gain = if frame < 64 {
                    1.0
                } else {
                    ramp(1.0, 0.0, frame - 63)
                };
                assert_eq!(
                    out[frame * 2 + c].to_bits(),
                    (0.0 + baseline[frame * 2 + c] * gain).to_bits()
                );
            }
        }
        outputs.push(bits(&out));
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[0], outputs[2]);
}
#[test]
fn interrupted_ramps_restart_from_last_emitted_f32_without_restarting_voices() {
    let f = Fixture::new(48000, 2, 4096);
    let s = f.song();
    let baseline = render(&s, 37);
    let mut e = Engine::new(&s, None).unwrap();
    let layout = e.mix_layout().unwrap().clone();
    let mute = BTreeMap::from([(
        TrackId(1),
        TrackMix {
            mute: true,
            ..Default::default()
        },
    )]);
    e.apply_mix_snapshot(&layout.snapshot(&mute).unwrap())
        .unwrap();
    let mut out = read(&mut e, 33, 37);
    e.apply_mix_snapshot(&layout.snapshot(&BTreeMap::new()).unwrap())
        .unwrap();
    out.extend(read(&mut e, 180, 512));
    let start = ramp(1.0, 0.0, 33);
    for frame in 0..213 {
        for c in 0..2 {
            let gain = if frame < 33 {
                ramp(1.0, 0.0, frame + 1)
            } else {
                ramp(start, 1.0, frame - 32)
            };
            assert_eq!(
                out[frame * 2 + c].to_bits(),
                (0.0 + baseline[frame * 2 + c] * gain).to_bits()
            );
        }
    }
}
#[test]
fn cold_seek_keeps_ramp_state_and_wrong_layout_refuses_atomically() {
    let f = Fixture::new(48000, 2, 4096);
    let s = f.song();
    let mut e = Engine::new(&s, None).unwrap();
    let mut reference = Engine::new(&s, None).unwrap();
    let layout = e.mix_layout().unwrap().clone();
    let target = BTreeMap::from([(
        TrackId(1),
        TrackMix {
            gain: 0.0,
            ..Default::default()
        },
    )]);
    e.apply_mix_snapshot(&layout.snapshot(&target).unwrap())
        .unwrap();
    read(&mut e, 64, 37);
    e.cold_seek(0).unwrap();
    reference.cold_seek(0).unwrap();
    let out = read(&mut e, 128, 512);
    let plain = read(&mut reference, 128, 1);
    for frame in 0..128 {
        for c in 0..2 {
            assert_eq!(
                out[frame * 2 + c].to_bits(),
                (0.0 + plain[frame * 2 + c] * ramp(1.0, 0.0, frame + 65)).to_bits()
            );
        }
    }
    let mut other = s.clone();
    other.title.push('x');
    let other = track_mix::preview(&other, config(&[])).unwrap();
    let snapshot = other.layout().unwrap().snapshot(&BTreeMap::new()).unwrap();
    assert!(e
        .apply_mix_snapshot(&snapshot)
        .unwrap_err()
        .contains("another"));
    assert!(read(&mut e, 8, 1).iter().all(|v| *v == 0.0));
}
#[test]
fn shared_legacy_preserves_reused_slot_history_and_requires_equal_targets() {
    let f = Fixture::new(48000, 1, 400);
    let mut s = f.legacy();
    add_track(&mut s, 2);
    cell(&mut s, 1, 0, 0.2);
    cell(&mut s, 2, 2, 0.8);
    let baseline = render(&s, 512);
    let p = track_mix::preview(
        &s,
        config(&[(1, 0.5, 0.0, false, false), (2, 0.5, 0.0, false, false)]),
    )
    .unwrap();
    assert_eq!(p.summary().tracks[0].shared_legacy_tracks, vec![TrackId(2)]);
    assert_eq!(
        p.summary().tracks[0].shared_legacy_instruments,
        vec![InstrumentId(1)]
    );
    let out = render(p.candidate(), 37);
    for (&a, &b) in baseline.iter().zip(&out) {
        assert_eq!((a * 0.5).to_bits(), b.to_bits());
    }
    let bad = config(&[(1, 0.5, 0.0, false, false)]);
    assert!(track_mix::preview(&s, bad)
        .unwrap_err()
        .contains("Legacy instrument"));
    let mut e = Engine::new(&s, None).unwrap();
    let before = read(&mut e, 12000, 37);
    assert!(e
        .mix_layout()
        .unwrap()
        .snapshot(&BTreeMap::from([(
            TrackId(1),
            TrackMix {
                gain: 0.5,
                ..Default::default()
            }
        )]))
        .is_err());
    assert_eq!(bits(&before), bits(&baseline[..24000]));
    assert_eq!(bits(&read(&mut e, 400, 37)), bits(&baseline[24000..24800]));
    let mut split = s.clone();
    split
        .instruments
        .insert(InstrumentId(2), split.instruments[&InstrumentId(1)].clone());
    split
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(2))
        .unwrap()
        .get_mut(&2)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .instrument = Some(InstrumentId(2));
    let split = render(&split, 512);
    assert_eq!(bits(&baseline[..24000]), bits(&split[..24000]));
    assert_ne!(bits(&baseline[24000..24800]), bits(&split[24000..24800]));
}
#[test]
fn actual_route_classification_not_instrument_kind_controls_legacy_constraints() {
    let f = Fixture::new(48000, 1, 4096);
    let mut s = f.legacy();
    add_track(&mut s, 2);
    cell(&mut s, 2, 2, 0.7);
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(2))
        .unwrap()
        .get_mut(&2)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .effects
        .offset = Some(0);
    let p = track_mix::preview(&s, config(&[(2, 0.25, 0.0, false, false)])).unwrap();
    assert_eq!(
        p.layout().unwrap().legacy_groups()[0].tracks,
        vec![TrackId(1)]
    );
    assert!(p
        .summary()
        .tracks
        .iter()
        .all(|t| t.shared_legacy_tracks.is_empty()));
    let baseline = render(&s, 37);
    let out = render(p.candidate(), 37);
    assert_eq!(bits(&baseline[..24000]), bits(&out[..24000]));
    for (&a, &b) in baseline[24000..].iter().zip(&out[24000..]) {
        assert_eq!((a * 0.25).to_bits(), b.to_bits());
    }
}
#[test]
fn logical_alias_owner_is_independent_for_controlled_regions() {
    let f = Fixture::new(48000, 1, 400);
    let mut s = f.song();
    add_track(&mut s, 5);
    s.set_track_alias(
        PatternId(1),
        TrackId(5),
        Some(TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        }),
    )
    .unwrap();
    let p = track_mix::preview(&s, config(&[(5, 0.0, 0.0, true, false)])).unwrap();
    assert_eq!(p.layout().unwrap().tracks(), &[TrackId(1), TrackId(5)]);
    assert!(p.layout().unwrap().legacy_groups().is_empty());
    let once = f.song();
    assert_eq!(bits(&render(p.candidate(), 37)), bits(&render(&once, 37)));
    let base = song_render::evaluate(&s, None).unwrap();
    let after = song_render::evaluate(p.candidate(), None).unwrap();
    assert_eq!(base.events, after.events);
}
#[test]
fn global_rr_choke_and_pad_provenance_survive_output_mute_and_solo() {
    let f = Fixture::new(48000, 1, 400);
    let mut s = f.song();
    add_track(&mut s, 2);
    let (instrument, kit) = s.create_drum_kit_instrument("Kit".into()).unwrap();
    let mut pad = DrumPad {
        slot: 0,
        name: "Pad".into(),
        note: 36,
        region: Some(RegionId(1)),
        velocity_zones: vec![],
        root_note: 60,
        tune_semitones: 0,
        gain: 1.0,
        pan: 0.0,
        mode: PadMode::OneShot,
        new_note: PadNewNote::Overlap,
        choke_group: Some(1),
    };
    pad.velocity_zones = vec![VelocityZone {
        min_velocity: 0,
        max_velocity: 127,
        alternatives: vec![
            PadLayer {
                id: PadLayerId(1),
                region: RegionId(1),
            },
            PadLayer {
                id: PadLayerId(2),
                region: RegionId(1),
            },
        ],
    }];
    pad.region = None;
    s.drum_kits
        .get_mut(&kit)
        .unwrap()
        .pads
        .insert(PadId(1), pad);
    for (track, row) in [(1, 0), (2, 1), (1, 2)] {
        cell(&mut s, track, row, 0.5);
        let c = s
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(track))
            .unwrap()
            .get_mut(&row)
            .unwrap()
            .get_mut(&0)
            .unwrap();
        c.instrument = Some(instrument);
        c.note = Note::Note(36);
    }
    let base = song_render::evaluate(&s, None).unwrap();
    let p = track_mix::preview(&s, config(&[(1, 1.0, 0.0, true, false)])).unwrap();
    let after = song_render::evaluate(p.candidate(), None).unwrap();
    assert_eq!(base.events, after.events);
    assert_eq!(base.pad_voices, after.pad_voices);
    assert_eq!(
        after
            .pad_voices
            .values()
            .map(|p| p.layer_selection.as_ref().unwrap().round_robin_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 0]
    );
    assert!(after.events.iter().any(|e| matches!(
        e.command,
        Command::Off {
            reason: song_render::Release::Choke,
            ..
        }
    )));
    assert!(render(p.candidate(), 37).iter().any(|v| v.abs() > 0.000001));
}
#[test]
fn configured_and_contributor_caps_do_not_become_global_neutral_song_limits() {
    let f = Fixture::new(48000, 1, 40);
    let mut s = f.song();
    for id in 2..=65 {
        add_track(&mut s, id);
    }
    let p = track_mix::preview(&s, config(&[(65, 0.5, 0.0, false, false)])).unwrap();
    assert_eq!(p.layout().unwrap().tracks().len(), 1);
    let mut cfg = config(&[]);
    for id in 1..=65 {
        cfg.settings.push(TrackMixSetting {
            track: TrackId(id),
            gain: 1.0,
            balance: 0.0,
            mute: false,
            solo: false,
        });
    }
    assert!(track_mix::preview(&s, cfg).is_err());
    for id in 2..=65 {
        cell(&mut s, id, 0, 0.001);
    }
    cell(&mut s, 1, 0, 0.001);
    let p = track_mix::preview(&s, config(&[])).unwrap();
    assert!(matches!(
        p.summary().layout_status,
        MixLayoutStatus::Unsupported {
            contributing_tracks: 65,
            ..
        }
    ));
    let e = Engine::new(&s, None).unwrap();
    assert!(e.mix_layout().is_none());
    assert!(e.mix_unavailable().unwrap().contains("64"));
    assert!(track_mix::preview(&s, config(&[(1, 0.5, 0.0, false, false)])).is_err());
    s.track_mix.insert(
        TrackId(1),
        TrackMix {
            gain: 0.5,
            ..Default::default()
        },
    );
    assert!(Engine::new(&s, None).is_err());
}
#[test]
fn bounds_precede_evaluation_and_clipboard_clock_history_is_unchanged() {
    let mut s = example_song();
    s.order.clear();
    s.title = "x".repeat(8 * 1024 * 1024);
    assert!(track_mix::preview(&s, config(&[]))
        .unwrap_err()
        .contains("8 MiB"));
    let mut s = example_song();
    s.order.clear();
    s.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(PathBuf::from("x".repeat(8193)));
    assert!(track_mix::preview(&s, config(&[]))
        .unwrap_err()
        .contains("8192"));
    s.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(PathBuf::from("x".repeat(8192)));
    assert!(track_mix::preview(&s, config(&[])).is_ok());
    let mut s = example_song();
    s.order.clear();
    let before = bytes(&s);
    let p = track_mix::preview(&s, config(&[(1, 0.5, 0.0, false, false)])).unwrap();
    let mut restored = p.candidate().clone();
    restored.track_mix.clear();
    assert_eq!(before, bytes(&restored));
    s.track_mix.insert(TrackId(99), TrackMix::default());
    assert!(s.validate().is_err());
}

#[test]
fn unused_nonneutral_metadata_never_imposes_optional_layout_limits_on_neutral_pcm() {
    let f = Fixture::new(48000, 2, 400);
    let plain = f.song();
    let baseline = render(&plain, 37);
    let mut song = plain.clone();
    add_track(&mut song, 2);
    song.instruments.insert(
        InstrumentId(2),
        Instrument {
            name: "Unused legacy path".into(),
            kind: InstrumentKind::Pitched,
            sample: Some(PathBuf::from(format!("/missing/{}", "x".repeat(8193)))),
            region: None,
        },
    );
    song.track_mix.insert(
        TrackId(2),
        TrackMix {
            gain: 0.5,
            ..Default::default()
        },
    );
    song.validate().unwrap();
    let engine = Engine::new(&song, None).unwrap();
    assert!(engine.mix_layout().is_none());
    assert!(engine.mix_unavailable().unwrap().contains("8192"));
    assert!(engine.manifest.track_mix.is_empty());
    assert_eq!(bits(&baseline), bits(&render(&song, 512)));
    // Actually gating the sounding track is nonneutral and needs a supported layout.
    song.track_mix.get_mut(&TrackId(2)).unwrap().solo = true;
    assert!(Engine::new(&song, None).is_err());
}

#[test]
fn output_mute_advances_voices_and_unmute_retains_existing_clipping_failure() {
    let f = Fixture::new(48000, 1, 4096);
    let mut song = f.song();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 100;
    let row = song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap();
    let on = row[&0].clone();
    for column in 1..100 {
        row.insert(column, on.clone());
    }
    let mut clipped = Engine::new(&song, None).unwrap();
    assert!(clipped
        .block(&mut [0.0; 1024], 512)
        .unwrap_err()
        .contains("clipping"));
    song.track_mix.insert(
        TrackId(1),
        TrackMix {
            mute: true,
            ..Default::default()
        },
    );
    let mut muted = Engine::new(&song, None).unwrap();
    assert_eq!(muted.mix_layout().unwrap().tracks(), &[TrackId(1)]);
    assert!(read(&mut muted, 64, 37).iter().all(|&v| v == 0.0));
    let update = muted
        .mix_layout()
        .unwrap()
        .snapshot(&BTreeMap::new())
        .unwrap();
    muted.apply_mix_snapshot(&update).unwrap();
    assert!(muted
        .block(&mut [0.0; 1024], 576)
        .unwrap_err()
        .contains("clipping"));
}

#[test]
fn unchanged_target_continues_existing_ramp_despite_new_full_snapshot() {
    let f = Fixture::new(48000, 2, 4096);
    let mut song = f.song();
    add_track(&mut song, 2);
    let baseline = render(&song, 37);
    let mut engine = Engine::new(&song, None).unwrap();
    let layout = engine.mix_layout().unwrap().clone();
    let mut settings = BTreeMap::from([(
        TrackId(1),
        TrackMix {
            gain: 0.0,
            ..Default::default()
        },
    )]);
    engine
        .apply_mix_snapshot(&layout.snapshot(&settings).unwrap())
        .unwrap();
    let mut out = read(&mut engine, 33, 37);
    // This is a genuinely changed full setting map, but track1 target is unchanged.
    settings.insert(
        TrackId(2),
        TrackMix {
            gain: 0.5,
            ..Default::default()
        },
    );
    engine
        .apply_mix_snapshot(&layout.snapshot(&settings).unwrap())
        .unwrap();
    out.extend(read(&mut engine, 128, 512));
    for frame in 0..161 {
        for channel in 0..2 {
            let expected = 0.0 + baseline[frame * 2 + channel] * ramp(1.0, 0.0, frame + 1);
            assert_eq!(out[frame * 2 + channel].to_bits(), expected.to_bits());
        }
    }
}

#[cfg(unix)]
#[test]
fn resample_bakes_mix_and_neutral_replay_does_not_apply_it_twice() {
    use aqua_pattern_core::resample::{self, ResampleConfig, TailPolicy};

    let f = Fixture::new(48_000, 2, 400);
    let source_bytes = std::fs::read(&f.asset.path).unwrap();
    let mut song = f.song();
    song.set_track_mix(config(&[(1, 0.5, -0.25, false, false)]))
        .unwrap();
    let before = bytes(&song);
    let baseline = render(&song, 37);
    let plan = resample::prepare(
        &song,
        ResampleConfig {
            version: 1,
            occurrence: 0,
            name: "Printed track balance".into(),
            root_note: 60,
            tail: TailPolicy::MusicalOnly,
            output: f.dir.join("printed.wav"),
        },
    )
    .unwrap();
    let warning = plan.summary().warnings.join(" ");
    assert!(warning.contains("already includes the current track mix"));
    assert!(warning.contains("neutral effective track mix"));
    assert!(warning.contains("held through the entire printed region"));
    let printed = resample::render(&plan, |_| Ok(()), || false).unwrap();
    assert_eq!(printed.asset().frames, 96_000);
    assert_eq!(printed.receipt().summary.warnings, plan.summary().warnings);
    let mut replay = audition::region_preview(
        printed.asset(),
        &SampleRegion {
            name: "Native neutral replay".into(),
            sample: SampleId(1),
            start_frame: 0,
            end_frame: printed.asset().frames,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        },
    )
    .unwrap()
    .song()
    .clone();
    assert!(replay.track_mix.is_empty());
    assert_eq!(bits(&render(&replay, 512)), bits(&baseline));

    // The print has already baked these coefficients. Applying the original
    // settings on replay deliberately attenuates/balances it a second time.
    replay.track_mix = song.track_mix.clone();
    let twice = render(&replay, 1);
    assert_ne!(bits(&twice), bits(&baseline));
    for (frame, expected) in baseline.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(
            twice[frame * 2].to_bits(),
            (0.0 + expected[0] * 0.5).to_bits()
        );
        assert_eq!(
            twice[frame * 2 + 1].to_bits(),
            (0.0 + expected[1] * 0.375).to_bits()
        );
    }
    assert_eq!(bytes(&song), before);
    assert_eq!(std::fs::read(&f.asset.path).unwrap(), source_bytes);
}
