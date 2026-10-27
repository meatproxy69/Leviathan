use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine, Release},
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
    song: Song,
    rate: u32,
    frames: u32,
}
fn signal(pad: usize, frame: usize, channel: usize) -> f32 {
    (0.02 + pad as f32 * 0.001 + (frame % 997) as f32 * 0.00001)
        * if channel == 0 { 1.0 } else { -0.7 }
}
impl Fixture {
    fn new(count: usize, rate: u32, frames: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-drums-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut song = example_song();
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "Kit pattern".into(),
                rows: 8,
                tracks: BTreeMap::new(),
            },
        )]);
        song.order = vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }];
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 16;
        let mut pads = BTreeMap::new();
        for pad in 0..count {
            let id = pad as u64 + 1;
            let path = dir.join(format!("pad-{pad}.wav"));
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
            for f in 0..frames {
                for c in 0..2 {
                    writer.write_sample(signal(pad, f as usize, c)).unwrap();
                }
            }
            writer.finalize().unwrap();
            song.register_sample(SampleId(id), sample_asset::import(&path).unwrap().asset)
                .unwrap();
            song.upsert_region(
                RegionId(id),
                SampleRegion {
                    amp_envelope: None,
                    forward_loop: None,
                    name: format!("Source {pad}"),
                    sample: SampleId(id),
                    start_frame: 0,
                    end_frame: u64::from(frames),
                    root_note: 72,
                },
            )
            .unwrap();
            pads.insert(
                PadId(id),
                DrumPad {
                    velocity_zones: Vec::new(),
                    slot: pad as u8,
                    name: match pad {
                        0 => "Kick".into(),
                        1 => "Snare".into(),
                        2 => "Closed hat".into(),
                        3 => "Open hat".into(),
                        _ => format!("Pad {pad}"),
                    },
                    note: 36 + pad as u8,
                    region: Some(RegionId(id)),
                    root_note: 60,
                    gain: 1.0,
                    pan: 0.0,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: None,
                    tune_semitones: 0,
                },
            );
        }
        song.upsert_drum_kit(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Sixteen independent samples".into(),
                pads,
            },
        )
        .unwrap();
        song.assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
        Self {
            dir,
            song,
            rate,
            frames,
        }
    }
    fn pad(&mut self, index: u64) -> &mut DrumPad {
        self.song
            .drum_kits
            .get_mut(&DrumKitId(1))
            .unwrap()
            .pads
            .get_mut(&PadId(index))
            .unwrap()
    }
    fn put(&mut self, row: u32, column: u16, cell: Cell) {
        self.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(row)
            .or_default()
            .insert(column, cell);
    }
    fn render(&self, block: usize) -> Vec<f32> {
        render(Engine::new(&self.song, None).unwrap(), block)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn note(index: u8) -> Cell {
    Cell {
        note: Note::Note(36 + index),
        instrument: Some(InstrumentId(1)),
        ..Default::default()
    }
}
fn render(mut engine: Engine, block: usize) -> Vec<f32> {
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut data = vec![0.0; block * 2];
    let mut pcm = Vec::new();
    while engine.position < end {
        let n = engine.block(&mut data, end).unwrap();
        pcm.extend_from_slice(&data[..n * 2]);
    }
    pcm
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-7, "actual {a} expected {b}");
}
fn manifest(f: &Fixture) -> song_render::Manifest {
    song_render::evaluate(&f.song, None).unwrap()
}
#[test]
fn sixteen_distinct_sample_pads_resolve_by_selector_not_pitch_and_overlap() {
    let mut f = Fixture::new(16, RATE, 128);
    for i in 0..16 {
        f.put(0, i, note(i as u8));
    }
    let m = manifest(&f);
    assert_eq!(m.schema, 6);
    assert_eq!(m.pad_voices.len(), 16);
    assert!(m.assets.is_empty());
    for (voice, pad) in &m.pad_voices {
        assert_eq!(pad.pad, PadId(*voice as u64));
        assert_eq!(pad.note, 35 + *voice as u8);
        assert_eq!(pad.region.root_note, 60);
    }
    assert!(m
        .events
        .iter()
        .filter_map(|e| if let Command::On { pitch, .. } = e.command {
            Some(pitch)
        } else {
            None
        })
        .all(|p| p == 60));
    let pcm = f.render(512);
    for frame in 0..128 {
        for channel in 0..2 {
            let expected = (0..16)
                .map(|pad| signal(pad, frame, channel) * 0.5)
                .sum::<f32>();
            close(pcm[frame * 2 + channel], expected);
        }
    }
    assert!(pcm[256..].iter().all(|v| *v == 0.0));
}
#[test]
fn native_rates_and_explicit_octave_tune_have_exact_source_phase_and_partition_invariance() {
    for rate in [24000, 44100, 96000] {
        for tune in [-48, -12, 0, 12, 48] {
            let mut f = Fixture::new(1, rate, 294);
            f.pad(1).root_note = 72;
            f.pad(1).tune_semitones = tune;
            f.put(0, 0, note(0));
            let pcm = f.render(512);
            assert_eq!(pcm, f.render(37));
            assert_eq!(pcm, f.render(1));
            let speed = 2.0f64.powf(f64::from(tune) / 12.0);
            let duration = (f64::from(f.frames) * f64::from(RATE) / (f64::from(f.rate) * speed))
                .ceil() as usize;
            for frame in 0..duration {
                let p = frame as f64 * speed * f64::from(rate) / f64::from(RATE);
                let low = p.floor() as usize;
                let t = (p - low as f64) as f32;
                for c in 0..2 {
                    let a = signal(0, low, c);
                    let b = if low + 1 < 294 {
                        signal(0, low + 1, c)
                    } else {
                        0.0
                    };
                    close(pcm[frame * 2 + c], (a + (b - a) * t) * 0.5);
                }
            }
            assert!(pcm[duration * 2..].iter().all(|v| *v == 0.0));
        }
    }
}
#[test]
fn pad_gain_pan_apply_immediately_and_compose_again_without_memory_compounding() {
    let mut f = Fixture::new(1, RATE, RATE);
    f.pad(1).gain = 0.25;
    f.pad(1).pan = 0.75;
    f.put(
        0,
        0,
        Cell {
            velocity: Some(0.5),
            pan: Some(0.25),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.25,
                }),
                ..Default::default()
            },
            ..note(0)
        },
    );
    f.put(
        1,
        0,
        Cell {
            velocity: Some(0.8),
            pan: Some(-0.5),
            ..Default::default()
        },
    );
    let m = manifest(&f);
    let ons: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On { volume, pan, .. } = e.command {
                Some((volume, pan))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ons, vec![(0.125, 1.0); 3]);
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command,Command::Volume{value,..} if value==0.2)));
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command,Command::Pan{value,..} if value==0.25)));
    let pcm = f.render(512);
    close(pcm[0], 0.0);
    close(
        pcm[1],
        signal(0, 0, 1) * 0.125 * 0.5 * std::f32::consts::SQRT_2,
    );
}
#[test]
fn one_shot_off_releases_logical_pointer_but_not_audio_gated_off_declicks() {
    for mode in [PadMode::OneShot, PadMode::Gated] {
        let mut f = Fixture::new(1, RATE, RATE);
        f.pad(1).mode = mode;
        f.put(0, 0, note(0));
        f.put(
            1,
            0,
            Cell {
                note: Note::Off,
                ..Default::default()
            },
        );
        f.put(
            2,
            0,
            Cell {
                velocity: Some(0.0),
                ..Default::default()
            },
        );
        let m = manifest(&f);
        assert!(!m
            .events
            .iter()
            .any(|e| matches!(e.command, Command::Volume { .. })));
        let pcm = f.render(512);
        if mode == PadMode::OneShot {
            close(pcm[6000 * 2], signal(0, 6000, 0) * 0.5);
            assert!(m.events.iter().any(|e| matches!(
                e.command,
                Command::Off {
                    reason: Release::SongEnd,
                    ..
                }
            )));
        } else {
            assert!(pcm[6000 * 2] > 0.0);
            assert!(pcm[12000 * 2].abs() < 1e-5);
        }
    }
}
#[test]
fn different_pads_same_column_overlap_and_same_pad_new_note_policy_is_explicit() {
    for policy in [PadNewNote::Overlap, PadNewNote::ReleasePrevious] {
        let mut f = Fixture::new(2, RATE, RATE);
        f.pad(1).new_note = policy;
        f.put(0, 0, note(0));
        f.put(1, 0, note(1));
        f.put(2, 0, note(0));
        let m = manifest(&f);
        assert!(!m.events.iter().any(|e| matches!(
            e.command,
            Command::Off {
                reason: Release::Replaced,
                ..
            }
        )));
        let replacements: Vec<_> = m
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.command,
                    Command::Off {
                        reason: Release::PadReplaced,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            replacements.len(),
            usize::from(policy == PadNewNote::ReleasePrevious)
        );
        let pcm = f.render(512);
        close(pcm[6000 * 2], (signal(0, 6000, 0) + signal(1, 0, 0)) * 0.5);
        if policy == PadNewNote::Overlap {
            close(
                pcm[12000 * 2],
                (signal(0, 12000, 0) + signal(1, 6000, 0) + signal(0, 0, 0)) * 0.5,
            );
        }
    }
}
#[test]
fn choke_is_pre_on_declick_group_scoped_and_zero_gain_trigger_still_chokes() {
    let mut f = Fixture::new(4, RATE, RATE);
    f.pad(3).choke_group = Some(1);
    f.pad(4).choke_group = Some(1);
    f.pad(3).gain = 0.0;
    f.song
        .assign_kit(InstrumentId(2), Some(DrumKitId(1)))
        .unwrap();
    f.put(0, 0, note(3));
    f.put(0, 1, note(0));
    f.put(
        0,
        2,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..note(3)
        },
    );
    f.put(1, 0, note(2));
    f.put(
        2,
        0,
        Cell {
            velocity: Some(1.0),
            ..Default::default()
        },
    );
    let m = manifest(&f);
    let at: Vec<_> = m.events.iter().filter(|e| e.frame == 6000).collect();
    assert!(matches!(
        at[0].command,
        Command::Off {
            voice: 1,
            reason: Release::Choke
        }
    ));
    assert!(matches!(at[1].command, Command::On { volume: 0.0, .. }));
    assert_eq!(
        m.events
            .iter()
            .filter(|e| matches!(
                e.command,
                Command::Off {
                    reason: Release::Choke,
                    ..
                }
            ))
            .count(),
        1
    );
    let pcm = f.render(512);
    let reference = (signal(0, 6000, 0) + signal(3, 6000, 0)) * 0.5;
    assert!(pcm[6000 * 2] > reference);
    assert!(pcm[6000 * 2] < reference + signal(3, 6000, 0) * 0.5); // old group voice fades, not hard zero
    close(
        pcm[12000 * 2],
        (signal(0, 12000, 0) + signal(3, 12000, 0)) * 0.5,
    ); // no instrument2 choke, zero-gain pad cannot be raised
}
fn phrase(f: &mut Fixture, mode: PadMode) {
    f.pad(1).mode = mode;
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Kit phrase".into(),
            instrument: InstrumentId(1),
            rows: 2,
            lines_per_beat: 8,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([
                (
                    0,
                    BTreeMap::from([(
                        0,
                        Cell {
                            instrument: None,
                            ..note(0)
                        },
                    )]),
                ),
                (
                    1,
                    BTreeMap::from([(
                        0,
                        Cell {
                            instrument: None,
                            ..note(0)
                        },
                    )]),
                ),
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
                cycles: 1,
            }),
            ..Default::default()
        },
    );
}
#[test]
fn phrase_end_and_parent_timed_release_cover_all_owned_overlapping_pad_voices() {
    for mode in [PadMode::OneShot, PadMode::Gated] {
        let mut f = Fixture::new(1, RATE, RATE);
        phrase(&mut f, mode);
        let m = manifest(&f);
        let ends = m
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.command,
                    Command::Off {
                        reason: Release::PhraseEnd,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(ends, if mode == PadMode::Gated { 2 } else { 0 });
        let pcm = f.render(512);
        if mode == PadMode::Gated {
            assert!(pcm[12000 * 2].abs() < 1e-5);
        } else {
            close(
                pcm[12000 * 2],
                (signal(0, 12000, 0) + signal(0, 9000, 0)) * 0.5,
            );
        }
    }
    let mut f = Fixture::new(1, RATE, RATE);
    phrase(&mut f, PadMode::OneShot);
    f.song.phrases.get_mut(&PhraseId(1)).unwrap().rows = 4;
    f.song
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
        .release = Some(0.75);
    let m = manifest(&f);
    assert_eq!(
        m.events
            .iter()
            .filter(|e| e.frame == 4500
                && matches!(
                    e.command,
                    Command::Off {
                        reason: Release::TimedRelease,
                        ..
                    }
                ))
            .count(),
        2
    );
    assert!(f.render(512)[12000 * 2].abs() < 1e-5);
}
#[test]
fn parent_hardcut_kills_all_phrase_owned_kit_tails_but_not_other_instrument() {
    let mut f = Fixture::new(1, RATE, RATE);
    phrase(&mut f, PadMode::OneShot);
    f.pad(1).new_note = PadNewNote::ReleasePrevious;
    f.song
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
        .cut = Some(0.75);
    f.song
        .assign_kit(InstrumentId(2), Some(DrumKitId(1)))
        .unwrap();
    f.put(
        0,
        1,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..note(0)
        },
    );
    let m = manifest(&f);
    assert_eq!(
        m.events
            .iter()
            .filter(|e| e.frame == 4500 && matches!(e.command, Command::Cut { .. }))
            .count(),
        2
    );
    let pcm = f.render(512);
    for frame in 4500..4600 {
        close(pcm[frame * 2], signal(0, frame, 0) * 0.5);
    }
}
#[test]
fn offset_reverse_retrigger_cut_and_absolute_glide_use_selected_pad_source() {
    let mut f = Fixture::new(2, RATE, 1024);
    f.pad(2).tune_semitones = 12;
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                direction: Some(Direction::Reverse),
                cut: Some(0.001),
                ..Default::default()
            },
            ..note(1)
        },
    );
    let pcm = f.render(512);
    for frame in 0..6 {
        for c in 0..2 {
            close(pcm[frame * 2 + c], signal(1, 100 - frame * 2, c) * 0.5);
        }
    }
    assert!(pcm[12..].iter().all(|v| *v == 0.0));
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..note(1)
        },
    );
    let glide = f.render(512);
    f.put(0, 0, note(1));
    assert_eq!(glide, f.render(512)); // initial root60+tune12 already target72, not84
    f.pad(2).tune_semitones = 0;
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.5,
                }),
                ..Default::default()
            },
            ..note(1)
        },
    );
    let pcm = f.render(512);
    close(pcm[3000 * 2], signal(1, 100, 0) * 0.5);
}
#[test]
fn invalid_mapping_and_parameter_edits_are_atomic_and_unused_kit_does_not_change_legacy() {
    let mut f = Fixture::new(1, RATE, 128);
    f.put(0, 0, note(0));
    let base = f.song.clone();
    for (root, tune) in [(0, -1), (127, 1), (60, 49)] {
        let mut pad = f.pad(1).clone();
        pad.root_note = root;
        pad.tune_semitones = tune;
        assert!(f.song.upsert_drum_pad(DrumKitId(1), PadId(1), pad).is_err());
        assert_eq!(f.song, base);
    }
    assert!(f.song.remove_drum_pad(DrumKitId(1), PadId(1)).is_err());
    assert_eq!(f.song, base);
    let mut unused = base.clone();
    unused.order.clear();
    assert!(unused.remove_drum_pad(DrumKitId(1), PadId(1)).is_err());
    let mut missing = base.clone();
    missing
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
        .note = Note::Note(127);
    assert!(Engine::new(&missing, None).is_err());
    let mut normal = base.clone();
    normal
        .assign_region(InstrumentId(1), Some(RegionId(1)))
        .unwrap();
    normal
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
        .note = Note::Note(72);
    normal.drum_kits.clear();
    let before = song_render::evaluate(&normal, None).unwrap();
    let pcm = render(Engine::new(&normal, None).unwrap(), 512);
    normal.drum_kits = base.drum_kits;
    normal
        .assign_kit(InstrumentId(4), Some(DrumKitId(1)))
        .unwrap();
    assert_eq!(song_render::evaluate(&normal, None).unwrap(), before);
    assert_eq!(render(Engine::new(&normal, None).unwrap(), 512), pcm);
    assert!(serde_json::to_value(&normal).unwrap()["instruments"]["1"]
        .get("kind")
        .is_none());
}
#[test]
fn project_stream_export_cold_seek_loops_and_stop_share_kit_semantics() {
    let mut f = Fixture::new(2, 44100, 294);
    f.put(0, 0, note(0));
    f.put(2, 0, note(1));
    let path = f.dir.join("kit.json");
    aqua_pattern_core::project::save(
        &f.song,
        &path,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    let (restored, _) = aqua_pattern_core::project::load(&path).unwrap();
    assert_eq!(restored, f.song);
    let wav = f.dir.join("kit.wav");
    let (stats, m) = song_render::export(&restored, None, &wav).unwrap();
    let expected = f.render(512);
    assert_eq!(
        hound::WavReader::open(wav)
            .unwrap()
            .samples::<f32>()
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
        expected
    );
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&restored, None, shared.clone()).unwrap();
    assert_eq!(producer.engine.manifest, m);
    let mut actual = Vec::new();
    while !shared.has_ended() {
        producer.pump_until_buffered().unwrap();
        let mut block = [0.0; 512];
        shared.fill(&mut block);
        actual.extend(block);
    }
    assert_eq!(&actual[..stats.frames as usize * 2], expected);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    let mut engine = Engine::new(&restored, None).unwrap();
    engine.cold_seek(1).unwrap();
    let cold = render(engine, 37);
    assert!(cold[..(12000 - 1) * 2].iter().all(|v| *v == 0.0));
    assert_eq!(&cold[(12000 - 1) * 2..], &expected[12000 * 2..]);
    let shared = Arc::new(Shared::default());
    shared.set_loop(true, m.musical_frames);
    let mut producer = Producer::new(&restored, None, shared.clone()).unwrap();
    let mut looped = Vec::new();
    let count = m.musical_frames as usize * 2;
    while looped.len() < count * 2 {
        producer.pump_until_buffered().unwrap();
        let mut block = [0.0; 512];
        shared.fill(&mut block);
        looped.extend(block);
    }
    assert_eq!(&looped[..count], &expected[..count]);
    assert_eq!(&looped[count..count * 2], &expected[..count]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut silence = [1.0; 512];
    shared.fill(&mut silence);
    assert!(silence.iter().all(|v| *v == 0.0));
}
#[test]
fn clipping_and_invalid_offset_export_cleanup_and_source_tamper_fail_safely() {
    let mut f = Fixture::new(1, RATE, RATE);
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(u64::from(RATE)),
                ..Default::default()
            },
            ..note(0)
        },
    );
    let output = f.dir.join("invalid.wav");
    assert!(song_render::export(&f.song, None, &output).is_err());
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
    for i in 0..256 {
        f.put(0, i, note(0));
    }
    let output = f.dir.join("clip.wav");
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("clipping"));
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
    std::fs::write(f.dir.join("pad-0.wav"), b"changed").unwrap();
    assert!(Engine::new(&f.song, None)
        .err()
        .unwrap()
        .contains("SHA-256"));
}

#[test]
fn unused_kit_pattern_cannot_reinterpret_actual_order_instrument_memory() {
    let mut f = Fixture::new(1, RATE, 128);
    f.song
        .assign_region(InstrumentId(1), Some(RegionId(1)))
        .unwrap();
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(72),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    f.song.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Remember pitched instrument".into(),
            rows: 1,
            tracks: BTreeMap::from([(
                TrackId(1),
                BTreeMap::from([(
                    0,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Note(72),
                            ..Default::default()
                        },
                    )]),
                )]),
            )]),
        },
    );
    f.song.order.push(Occurrence {
        muted_tracks: Default::default(),
        section: None,
        pattern: PatternId(2),
    });
    let before = manifest(&f);
    let pcm = f.render(512);
    f.song
        .assign_kit(InstrumentId(4), Some(DrumKitId(1)))
        .unwrap();
    f.song.patterns.insert(
        PatternId(3),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Unused kit pattern".into(),
            rows: 1,
            tracks: BTreeMap::from([(
                TrackId(1),
                BTreeMap::from([(
                    0,
                    BTreeMap::from([(
                        0,
                        Cell {
                            instrument: Some(InstrumentId(4)),
                            ..note(0)
                        },
                    )]),
                )]),
            )]),
        },
    );
    f.song.validate().unwrap();
    assert_eq!(manifest(&f), before);
    assert_eq!(f.render(512), pcm);
    assert!(f.song.remove_drum_pad(DrumKitId(1), PadId(1)).is_err());
}
#[test]
fn expanded_kit_roll_budget_rejects_without_output_creation_or_voice_stealing() {
    let mut f = Fixture::new(1, RATE, 128);
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
                ..note(0)
            },
        );
    }
    let output = f.dir.join("overbudget.wav");
    let error = song_render::export(&f.song, None, &output).unwrap_err();
    assert!(error.contains("2048"), "{error}");
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
}

#[test]
fn late_trigger_owned_timed_release_stops_one_shot_phrase_tail_after_clock_end() {
    let mut f = Fixture::new(1, RATE, RATE);
    phrase(&mut f, PadMode::OneShot);
    f.song.lines_per_beat = 1;
    let phrase = f.song.phrases.get_mut(&PhraseId(1)).unwrap();
    phrase.rows = 1;
    phrase.lines_per_beat = 256;
    phrase.cells.remove(&1);
    f.song
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
        .release = Some(0.005);
    f.song
        .assign_kit(InstrumentId(2), Some(DrumKitId(1)))
        .unwrap();
    f.put(
        0,
        1,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..note(0)
        },
    );
    let m = manifest(&f);
    let release: Vec<_> = m
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.command,
                Command::Off {
                    reason: Release::TimedRelease,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(release.len(), 1);
    assert_eq!(release[0].frame, 120);
    assert_eq!(release[0].instrument, InstrumentId(1));
    assert!(release[0].phrase.is_some());
    let pcm = f.render(512);
    close(pcm[119 * 2], signal(0, 119, 0));
    assert!(pcm[120 * 2] > signal(0, 120, 0) * 0.5);
    assert!(pcm[120 * 2] < signal(0, 120, 0));
    close(pcm[6000 * 2], signal(0, 6000, 0) * 0.5);
}
