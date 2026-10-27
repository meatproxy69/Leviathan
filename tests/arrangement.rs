use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{self, Command, Engine, Release},
    transport::{Producer, Shared},
};
use std::{
    collections::{BTreeMap, BTreeSet},
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
fn pattern(rows: u32) -> Pattern {
    Pattern {
        generation: None,
        name: "Block".into(),
        rows,
        tracks: BTreeMap::new(),
        track_aliases: BTreeMap::new(),
    }
}
fn occurrence(id: u64) -> Occurrence {
    Occurrence {
        pattern: PatternId(id),
        muted_tracks: BTreeSet::new(),
        section: None,
    }
}
fn note(pitch: u8, instrument: u64) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(instrument)),
        ..Cell::default()
    }
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-arrangement-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("source.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 44100,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..88200 {
            writer
                .write_sample(0.03 + (frame % 71) as f32 * 0.0001)
                .unwrap();
            writer.write_sample(-0.02f32).unwrap();
        }
        writer.finalize().unwrap();
        let mut song = example_song();
        song.patterns = (1..=3).map(|id| (PatternId(id), pattern(2))).collect();
        song.order = vec![occurrence(1), occurrence(2), occurrence(3)];
        song.register_sample(SampleId(1), sample_asset::import(&path).unwrap().asset)
            .unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: "Signal".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 88200,
                root_note: 60,
            },
        )
        .unwrap();
        for instrument in song.instruments.values_mut() {
            instrument.sample = None;
            instrument.region = Some(RegionId(1));
        }
        Self { dir, song }
    }
    fn put(&mut self, p: u64, t: u64, row: u32, column: u16, cell: Cell) {
        self.song
            .patterns
            .get_mut(&PatternId(p))
            .unwrap()
            .tracks
            .entry(TrackId(t))
            .or_default()
            .entry(row)
            .or_default()
            .insert(column, cell);
    }
    fn alias(&mut self, p: u64, t: u64, source: u64, source_track: u64) {
        self.song
            .set_track_alias(
                PatternId(p),
                TrackId(t),
                Some(TrackAlias {
                    pattern: PatternId(source),
                    track: TrackId(source_track),
                }),
            )
            .unwrap();
    }
    fn mute(&mut self, occurrence: usize, track: u64) {
        self.song
            .set_occurrence_track_mute(occurrence, TrackId(track), true)
            .unwrap();
    }
    fn kit(&mut self) {
        let pads = (0..2)
            .map(|slot| {
                (
                    PadId(u64::from(slot) + 1),
                    DrumPad {
                        velocity_zones: Vec::new(),
                        slot,
                        name: format!("Pad{slot}"),
                        note: 36 + slot,
                        region: Some(RegionId(1)),
                        root_note: 60,
                        tune_semitones: 0,
                        gain: 1.0,
                        pan: 0.0,
                        mode: PadMode::OneShot,
                        new_note: PadNewNote::Overlap,
                        choke_group: None,
                    },
                )
            })
            .collect();
        self.song
            .upsert_drum_kit(
                DrumKitId(1),
                DrumKitInstrument {
                    name: "Kit".into(),
                    pads,
                },
            )
            .unwrap();
        self.song
            .assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn pcm(song: &Song, block: usize) -> Vec<f32> {
    let mut engine = Engine::new(song, None).unwrap();
    let end = engine.manifest.musical_frames + engine.manifest.tail_frames;
    let mut out = Vec::new();
    let mut buffer = vec![0.; block * 2];
    while engine.position < end {
        let n = engine.block(&mut buffer, end).unwrap();
        out.extend_from_slice(&buffer[..n * 2]);
    }
    out
}
fn manifest(song: &Song) -> song_render::Manifest {
    song_render::evaluate(song, None).unwrap()
}
fn starts(song: &Song) -> Vec<(u64, TrackId, usize, u8, f32)> {
    manifest(song)
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On {
                voice,
                pitch,
                volume,
                ..
            } = e.command
            {
                Some((e.frame, e.track, voice, pitch, volume))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn chained_cross_track_aliases_equal_materialized_pcm_and_keep_target_identity() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.alias(2, 2, 1, 1);
    f.alias(3, 3, 2, 2);
    let chain = f
        .song
        .resolve_track_chain(PatternId(3), TrackId(3))
        .unwrap();
    assert_eq!(chain.len(), 3);
    assert_eq!(
        starts(&f.song).iter().map(|s| s.1).collect::<Vec<_>>(),
        vec![TrackId(1), TrackId(2), TrackId(3)]
    );
    let m = manifest(&f.song);
    assert_eq!(m.schema, 7);
    assert_eq!(m.arrangement.as_ref().unwrap().aliases.len(), 2);
    let mut unique = f.song.clone();
    unique.make_track_unique(PatternId(2), TrackId(2)).unwrap();
    unique.make_track_unique(PatternId(3), TrackId(3)).unwrap();
    assert_eq!(m.events, manifest(&unique).events);
    assert_eq!(pcm(&f.song, 512), pcm(&unique, 512));
    assert_eq!(pcm(&f.song, 1), pcm(&f.song, 37));
    assert_eq!(pcm(&f.song, 37), pcm(&f.song, 512));
    f.put(1, 1, 0, 0, note(65, 1));
    assert_eq!(
        starts(&f.song).iter().map(|s| s.3).collect::<Vec<_>>(),
        vec![65, 65, 65]
    );
    assert_eq!(starts(&unique)[1].3, 60);
}
#[test]
fn alias_target_memory_does_not_leak_from_source_and_repeated_blocks_share_only_content() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.put(1, 2, 0, 0, note(72, 2));
    f.put(
        2,
        1,
        0,
        0,
        Cell {
            note: Note::Note(64),
            ..Cell::default()
        },
    );
    f.alias(2, 2, 2, 1);
    f.song.order = vec![occurrence(1), occurrence(2), occurrence(2)];
    let m = manifest(&f.song);
    let on: Vec<_> = m
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .collect();
    assert_eq!(
        on.iter().map(|e| e.instrument).collect::<Vec<_>>(),
        vec![
            InstrumentId(1),
            InstrumentId(2),
            InstrumentId(1),
            InstrumentId(2),
            InstrumentId(1),
            InstrumentId(2)
        ]
    );
    f.song.order = vec![occurrence(2)];
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("selected instrument"));
}
#[test]
fn invalid_alias_graphs_lengths_columns_shadow_content_and_dangling_deletion_are_atomic() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.alias(2, 2, 1, 1);
    let before = f.song.clone();
    assert!(f
        .song
        .set_track_alias(
            PatternId(1),
            TrackId(1),
            Some(TrackAlias {
                pattern: PatternId(2),
                track: TrackId(2)
            })
        )
        .is_err());
    assert_eq!(before, f.song);
    let mut bad = before.clone();
    bad.patterns.get_mut(&PatternId(1)).unwrap().rows = 3;
    assert!(bad.validate().is_err());
    let mut bad = before.clone();
    bad.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 3;
    assert!(bad.validate().is_err());
    let mut bad = before.clone();
    bad.patterns.remove(&PatternId(1));
    assert!(bad.validate().is_err());
    let mut bad = before.clone();
    bad.patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .insert(TrackId(2), BTreeMap::new());
    assert!(bad.validate().is_err());
    let mut cycle = before.clone();
    cycle
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .remove(&TrackId(1));
    cycle
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(2),
                track: TrackId(2),
            },
        );
    assert!(cycle.validate().unwrap_err().contains("cycle"));
    assert!(f
        .song
        .require_local_track(PatternId(2), TrackId(2))
        .unwrap_err()
        .contains("Make unique"));
    f.song.clone_pattern(PatternId(2), PatternId(4)).unwrap();
    assert!(f.song.patterns[&PatternId(4)].track_aliases.is_empty());
    assert_eq!(
        f.song.patterns[&PatternId(4)].tracks[&TrackId(2)],
        f.song.patterns[&PatternId(1)].tracks[&TrackId(1)]
    );
    f.song
        .set_track_alias(PatternId(2), TrackId(2), None)
        .unwrap();
    assert!(f
        .song
        .resolved_track(PatternId(2), TrackId(2))
        .unwrap()
        .is_none());
}
#[test]
fn depth_and_alias_expansion_budgets_cover_unused_graphs_before_render() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    for id in 4..=35 {
        let mut p = pattern(2);
        p.track_aliases.insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(if id == 4 { 1 } else { id - 1 }),
                track: TrackId(1),
            },
        );
        f.song.patterns.insert(PatternId(id), p);
    }
    f.song.validate().unwrap();
    let mut too_long = pattern(2);
    too_long.track_aliases.insert(
        TrackId(1),
        TrackAlias {
            pattern: PatternId(35),
            track: TrackId(1),
        },
    );
    f.song.patterns.insert(PatternId(36), too_long);
    assert!(f.song.validate().unwrap_err().contains("32 links"));
    f.song.patterns.retain(|id, _| id.0 <= 3);
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 65536;
    for row in 0..32769 {
        f.put(1, 1, row, 0, Cell::default());
    }
    for id in [2, 3] {
        let p = f.song.patterns.get_mut(&PatternId(id)).unwrap();
        p.rows = 65536;
        p.track_aliases.insert(
            TrackId(1),
            TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
    }
    assert!(f
        .song
        .validate()
        .unwrap_err()
        .contains("65536 resolved cells"));
}
#[test]
fn mute_releases_at_boundary_ignores_parameters_and_preserves_prior_selection() {
    let mut f = Fixture::new();
    f.put(
        1,
        1,
        0,
        0,
        Cell {
            velocity: Some(0.3),
            ..note(60, 1)
        },
    );
    f.put(1, 2, 0, 0, note(67, 2));
    f.put(
        2,
        1,
        0,
        0,
        Cell {
            instrument: Some(InstrumentId(3)),
            velocity: Some(0.9),
            pan: Some(1.0),
            ..Cell::default()
        },
    );
    f.put(
        3,
        1,
        0,
        0,
        Cell {
            note: Note::Note(64),
            ..Cell::default()
        },
    );
    f.mute(1, 1);
    let m = manifest(&f.song);
    let off = m
        .events
        .iter()
        .find(|e| {
            matches!(
                e.command,
                Command::Off {
                    reason: Release::TrackMute,
                    ..
                }
            )
        })
        .unwrap();
    assert_eq!(off.frame, 12000);
    assert_eq!(off.track, TrackId(1));
    let last = m
        .events
        .iter()
        .find(|e| e.frame == 24000 && matches!(e.command, Command::On { .. }))
        .unwrap();
    assert_eq!(last.instrument, InstrumentId(1));
    assert!(matches!(
        last.command,
        Command::On {
            volume: 0.3,
            pan: 0.0,
            ..
        }
    ));
    let data = pcm(&f.song, 512);
    assert!(data[24000..24200].iter().any(|s| *s != 0.)); // declick, not hard zero
    let mut reference = f.song.clone();
    reference.order[1].muted_tracks.clear();
    reference
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .clear();
    reference
        .patterns
        .get_mut(&PatternId(2))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    0,
                    Cell {
                        effects: StepEffects {
                            release: Some(0.0),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )]),
            )]),
        );
    assert_eq!(data, pcm(&reference, 512));
}
#[test]
fn all_muted_missing_assets_render_exact_silence_without_source_io_but_blank_song_still_errors() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.mute(0, 1);
    f.song.samples.get_mut(&SampleId(1)).unwrap().path = f.dir.join("missing.wav");
    let m = manifest(&f.song);
    assert!(m.events.is_empty());
    assert!(m.assets.is_empty());
    assert!(m.regions.is_empty());
    assert!(m.pad_voices.is_empty());
    assert_eq!(m.musical_frames, 36000);
    let data = pcm(&f.song, 37);
    assert_eq!(data.len(), (36000 + 24000) * 2);
    assert!(data.iter().all(|s| *s == 0.));
    let (stats, _) = song_render::export(&f.song, None, &f.dir.join("silence.wav")).unwrap();
    assert_eq!(stats.peak, 0.);
    assert_eq!(stats.energy, 0.);
    f.song.order[0].muted_tracks.clear();
    assert!(Engine::new(&f.song, None).is_err());
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    f.mute(0, 1);
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("no note triggers"));
}
#[test]
fn mute_cancels_only_own_pending_phrase_children_and_late_controls() {
    let mut f = Fixture::new();
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Phrase".into(),
            instrument: InstrumentId(1),
            rows: 8,
            lines_per_beat: 4,
            note_columns: 1,
            root_note: 60,
            cells: (0..8)
                .map(|row| (row, BTreeMap::from([(0, note(60, 1))])))
                .collect(),
        },
    );
    let trigger = Cell {
        phrase: Some(PhraseTrigger {
            phrase: PhraseId(1),
            cycles: 2,
        }),
        ..note(60, 1)
    };
    f.put(1, 1, 0, 0, trigger.clone());
    f.put(1, 2, 0, 0, trigger);
    f.mute(1, 1);
    let m = manifest(&f.song);
    assert!(m
        .events
        .iter()
        .filter(|e| e.track == TrackId(1) && e.frame >= 12000)
        .all(|e| matches!(
            e.command,
            Command::Off {
                reason: Release::TrackMute,
                ..
            }
        )));
    assert!(m.events.iter().any(|e| e.track == TrackId(2)
        && e.frame == 12000
        && matches!(e.command, Command::On { .. })));
    assert!(m.events.iter().any(|e| e.track == TrackId(2)
        && e.frame == 30000
        && matches!(e.command, Command::On { .. })));
    f.alias(2, 3, 1, 2);
    let mut reference = f.song.clone();
    reference
        .make_track_unique(PatternId(2), TrackId(3))
        .unwrap();
    assert_eq!(manifest(&f.song).events, manifest(&reference).events);
    assert_eq!(pcm(&f.song, 512), pcm(&reference, 512));
}
#[test]
fn mute_releases_all_held_oneshots_even_dropped_pointers_and_preserves_other_track_same_instrument()
{
    let mut f = Fixture::new();
    f.kit();
    f.put(1, 1, 0, 0, note(36, 1));
    f.put(1, 1, 1, 0, note(37, 1));
    f.put(1, 2, 0, 0, note(36, 1));
    f.mute(1, 1);
    let m = manifest(&f.song);
    let muted: Vec<_> = m
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.command,
                Command::Off {
                    reason: Release::TrackMute,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(muted.len(), 2);
    assert!(muted
        .iter()
        .all(|e| e.track == TrackId(1) && e.frame == 12000));
    let other = m
        .events
        .iter()
        .find(|e| e.track == TrackId(2) && matches!(e.command, Command::On { .. }))
        .unwrap();
    let Command::On { voice, .. } = other.command else {
        unreachable!()
    };
    assert!(!m
        .events
        .iter()
        .any(|e| matches!(e.command,Command::Off{voice:v,..}if v==voice) && e.frame < 36000));
    // Logical pointer was already cleared by OFF, but the OneShot still releases.
    f.put(
        1,
        1,
        1,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    assert_eq!(
        manifest(&f.song)
            .events
            .iter()
            .filter(|e| matches!(
                e.command,
                Command::Off {
                    reason: Release::TrackMute,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(pcm(&f.song, 512).iter().all(|v| v.is_finite()));
}
#[test]
fn rounded_prior_row_muted_spill_cannot_choke_other_track_or_latch_parameters() {
    let mut f = Fixture::new();
    f.kit();
    for pad in f
        .song
        .drum_kits
        .get_mut(&DrumKitId(1))
        .unwrap()
        .pads
        .values_mut()
    {
        pad.choke_group = Some(1);
    }
    f.put(1, 2, 0, 0, note(36, 1));
    f.put(
        1,
        1,
        0,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            velocity: Some(0.2),
            ..Default::default()
        },
    );
    f.put(
        1,
        1,
        1,
        0,
        Cell {
            delay: Some(0.999999),
            velocity: Some(0.9),
            ..note(37, 1)
        },
    );
    f.mute(1, 1);
    f.put(
        3,
        1,
        0,
        0,
        Cell {
            note: Note::Note(60),
            ..Default::default()
        },
    );
    let m = manifest(&f.song);
    assert_eq!(m.pad_voices.len(), 1);
    assert!(!m.events.iter().any(|e| matches!(
        e.command,
        Command::Off {
            reason: Release::Choke,
            ..
        }
    )));
    let last = m
        .events
        .iter()
        .find(|e| e.frame == 24000 && matches!(e.command, Command::On { .. }))
        .unwrap();
    assert_eq!(last.instrument, InstrumentId(2));
    assert!(matches!(last.command, Command::On { volume: 0.2, .. }));
    // Reference removes the delayed spill entirely, retaining same boundaries.
    let mut reference = f.song.clone();
    reference
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .remove(&1);
    assert_eq!(pcm(&f.song, 512), pcm(&reference, 512));
}
#[test]
fn unused_aliases_are_audibly_transparent_and_serde_defaults_preserve_old_shape() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.song.order = vec![occurrence(1)];
    let before = manifest(&f.song);
    let bytes = pcm(&f.song, 512);
    f.alias(2, 2, 1, 1);
    assert_eq!(before, manifest(&f.song));
    assert_eq!(bytes, pcm(&f.song, 512));
    let json = serde_json::to_string(&f.song).unwrap();
    assert_eq!(f.song, serde_json::from_str::<Song>(&json).unwrap());
    let mut legacy = f.song.clone();
    legacy.patterns.remove(&PatternId(2));
    let json = serde_json::to_string(&legacy).unwrap();
    assert!(!json.contains("track_aliases"));
    assert!(!json.contains("muted_tracks"));
}
#[test]
fn mute_metadata_moves_with_occurrence_and_repeat_is_fresh_unmuted() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.mute(1, 1);
    f.song.move_occurrence(0, 1).unwrap();
    assert_eq!(f.song.order[0].pattern, PatternId(2));
    assert!(f.song.order[0].muted_tracks.contains(&TrackId(1)));
    f.song.append_repeat(PatternId(2)).unwrap();
    assert!(f.song.order.last().unwrap().muted_tracks.is_empty());
    let before = f.song.clone();
    assert!(f.song.move_occurrence(99, 0).is_err());
    assert_eq!(before, f.song);
}
#[test]
fn alias_expanded_starts_obey_existing_budget_and_export_cleanup() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1025;
    for row in 0..1025 {
        f.put(1, 1, row, 0, note(60, 1));
    }
    f.song.patterns.get_mut(&PatternId(2)).unwrap().rows = 1025;
    f.alias(2, 2, 1, 1);
    f.song.order = vec![occurrence(1), occurrence(2)];
    let output = f.dir.join("overbudget.wav");
    assert!(song_render::export(&f.song, None, &output)
        .unwrap_err()
        .contains("2048"));
    assert!(!output.exists());
    assert!(!output.with_extension("events.json").exists());
}
#[test]
fn alias_mute_streaming_and_export_share_events_and_loop_reset() {
    let mut f = Fixture::new();
    f.put(1, 1, 0, 0, note(60, 1));
    f.alias(2, 1, 1, 1);
    f.mute(1, 1);
    f.alias(3, 1, 1, 1);
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut result = Vec::new();
    let expected = pcm(&f.song, 512);
    while result.len() < expected.len() {
        producer.pump_until_buffered().unwrap();
        let mut buffer = vec![0.; 1024.min(expected.len() - result.len())];
        shared.fill(&mut buffer);
        result.extend_from_slice(&buffer);
    }
    assert_eq!(result, expected);
    let shared = Arc::new(Shared::default());
    shared.set_loop(true, 36000);
    let mut producer = Producer::new(&f.song, None, shared.clone()).unwrap();
    let mut looped = Vec::new();
    while looped.len() < 144000 {
        producer.pump_until_buffered().unwrap();
        let mut buffer = [0.; 512];
        shared.fill(&mut buffer);
        looped.extend(buffer);
    }
    assert_eq!(&looped[..72000], &expected[..72000]);
    assert_eq!(&looped[72000..144000], &expected[..72000]);
    shared.stop();
    producer.pump_until_buffered().unwrap();
    let mut silence = [1.; 512];
    shared.fill(&mut silence);
    assert!(silence.iter().all(|v| *v == 0.));
    let mut cold = Engine::new(&f.song, None).unwrap();
    cold.cold_seek(12000).unwrap();
    let mut buffer = [1.; 74];
    while cold.position < 24000 {
        let n = cold.block(&mut buffer, 24000).unwrap();
        assert!(buffer[..n * 2].iter().all(|v| *v == 0.));
    }
    let output = f.dir.join("arrangement.wav");
    let (_, m) = song_render::export(&f.song, None, &output).unwrap();
    assert_eq!(m, manifest(&f.song));
    let audio: Vec<f32> = hound::WavReader::open(output)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(audio, expected);
}

#[test]
fn sparse_empty_row_alias_amplification_is_bounded_before_mapping_scan() {
    let mut f = Fixture::new();
    for p in f.song.patterns.values_mut() {
        p.rows = 65536;
    }
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .insert(
            TrackId(1),
            (0..65536).map(|row| (row, BTreeMap::new())).collect(),
        );
    for id in [2, 3] {
        f.song
            .patterns
            .get_mut(&PatternId(id))
            .unwrap()
            .track_aliases
            .insert(
                TrackId(1),
                TrackAlias {
                    pattern: PatternId(1),
                    track: TrackId(1),
                },
            );
    }
    assert!(f.song.validate().unwrap_err().contains("row containers"));
}

#[test]
fn ordered_alias_default_cell_work_is_bounded_and_muted_explicit_pad_refs_are_protected() {
    let mut f = Fixture::new();
    for row in 0..2 {
        f.put(1, 1, row, 0, Cell::default());
    }
    f.alias(2, 1, 1, 1);
    f.song.order = vec![occurrence(2); 32769];
    assert!(f
        .song
        .validate()
        .unwrap_err()
        .contains("ordered alias cells"));
    f.song.order = vec![occurrence(1), occurrence(2)];
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    f.kit();
    f.put(1, 1, 0, 0, note(36, 1));
    f.mute(0, 1);
    f.mute(1, 1);
    let before = f.song.clone();
    assert!(f.song.remove_drum_pad(DrumKitId(1), PadId(1)).is_err());
    assert_eq!(before, f.song);
}

#[test]
fn muted_locally_inherited_pad_references_survive_aliasing_and_cannot_be_deleted() {
    let mut f = Fixture::new();
    f.kit();
    f.put(1, 1, 0, 0, note(36, 1));
    f.put(
        1,
        1,
        1,
        0,
        Cell {
            note: Note::Note(37),
            ..Default::default()
        },
    );
    f.alias(2, 2, 1, 1);
    f.mute(0, 1);
    f.mute(1, 2);
    let before = f.song.clone();
    assert!(f.song.remove_drum_pad(DrumKitId(1), PadId(2)).is_err());
    assert_eq!(f.song, before);
    let mut pad = f.song.drum_kits[&DrumKitId(1)].pads[&PadId(2)].clone();
    pad.region = None;
    assert!(f.song.upsert_drum_pad(DrumKitId(1), PadId(2), pad).is_err());
    assert_eq!(f.song, before);
    assert!(manifest(&f.song).events.is_empty());
}
