use aqua_pattern_core::{
    generation::*,
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
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-generation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("source.wav");
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 44100,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..88200 {
            w.write_sample((i % 101) as f32 * 0.0001).unwrap();
            w.write_sample(-0.002f32).unwrap();
        }
        w.finalize().unwrap();
        let mut song = example_song();
        song.order.clear();
        song.patterns.clear();
        song.register_sample(SampleId(1), sample_asset::import(&path).unwrap().asset)
            .unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: "Native".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 88200,
                root_note: 60,
            },
        )
        .unwrap();
        for instrument in song.instruments.values_mut() {
            instrument.region = Some(RegionId(1));
            instrument.sample = None;
        }
        Self { dir, song }
    }
    fn prefix(&mut self, rows: u32) {
        self.song.patterns.insert(
            PatternId(9),
            Pattern {
                generation: None,
                name: "Existing".into(),
                rows,
                tracks: BTreeMap::new(),
                track_aliases: BTreeMap::new(),
            },
        );
        self.song.order.push(Occurrence {
            pattern: PatternId(9),
            muted_tracks: Default::default(),
            section: None,
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn config() -> GenerationConfig {
    GenerationConfig {
        version: VERSION,
        name: "Euclidean".into(),
        bars: 2,
        instrument: InstrumentId(1),
        seed: 0,
        preset: GenerationPreset::Euclidean {
            steps: 16,
            pulses: 5,
            rotation: 0,
            probability: 1.0,
        },
        sequence: vec![
            GenerationNote {
                note: 60,
                velocity: 0.3,
            },
            GenerationNote {
                note: 67,
                velocity: 0.2,
            },
        ],
    }
}
fn pcm(song: &Song, block: usize) -> Vec<f32> {
    let mut engine = Engine::new(song, None).unwrap();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut out = Vec::new();
    let mut buffer = vec![0.0; block * 2];
    while engine.position < end {
        let n = engine.block(&mut buffer, end).unwrap();
        out.extend_from_slice(&buffer[..n * 2]);
    }
    out
}
#[test]
fn real_pattrns_stream_and_sequence_match_direct_pinned_execution() {
    use pattrns::prelude::*;
    let f = Fixture::new();
    let mut c = config();
    c.bars = 3;
    c.seed = u64::MAX;
    c.preset = GenerationPreset::Euclidean {
        steps: 16,
        pulses: 9,
        rotation: 3,
        probability: 0.55,
    };
    let p = preview(&f.song, c.clone()).unwrap();
    let rhythm: Vec<f32> = euclidean::euclidean(9, 16, 3)
        .into_iter()
        .map(|v| if v { 0.55 } else { 0.0 })
        .collect();
    let sequence = c
        .sequence
        .iter()
        .map(|n| new_note((pattrns::Note::from(n.note - 12), None, n.velocity, 0.0)))
        .collect();
    let mut raw = BeatTimeBase {
        beats_per_min: 60.0,
        beats_per_bar: 4,
        samples_per_sec: 1024,
    }
    .every_nth_beat(1.0)
    .with_rhythm(rhythm.to_rhythm())
    .with_gate(ProbabilityGate::new(Some(u64::MAX)))
    .emit(new_note_sequence_emitter(sequence));
    let collect = |raw: &mut dyn pattrns::Pattern| {
        let mut ticks = Vec::new();
        let mut hits = Vec::new();
        while let Some(e) = raw.run_until_time(48 * 1024) {
            let note = match e.event {
                Some(pattrns::Event::NoteEvents(n)) => {
                    let n = n[0].as_ref().unwrap();
                    hits.push((e.time / 1024, n.note as u8 + 12, n.volume));
                    Some(
                        serde_json::json!({"note":n.note as u8,"volume":n.volume,"panning":n.panning,"delay":n.delay}),
                    )
                }
                None => None,
                _ => panic!(),
            };
            ticks.push(serde_json::json!({"time":e.time,"duration":e.duration,"note":note}));
        }
        (ticks, hits)
    };
    let (ticks, hits) = collect(&mut raw);
    raw.reset();
    assert_eq!(collect(&mut raw), (ticks.clone(), hits.clone()));
    assert_eq!(ticks.len(), 48);
    assert_eq!(
        p.cells()
            .iter()
            .map(|v| (
                u64::from(v.step),
                match v.cell.note {
                    aqua_pattern_core::song::Note::Note(n) => n,
                    _ => panic!(),
                },
                v.cell.velocity.unwrap()
            ))
            .collect::<Vec<_>>(),
        hits
    );
    assert_eq!(
        p.provenance().pattrns_revision,
        aqua_pattern_core::pattern::REVISION
    );
}
#[test]
fn seed_reproducibility_and_variation_have_versioned_raw_provenance() {
    let f = Fixture::new();
    let mut c = config();
    c.bars = 4;
    c.preset = GenerationPreset::Euclidean {
        steps: 32,
        pulses: 24,
        rotation: 0,
        probability: 0.5,
    };
    let a = preview(&f.song, c.clone()).unwrap();
    let b = preview(&f.song, c.clone()).unwrap();
    assert_eq!(a.id(), b.id());
    assert_eq!(a.candidate(), b.candidate());
    assert_eq!(a.cells(), b.cells());
    c.seed = u64::MAX;
    let other = preview(&f.song, c).unwrap();
    assert_ne!(a.summary().raw_sha256, other.summary().raw_sha256);
    assert_ne!(a.cells(), other.cells());
    assert_eq!(
        a.summary().raw_sha256,
        "b493ea7ec3ea8296ec060f9ce8e3ec205efddcc9cd9d269f167b9d8a53244c27"
    );
    assert_eq!(
        other.summary().raw_sha256,
        "5070930966c6bf5f924e825d952a62a7f51d8eb14835cea7587e3e70e06de870"
    );
    assert_eq!(a.provenance().gate_algorithm, GATE_ALGORITHM);
}
#[test]
fn dense_steps_allocate_columns_without_collisions_or_lost_hits() {
    let mut f = Fixture::new();
    f.song.lines_per_beat = 1;
    let mut c = config();
    c.bars = 1;
    c.preset = GenerationPreset::Euclidean {
        steps: 128,
        pulses: 128,
        rotation: 0,
        probability: 1.0,
    };
    let p = preview(&f.song, c).unwrap();
    assert_eq!(p.summary().hits, 128);
    assert_eq!(p.summary().columns, 32);
    assert_eq!(p.summary().rows, 4);
    for (i, cell) in p.cells().iter().enumerate() {
        assert_eq!(cell.row, (i / 32) as u32);
        assert_eq!(cell.column, (i % 32) as u16);
        assert_eq!(cell.frame, (i as u64 * 48000 * 60) / (120 * 32));
        assert_eq!(cell.cell.instrument, Some(InstrumentId(1)));
        assert_eq!(cell.cell.pan, Some(0.0));
    }
}
#[test]
fn exact_integer_clock_prefixes_nondivisible_steps_and_json_roundtrip() {
    let mut f = Fixture::new();
    for bpm in [70u64, 137, 170, 399] {
        for lpb in [1u16, 3, 7, 128] {
            for steps in [7u16, 16, 128] {
                f.song.bpm = bpm as f64;
                f.song.lines_per_beat = lpb;
                f.song.order.clear();
                f.song.patterns.clear();
                f.prefix(17);
                let mut c = config();
                c.bars = 1;
                c.preset = GenerationPreset::Euclidean {
                    steps,
                    pulses: steps,
                    rotation: 0,
                    probability: 1.0,
                };
                let p = preview(&f.song, c).unwrap();
                for cell in p.cells() {
                    let n = (17u64 * u64::from(steps) + u64::from(cell.step) * 4 * u64::from(lpb))
                        * 48000
                        * 60;
                    let d = u64::from(lpb) * u64::from(steps) * bpm;
                    assert_eq!(cell.frame, (n + d / 2) / d);
                }
                let restored: Song =
                    serde_json::from_slice(&serde_json::to_vec(p.candidate()).unwrap()).unwrap();
                assert_eq!(&restored, p.candidate());
                assert_eq!(
                    song_render::evaluate(&restored, None).unwrap(),
                    song_render::evaluate(p.candidate(), None).unwrap()
                );
            }
        }
    }
}
#[test]
fn fresh_ids_append_only_and_stale_base_reject() {
    let mut f = Fixture::new();
    f.prefix(7);
    let before = f.song.clone();
    let p = preview(&f.song, config()).unwrap();
    assert_eq!(f.song, before);
    let after = p.commit(&before).unwrap();
    assert_eq!(&after.order[..before.order.len()], &before.order);
    assert_eq!(
        after.patterns[&PatternId(9)],
        before.patterns[&PatternId(9)]
    );
    assert_eq!(after.bpm, before.bpm);
    assert_eq!(after.lines_per_beat, before.lines_per_beat);
    assert_eq!(p.summary().pattern, PatternId(10));
    let mut changed = before.clone();
    changed.title.push('x');
    assert!(p.commit(&changed).is_err());
    f.song
        .patterns
        .insert(PatternId(u64::MAX), f.song.patterns[&PatternId(9)].clone());
    assert!(preview(&f.song, config()).unwrap_err().contains("ID"));
}
#[test]
fn provenance_is_historical_ordinary_edit_clone_and_source_removal() {
    let f = Fixture::new();
    let p = preview(&f.song, config()).unwrap();
    let id = p.summary().pattern;
    let mut song = p.candidate().clone();
    assert_eq!(content_matches(&song.patterns[&id]).unwrap(), Some(true));
    let history = song.patterns[&id].generation.clone();
    song.clone_pattern(id, PatternId(id.0 + 1)).unwrap();
    assert_eq!(
        content_matches(&song.patterns[&PatternId(id.0 + 1)]).unwrap(),
        Some(true)
    );
    song.patterns.get_mut(&id).unwrap().tracks.clear();
    song.patterns
        .get_mut(&PatternId(id.0 + 1))
        .unwrap()
        .tracks
        .clear();
    song.instruments.remove(&InstrumentId(1));
    song.validate().unwrap();
    assert_eq!(song.patterns[&id].generation, history);
    assert_eq!(content_matches(&song.patterns[&id]).unwrap(), Some(false));
    let text = serde_json::to_string(&song).unwrap();
    assert_eq!(serde_json::from_str::<Song>(&text).unwrap(), song);
}
#[test]
fn invalid_and_empty_results_reject_without_mutation() {
    let f = Fixture::new();
    let before = f.song.clone();
    let mut bad = Vec::new();
    let mut c = config();
    c.version = 2;
    bad.push(c);
    let mut c = config();
    c.bars = 0;
    bad.push(c);
    let mut c = config();
    c.bars = 17;
    bad.push(c);
    let mut c = config();
    c.instrument = InstrumentId(0);
    bad.push(c);
    let mut c = config();
    c.sequence.clear();
    bad.push(c);
    let mut c = config();
    c.sequence = vec![c.sequence[0].clone(); 17];
    bad.push(c);
    let mut c = config();
    c.name = " ".into();
    bad.push(c);
    let mut c = config();
    c.name = "x".repeat(257);
    bad.push(c);
    for c in bad {
        assert!(preview(&f.song, c).is_err());
    }
    let mut c = config();
    c.preset = GenerationPreset::Euclidean {
        steps: 16,
        pulses: 0,
        rotation: 0,
        probability: 1.0,
    };
    assert!(preview(&f.song, c).is_err());
    let mut c = config();
    c.preset = GenerationPreset::Euclidean {
        steps: 16,
        pulses: 4,
        rotation: 16,
        probability: 1.0,
    };
    assert!(preview(&f.song, c).is_err());
    let mut c = config();
    c.preset = GenerationPreset::Euclidean {
        steps: 16,
        pulses: 4,
        rotation: 0,
        probability: 0.0,
    };
    assert!(preview(&f.song, c).unwrap_err().contains("no hits"));
    for note in [0, 11, 128, 255] {
        let mut c = config();
        c.sequence[0].note = note;
        assert!(preview(&f.song, c).is_err());
    }
    for scale in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let mut c = config();
        c.sequence[0].velocity = scale;
        assert!(preview(&f.song, c).is_err());
    }
    assert_eq!(f.song, before);
}
#[test]
fn pure_preview_does_not_read_missing_source_and_aggregate_starts_are_bounded() {
    let mut f = Fixture::new();
    std::fs::remove_file(f.dir.join("source.wav")).unwrap();
    assert!(preview(&f.song, config()).is_ok());
    let mut c = config();
    c.bars = 16;
    c.preset = GenerationPreset::Euclidean {
        steps: 128,
        pulses: 128,
        rotation: 0,
        probability: 1.0,
    };
    let full = preview(&f.song, c.clone()).unwrap();
    assert_eq!(full.summary().hits, 2048);
    f.prefix(1);
    f.song
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(0)
        .or_default()
        .insert(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        );
    assert!(preview(&f.song, c).unwrap_err().contains("2048"));
}
#[test]
fn native_audio_matches_ordinary_cells_without_provenance_and_zero_starts_are_silent() {
    let f = Fixture::new();
    let mut c = config();
    c.bars = 1;
    let p = preview(&f.song, c.clone()).unwrap();
    let a = pcm(p.candidate(), 512);
    let mut ordinary = p.candidate().clone();
    ordinary
        .patterns
        .get_mut(&p.summary().pattern)
        .unwrap()
        .generation = None;
    assert_eq!(a, pcm(&ordinary, 512));
    assert!(a.iter().any(|x| *x != 0.0));
    assert_eq!(a, pcm(p.candidate(), 37));
    for n in &mut c.sequence {
        n.velocity = 0.0;
    }
    let zero = preview(&f.song, c).unwrap();
    assert!(pcm(zero.candidate(), 512).iter().all(|x| *x == 0.0));
}
#[test]
fn kit_choices_are_explicit_selectors_not_transposed_and_all_choices_validate() {
    let mut f = Fixture::new();
    f.song
        .upsert_drum_kit(
            DrumKitId(1),
            DrumKitInstrument {
                name: "Kit".into(),
                pads: BTreeMap::new(),
            },
        )
        .unwrap();
    for (id, note, tune) in [(1, 36, 0), (2, 127, 12)] {
        f.song
            .upsert_drum_pad(
                DrumKitId(1),
                PadId(id),
                DrumPad {
                    velocity_zones: Vec::new(),
                    slot: (id - 1) as u8,
                    name: format!("Pad{id}"),
                    note,
                    region: Some(RegionId(1)),
                    root_note: 60,
                    tune_semitones: tune,
                    gain: 1.0,
                    pan: 0.0,
                    mode: PadMode::OneShot,
                    new_note: PadNewNote::Overlap,
                    choke_group: None,
                },
            )
            .unwrap();
    }
    f.song
        .assign_kit(InstrumentId(1), Some(DrumKitId(1)))
        .unwrap();
    let mut c = config();
    c.bars = 1;
    c.sequence = vec![
        GenerationNote {
            note: 36,
            velocity: 0.3,
        },
        GenerationNote {
            note: 127,
            velocity: 0.2,
        },
    ];
    let p = preview(&f.song, c.clone()).unwrap();
    let m = song_render::evaluate(p.candidate(), None).unwrap();
    let notes: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| match e.command {
            Command::On { pitch, .. } => Some(pitch),
            _ => None,
        })
        .collect();
    assert_eq!(&notes[..4], &[60, 72, 60, 72]);
    assert!(pcm(p.candidate(), 512).iter().any(|x| *x != 0.0));
    c.sequence[1].note = 37;
    assert!(preview(&f.song, c).unwrap_err().contains("selector"));
}
#[test]
fn extension_of_song_end_preserves_content_but_can_extend_old_held_voice() {
    let mut f = Fixture::new();
    f.prefix(1);
    f.song
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        ..Default::default()
                    },
                )]),
            )]),
        );
    let old = song_render::evaluate(&f.song, None).unwrap();
    let p = preview(&f.song, config()).unwrap();
    let new = song_render::evaluate(p.candidate(), None).unwrap();
    assert!(new.musical_frames > old.musical_frames);
    assert!(p
        .summary()
        .warnings
        .iter()
        .any(|w| w.contains("Extending SongEnd")));
    let old_off = old
        .events
        .iter()
        .find(|e| e.track == TrackId(1) && matches!(e.command, Command::Off { .. }))
        .unwrap();
    let new_off = new
        .events
        .iter()
        .find(|e| e.track == TrackId(1) && matches!(e.command, Command::Off { .. }))
        .unwrap();
    assert_eq!(old_off.frame, old.musical_frames);
    assert_eq!(new_off.frame, new.musical_frames);
    assert_eq!(
        p.candidate().patterns[&PatternId(9)],
        f.song.patterns[&PatternId(9)]
    );
}

#[test]
fn probability_one_ignores_seed_and_min_max_bridge_pitches_are_exact() {
    let f = Fixture::new();
    let mut c = config();
    c.bars = 1;
    c.sequence = vec![
        GenerationNote {
            note: 12,
            velocity: 0.0,
        },
        GenerationNote {
            note: 127,
            velocity: 1.0,
        },
    ];
    let a = preview(&f.song, c.clone()).unwrap();
    c.seed = u64::MAX;
    let b = preview(&f.song, c).unwrap();
    assert_eq!(a.cells(), b.cells());
    assert_eq!(a.summary().raw_sha256, b.summary().raw_sha256);
    assert_ne!(a.id(), b.id());
    assert_eq!(a.cells()[0].cell.note, Note::Note(12));
    assert_eq!(a.cells()[1].cell.note, Note::Note(127));
}
#[test]
fn malformed_strict_config_history_and_checked_track_ids_reject() {
    let mut f = Fixture::new();
    let mut value = serde_json::to_value(config()).unwrap();
    value["seed_code"] = serde_json::json!("unsafe");
    assert!(serde_json::from_value::<GenerationConfig>(value).is_err());
    let mut value = serde_json::to_value(config()).unwrap();
    value["preset"]["Euclidean"]["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<GenerationConfig>(value).is_err());
    let p = preview(&f.song, config()).unwrap();
    let mut invalid = p.candidate().clone();
    invalid
        .patterns
        .get_mut(&p.summary().pattern)
        .unwrap()
        .generation
        .as_mut()
        .unwrap()
        .raw_sha256 = "bad".into();
    assert!(invalid.validate().is_err());
    f.song.tracks.insert(
        TrackId(u64::MAX),
        Track {
            name: "Last".into(),
            note_columns: 1,
        },
    );
    f.song.track_order.push(TrackId(u64::MAX));
    assert!(preview(&f.song, config()).unwrap_err().contains("ID"));
}
#[test]
fn pure_relative_sources_and_unassigned_existing_content_are_explicit() {
    let mut f = Fixture::new();
    f.song.samples.get_mut(&SampleId(1)).unwrap().path = "not/read/relative.wav".into();
    let p = preview(&f.song, config()).unwrap();
    assert!(song_render::evaluate(p.candidate(), None).is_err());
    f.prefix(1);
    f.song.instruments.get_mut(&InstrumentId(2)).unwrap().region = None;
    f.song
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(2)),
                        ..Default::default()
                    },
                )]),
            )]),
        );
    assert!(preview(&f.song, config())
        .unwrap_err()
        .contains("no sample assigned"));
}
#[test]
fn exported_audio_is_reproducible_after_ordinary_project_json_reopen() {
    let f = Fixture::new();
    let p = preview(&f.song, config()).unwrap();
    let restored: Song =
        serde_json::from_slice(&serde_json::to_vec(p.candidate()).unwrap()).unwrap();
    let a = f.dir.join("a.wav");
    let b = f.dir.join("b.wav");
    let (_, ma) = song_render::export(p.candidate(), None, &a).unwrap();
    let (_, mb) = song_render::export(&restored, None, &b).unwrap();
    assert_eq!(ma, mb);
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
}

#[test]
fn append_can_untruncate_old_phrase_children_without_changing_its_definition() {
    let mut f = Fixture::new();
    f.prefix(1);
    let cells = BTreeMap::from([
        (
            0,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(60),
                    ..Default::default()
                },
            )]),
        ),
        (
            2,
            BTreeMap::from([(
                0,
                Cell {
                    note: Note::Note(67),
                    ..Default::default()
                },
            )]),
        ),
    ]);
    f.song
        .upsert_phrase(
            PhraseId(1),
            Phrase {
                name: "Long".into(),
                instrument: InstrumentId(2),
                rows: 4,
                lines_per_beat: 4,
                note_columns: 1,
                root_note: 60,
                cells,
            },
        )
        .unwrap();
    f.song
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        note: Note::Note(60),
                        phrase: Some(PhraseTrigger {
                            phrase: PhraseId(1),
                            cycles: 1,
                        }),
                        ..Default::default()
                    },
                )]),
            )]),
        );
    let old = song_render::evaluate(&f.song, None).unwrap();
    assert_eq!(
        old.events
            .iter()
            .filter(|e| e.track == TrackId(1) && matches!(e.command, Command::On { .. }))
            .count(),
        1
    );
    let p = preview(&f.song, config()).unwrap();
    let new = song_render::evaluate(p.candidate(), None).unwrap();
    assert_eq!(
        new.events
            .iter()
            .filter(|e| e.track == TrackId(1) && matches!(e.command, Command::On { .. }))
            .count(),
        2
    );
    assert_eq!(p.candidate().phrases, f.song.phrases);
    assert!(p
        .summary()
        .warnings
        .iter()
        .any(|w| w.contains("phrase child")));
}

#[test]
fn stored_f64_decimal_bpm_frames_use_one_exact_rational_round() {
    let mut f = Fixture::new();
    for bpm in [137.123456789, 399.72241498959056, 20.000000000000004] {
        for lpb in [3u16, 7, 256] {
            f.song.bpm = bpm;
            f.song.lines_per_beat = lpb;
            f.song.patterns.clear();
            f.song.order.clear();
            f.prefix(101);
            let mut c = config();
            c.bars = 1;
            c.preset = GenerationPreset::Euclidean {
                steps: 127,
                pulses: 5,
                rotation: 126,
                probability: 1.0,
            };
            let p = preview(&f.song, c).unwrap();
            let bits = bpm.to_bits();
            let significand = u128::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52));
            let shift = 1075 - ((bits >> 52) & 0x7ff);
            for cell in p.cells() {
                let n = (101u128 * 127 + u128::from(cell.step) * 4 * u128::from(lpb))
                    * 2_880_000
                    * (1u128 << shift);
                let d = u128::from(lpb) * 127 * significand;
                let expected = n / d + u128::from(n % d >= d - n % d);
                assert_eq!(u128::from(cell.frame), expected);
            }
        }
    }
}

#[test]
fn pretty_project_budget_rejects_dense_append_before_candidate_publish() {
    let mut f = Fixture::new();
    f.song.title = "x".repeat(7_900_000);
    assert!(aqua_pattern_core::project::check_serialized_size(&f.song).unwrap() < 8 * 1024 * 1024);
    let before = f.song.clone();
    let mut c = config();
    c.bars = 16;
    c.preset = GenerationPreset::Euclidean {
        steps: 128,
        pulses: 128,
        rotation: 0,
        probability: 1.0,
    };
    let error = preview(&f.song, c).unwrap_err();
    assert!(error.contains("8 MiB"), "{error}");
    assert_eq!(f.song, before);
    // A near-limit sparse append is genuinely saveable with the actual wrapper.
    let mut c = config();
    c.bars = 1;
    c.preset = GenerationPreset::Euclidean {
        steps: 1,
        pulses: 1,
        rotation: 0,
        probability: 1.0,
    };
    let p = preview(&f.song, c).unwrap();
    let estimate = aqua_pattern_core::project::check_serialized_size(p.candidate()).unwrap();
    let path = f.dir.join("near-limit.json");
    aqua_pattern_core::project::save(
        p.candidate(),
        &path,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    assert!(std::fs::metadata(path).unwrap().len() <= estimate as u64);
}

#[test]
fn ordered_context_budget_counts_empty_blocks_before_validation() {
    let mut f = Fixture::new();
    f.song.lines_per_beat = 256;
    f.prefix(1);
    for id in 5..=255 {
        f.song.tracks.insert(
            TrackId(id),
            Track {
                name: String::new(),
                note_columns: 1,
            },
        );
        f.song.track_order.push(TrackId(id));
    }
    f.song.order = vec![f.song.order[0].clone(); 255];
    let mut c = config();
    c.bars = 1;
    c.preset = GenerationPreset::Euclidean {
        steps: 1,
        pulses: 1,
        rotation: 0,
        probability: 1.0,
    };
    f.song.validate().unwrap();
    assert!((f.song.order.len() + 1) * (f.song.track_order.len() + 1) == MAX_VISITS);
    assert!(preview(&f.song, c.clone()).is_ok());
    f.song.tracks.insert(
        TrackId(256),
        Track {
            name: String::new(),
            note_columns: 1,
        },
    );
    f.song.track_order.push(TrackId(256));
    f.song.drum_kits.insert(
        DrumKitId(1),
        DrumKitInstrument {
            name: "Unused".into(),
            pads: BTreeMap::new(),
        },
    );
    f.song.instruments.get_mut(&InstrumentId(4)).unwrap().kind =
        InstrumentKind::DrumKit { kit: DrumKitId(1) };
    f.song.instruments.get_mut(&InstrumentId(4)).unwrap().region = None;
    f.song.validate().unwrap();
    let before = f.song.clone();
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("ordered track blocks"));
    assert_eq!(f.song, before);
    // Guard precedes full validation, not a late check after expensive work.
    f.song.track_order.push(TrackId(999999));
    assert!(preview(&f.song, c)
        .unwrap_err()
        .contains("ordered track blocks"));
}
#[test]
fn ordered_empty_rows_and_default_cells_have_independent_exact_work_caps() {
    let mut f = Fixture::new();
    f.song.lines_per_beat = 256;
    f.prefix(257);
    f.song
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            (0..257).map(|row| (row, BTreeMap::new())).collect(),
        );
    f.song.order = vec![f.song.order[0].clone(); 255];
    let mut c = config();
    c.bars = 1;
    c.preset = GenerationPreset::Euclidean {
        steps: 1,
        pulses: 1,
        rotation: 0,
        probability: 1.0,
    };
    // 255*257 + one generated nonempty row == 65536, with no phantom notes.
    assert!(preview(&f.song, c.clone()).is_ok());
    f.song.order.push(f.song.order[0].clone());
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("row-container visits"));
    // Defaults are explicitly stored cells, not free work. 257*255+1==65536.
    f.song.patterns.get_mut(&PatternId(9)).unwrap().rows = 1;
    f.song
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                (0..255).map(|column| (column, Cell::default())).collect(),
            )]),
        );
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 255;
    f.song.order = vec![f.song.order[0].clone(); 257];
    assert!(preview(&f.song, c.clone()).is_ok());
    f.song.order.push(f.song.order[0].clone());
    assert!(preview(&f.song, c)
        .unwrap_err()
        .contains("ordered cell visits"));
}

#[test]
fn prevalidation_work_counts_reject_bad_references_without_panicking() {
    let mut f = Fixture::new();
    f.prefix(1);
    let mut invalid = f.song.clone();
    invalid.order[0].pattern = PatternId(999);
    assert!(preview(&invalid, config()).is_err());
    let mut invalid = f.song.clone();
    invalid
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .tracks
        .insert(TrackId(999), BTreeMap::new());
    assert!(preview(&invalid, config()).is_err());
    let mut invalid = f.song.clone();
    invalid
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(999),
                track: TrackId(1),
            },
        );
    assert!(preview(&invalid, config()).is_err());
    let mut invalid = f.song.clone();
    invalid
        .patterns
        .get_mut(&PatternId(9))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(9),
                track: TrackId(1),
            },
        );
    assert!(preview(&invalid, config()).unwrap_err().contains("cycle"));
}
