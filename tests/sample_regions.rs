use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine},
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
}
impl Fixture {
    fn new(rate: u32, channels: u16, frames: u32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-regions-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut writer = hound::WavWriter::create(
            dir.join("source.wav"),
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..frames {
            for channel in 0..channels {
                writer.write_sample(value(frame, channel)).unwrap();
            }
        }
        writer.finalize().unwrap();
        let mut song = example_song();
        song.bpm = 120.0;
        song.lines_per_beat = 4;
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: Default::default(),
                name: "Slices".into(),
                rows: 16,
                tracks: BTreeMap::new(),
            },
        )]);
        song.order = vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }];
        let imported = sample_asset::import(&dir.join("source.wav")).unwrap();
        song.register_sample(SampleId(1), imported.asset).unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: "Window".into(),
                sample: SampleId(1),
                start_frame: 10,
                end_frame: 30,
                root_note: 60,
            },
        )
        .unwrap();
        song.assign_region(InstrumentId(2), Some(RegionId(1)))
            .unwrap();
        let mut f = Self { dir, song };
        f.put(
            0,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                ..Default::default()
            },
        );
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
    fn region(&mut self, start: u64, end: u64, root: u8) {
        let region = self.song.regions.get_mut(&RegionId(1)).unwrap();
        region.start_frame = start;
        region.end_frame = end;
        region.root_note = root;
    }
    fn pcm(&self, block: usize) -> Vec<f32> {
        render(Engine::new(&self.song, None).unwrap(), block)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn value(frame: u32, channel: u16) -> f32 {
    ((frame % 997) as f32 + 1.0) * if channel == 0 { 0.0001 } else { -0.00007 }
}
fn render(mut engine: Engine, block: usize) -> Vec<f32> {
    let mut output = vec![0.0; block * 2];
    let mut pcm = Vec::new();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    while engine.position < end {
        let count = engine.block(&mut output, end).unwrap();
        pcm.extend_from_slice(&output[..count * 2]);
    }
    pcm
}
fn assert_close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
#[test]
fn strict_wav_and_flac_import_retain_native_metadata_exact_pcm_and_guard() {
    for rate in [44100, 48000, 96000] {
        for channels in [1, 2] {
            let f = Fixture::new(rate, channels, 256);
            let imported = sample_asset::import(&f.dir.join("source.wav")).unwrap();
            assert_eq!(
                (
                    imported.asset.sample_rate,
                    imported.asset.channels,
                    imported.asset.frames
                ),
                (rate, channels, 256)
            );
            assert_eq!(imported.buffer.frame_count(), 257);
            for frame in 0..256 {
                for channel in 0..channels {
                    assert_eq!(
                        imported.buffer.buffer()
                            [frame * usize::from(channels) + usize::from(channel)],
                        value(frame as u32, channel)
                    );
                }
            }
            assert!(imported.buffer.buffer()[256 * usize::from(channels)..]
                .iter()
                .all(|v| *v == 0.0));
        }
    }
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slice-stereo-44100.flac");
    let imported = sample_asset::import(&path).unwrap();
    assert_eq!(
        (
            imported.asset.sample_rate,
            imported.asset.channels,
            imported.asset.frames
        ),
        (44100, 2, 256)
    );
    for i in 0..256 {
        assert_eq!(
            imported.buffer.buffer()[i * 2],
            (i as f32 * 31.0 - 4000.0) / 32768.0
        );
        assert_eq!(
            imported.buffer.buffer()[i * 2 + 1],
            (3500.0 - i as f32 * 19.0) / 32768.0
        );
    }
}
#[test]
fn native_rates_mono_stereo_root_pitch_and_exclusive_region_edges_are_exact() {
    for rate in [44100, 48000, 96000] {
        for channels in [1, 2] {
            let mut f = Fixture::new(rate, channels, 1024);
            f.region(10, 30, 48);
            f.put(
                0,
                0,
                Cell {
                    note: Note::Note(48),
                    instrument: Some(InstrumentId(2)),
                    ..Default::default()
                },
            );
            let pcm = f.pcm(512);
            let ratio = f64::from(rate) / f64::from(RATE);
            for frame in 0..40 {
                let position = 10.0 + frame as f64 * ratio;
                for channel in 0..2 {
                    let expected = if position >= 30.0 {
                        0.0
                    } else {
                        let lower = position.floor() as u32;
                        let fraction = (position - f64::from(lower)) as f32;
                        let source_channel = if channels == 1 { 0 } else { channel as u16 };
                        let a = value(lower, source_channel);
                        let b = if lower + 1 < 30 {
                            value(lower + 1, source_channel)
                        } else {
                            0.0
                        };
                        (a + (b - a) * fraction) * 0.5
                    };
                    assert_close(pcm[frame * 2 + channel], expected);
                }
            }
        }
    }
}
#[test]
fn reverse_defaults_to_last_region_frame_and_offset_stays_absolute() {
    let mut f = Fixture::new(48000, 2, 1024);
    for offset in [None, Some(22)] {
        f.put(
            0,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                effects: StepEffects {
                    direction: Some(Direction::Reverse),
                    offset,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let pcm = f.pcm(17);
        let start = offset.unwrap_or(29);
        for frame in 0..25 {
            for channel in 0..2 {
                let expected = if frame as u64 <= start - 10 {
                    value((start - frame as u64) as u32, channel as u16) * 0.5
                } else {
                    0.0
                };
                assert_close(pcm[frame * 2 + channel], expected);
            }
        }
    }
    for offset in [9, 30, 1023] {
        f.put(
            0,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                effects: StepEffects {
                    offset: Some(offset),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let output = f.dir.join(format!("bad-{offset}.wav"));
        assert!(song_render::export(&f.song, None, &output).is_err());
        assert!(!output.exists());
        assert!(!output.with_extension("events.json").exists());
    }
}
#[test]
fn live_direction_and_offset_preserve_window_cursor_and_exhaustion() {
    let mut f = Fixture::new(48000, 1, 1024);
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    f.region(100, 500, 60);
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                direction: Some(Direction::Reverse),
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
                offset: Some(200),
                direction: Some(Direction::Forward),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let pcm = f.pcm(13);
    for frame in 0..28 {
        assert_close(pcm[frame * 2], value(100 + frame as u32, 0) * 0.5);
    }
    for frame in 28..56 {
        assert_close(pcm[frame * 2], value(128 - (frame - 28) as u32, 0) * 0.5);
    }
    for frame in 56..80 {
        assert_close(pcm[frame * 2], value(200 + (frame - 56) as u32, 0) * 0.5);
    }
    f.region(100, 128, 60);
    f.put(2, 0, Cell::default());
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(100),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let pcm = f.pcm(28);
    assert!(pcm[28 * 2..].iter().all(|v| *v == 0.0));
}
#[test]
fn changed_source_metadata_corruption_and_budget_fail_before_output_creation() {
    let f = Fixture::new(48000, 1, 256);
    let source = &f.song.samples[&SampleId(1)];
    assert!(sample_asset::load_verified(source, &source.path, 10).is_err());
    for mutate in [0, 1, 2] {
        let mut song = f.song.clone();
        let asset = song.samples.get_mut(&SampleId(1)).unwrap();
        match mutate {
            0 => asset.sample_rate = 44100,
            1 => asset.frames = 255,
            _ => asset.channels = 2,
        };
        let path = f.dir.join(format!("metadata-{mutate}.wav"));
        assert!(song_render::export(&song, None, &path).is_err());
        assert!(!path.exists());
    }
    let mut bytes = std::fs::read(&source.path).unwrap();
    let original = bytes.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&source.path, &bytes).unwrap();
    let output = f.dir.join("changed.wav");
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("SHA-256"));
    assert!(!output.exists());
    for (name, data) in [
        ("truncated", original[..original.len() - 9].to_vec()),
        ("garbage", b"not audio".to_vec()),
    ] {
        let path = f.dir.join(name);
        std::fs::write(&path, data).unwrap();
        assert!(sample_asset::import(&path).is_err());
    }
    let path = f.dir.join("bad.flac");
    let encoded = include_bytes!("fixtures/slice-stereo-44100.flac");
    std::fs::write(&path, &encoded[..encoded.len() - 7]).unwrap();
    assert!(sample_asset::import(&path).is_err());
}
#[test]
fn zero_start_cut_and_retrigger_are_shared_and_partition_invariant() {
    let mut f = Fixture::new(48000, 2, 1024);
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    f.region(10, 900, 60);
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            velocity: Some(0.0),
            ..Default::default()
        },
    );
    assert!(f.pcm(512).iter().all(|x| *x == 0.0));
    f.put(
        1,
        0,
        Cell {
            velocity: Some(0.5),
            ..Default::default()
        },
    );
    let pcm = f.pcm(512);
    assert!(pcm[..28 * 2].iter().all(|x| *x == 0.0));
    assert!(pcm[29 * 2..].iter().any(|x| *x != 0.0));
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            effects: StepEffects {
                offset: Some(200),
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.25,
                }),
                cut: Some(0.75),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(1, 0, Cell::default());
    let a = f.pcm(512);
    let b = f.pcm(7);
    assert_eq!(a, b);
    let m = song_render::evaluate(&f.song, None).unwrap();
    let ons: Vec<_> = m
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .collect();
    assert_eq!(ons.iter().map(|e| e.frame).collect::<Vec<_>>(), [0, 7, 14]);
    for event in ons {
        assert!(m.events.iter().any(|e| e.frame == event.frame
            && matches!(
                e.command,
                Command::SampleStart {
                    offset: Some(200),
                    ..
                }
            )));
    }
    assert!(m
        .events
        .iter()
        .any(|e| e.frame == 21 && matches!(e.command, Command::Cut { .. })));
}

#[test]
fn four_source_beats_at_70_bpm_resequence_at_170_without_repitch_or_stretch() {
    // At 44.1kHz a 70 BPM beat is exactly 37800 source frames. No source-BPM
    // ratio enters the voice: the editor maps independent source regions to notes.
    let mut f = Fixture::new(44100, 1, 151200);
    let source = f.dir.join("break70.wav");
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
    for frame in 0..151200 {
        let within_beat = frame % 37800;
        writer
            .write_sample(if within_beat < 256 {
                value(within_beat, 0)
            } else {
                0.0
            })
            .unwrap();
    }
    writer.finalize().unwrap();
    // Replace the fixture's setup source explicitly, before compiling any audio.
    f.song.assign_region(InstrumentId(2), None).unwrap();
    f.song.remove_region(RegionId(1)).unwrap();
    f.song.remove_sample(SampleId(1)).unwrap();
    f.song
        .register_sample(SampleId(1), sample_asset::import(&source).unwrap().asset)
        .unwrap();
    let original = std::fs::read(&source).unwrap();
    f.song.bpm = 170.0;
    for i in 0..4u64 {
        let id = RegionId(i + 1);
        f.song
            .upsert_region(
                id,
                SampleRegion {
                    amp_envelope: None,
                    forward_loop: None,
                    name: format!("Hit {i}"),
                    sample: SampleId(1),
                    start_frame: i * 37800,
                    end_frame: i * 37800 + 256,
                    root_note: 60,
                },
            )
            .unwrap();
        let instrument = InstrumentId(i + 1);
        f.song.assign_region(instrument, Some(id)).unwrap();
        f.put(
            i as u32 * 4,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(instrument),
                ..Default::default()
            },
        );
    }
    let m = song_render::evaluate(&f.song, None).unwrap();
    let starts: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| matches!(e.command, Command::On { .. }).then_some(e.frame))
        .collect();
    assert_eq!(starts, [0, 16941, 33882, 50824]);
    let pcm = f.pcm(257);
    for &start in &starts {
        for frame in 0..279 {
            let position = frame as f64 * 44100.0 / 48000.0;
            let lower = position.floor() as u32;
            let fraction = (position - f64::from(lower)) as f32;
            let a = value(lower, 0);
            let b = if lower + 1 < 256 {
                value(lower + 1, 0)
            } else {
                0.0
            };
            assert_close(
                pcm[(start as usize + frame) * 2],
                (a + (b - a) * fraction) * 0.5,
            );
        }
        assert_eq!(pcm[(start as usize + 279) * 2], 0.0);
    }
    assert_eq!(m.regions.len(), 4);
    assert_eq!(m.schema, 5);
    assert_eq!(std::fs::read(source).unwrap(), original);
}
#[test]
fn exact_roll_subdivisions_16_through_128_are_independent_of_source_rate() {
    let mut f = Fixture::new(96000, 1, 1024);
    f.song.bpm = 170.0;
    for (row, hits) in [(0, 1u8), (1, 2), (2, 4), (3, 8)] {
        f.put(
            row,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                effects: StepEffects {
                    retrigger: (hits > 1).then_some(Retrigger {
                        repeats: hits - 1,
                        spacing: 1.0 / f64::from(hits),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }
    let m = song_render::evaluate(&f.song, None).unwrap();
    let starts: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| matches!(e.command, Command::On { .. }).then_some(e.frame))
        .collect();
    let expected: Vec<_> = [1u8, 2, 4, 8]
        .into_iter()
        .enumerate()
        .flat_map(|(row, hits)| {
            (0..hits).map(move |hit| {
                ((row as f64 + f64::from(hit) / f64::from(hits)) * 48000.0 * 60.0 / (170.0 * 4.0))
                    .round() as u64
            })
        })
        .collect();
    assert_eq!(starts, expected);
    assert_eq!(starts.len(), 15);
    let pcm = f.pcm(512);
    for start in starts {
        assert_close(pcm[start as usize * 2], value(10, 0) * 0.5);
        assert_eq!(pcm[(start as usize + 10) * 2], 0.0);
    }
}
#[test]
fn region_project_roundtrip_stream_export_cold_seek_loop_and_stop() {
    let mut f = Fixture::new(44100, 2, 1024);
    f.put(
        2,
        0,
        Cell {
            note: Note::Note(60),
            ..Default::default()
        },
    );
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 4;
    let path = f.dir.join("region.json");
    aqua_pattern_core::project::save(
        &f.song,
        &path,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    let (restored, _) = aqua_pattern_core::project::load(&path).unwrap();
    assert_eq!(restored, f.song);
    let wav = f.dir.join("out.wav");
    let (stats, manifest) = song_render::export(&restored, None, &wav).unwrap();
    let expected: Vec<f32> = hound::WavReader::open(&wav)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&restored, None, shared.clone()).unwrap();
    assert_eq!(producer.engine.manifest, manifest);
    let mut actual = Vec::new();
    for _ in 0..1000 {
        producer.pump_until_buffered().unwrap();
        let mut data = [0.0; 512];
        shared.fill(&mut data);
        actual.extend(data);
        if shared.has_ended() {
            break;
        }
    }
    assert!(shared.has_ended());
    assert_eq!(&actual[..stats.frames as usize * 2], expected);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    let mut engine = Engine::new(&restored, None).unwrap();
    engine.cold_seek(1).unwrap();
    let cold = render(engine, 17);
    assert!(cold[..(12000 - 1) * 2].iter().all(|v| *v == 0.0));
    assert_eq!(&cold[(12000 - 1) * 2..], &expected[12000 * 2..]);
    let shared = Arc::new(Shared::default());
    shared.set_loop(true, manifest.musical_frames);
    let mut producer = Producer::new(&restored, None, shared.clone()).unwrap();
    let mut looped = Vec::new();
    while looped.len() < manifest.musical_frames as usize * 4 {
        producer.pump_until_buffered().unwrap();
        let mut data = [0.0; 512];
        shared.fill(&mut data);
        looped.extend(data);
    }
    let count = manifest.musical_frames as usize * 2;
    assert_eq!(&looped[..count], &expected[..count]);
    assert_eq!(&looped[count..count * 2], &expected[..count]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut silence = [1.0; 512];
    shared.fill(&mut silence);
    assert!(silence.iter().all(|v| *v == 0.0));
}
#[test]
fn reverse_fractional_boundary_convention_does_not_read_neighboring_slices() {
    // Existing M6 cursor convention: reverse starts at last real frame, stops
    // below the first; unlike forward, there is no fractional lower guard tail.
    let mut f = Fixture::new(24000, 1, 256);
    for length in [1u64, 4] {
        f.region(10, 10 + length, 60);
        for reverse in [false, true] {
            f.put(
                0,
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(2)),
                    effects: StepEffects {
                        direction: reverse.then_some(Direction::Reverse),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            let pcm = f.pcm(3);
            let frames = if reverse { length * 2 - 1 } else { length * 2 };
            assert!(pcm[..frames as usize * 2].iter().all(|v| *v > 0.0));
            assert!(pcm[frames as usize * 2..].iter().all(|v| *v == 0.0));
        }
    }
}
#[test]
fn new_region_metadata_is_omitted_for_legacy_documents_and_unused_banks() {
    let mut f = Fixture::new(48000, 1, 256);
    f.song.assign_region(InstrumentId(2), None).unwrap();
    f.song.instruments.get_mut(&InstrumentId(2)).unwrap().sample = Some(f.dir.join("source.wav"));
    let with_bank = song_render::evaluate(&f.song, None).unwrap();
    let pcm = f.pcm(512);
    f.song.regions.clear();
    f.song.samples.clear();
    assert_eq!(song_render::evaluate(&f.song, None).unwrap(), with_bank);
    assert_eq!(f.pcm(512), pcm);
    let json = serde_json::to_value(&f.song).unwrap();
    assert!(json.get("samples").is_none());
    assert!(json.get("regions").is_none());
    assert!(json["instruments"]["2"].get("region").is_none());
}
#[test]
fn strict_import_rejects_hostile_headers_unknown_length_flac_nonfinite_and_oversize() {
    let f = Fixture::new(48000, 1, 256);
    let original = std::fs::read(f.dir.join("source.wav")).unwrap();
    let data = original.windows(4).position(|w| w == b"data").unwrap();
    let mut huge = original.clone();
    huge[data + 4..data + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    let path = f.dir.join("huge.wav");
    std::fs::write(&path, huge).unwrap();
    assert!(sample_asset::import(&path).is_err());
    let mut nan = original;
    nan[data + 8..data + 12].copy_from_slice(&f32::NAN.to_le_bytes());
    let path = f.dir.join("nan.wav");
    std::fs::write(&path, nan).unwrap();
    assert!(sample_asset::import(&path)
        .err()
        .unwrap()
        .contains("non-finite"));
    let mut flac = include_bytes!("fixtures/slice-stereo-44100.flac").to_vec();
    flac[21] &= 0xf0;
    flac[22..26].fill(0);
    let path = f.dir.join("unknown.flac");
    std::fs::write(&path, flac).unwrap();
    assert!(sample_asset::import(&path).is_err());
    let path = f.dir.join("oversize.wav");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(sample_asset::MAX_ENCODED_BYTES as u64 + 1)
        .unwrap();
    assert!(sample_asset::import(&path).is_err());
}
#[test]
fn region_root_note_transposes_pitch_and_glide_without_changing_event_clock() {
    let mut f = Fixture::new(48000, 1, 20000);
    f.region(10, 19000, 60);
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 24.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let original = f.pcm(512);
    let before = song_render::evaluate(&f.song, None).unwrap();
    f.region(10, 19000, 48);
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(48),
            instrument: Some(InstrumentId(2)),
            effects: StepEffects {
                glide: Some(Glide {
                    target: 60,
                    semitones_per_second: 24.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(f.pcm(512), original);
    assert_eq!(
        song_render::evaluate(&f.song, None).unwrap().musical_frames,
        before.musical_frames
    );
    f.region(10, 30, 60);
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(72),
            instrument: Some(InstrumentId(2)),
            ..Default::default()
        },
    );
    let pcm = f.pcm(512);
    for frame in 0..10 {
        assert_close(pcm[frame * 2], value(10 + frame as u32 * 2, 0) * 0.5);
    }
    assert!(pcm[20..].iter().all(|v| *v == 0.0));
}
#[test]
fn region_cut_isolates_voices_and_clipping_cleans_both_output_files() {
    let mut f = Fixture::new(48000, 1, 1024);
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    f.region(10, 900, 60);
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            effects: StepEffects {
                cut: Some(0.5),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(
        0,
        1,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            ..Default::default()
        },
    );
    let pcm = f.pcm(7);
    for frame in 0..50 {
        assert_close(
            pcm[frame * 2],
            value(10 + frame as u32, 0) * if frame < 14 { 1.0 } else { 0.5 },
        );
    }
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 64;
    for column in 0..64 {
        f.put(
            0,
            column,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                ..Default::default()
            },
        );
    }
    let path = f.dir.join("clipped.wav");
    assert!(song_render::export(&f.song, None, &path)
        .unwrap_err()
        .contains("clipping"));
    assert!(!path.exists());
    assert!(!path.with_extension("events.json").exists());
}

#[test]
fn loaded_engine_owns_immutable_pcm_snapshot_when_external_source_bytes_change() {
    let f = Fixture::new(48000, 2, 1024);
    let expected = f.pcm(512);
    let engine = Engine::new(&f.song, None).unwrap();
    std::fs::write(f.dir.join("source.wav"), b"external replacement").unwrap();
    assert_eq!(render(engine, 512), expected);
    assert!(Engine::new(&f.song, None)
        .err()
        .unwrap()
        .contains("SHA-256"));
}

#[test]
fn fractional_native_rate_exhausts_exactly_without_offset_revival_or_partition_drift() {
    for (source_frames, bpm, lpb, end) in [
        (294u64, 300.0, 30, 320usize),
        (441, 300.0, 20, 480),
        (44100, 60.0, 1, 48000),
    ] {
        for start in [0, 17] {
            for reverse in [false, true] {
                let mut f = Fixture::new(44100, 1, source_frames as u32 + 100);
                f.region(start, start + source_frames, 60);
                f.song.bpm = bpm;
                f.song.lines_per_beat = lpb;
                f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 2;
                f.put(
                    0,
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(2)),
                        effects: StepEffects {
                            direction: reverse.then_some(Direction::Reverse),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                );
                f.put(
                    1,
                    0,
                    Cell {
                        effects: StepEffects {
                            offset: Some(start),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                );
                let manifest = song_render::evaluate(&f.song, None).unwrap();
                assert!(manifest.events.iter().any(|event| event.frame == end as u64
                    && matches!(event.command, Command::Offset { .. })));
                let large = f.pcm(512);
                assert_eq!(large, f.pcm(37));
                assert_eq!(large, f.pcm(1));
                let audible = if reverse {
                    ((source_frames - 1) * 48000 / 44100 + 1) as usize
                } else {
                    end
                };
                assert!(large[..audible * 2].iter().all(|sample| *sample > 0.0));
                assert!(large[audible * 2..].iter().all(|sample| *sample == 0.0), "source frames {source_frames}, start {start}, reverse {reverse} revived at output {end}");
            }
        }
    }
    // Reverse reaching the first real frame exactly must still render it once,
    // not epsilon-snap to exhaustion before that last legitimate sample.
    let mut f = Fixture::new(44100, 1, 512);
    f.region(17, 312, 60); // 295 frames: reverse reaches frame17 at output320.
    f.song.bpm = 48000.0 * 60.0 / (30.0 * 321.0);
    f.song.lines_per_beat = 30;
    f.put(
        0,
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(2)),
            effects: StepEffects {
                direction: Some(Direction::Reverse),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                offset: Some(17),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let pcm = f.pcm(37);
    assert_close(pcm[320 * 2], value(17, 0) * 0.5);
    assert!(pcm[321 * 2..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn legacy_fractional_rate_offset_voices_also_exhaust_without_boundary_revival() {
    for (frames, bpm, lpb, end) in [
        (294u32, 300.0, 30, 320usize),
        (441, 300.0, 20, 480),
        (44100, 60.0, 1, 48000),
    ] {
        let mut f = Fixture::new(44100, 1, frames);
        f.song.assign_region(InstrumentId(2), None).unwrap();
        f.song.instruments.get_mut(&InstrumentId(2)).unwrap().sample =
            Some(f.dir.join("source.wav"));
        f.song.bpm = bpm;
        f.song.lines_per_beat = lpb;
        f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 2;
        f.put(
            0,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(2)),
                effects: StepEffects {
                    offset: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        f.put(
            1,
            0,
            Cell {
                effects: StepEffects {
                    offset: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let pcm = f.pcm(512);
        assert_eq!(pcm, f.pcm(37));
        assert_eq!(pcm, f.pcm(1));
        assert!(pcm[..end * 2].iter().all(|value| *value > 0.0));
        assert!(pcm[end * 2..].iter().all(|value| *value == 0.0));
    }
}
