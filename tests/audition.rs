use aqua_pattern_core::{
    audition::{self, AuditionPreview, AuditionTarget},
    sample_asset,
    song::*,
    song_render::{Command, Engine, Release},
    transport::{Producer, Shared},
    RATE,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    asset: SampleAsset,
    region: SampleRegion,
}
fn value(frame: usize, channel: usize) -> f32 {
    (0.125 + (frame % 31) as f32 * 0.001) * if channel == 0 { 1.0 } else { -0.5 }
}
impl Fixture {
    fn new(rate: u32, channels: u16) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-audition-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("authored.wav");
        let mut writer = hound::WavWriter::create(
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
            for channel in 0..channels {
                writer.write_sample(value(frame, channel as usize)).unwrap();
            }
        }
        writer.finalize().unwrap();
        let asset = sample_asset::import(&path).unwrap().asset;
        Self {
            dir,
            asset,
            region: SampleRegion {
                name: "Unsaved draft".into(),
                sample: SampleId(999),
                start_frame: 7,
                end_frame: 1031,
                root_note: 12,
                amp_envelope: None,
                forward_loop: None,
            },
        }
    }
    fn song(&self) -> Song {
        let mut song = example_song();
        song.samples.insert(SampleId(11), self.asset.clone());
        let mut region = self.region.clone();
        region.sample = SampleId(11);
        song.regions.insert(RegionId(21), region);
        song.instruments.insert(
            InstrumentId(41),
            Instrument {
                name: "Selected kit".into(),
                kind: InstrumentKind::DrumKit { kit: DrumKitId(51) },
                sample: None,
                region: None,
            },
        );
        song.drum_kits.insert(
            DrumKitId(51),
            DrumKitInstrument {
                name: "Kit".into(),
                pads: BTreeMap::from([(
                    PadId(61),
                    DrumPad {
                        slot: 7,
                        name: "Selected pad".into(),
                        note: 40,
                        region: Some(RegionId(21)),
                        velocity_zones: vec![],
                        root_note: 60,
                        tune_semitones: 12,
                        gain: 0.5,
                        pan: -0.25,
                        mode: PadMode::OneShot,
                        new_note: PadNewNote::ReleasePrevious,
                        choke_group: Some(3),
                    },
                )]),
            },
        );
        song
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn pcm(preview: &AuditionPreview, block: usize) -> Vec<f32> {
    let mut engine = Engine::new(preview.song(), None).unwrap();
    let mut result = Vec::new();
    let mut buffer = vec![0.0; block * 2];
    while engine.position < audition::TOTAL_FRAMES {
        let count = engine.block(&mut buffer, audition::TOTAL_FRAMES).unwrap();
        result.extend_from_slice(&buffer[..count * 2]);
    }
    result
}

#[test]
fn draft_prepare_is_pure_fixed_bounded_native_and_deterministic() {
    let fixture = Fixture::new(44100, 2);
    let mut asset = fixture.asset.clone();
    asset.path = fixture.dir.join("missing.wav");
    let before = serde_json::to_vec(&(&asset, &fixture.region)).unwrap();
    let first = audition::region_preview(&asset, &fixture.region).unwrap();
    let second = audition::region_preview(&asset, &fixture.region).unwrap();
    assert_eq!(first.id(), second.id());
    assert_eq!(first.summary(), second.summary());
    assert_eq!(
        before,
        serde_json::to_vec(&(&asset, &fixture.region)).unwrap()
    );
    assert_eq!(first.summary().target, AuditionTarget::RegionDraft);
    assert_eq!(first.summary().sounding_note, 12);
    assert_eq!(first.summary().region.sample, SampleId(1));
    assert_eq!(first.summary().region.start_frame, 7);
    assert_eq!(first.summary().region.end_frame, 1031);
    assert_eq!(first.summary().musical_frames, 96000);
    assert_eq!(first.summary().tail_frames, 24000);
    assert_eq!(first.summary().total_frames, 120000);
    assert_eq!(first.summary().input_velocity, 1.0);
    assert_eq!(first.summary().velocity_bin, 127);
    assert!(!first.summary().source_identity_verified);
    assert!(Engine::new(first.song(), None).is_err());
}

#[test]
fn invalid_draft_bounds_root_loop_envelope_and_relative_path_refuse() {
    let f = Fixture::new(48000, 1);
    for change in 0..6 {
        let mut region = f.region.clone();
        match change {
            0 => region.end_frame = region.start_frame,
            1 => region.end_frame = f.asset.frames + 1,
            2 => region.root_note = 128,
            3 => {
                region.amp_envelope = Some(AmpEnvelope {
                    release_ms: 501,
                    ..Default::default()
                })
            }
            4 => {
                region.forward_loop = Some(ForwardLoop {
                    start_frame: 7,
                    end_frame: 8,
                })
            }
            _ => {
                region.forward_loop = Some(ForwardLoop {
                    start_frame: 6,
                    end_frame: 30,
                })
            }
        }
        assert!(audition::region_preview(&f.asset, &region).is_err());
    }
    let mut asset = f.asset.clone();
    asset.path = "relative.wav".into();
    assert!(audition::region_preview(&asset, &f.region)
        .unwrap_err()
        .contains("absolute"));
}

#[test]
fn native_region_samples_match_independent_linear_rate_oracle_and_roots() {
    for (rate, channels) in [(8000, 1), (44100, 2), (48000, 2), (96000, 1), (384000, 2)] {
        let f = Fixture::new(rate, channels);
        let mut first = None;
        for root in [0, 12, 60, 127] {
            let mut region = f.region.clone();
            region.root_note = root;
            let preview = audition::region_preview(&f.asset, &region).unwrap();
            let actual = pcm(&preview, 37);
            for output_frame in 0..((1023_u64 * u64::from(RATE)) / u64::from(rate)) as usize {
                let phase = 7.0 + output_frame as f64 * f64::from(rate) / f64::from(RATE);
                let source_frame = phase.floor() as usize;
                let fraction = (phase - source_frame as f64) as f32;
                for channel in 0..2 {
                    let c = if channels == 1 { 0 } else { channel };
                    let a = value(source_frame, c);
                    let b = value(source_frame + 1, c);
                    let expected = (a + (b - a) * fraction) * 0.5;
                    assert!(
                        (actual[output_frame * 2 + channel] - expected).abs() < 2e-8,
                        "rate {rate} root {root} frame {output_frame}"
                    );
                }
            }
            if let Some(previous) = &first {
                assert_eq!(previous, &actual);
            } else {
                first = Some(actual);
            }
        }
    }
}

#[test]
fn draft_loop_adsr_uses_ordinary_song_end_and_finite_tail() {
    let f = Fixture::new(48000, 2);
    let mut region = f.region.clone();
    region.forward_loop = Some(ForwardLoop {
        start_frame: 17,
        end_frame: 101,
    });
    region.amp_envelope = Some(AmpEnvelope {
        attack_ms: 10,
        decay_ms: 20,
        sustain_percent: 50,
        release_ms: 500,
    });
    let preview = audition::region_preview(&f.asset, &region).unwrap();
    let engine = Engine::new(preview.song(), None).unwrap();
    assert!(matches!(
        engine.manifest.events[1].command,
        Command::Off {
            reason: Release::SongEnd,
            ..
        }
    ));
    assert_eq!(engine.manifest.events[1].frame, 96000);
    let reference = pcm(&preview, 512);
    assert_eq!(reference.len(), 240000);
    assert_eq!(reference[0], 0.0);
    assert!(reference[96000 * 2].abs() > 0.01);
    assert!(reference[239998].abs() < 0.00001);
    for block in [1, 37, 128, 256] {
        assert_eq!(reference, pcm(&preview, block));
    }
}

#[test]
fn selected_committed_pad_preserves_mapping_and_exact_resolver_provenance() {
    let f = Fixture::new(44100, 2);
    let mut song = f.song();
    let mut unused = f.asset.clone();
    unused.path = f.dir.join("unused-missing.wav");
    song.samples.insert(SampleId(12), unused);
    let mut unused_region = f.region.clone();
    unused_region.sample = SampleId(12);
    song.regions.insert(RegionId(22), unused_region);
    let pad = song
        .drum_kits
        .get_mut(&DrumKitId(51))
        .unwrap()
        .pads
        .get_mut(&PadId(61))
        .unwrap();
    pad.region = None;
    pad.velocity_zones = vec![
        VelocityZone {
            min_velocity: 0,
            max_velocity: 126,
            alternatives: vec![PadLayer {
                id: PadLayerId(99),
                region: RegionId(22),
            }],
        },
        VelocityZone {
            min_velocity: 127,
            max_velocity: 127,
            alternatives: vec![
                PadLayer {
                    id: PadLayerId(17),
                    region: RegionId(21),
                },
                PadLayer {
                    id: PadLayerId(3),
                    region: RegionId(22),
                },
            ],
        },
    ];
    let exact_pad = pad.clone();
    let before = serde_json::to_vec(&song).unwrap();
    let preview = audition::pad_preview(&song, InstrumentId(41), PadId(61)).unwrap();
    assert_eq!(before, serde_json::to_vec(&song).unwrap());
    assert_eq!(
        preview.song().drum_kits[&DrumKitId(51)].pads[&PadId(61)],
        exact_pad
    );
    let summary = preview.summary();
    assert_eq!(summary.region.region, RegionId(21));
    assert_eq!(summary.region.root_note, 60);
    assert_eq!(summary.sounding_note, 72);
    assert_eq!(summary.on_gain, 0.5);
    assert_eq!(summary.pan, -0.25);
    let layer = summary.layer_selection.as_ref().unwrap();
    assert_eq!(layer.layer, PadLayerId(17));
    assert_eq!(
        (
            layer.round_robin_index,
            layer.round_robin_ordinal,
            layer.velocity_bin
        ),
        (0, 0, 127)
    );
    assert_eq!((summary.retained_regions, summary.retained_sources), (2, 2));
    let actual = pcm(&preview, 512); // missing unused alternatives are not accessed
    assert!(actual.iter().any(|v| v.abs() > 0.001));
    assert_eq!(actual, pcm(&preview, 37));
    assert_eq!(
        preview.id(),
        audition::pad_preview(&song, InstrumentId(41), PadId(61))
            .unwrap()
            .id()
    );
}

#[test]
fn unrelated_invalid_arrangement_or_legacy_asset_is_not_copied_or_loaded() {
    let f = Fixture::new(48000, 1);
    let mut song = f.song();
    song.order.clear();
    song.patterns.clear();
    song.bpm = f64::NAN;
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some(f.dir.join("unrelated-missing.wav"));
    let kit = song.drum_kits.get_mut(&DrumKitId(51)).unwrap();
    let mut neighbouring_pad = kit.pads[&PadId(61)].clone();
    neighbouring_pad.region = Some(RegionId(999));
    neighbouring_pad.note = 41;
    neighbouring_pad.slot = 8;
    kit.pads.insert(PadId(62), neighbouring_pad);
    let preview = audition::pad_preview(&song, InstrumentId(41), PadId(61)).unwrap();
    assert_eq!(preview.song().instruments.len(), 1);
    assert_eq!(preview.song().samples.len(), 1);
    assert_eq!(preview.song().drum_kits[&DrumKitId(51)].pads.len(), 1);
    assert!(Engine::new(preview.song(), None).is_ok());
    assert!(song.bpm.is_nan());
}

#[test]
fn malformed_selected_mapping_and_unused_references_refuse_before_io() {
    let f = Fixture::new(48000, 1);
    for change in 0..7 {
        let mut song = f.song();
        let pad = song
            .drum_kits
            .get_mut(&DrumKitId(51))
            .unwrap()
            .pads
            .get_mut(&PadId(61))
            .unwrap();
        match change {
            0 => pad.region = None,
            1 => pad.region = Some(RegionId(999)),
            2 => pad.gain = f32::NAN,
            3 => pad.root_note = 127,
            4 => {
                pad.region = None;
                pad.velocity_zones = vec![VelocityZone {
                    min_velocity: 0,
                    max_velocity: 127,
                    alternatives: vec![
                        PadLayer {
                            id: PadLayerId(1),
                            region: RegionId(21),
                        },
                        PadLayer {
                            id: PadLayerId(2),
                            region: RegionId(999),
                        },
                    ],
                }];
            }
            5 => {
                pad.region = None;
                pad.velocity_zones = vec![VelocityZone {
                    min_velocity: 0,
                    max_velocity: 127,
                    alternatives: vec![
                        PadLayer {
                            id: PadLayerId(1),
                            region: RegionId(21)
                        };
                        9
                    ],
                }];
            }
            _ => {
                pad.region = None;
                pad.velocity_zones = vec![
                    VelocityZone {
                        min_velocity: 0,
                        max_velocity: 127,
                        alternatives: vec![PadLayer {
                            id: PadLayerId(1),
                            region: RegionId(21)
                        }]
                    };
                    9
                ];
            }
        }
        assert!(
            audition::pad_preview(&song, InstrumentId(41), PadId(61)).is_err(),
            "change {change}"
        );
    }
    assert!(audition::pad_preview(&f.song(), InstrumentId(1), PadId(61)).is_err());
    assert!(audition::pad_preview(&f.song(), InstrumentId(41), PadId(999)).is_err());
}

#[test]
fn chosen_source_byte_or_metadata_changes_fail_on_worker_not_prepare() {
    let f = Fixture::new(48000, 1);
    let preview = audition::region_preview(&f.asset, &f.region).unwrap();
    let bytes = std::fs::read(&f.asset.path).unwrap();
    std::fs::write(&f.asset.path, b"tampered").unwrap();
    assert!(audition::region_preview(&f.asset, &f.region).is_ok());
    assert!(Engine::new(preview.song(), None).is_err());
    std::fs::write(&f.asset.path, &bytes).unwrap();
    assert!(Engine::new(preview.song(), None).is_ok());
    let mut wrong = f.asset.clone();
    wrong.sample_rate = 44100;
    let preview = audition::region_preview(&wrong, &f.region).unwrap();
    assert!(Engine::new(preview.song(), None).is_err());
    assert_eq!(bytes, std::fs::read(&f.asset.path).unwrap());
}

#[test]
fn existing_producer_callback_matches_offline_and_stop_is_terminal() {
    let f = Fixture::new(44100, 2);
    let mut region = f.region.clone();
    region.forward_loop = Some(ForwardLoop {
        start_frame: 7,
        end_frame: 1031,
    });
    let preview = audition::region_preview(&f.asset, &region).unwrap();
    let reference = pcm(&preview, 512);
    for sizes in [vec![128], vec![256], vec![512], vec![1, 37, 127, 513]] {
        let shared = Arc::new(Shared::default());
        let mut producer = Producer::new(preview.song(), None, shared.clone()).unwrap();
        let mut result = Vec::new();
        let mut calls = 0;
        while shared.is_running() {
            producer.pump_until_buffered().unwrap();
            let mut output = vec![0.0; sizes[calls % sizes.len()] * 2];
            shared.fill(&mut output);
            result.extend(output);
            calls += 1;
            assert!(calls < 10000);
        }
        assert_eq!(reference, result[..reference.len()]);
        assert!(result[reference.len()..].iter().all(|v| *v == 0.0));
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
        shared.stop();
        shared.seek(0);
        let mut output = [1.0; 257];
        shared.fill(&mut output);
        assert_eq!(output, [0.0; 257]);
    }
    let shared = Arc::new(Shared::default());
    shared.stop(); // cancellation while worker still loading cannot publish readiness
    let mut producer = Producer::new(preview.song(), None, shared.clone()).unwrap();
    producer.pump_until_buffered().unwrap();
    let mut output = [1.0; 1024];
    shared.fill(&mut output);
    assert_eq!(output, [0.0; 1024]);
    assert!(!shared.is_ready());
}

#[test]
fn both_pad_modes_keep_song_end_not_a_held_input_off() {
    let f = Fixture::new(48000, 1);
    let mut song = f.song();
    song.regions.get_mut(&RegionId(21)).unwrap().forward_loop = Some(ForwardLoop {
        start_frame: 7,
        end_frame: 1031,
    });
    for mode in [PadMode::OneShot, PadMode::Gated] {
        song.drum_kits
            .get_mut(&DrumKitId(51))
            .unwrap()
            .pads
            .get_mut(&PadId(61))
            .unwrap()
            .mode = mode;
        let preview = audition::pad_preview(&song, InstrumentId(41), PadId(61)).unwrap();
        let engine = Engine::new(preview.song(), None).unwrap();
        assert_eq!(engine.manifest.events.len(), 2);
        assert_eq!(engine.manifest.events[0].frame, 0);
        assert_eq!(engine.manifest.events[1].frame, 96000);
        assert!(matches!(
            engine.manifest.events[1].command,
            Command::Off {
                reason: Release::SongEnd,
                ..
            }
        ));
    }
}

#[test]
fn local_metadata_path_and_name_boundaries_are_explicit() {
    let f = Fixture::new(48000, 1);
    let mut source = f.asset.clone();
    let mut boundary_path = std::env::temp_dir();
    let remaining = audition::MAX_PATH_BYTES - boundary_path.as_os_str().len();
    boundary_path.push("x".repeat(remaining));
    assert_eq!(boundary_path.as_os_str().len(), audition::MAX_PATH_BYTES);
    source.path = boundary_path;
    let mut region = f.region.clone();
    region.name = "x".repeat(256);
    assert!(audition::region_preview(&source, &region).is_ok());
    source.path.push("x");
    assert!(audition::region_preview(&source, &region)
        .unwrap_err()
        .contains("8192"));
    region.name.push('x');
    assert!(audition::region_preview(&f.asset, &region).is_err());
}

#[test]
fn full_32_alternative_metadata_is_bounded_before_evaluation_or_decode() {
    let f = Fixture::new(48000, 1);
    let mut song = f.song();
    let mut zones = Vec::new();
    for zone in 0..4_u8 {
        let mut alternatives = Vec::new();
        for index in 0..8_u64 {
            let id = u64::from(zone) * 8 + index + 1;
            let mut asset = f.asset.clone();
            asset.path = f.dir.join(format!("missing-{id}.wav"));
            let mut region = f.region.clone();
            region.sample = SampleId(id);
            song.samples.insert(SampleId(id), asset);
            song.regions.insert(RegionId(id), region);
            alternatives.push(PadLayer {
                id: PadLayerId(id),
                region: RegionId(id),
            });
        }
        zones.push(VelocityZone {
            min_velocity: zone * 32,
            max_velocity: zone * 32 + 31,
            alternatives,
        });
    }
    let pad = song
        .drum_kits
        .get_mut(&DrumKitId(51))
        .unwrap()
        .pads
        .get_mut(&PadId(61))
        .unwrap();
    pad.region = None;
    pad.velocity_zones = zones;
    let preview = audition::pad_preview(&song, InstrumentId(41), PadId(61)).unwrap();
    assert_eq!(
        (
            preview.summary().retained_regions,
            preview.summary().retained_sources
        ),
        (32, 32)
    );
    assert_eq!(
        preview.summary().layer_selection.as_ref().unwrap().layer,
        PadLayerId(25)
    );
    for asset in song.samples.values_mut() {
        asset.path = format!("/{}", "x".repeat(audition::MAX_PATH_BYTES - 1)).into();
    }
    assert!(audition::pad_preview(&song, InstrumentId(41), PadId(61))
        .unwrap_err()
        .contains("256 KiB"));
}

#[test]
fn zero_gain_pad_is_a_real_resolved_on_and_unused_relative_path_is_not_opened() {
    let f = Fixture::new(48000, 1);
    let mut song = f.song();
    let mut unused_asset = f.asset.clone();
    unused_asset.path = "not-opened-relative.wav".into();
    song.samples.insert(SampleId(12), unused_asset);
    let mut unused_region = f.region.clone();
    unused_region.sample = SampleId(12);
    song.regions.insert(RegionId(22), unused_region);
    let pad = song
        .drum_kits
        .get_mut(&DrumKitId(51))
        .unwrap()
        .pads
        .get_mut(&PadId(61))
        .unwrap();
    pad.gain = 0.0;
    pad.region = None;
    pad.velocity_zones = vec![VelocityZone {
        min_velocity: 0,
        max_velocity: 127,
        alternatives: vec![
            PadLayer {
                id: PadLayerId(1),
                region: RegionId(21),
            },
            PadLayer {
                id: PadLayerId(2),
                region: RegionId(22),
            },
        ],
    }];
    let preview = audition::pad_preview(&song, InstrumentId(41), PadId(61)).unwrap();
    assert_eq!(preview.summary().input_velocity, 1.0);
    assert_eq!(preview.summary().on_gain, 0.0);
    assert_eq!(
        preview
            .summary()
            .layer_selection
            .as_ref()
            .unwrap()
            .round_robin_ordinal,
        0
    );
    assert!(pcm(&preview, 512).iter().all(|v| *v == 0.0));
}
