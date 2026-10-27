use aqua_pattern_core::{
    break_compiler::*,
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
    fn new(rate: u32, markers: Vec<u64>, source_bpm: Option<f64>) -> Self {
        Self::with_signal(rate, markers, source_bpm, |_, slice| {
            0.1 + slice as f32 * 0.01
        })
    }
    fn with_signal(
        rate: u32,
        markers: Vec<u64>,
        source_bpm: Option<f64>,
        signal: impl Fn(u64, usize) -> f32,
    ) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-compiler-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let frames = *markers.last().unwrap();
        let mut writer = hound::WavWriter::create(
            dir.join("source.wav"),
            hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        let mut slice = 0;
        for frame in 0..frames {
            while slice + 1 < markers.len() - 1 && frame >= markers[slice + 1] {
                slice += 1;
            }
            writer.write_sample(signal(frame, slice)).unwrap();
        }
        writer.finalize().unwrap();
        let mut song = example_song();
        song.patterns.clear();
        song.order.clear();
        song.register_sample(
            SampleId(1),
            sample_asset::import(&dir.join("source.wav")).unwrap().asset,
        )
        .unwrap();
        song.upsert_marker_set(
            MarkerSetId(1),
            MarkerSet {
                name: "Accepted boundaries".into(),
                locked_frames: Vec::new(),
                sample: SampleId(1),
                source_bpm,
                markers,
            },
        )
        .unwrap();
        Self { dir, song }
    }
    fn config(&self) -> CompileConfig {
        CompileConfig {
            marker_set: MarkerSetId(1),
            target_bpm: 170.0,
            timing: CompileTiming::SourceRhythm,
            cycles: 1,
            roll: None,
            tail: TailPolicy::Overlap,
            minimum_slice_frames: 1,
            root_note: 60,
            pad_tails: false,
            name: "Compiled".into(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn audio(song: &Song) -> Vec<f32> {
    let mut engine = Engine::new(song, None).unwrap();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut pcm = Vec::new();
    while engine.position < end {
        let mut block = [0.0; 1024];
        let count = engine.block(&mut block, end).unwrap();
        pcm.extend_from_slice(&block[..count * 2]);
    }
    pcm
}
fn starts(song: &Song) -> Vec<u64> {
    song_render::evaluate(song, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| matches!(event.command, Command::On { .. }).then_some(event.frame))
        .collect()
}
#[test]
fn source_70_to_target_170_compiles_exact_clock_and_ordinary_editable_content() {
    let drum = |frame: u64, slice: usize| {
        let local = (frame % 37800) as f32;
        0.4 * (-local / (44100.0 * 0.04)).exp()
            * (std::f32::consts::TAU * (90.0 + slice as f32 * 20.0) * local / 44100.0).sin()
    };
    let f = Fixture::with_signal(
        44100,
        vec![0, 37800, 75600, 113400, 151200],
        Some(70.0),
        drum,
    );
    let before = serde_json::to_vec(&f.song).unwrap();
    let config = f.config();
    let plan = preview(&f.song, &config).unwrap();
    assert_eq!(serde_json::to_vec(&f.song).unwrap(), before);
    assert_eq!(plan.id(), preview(&f.song, &config).unwrap().id());
    assert_eq!(
        plan.candidate(),
        preview(&f.song, &config).unwrap().candidate()
    );
    assert_eq!(starts(plan.candidate()), [0, 16941, 33882, 50824]);
    assert_eq!(
        song_render::evaluate(plan.candidate(), None)
            .unwrap()
            .musical_frames,
        67765
    );
    assert_eq!(plan.summary().sequence_rows, 16);
    assert_eq!(plan.summary().tail_padding_rows, 0);
    assert_eq!(plan.summary().slice_count, 4);
    assert!(plan
        .summary()
        .instruments
        .iter()
        .all(|id| id.0 > f.song.instruments.keys().next_back().unwrap().0));
    assert_eq!(plan.commit(&f.song).unwrap(), *plan.candidate());
    let cells = &plan.candidate().patterns[&plan.summary().pattern].tracks[&plan.summary().track];
    assert_eq!(cells.values().map(BTreeMap::len).sum::<usize>(), 4);
    assert!(cells
        .values()
        .flat_map(|row| row.values())
        .all(|cell| matches!(cell.note, Note::Note(60)) && cell.phrase.is_none()));
    let pcm = audio(plan.candidate());
    for frame in 0..16000 {
        let position = frame as f64 * 44100.0 / 48000.0;
        let lower = position.floor() as u64;
        let fraction = (position - lower as f64) as f32;
        let a = drum(lower, 0);
        let b = drum(lower + 1, 0);
        assert!((pcm[frame * 2] - (a + (b - a) * fraction) * 0.5).abs() < 1e-7);
    }
    let mut padded = config;
    padded.pad_tails = true;
    let padded = preview(&f.song, &padded).unwrap();
    assert_eq!(padded.summary().sequence_rows, 16);
    assert_eq!(padded.summary().pattern_rows, 22);
    assert_eq!(padded.summary().tail_padding_rows, 6);
    assert_eq!(starts(padded.candidate()), [0, 16941, 33882, 50824]);
}
#[test]
fn cleanup_is_a_review_only_proposal_and_short_prefix_requires_explicit_acceptance() {
    let f = Fixture::new(44100, vec![0, 16, 1000, 5000], Some(70.0));
    let before = f.song.clone();
    let source = &f.song.samples[&SampleId(1)];
    let cleanup = suggest_marker_cleanup(source, &[16, 1000, 1100, 4999, 1000], 221).unwrap();
    assert_eq!(cleanup.markers, [0, 1000, 5000]);
    assert!(cleanup.removed.contains(&RemovedMarker {
        frame: 16,
        reason: CleanupReason::NearStart
    }));
    assert!(cleanup.removed.contains(&RemovedMarker {
        frame: 4999,
        reason: CleanupReason::NearEnd
    }));
    assert_eq!(f.song, before);
    let mut config = f.config();
    config.minimum_slice_frames = 221;
    assert!(preview(&f.song, &config).unwrap_err().contains("shorter"));
    let mut accepted = f.song.clone();
    accepted
        .marker_sets
        .get_mut(&MarkerSetId(1))
        .unwrap()
        .markers = cleanup.markers;
    assert!(preview(&accepted, &config).is_ok());
    config.minimum_slice_frames = 1;
    assert_eq!(preview(&f.song, &config).unwrap().summary().slice_count, 3);
    assert!(suggest_marker_cleanup(source, &[5001], 1).is_err());
    assert!(suggest_marker_cleanup(source, &[], 0).is_err());
    assert!(suggest_marker_cleanup(source, &[], 5001).is_err());
}
#[test]
fn source_bpm_is_explicit_unknown_rejected_and_grid_is_distinct_from_source_rhythm() {
    let mut f = Fixture::new(48000, vec![0, 12000, 36000, 48000], None);
    let mut config = f.config();
    assert!(preview(&f.song, &config)
        .unwrap_err()
        .contains("explicit source BPM"));
    f.song
        .marker_sets
        .get_mut(&MarkerSetId(1))
        .unwrap()
        .source_bpm = Some(120.0);
    let rhythm = preview(&f.song, &config).unwrap();
    assert_eq!(starts(rhythm.candidate()), [0, 8471, 25412]);
    config.timing = CompileTiming::Grid {
        division: Subdivision::Sixteenth,
    };
    let grid = preview(&f.song, &config).unwrap();
    assert_eq!(starts(grid.candidate()), [0, 4235, 8471]);
    assert_eq!(grid.source(), rhythm.source());
    assert_eq!(grid.candidate().samples, f.song.samples);
    assert_eq!(grid.candidate().marker_sets, f.song.marker_sets);
}
#[test]
fn rational_rolls_16_to_128_use_absolute_positions_at_nonpower_of_two_lpb() {
    let mut f = Fixture::new(48000, vec![0, 48000], Some(60.0));
    f.song.lines_per_beat = 7;
    for division in [
        Subdivision::Sixteenth,
        Subdivision::ThirtySecond,
        Subdivision::SixtyFourth,
        Subdivision::OneTwentyEighth,
    ] {
        let mut config = f.config();
        config.roll = Some(CompileRoll { division, hits: 4 });
        let plan = preview(&f.song, &config).unwrap();
        let expected: Vec<_> = (0..4)
            .map(|n| {
                (f64::from(n) * 4.0 * 48000.0 * 60.0 / (170.0 * f64::from(division.denominator())))
                    .round() as u64
            })
            .collect();
        assert_eq!(starts(plan.candidate()), expected);
        assert_eq!(plan.summary().note_starts, 4);
        assert_eq!(plan.candidate().lines_per_beat, 7);
    }
    let mut config = f.config();
    config.timing = CompileTiming::Grid {
        division: Subdivision::Sixteenth,
    };
    config.roll = Some(CompileRoll {
        division: Subdivision::OneTwentyEighth,
        hits: 8,
    });
    let plan = preview(&f.song, &config).unwrap();
    assert_eq!(
        starts(plan.candidate()),
        [0, 529, 1059, 1588, 2118, 2647, 3176, 3706]
    );
    config.roll.as_mut().unwrap().hits = 9;
    assert!(preview(&f.song, &config)
        .unwrap_err()
        .contains("next source/grid hit"));
}
#[test]
fn next_hit_release_and_cut_address_previous_voice_for_same_and_different_rows() {
    for lpb in [1, 4] {
        let mut f = Fixture::new(48000, vec![0, 6000, 12000], Some(120.0));
        f.song.lines_per_beat = lpb;
        for tail in [
            TailPolicy::Overlap,
            TailPolicy::ReleasePrevious,
            TailPolicy::CutPrevious,
        ] {
            let mut config = f.config();
            config.target_bpm = 240.0;
            config.tail = tail;
            config.pad_tails = true;
            let plan = preview(&f.song, &config).unwrap();
            assert_eq!(starts(plan.candidate()), [0, 3000]);
            let pcm = audio(plan.candidate());
            assert!((pcm[2999 * 2] - 0.05).abs() < 1e-7);
            match tail {
                TailPolicy::Overlap => assert!((pcm[3000 * 2] - 0.105).abs() < 1e-7),
                TailPolicy::ReleasePrevious => {
                    assert!(pcm[3000 * 2] > 0.055 && pcm[3000 * 2] < 0.105);
                    assert!(pcm[3500 * 2] < pcm[3000 * 2]);
                }
                TailPolicy::CutPrevious => {
                    assert!((pcm[3000 * 2] - 0.055).abs() < 1e-7);
                    let manifest = song_render::evaluate(plan.candidate(), None).unwrap();
                    assert!(manifest.events.iter().any(|event| event.frame == 3000
                        && matches!(event.command, Command::Cut { voice: 1 })));
                    assert!(!manifest.events.iter().any(|event| event.frame == 3000
                        && matches!(event.command, Command::Cut { voice: 2 })));
                }
            }
            assert!((pcm[6000 * 2] - 0.055).abs() < 1e-7);
            assert!(pcm[9000 * 2..].iter().all(|value| *value == 0.0));
        }
    }
}
#[test]
fn release_tail_padding_accounts_for_earlier_voice_and_cut_padding_only_last() {
    let f = Fixture::new(48000, vec![0, 12000, 12001], Some(120.0));
    let mut config = f.config();
    config.target_bpm = 400.0;
    config.timing = CompileTiming::Grid {
        division: Subdivision::OneTwentyEighth,
    };
    config.pad_tails = true;
    config.tail = TailPolicy::ReleasePrevious;
    let release = preview(&f.song, &config).unwrap();
    assert!(release.summary().pattern_rows >= 7);
    let pcm = audio(release.candidate());
    assert!(pcm[1000 * 2] > 0.0);
    assert!(pcm[12000 * 2..].iter().all(|value| *value == 0.0));
    config.tail = TailPolicy::CutPrevious;
    let cut = preview(&f.song, &config).unwrap();
    assert_eq!(cut.summary().pattern_rows, 1);
    assert!(audio(cut.candidate())[226 * 2..]
        .iter()
        .all(|value| *value == 0.0));
}
#[test]
fn simultaneous_row_hits_are_not_overwritten_and_more_than_four_instruments_allocate() {
    let markers = (0..=12).map(|n| n * 100).collect();
    let f = Fixture::new(48000, markers, Some(120.0));
    let plan = preview(&f.song, &f.config()).unwrap();
    assert_eq!(plan.summary().slice_count, 12);
    assert_eq!(plan.summary().instruments.len(), 12);
    assert_eq!(plan.summary().regions.len(), 12);
    assert_eq!(plan.summary().note_columns, 12);
    assert_eq!(starts(plan.candidate()).len(), 12);
    assert_eq!(
        plan.candidate().patterns[&plan.summary().pattern].tracks[&plan.summary().track][&0].len(),
        12
    );
}
#[test]
fn preview_staleness_allocation_overflow_and_invalid_options_are_atomic() {
    let mut f = Fixture::new(48000, vec![0, 4800], Some(120.0));
    let config = f.config();
    let plan = preview(&f.song, &config).unwrap();
    let mut changed = f.song.clone();
    changed.title.push('x');
    assert!(plan.commit(&changed).unwrap_err().contains("stale"));
    let before = f.song.clone();
    f.song.instruments.insert(
        InstrumentId(u64::MAX),
        Instrument {
            name: "Reserved max".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: None,
        },
    );
    assert!(preview(&f.song, &config).unwrap_err().contains("IDs"));
    f.song = before;
    for bad in [f64::NAN, 19.0, 401.0] {
        let mut config = f.config();
        config.target_bpm = bad;
        assert!(preview(&f.song, &config).is_err());
    }
    let mut invalid = f.config();
    invalid.cycles = 17;
    assert!(preview(&f.song, &invalid).is_err());
    invalid = f.config();
    invalid.roll = Some(CompileRoll {
        division: Subdivision::OneTwentyEighth,
        hits: 1,
    });
    assert!(preview(&f.song, &invalid).is_err());
    assert!(serde_json::from_str::<CompileTiming>(
        r#"{"Grid":{"division":"Sixteenth","extra":true}}"#
    )
    .is_err());
}
#[test]
fn pure_compiler_budgets_reject_runaway_notes_columns_and_duration_without_source_io() {
    let mut f = Fixture::new(8000, vec![0, 8000], Some(60.0));
    f.song.samples.get_mut(&SampleId(1)).unwrap().frames = 256 * 16000;
    f.song.samples.get_mut(&SampleId(1)).unwrap().path =
        f.dir.join("not-read-by-pure-compiler.wav");
    f.song.marker_sets.get_mut(&MarkerSetId(1)).unwrap().markers =
        (0..=256).map(|n| n * 16000).collect();
    let mut config = f.config();
    config.cycles = 16;
    assert!(preview(&f.song, &config).unwrap_err().contains("2048"));
    config.cycles = 1;
    config.target_bpm = 400.0;
    config.timing = CompileTiming::Grid {
        division: Subdivision::SixtyFourth,
    };
    config.roll = Some(CompileRoll {
        division: Subdivision::OneTwentyEighth,
        hits: 2,
    });
    assert!(preview(&f.song, &config)
        .unwrap_err()
        .contains("256 note columns"));
    config.roll = None;
    config.timing = CompileTiming::SourceRhythm;
    config.cycles = 16;
    assert!(preview(&f.song, &config).is_err());
}
#[test]
fn preview_discloses_global_tempo_change_and_preserves_existing_arrangement() {
    let mut f = Fixture::new(48000, vec![0, 4800], Some(120.0));
    f.song.patterns.insert(
        PatternId(77),
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: "Existing".into(),
            rows: 4,
            tracks: BTreeMap::new(),
        },
    );
    f.song.order.push(Occurrence {
        muted_tracks: Default::default(),
        section: None,
        pattern: PatternId(77),
    });
    let before = f.song.clone();
    let plan = preview(&f.song, &f.config()).unwrap();
    assert_eq!(plan.summary().occurrence, 1);
    assert_eq!(plan.candidate().order[0], before.order[0]);
    assert_eq!(
        plan.candidate().patterns[&PatternId(77)],
        before.patterns[&PatternId(77)]
    );
    assert_eq!(plan.candidate().bpm, 170.0);
    assert!(plan
        .summary()
        .warnings
        .iter()
        .any(|warning| warning.contains("GLOBAL")));
    assert_eq!(f.song, before);
}

#[test]
fn overlap_lane_reuse_waits_for_actual_integer_exhaustion_not_continuous_beats() {
    let mut f = Fixture::with_signal(
        44100,
        (0..=40).map(|i| i * 26).collect(),
        Some(20.0),
        |_, _| 0.01,
    );
    f.song.lines_per_beat = 256;
    let mut config = f.config();
    config.target_bpm = 400.0;
    config.pad_tails = true;
    let plan = preview(&f.song, &config).unwrap();
    assert!(plan.summary().note_columns >= 21);
    let manifest = song_render::evaluate(plan.candidate(), None).unwrap();
    let on: Vec<_> = manifest
        .events
        .iter()
        .filter_map(|event| {
            matches!(event.command, Command::On { .. }).then_some((event.frame, event.instrument))
        })
        .collect();
    let pcm = audio(plan.candidate());
    for frame in 0..100usize {
        let mut expected = 0.0f32;
        for &(start, instrument) in &on {
            if frame as u64 >= start && (frame as u64) < start + 29 {
                let region = &manifest.regions[&instrument];
                let age = frame as u64 - start;
                let position =
                    (region.start_frame as f64 * 48000.0 + age as f64 * 44100.0) / 48000.0;
                let lower = position.floor() as u64;
                let fraction = (position - lower as f64) as f32;
                let a = 0.01f32;
                let b = if lower + 1 < region.end_frame {
                    0.01
                } else {
                    0.0
                };
                expected += (a + (b - a) * fraction) * 0.5;
            }
        }
        assert!(
            (pcm[frame * 2] - expected).abs() < 1e-7,
            "frame {frame}: {} != {expected}",
            pcm[frame * 2]
        );
    }
}
#[test]
fn padding_covers_native_ceil_after_rounded_on_and_global_order_prefix() {
    for prefix in [false, true] {
        let mut f = Fixture::new(384000, vec![0, 24025], Some(137.0));
        f.song.lines_per_beat = 7;
        if prefix {
            f.song.patterns.insert(
                PatternId(9),
                Pattern {
                    generation: None,
                    track_aliases: Default::default(),
                    name: "Prefix".into(),
                    rows: 1,
                    tracks: BTreeMap::new(),
                },
            );
            f.song.order.push(Occurrence {
                muted_tracks: Default::default(),
                section: None,
                pattern: PatternId(9),
            });
        }
        let mut config = f.config();
        config.target_bpm = 137.0;
        config.pad_tails = true;
        let plan = preview(&f.song, &config).unwrap();
        assert_eq!(plan.summary().sequence_rows, 1);
        assert_eq!(plan.summary().pattern_rows, 2);
        let manifest = song_render::evaluate(plan.candidate(), None).unwrap();
        let start = starts(plan.candidate())[0] as usize;
        assert!(manifest.musical_frames >= start as u64 + 3004);
        let pcm = audio(plan.candidate());
        assert!((pcm[(start + 3003) * 2] - 0.05).abs() < 1e-7);
        assert!(pcm[(start + 3004) * 2..].iter().all(|value| *value == 0.0));
    }
}

#[test]
fn exact_rational_half_ties_and_prefix_clock_survive_project_roundtrip() {
    // Expected frames independently computed with Python Fraction, interpreting
    // BPM values as their exact binary f64 values (not guessed decimal tempos).
    for (prefix, lpb, source_bpm, target, expected) in [
        (0u32, 3, 70.0, 128.0, [0u64, 88]),
        (4, 3, 70.0, 128.0, [30000, 30088]),
        (7, 56, 70.0, 384.0, [938, 967]),
        (11, 7, 70.1, 137.1, [33010, 33092]),
    ] {
        let mut f = Fixture::new(44100, vec![0, 147, 1000], Some(source_bpm));
        f.song.lines_per_beat = lpb;
        if prefix > 0 {
            f.song.patterns.insert(
                PatternId(8),
                Pattern {
                    generation: None,
                    track_aliases: Default::default(),
                    name: "Prefix".into(),
                    rows: prefix,
                    tracks: BTreeMap::new(),
                },
            );
            f.song.order.push(Occurrence {
                muted_tracks: Default::default(),
                section: None,
                pattern: PatternId(8),
            });
        }
        let mut config = f.config();
        config.target_bpm = target;
        config.pad_tails = true;
        let plan = preview(&f.song, &config).unwrap();
        assert_eq!(starts(plan.candidate()), expected);
        let path = f.dir.join("exact.json");
        aqua_pattern_core::project::save(
            plan.candidate(),
            &path,
            aqua_pattern_core::project::SaveMode::CreateNew,
        )
        .unwrap();
        let (restored, _) = aqua_pattern_core::project::load(&path).unwrap();
        assert_eq!(&restored, plan.candidate());
        assert_eq!(starts(&restored), expected);
        assert_eq!(audio(&restored), audio(plan.candidate()));
    }
}

#[test]
fn generated_fractional_delays_roundtrip_without_one_ulp_json_parser_drift() {
    // Ordinary generated fractions, not only exact half-frame ties. The default
    // serde_json fast parser used to move this generated delay by one ULP; its
    // pinned float_roundtrip feature preserves full editable project state.
    let f = Fixture::new(44100, vec![0, 10, 100], Some(70.0));
    let mut config = f.config();
    config.pad_tails = true;
    let plan = preview(&f.song, &config).unwrap();
    let path = f.dir.join("fractional.json");
    aqua_pattern_core::project::save(
        plan.candidate(),
        &path,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    let (restored, _) = aqua_pattern_core::project::load(&path).unwrap();
    assert_eq!(&restored, plan.candidate());
    assert_eq!(starts(&restored), starts(plan.candidate()));
    assert_eq!(audio(&restored), audio(plan.candidate()));
    let json = serde_json::to_string(plan.candidate()).unwrap();
    assert_eq!(
        serde_json::from_str::<Song>(&json).unwrap(),
        *plan.candidate()
    );
}
