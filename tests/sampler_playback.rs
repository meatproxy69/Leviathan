use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine, Release},
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
}
fn constant(_: u32, channel: u16) -> f32 {
    if channel == 0 {
        0.25
    } else {
        -0.125
    }
}
fn ramp(frame: u32, channel: u16) -> f32 {
    (frame as f32 + 1.0) * if channel == 0 { 0.001 } else { -0.0005 }
}
impl Fixture {
    fn new(rate: u32, frames: u32, signal: fn(u32, u16) -> f32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-sampler-playback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("source.wav");
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..frames {
            for c in 0..2 {
                w.write_sample(signal(frame, c)).unwrap();
            }
        }
        w.finalize().unwrap();
        let mut song = example_song();
        song.bpm = 120.0;
        song.lines_per_beat = 100; // exactly 240 output frames per row
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "Sampler".into(),
                rows: 8,
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
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                name: "Region".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: frames.into(),
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        song.assign_region(InstrumentId(1), Some(RegionId(1)))
            .unwrap();
        let mut f = Self { dir, song };
        f.put(0, 0, note(60));
        f
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
    fn settings(&mut self, envelope: Option<AmpEnvelope>, loop_: Option<ForwardLoop>) {
        self.song
            .set_region_playback(RegionId(1), envelope, loop_)
            .unwrap();
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
fn note(pitch: u8) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(1)),
        ..Default::default()
    }
}
fn off() -> Cell {
    Cell {
        note: Note::Off,
        ..Default::default()
    }
}
fn env(a: u16, d: u16, s: u8, r: u16) -> AmpEnvelope {
    AmpEnvelope {
        attack_ms: a,
        decay_ms: d,
        sustain_percent: s,
        release_ms: r,
    }
}
fn render(song: &Song, block: usize) -> Vec<f32> {
    render_engine(Engine::new(song, None).unwrap(), block)
}
fn render_engine(mut engine: Engine, block: usize) -> Vec<f32> {
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut pcm = Vec::new();
    let mut buf = vec![0.0; block * 2];
    while engine.position < end {
        let n = engine.block(&mut buf, end).unwrap();
        pcm.extend_from_slice(&buf[..n * 2]);
    }
    pcm
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn zero_after(pcm: &[f32], frame: usize) {
    assert!(pcm[frame * 2..].iter().all(|x| *x == 0.0));
}
#[test]
fn additive_defaults_and_atomic_bounds_do_not_touch_sources() {
    let mut f = Fixture::new(48000, 64, constant);
    let old = f.song.clone();
    let json = serde_json::to_string(&old).unwrap();
    assert!(!json.contains("amp_envelope"));
    assert!(!json.contains("forward_loop"));
    assert_eq!(serde_json::from_str::<Song>(&json).unwrap(), old);
    for e in [
        env(10001, 0, 100, 50),
        env(0, 10001, 100, 50),
        env(0, 0, 101, 50),
        env(0, 0, 100, 501),
    ] {
        assert!(f
            .song
            .set_region_playback(RegionId(1), Some(e), None)
            .is_err());
        assert_eq!(f.song, old);
    }
    for l in [
        ForwardLoop {
            start_frame: 0,
            end_frame: 1,
        },
        ForwardLoop {
            start_frame: 2,
            end_frame: 1,
        },
        ForwardLoop {
            start_frame: 0,
            end_frame: 65,
        },
        ForwardLoop {
            start_frame: u64::MAX,
            end_frame: 2,
        },
    ] {
        assert!(f
            .song
            .set_region_playback(RegionId(1), None, Some(l))
            .is_err());
        assert_eq!(f.song, old);
    }
    assert!(f
        .song
        .set_region_playback(RegionId(999), None, None)
        .is_err());
    f.settings(
        Some(AmpEnvelope::default()),
        Some(ForwardLoop {
            start_frame: 1,
            end_frame: 63,
        }),
    );
    let mut shrink = f.song.regions[&RegionId(1)].clone();
    shrink.end_frame = 62;
    assert!(f.song.upsert_region(RegionId(1), shrink).is_err());
    let json = serde_json::to_string(&f.song).unwrap();
    assert_eq!(serde_json::from_str::<Song>(&json).unwrap(), f.song);
    assert!(serde_json::from_str::<AmpEnvelope>(
        r#"{"attack_ms":0,"decay_ms":0,"sustain_percent":100,"release_ms":50,"unknown":0}"#
    )
    .is_err());
    assert_eq!(
        sample_asset::import(&f.dir.join("source.wav"))
            .unwrap()
            .asset,
        old.samples[&SampleId(1)]
    );
}
#[test]
fn adsr_exact_attack_decay_sustain_and_release_samples() {
    let mut f = Fixture::new(48000, 10000, constant);
    f.settings(Some(env(10, 10, 25, 20)), None);
    f.put(5, 0, off());
    let pcm = f.pcm(37);
    for n in 0..2161 {
        let gain = if n < 480 {
            n as f64 / 480.0
        } else if n < 960 {
            1.0 - 0.75 * (n - 480) as f64 / 480.0
        } else if n < 1200 {
            0.25
        } else if n < 2160 {
            0.25 * (1.0 - (n - 1200) as f64 / 960.0)
        } else {
            0.0
        };
        near(pcm[n * 2], 0.125 * gain as f32);
        near(pcm[n * 2 + 1], -0.0625 * gain as f32);
    }
    zero_after(&pcm, 2160);
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
}
#[test]
fn off_during_attack_or_decay_starts_from_next_frame_gain() {
    for (settings, row, start) in [
        (env(20, 10, 50, 10), 1, 0.25),
        (env(10, 20, 50, 10), 3, 0.875),
    ] {
        let mut f = Fixture::new(48000, 10000, constant);
        f.settings(Some(settings), None);
        f.put(row, 0, off());
        let pcm = f.pcm(37);
        let at = row as usize * 240;
        near(pcm[at * 2], 0.125 * start);
        near(pcm[(at + 240) * 2], 0.125 * start * 0.5);
        zero_after(&pcm, at + 480);
    }
}
#[test]
fn zero_stages_and_zero_release_have_explicit_boundaries() {
    let mut f = Fixture::new(48000, 10000, constant);
    f.settings(Some(env(0, 0, 25, 0)), None);
    f.put(1, 0, off());
    let pcm = f.pcm(512);
    for n in 0..240 {
        near(pcm[n * 2], 0.03125);
    }
    zero_after(&pcm, 240);
    f.settings(
        Some(env(0, 0, 0, 500)),
        Some(ForwardLoop {
            start_frame: 0,
            end_frame: 10,
        }),
    );
    assert!(f.pcm(37).iter().all(|x| *x == 0.0));
    f.settings(Some(env(0, 10, 50, 0)), None);
    let pcm = f.pcm(1);
    near(pcm[0], 0.125);
    near(pcm[239 * 2], 0.125 * (1.0 - 0.5 * 239.0 / 480.0));
}
#[test]
fn maximum_release_fits_song_end_tail_and_loop_none_envelope_still_stops() {
    let mut f = Fixture::new(48000, 10, constant);
    f.settings(
        Some(env(0, 0, 100, 500)),
        Some(ForwardLoop {
            start_frame: 2,
            end_frame: 8,
        }),
    );
    let pcm = f.pcm(37);
    let end = 1920;
    assert_eq!(pcm.len(), (end + 24000) * 2);
    near(pcm[end * 2], 0.125);
    near(pcm[(end + 12000) * 2], 0.0625);
    near(pcm[(end + 23999) * 2], 0.125 / 24000.0);
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
    f.settings(
        None,
        Some(ForwardLoop {
            start_frame: 2,
            end_frame: 8,
        }),
    );
    let pcm = f.pcm(37);
    near(pcm[(end - 1) * 2], 0.125);
    zero_after(&pcm, end + 5000);
}
#[test]
fn loop_prefix_seam_native_rate_and_partition_oracle() {
    for rate in [24000, 44100, 48000, 96000] {
        let mut f = Fixture::new(rate, 100, ramp);
        f.song.regions.get_mut(&RegionId(1)).unwrap().start_frame = 3;
        f.settings(
            None,
            Some(ForwardLoop {
                start_frame: 10,
                end_frame: 17,
            }),
        );
        let pcm = f.pcm(37);
        for n in 0..1920 {
            let mut p = (3.0 * 48000.0 + n as f64 * f64::from(rate)) / 48000.0;
            if p >= 17.0 {
                p = 10.0 + (p - 10.0).rem_euclid(7.0);
            }
            let lo = p.floor() as u32;
            let next = if lo + 1 == 17 { 10 } else { lo + 1 };
            for c in 0..2 {
                let a = ramp(lo, c);
                let b = ramp(next, c);
                near(
                    pcm[n * 2 + usize::from(c)],
                    (a + (b - a) * (p - p.floor()) as f32) * 0.5,
                );
            }
        }
        assert_eq!(pcm, f.pcm(1));
        assert_eq!(pcm, f.pcm(512));
    }
}
#[test]
fn high_pitch_wraps_many_times_per_frame_without_source_bleed() {
    let mut f = Fixture::new(384000, 100, ramp);
    f.song.regions.get_mut(&RegionId(1)).unwrap().root_note = 0;
    f.put(0, 0, note(127));
    f.settings(
        None,
        Some(ForwardLoop {
            start_frame: 5,
            end_frame: 7,
        }),
    );
    let pcm = f.pcm(37);
    let speed = 2.0_f64.powf(127.0 / 12.0);
    for n in 0..1920 {
        let mut p = n as f64 * speed * 384000.0 / 48000.0;
        if p >= 7.0 {
            p = 5.0 + (p - 5.0).rem_euclid(2.0);
        }
        let lo = p.floor() as u32;
        let next = if lo + 1 == 7 { 5 } else { lo + 1 };
        near(
            pcm[n * 2],
            (ramp(lo, 0) + (ramp(next, 0) - ramp(lo, 0)) * (p - p.floor()) as f32) * 0.5,
        );
    }
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
}
#[test]
fn unsupported_loop_controls_reject_before_export_creation_but_envelope_only_allows_them() {
    let mut f = Fixture::new(44100, 100, ramp);
    for effects in [
        StepEffects {
            offset: Some(5),
            ..Default::default()
        },
        StepEffects {
            direction: Some(Direction::Reverse),
            ..Default::default()
        },
    ] {
        f.settings(
            Some(AmpEnvelope::default()),
            Some(ForwardLoop {
                start_frame: 3,
                end_frame: 20,
            }),
        );
        let mut on = note(60);
        on.effects = effects.clone();
        f.put(0, 0, on);
        assert!(song_render::evaluate(&f.song, None)
            .unwrap_err()
            .contains("forward loop does not support"));
        let out = f.dir.join("invalid.wav");
        assert!(song_render::export(&f.song, None, &out).is_err());
        assert!(!out.exists());
        assert!(!out.with_extension("events.json").exists());
        f.put(0, 0, note(60));
        f.put(
            1,
            0,
            Cell {
                effects: effects.clone(),
                ..Default::default()
            },
        );
        assert!(song_render::evaluate(&f.song, None).is_err());
        f.settings(Some(AmpEnvelope::default()), None);
        assert!(Engine::new(&f.song, None).is_ok());
    }
    f.settings(
        None,
        Some(ForwardLoop {
            start_frame: 3,
            end_frame: 20,
        }),
    );
    // The existing pathname is authorized, but bytes are deliberately corrupt.
    // Unsupported controls must win before source hashing/decoding or outputs.
    std::fs::write(f.dir.join("source.wav"), b"not a WAV").unwrap();
    assert!(Engine::new(&f.song, None)
        .err()
        .unwrap()
        .contains("forward loop does not support"));
}
#[test]
fn envelope_only_reverse_and_offset_preserve_source_cursor() {
    let mut f = Fixture::new(48000, 100, ramp);
    f.settings(Some(env(0, 0, 50, 10)), None);
    let mut on = note(60);
    on.effects.offset = Some(80);
    on.effects.direction = Some(Direction::Reverse);
    f.put(0, 0, on);
    let pcm = f.pcm(1);
    for n in 0..81 {
        near(pcm[n * 2], ramp(80 - n as u32, 0) * 0.25);
    }
    zero_after(&pcm, 81);
}
#[test]
fn loop_glide_and_forward_control_are_continuous_partition_invariant() {
    let mut f = Fixture::new(44100, 100, ramp);
    f.settings(
        Some(env(1, 2, 70, 20)),
        Some(ForwardLoop {
            start_frame: 3,
            end_frame: 13,
        }),
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 120.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let baseline = f.pcm(37);
    f.put(
        2,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Forward),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(baseline, f.pcm(37));
    assert_eq!(baseline, f.pcm(1));
    assert_eq!(baseline, f.pcm(512));
}
#[test]
fn unused_options_and_clear_to_none_leave_existing_manifest_and_pcm_exact() {
    let mut f = Fixture::new(48000, 10000, constant);
    let manifest = song_render::evaluate(&f.song, None).unwrap();
    let pcm = f.pcm(37);
    let mut other = f.song.regions[&RegionId(1)].clone();
    other.amp_envelope = Some(AmpEnvelope::default());
    other.forward_loop = Some(ForwardLoop {
        start_frame: 2,
        end_frame: 10,
    });
    f.song.upsert_region(RegionId(2), other).unwrap();
    assert_eq!(manifest, song_render::evaluate(&f.song, None).unwrap());
    assert_eq!(pcm, f.pcm(37));
    f.settings(Some(AmpEnvelope::default()), None);
    assert_eq!(song_render::evaluate(&f.song, None).unwrap().schema, 8);
    f.settings(None, None);
    assert_eq!(manifest, song_render::evaluate(&f.song, None).unwrap());
    assert_eq!(pcm, f.pcm(37));
}
fn make_kit(f: &mut Fixture, mode: PadMode, new_note: PadNewNote, choke: Option<u8>) {
    let pads = (0..2)
        .map(|n| {
            (
                PadId(n + 1),
                DrumPad {
                    velocity_zones: Vec::new(),
                    slot: n as u8,
                    name: format!("Pad {n}"),
                    note: 60 + n as u8,
                    region: Some(RegionId(1)),
                    root_note: 60,
                    tune_semitones: 0,
                    gain: 1.0,
                    pan: 0.0,
                    mode,
                    new_note,
                    choke_group: choke,
                },
            )
        })
        .collect();
    f.song
        .upsert_drum_kit(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Kit".into(),
                pads,
            },
        )
        .unwrap();
    f.song
        .assign_kit(InstrumentId(1), Some(DrumKitId(1)))
        .unwrap();
}
#[test]
fn choke_and_pad_replacement_use_fixed_safety_release_even_at_zero_new_velocity() {
    for (next_pitch, policy, choke, reason) in [
        (61, PadNewNote::Overlap, Some(1), Release::Choke),
        (60, PadNewNote::ReleasePrevious, None, Release::PadReplaced),
    ] {
        let mut f = Fixture::new(RATE, 20, constant);
        f.settings(
            Some(env(0, 0, 100, 0)),
            Some(ForwardLoop {
                start_frame: 2,
                end_frame: 10,
            }),
        );
        make_kit(&mut f, PadMode::OneShot, policy, choke);
        let mut next = note(next_pitch);
        next.velocity = Some(0.0);
        f.put(1, 0, next);
        let m = song_render::evaluate(&f.song, None).unwrap();
        assert!(m.events.iter().any(
            |e| e.frame == 240 && matches!(&e.command,Command::Off{reason:r,..} if *r==reason)
        ));
        let pcm = f.pcm(37);
        near(pcm[240 * 2], 0.125);
        near(pcm[1440 * 2], 0.0625);
        zero_after(&pcm, 2640);
    }
}
#[test]
fn track_mute_releases_loop_with_safety_ramp_and_ignores_following_off() {
    let mut f = Fixture::new(RATE, 20, constant);
    f.settings(
        Some(env(0, 0, 100, 500)),
        Some(ForwardLoop {
            start_frame: 2,
            end_frame: 10,
        }),
    );
    let p = f.song.patterns.get_mut(&PatternId(1)).unwrap();
    p.rows = 2;
    f.song.patterns.insert(
        PatternId(2),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Mute".into(),
            rows: 16,
            tracks: BTreeMap::from([(
                TrackId(1),
                BTreeMap::from([(1, BTreeMap::from([(0, off())]))]),
            )]),
        },
    );
    f.song.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: std::collections::BTreeSet::from([TrackId(1)]),
        section: None,
    });
    let pcm = f.pcm(37);
    near(pcm[480 * 2], 0.125);
    near(pcm[1680 * 2], 0.0625);
    zero_after(&pcm, 2880);
}
#[test]
fn one_shot_ignores_off_gated_honors_it_and_hard_cut_is_exact() {
    for mode in [PadMode::OneShot, PadMode::Gated] {
        let mut f = Fixture::new(RATE, 20, constant);
        f.settings(
            Some(env(0, 0, 100, 10)),
            Some(ForwardLoop {
                start_frame: 2,
                end_frame: 10,
            }),
        );
        make_kit(&mut f, mode, PadNewNote::Overlap, None);
        f.put(1, 0, off());
        let pcm = f.pcm(37);
        if mode == PadMode::OneShot {
            near(pcm[1000 * 2], 0.125);
            zero_after(&pcm, 2400);
        } else {
            zero_after(&pcm, 720);
        }
        f.put(
            1,
            0,
            Cell {
                effects: StepEffects {
                    cut: Some(0.0),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let pcm = f.pcm(37);
        near(pcm[239 * 2], 0.125);
        zero_after(&pcm, 240);
    }
}
#[test]
fn retrigger_restarts_envelope_without_resetting_older_release() {
    let mut f = Fixture::new(RATE, 20, constant);
    f.settings(
        Some(env(10, 0, 100, 10)),
        Some(ForwardLoop {
            start_frame: 2,
            end_frame: 10,
        }),
    );
    let mut on = note(60);
    on.effects.retrigger = Some(Retrigger {
        repeats: 1,
        spacing: 0.5,
    });
    f.put(0, 0, on);
    let pcm = f.pcm(37);
    // At120 old attack is.25, release starts there; new attack starts0.
    near(pcm[120 * 2], 0.125 * 0.25);
    near(pcm[360 * 2], 0.125 * (0.25 * 0.5 + 0.5));
    // Old release finished at600; new attack reached peak exactly there.
    near(pcm[600 * 2], 0.125);
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
}
#[test]
fn shared_pcm_regions_keep_independent_loop_and_envelope_state() {
    let mut f = Fixture::new(44100, 100, ramp);
    f.settings(
        Some(env(2, 3, 40, 10)),
        Some(ForwardLoop {
            start_frame: 3,
            end_frame: 17,
        }),
    );
    let mut other = f.song.regions[&RegionId(1)].clone();
    other.start_frame = 20;
    other.amp_envelope = Some(env(1, 2, 70, 5));
    other.forward_loop = Some(ForwardLoop {
        start_frame: 25,
        end_frame: 37,
    });
    f.song.upsert_region(RegionId(2), other).unwrap();
    f.song
        .assign_region(InstrumentId(2), Some(RegionId(2)))
        .unwrap();
    let first = f.pcm(37);
    let mut second = f.song.clone();
    second
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    second
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    1,
                    Cell {
                        instrument: Some(InstrumentId(2)),
                        ..note(60)
                    },
                )]),
            )]),
        );
    let second = render(&second, 37);
    f.put(
        0,
        1,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..note(60)
        },
    );
    let mixed = f.pcm(37);
    for ((a, b), m) in first.iter().zip(second).zip(mixed) {
        near(*a + b, m);
    }
}
#[test]
fn phrase_end_and_parent_cut_retain_independent_loop_voice_ownership() {
    for mode in [PadMode::OneShot, PadMode::Gated] {
        let mut f = Fixture::new(RATE, 20, constant);
        f.settings(
            Some(env(0, 0, 100, 10)),
            Some(ForwardLoop {
                start_frame: 2,
                end_frame: 10,
            }),
        );
        make_kit(&mut f, mode, PadNewNote::Overlap, None);
        f.song.phrases.insert(
            PhraseId(1),
            Phrase {
                name: "Two loops".into(),
                instrument: InstrumentId(1),
                rows: 2,
                lines_per_beat: 100,
                note_columns: 1,
                root_note: 60,
                cells: BTreeMap::from([
                    (0, BTreeMap::from([(0, note(60))])),
                    (1, BTreeMap::from([(0, note(60))])),
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
        let pcm = f.pcm(37);
        if mode == PadMode::Gated {
            zero_after(&pcm, 960);
        } else {
            near(pcm[1000 * 2], 0.25);
        }
        // Scheduled parent cut after natural phrase end must still kill both tails.
        f.song.lines_per_beat = 4;
        let cell = f
            .song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap();
        cell.effects.cut = Some(0.1);
        let pcm = f.pcm(37);
        zero_after(&pcm, 600);
    }
}
#[test]
fn exact_export_roundtrip_cold_seek_and_streaming_reuse_same_loop_engine() {
    use aqua_pattern_core::transport::{Producer, Shared};
    use std::sync::Arc;
    let mut f = Fixture::new(44100, 100, ramp);
    f.settings(
        Some(env(1, 2, 60, 50)),
        Some(ForwardLoop {
            start_frame: 3,
            end_frame: 20,
        }),
    );
    f.put(4, 0, note(60));
    let json = serde_json::to_string(&f.song).unwrap();
    let reopened: Song = serde_json::from_str(&json).unwrap();
    assert_eq!(reopened, f.song);
    let a = f.dir.join("a.wav");
    let b = f.dir.join("b.wav");
    song_render::export(&f.song, None, &a).unwrap();
    song_render::export(&reopened, None, &b).unwrap();
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
    let mut engine = Engine::new(&f.song, None).unwrap();
    engine.cold_seek(240).unwrap();
    let mut buf = [0.0; 74];
    let n = engine.block(&mut buf, 960).unwrap();
    assert!(buf[..n * 2].iter().all(|x| *x == 0.0));
    engine.cold_seek(0).unwrap();
    let n = engine.block(&mut buf, 1920).unwrap();
    assert!(buf[..n * 2].iter().any(|x| *x != 0.0));
    let expected = f.pcm(512);
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut actual = Vec::new();
    while !shared.has_ended() {
        producer.pump_until_buffered().unwrap();
        let mut out = [0.0; 512];
        shared.fill(&mut out);
        actual.extend(out);
    }
    assert_eq!(&actual[..expected.len()], expected);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    let shared = Arc::new(Shared::default());
    shared.set_loop(true, 1920);
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut looped = Vec::new();
    while looped.len() < 1920 * 4 {
        producer.pump_until_buffered().unwrap();
        let mut out = [0.0; 512];
        shared.fill(&mut out);
        looped.extend(out);
    }
    assert_eq!(&looped[..1920 * 2], &expected[..1920 * 2]);
    assert_eq!(&looped[1920 * 2..1920 * 4], &expected[..1920 * 2]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut out = [1.0; 74];
    shared.fill(&mut out);
    assert!(out.iter().all(|x| *x == 0.0));
}
#[test]
fn repeated_off_never_restarts_or_extends_an_envelope_release() {
    let mut f = Fixture::new(RATE, 10000, constant);
    f.settings(
        Some(env(0, 0, 100, 10)),
        Some(ForwardLoop {
            start_frame: 3,
            end_frame: 20,
        }),
    );
    f.put(1, 0, off());
    let expected = f.pcm(37);
    let mut engine = Engine::new(&f.song, None).unwrap();
    let mut duplicate = engine
        .manifest
        .events
        .iter()
        .find(|e| e.frame == 240 && matches!(e.command, Command::Off { .. }))
        .unwrap()
        .clone();
    duplicate.frame = 400;
    duplicate.command = Command::Off {
        voice: match duplicate.command {
            Command::Off { voice, .. } => voice,
            _ => unreachable!(),
        },
        reason: Release::TrackMute,
    };
    engine.manifest.events.push(duplicate);
    engine.manifest.events.sort_by_key(|e| e.frame);
    assert_eq!(expected, render_engine(engine, 37));
    zero_after(&expected, 720);
}
#[test]
fn natural_nonloop_exhaustion_wins_and_late_offset_cannot_revive() {
    let mut f = Fixture::new(44100, 294, constant);
    f.settings(Some(env(0, 0, 100, 500)), None);
    f.put(
        1,
        0,
        Cell {
            delay: Some(1.0 / 3.0),
            effects: StepEffects {
                offset: Some(0),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let pcm = f.pcm(37);
    assert!(pcm[319 * 2] > 0.0);
    zero_after(&pcm, 320);
    assert_eq!(pcm, f.pcm(1));
    assert_eq!(pcm, f.pcm(512));
}
#[test]
fn loop_fx_conflicts_follow_active_voice_not_future_instrument_selection() {
    let mut f = Fixture::new(RATE, 10000, constant);
    let mut plain = f.song.regions[&RegionId(1)].clone();
    plain.name = "Plain".into();
    f.song.upsert_region(RegionId(2), plain).unwrap();
    f.song
        .assign_region(InstrumentId(2), Some(RegionId(2)))
        .unwrap();
    f.settings(
        None,
        Some(ForwardLoop {
            start_frame: 3,
            end_frame: 20,
        }),
    );
    f.put(
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..Default::default()
        },
    );
    f.put(
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
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("forward loop"));
    f.put(
        0,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            ..note(60)
        },
    );
    f.put(
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    assert!(Engine::new(&f.song, None).is_ok());
}
#[test]
fn overlapping_loop_envelopes_reject_clipping_without_output_leftovers() {
    let mut f = Fixture::new(RATE, 20, constant);
    f.settings(
        Some(env(1, 0, 100, 10)),
        Some(ForwardLoop {
            start_frame: 2,
            end_frame: 10,
        }),
    );
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 16;
    for column in 1..9 {
        f.put(0, column, note(60));
    }
    let out = f.dir.join("clipped.wav");
    let error = song_render::export(&f.song, None, &out).unwrap_err();
    assert!(error.to_lowercase().contains("clip"));
    assert!(!out.exists());
    assert!(!out.with_extension("events.json").exists());
}
