use aqua_pattern_core::{
    break_compiler::Subdivision,
    sample_asset,
    song::*,
    song_render::{self, Command, Engine},
    transforms::*,
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
fn pattern(rows: u32) -> Pattern {
    Pattern {
        generation: None,
        name: "Transform".into(),
        rows,
        tracks: BTreeMap::new(),
        track_aliases: BTreeMap::new(),
    }
}
fn occurrence(id: u64) -> Occurrence {
    Occurrence {
        pattern: PatternId(id),
        muted_tracks: Default::default(),
        section: None,
    }
}
fn note(pitch: u8, instrument: u64) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(instrument)),
        ..Default::default()
    }
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-transform-{}-{}",
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
            w.write_sample(0.02 + (i % 311) as f32 * 0.0001).unwrap();
            w.write_sample(-0.03f32).unwrap();
        }
        w.finalize().unwrap();
        let mut song = example_song();
        song.patterns = BTreeMap::from([(PatternId(1), pattern(8))]);
        song.order = vec![occurrence(1)];
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
        for i in song.instruments.values_mut() {
            i.sample = None;
            i.region = Some(RegionId(1));
        }
        Self { dir, song }
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
    fn cell(&self, row: u32, column: u16) -> &Cell {
        &self.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&column]
    }
    fn kit(&mut self) {
        self.song
            .upsert_drum_kit(
                DrumKitId(1),
                DrumKitInstrument {
                    name: "Kit".into(),
                    pads: BTreeMap::from([(
                        PadId(1),
                        DrumPad {
                            velocity_zones: Vec::new(),
                            slot: 0,
                            name: "Pad".into(),
                            note: 36,
                            region: Some(RegionId(1)),
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
        self.song
            .assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
    }
    fn config(&self) -> TransformConfig {
        TransformConfig {
            scope: TransformScope {
                pattern: PatternId(1),
                track: TrackId(1),
                row_start: 0,
                row_end: 8,
                columns: vec![0],
            },
            fields: TransformFields::default(),
            transpose_semitones: 0,
            velocity_scale: 1.0,
            nudge_frames: 0,
            swing: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn cell(song: &Song, row: u32, column: u16) -> &Cell {
    &song.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&column]
}
fn events(song: &Song) -> Vec<song_render::Event> {
    song_render::evaluate(song, None).unwrap().events
}
fn pcm(song: &Song) -> Vec<f32> {
    let mut e = Engine::new(song, None).unwrap();
    let end = e.manifest.musical_frames + e.manifest.tail_frames;
    let mut out = Vec::new();
    let mut buffer = [0.; 1024];
    while e.position < end {
        let n = e.block(&mut buffer, end).unwrap();
        out.extend_from_slice(&buffer[..n * 2]);
    }
    out
}
#[test]
fn identity_preserves_sparse_storage_none_zero_tiny_and_near_one_delay_bits() {
    let mut f = Fixture::new();
    f.put(0, 0, note(60, 1));
    f.put(
        1,
        0,
        Cell {
            delay: Some(1.0f64.next_down()),
            ..note(62, 1)
        },
    );
    f.put(
        2,
        0,
        Cell {
            delay: Some(0.0),
            ..note(64, 1)
        },
    );
    f.put(
        3,
        0,
        Cell {
            delay: Some(f64::from_bits(1)),
            ..note(65, 1)
        },
    );
    f.put(4, 0, Cell::default());
    let before = serde_json::to_vec(&f.song).unwrap();
    let mut c = f.config();
    c.fields = TransformFields {
        notes: true,
        glide_targets: true,
        velocity: true,
        timing: true,
    };
    c.swing = Some(TransformSwing {
        division: Subdivision::Sixteenth,
        amount_percent: 0,
    });
    let p = preview(&f.song, c.clone()).unwrap();
    assert!(p.changes().is_empty());
    assert_eq!(before, serde_json::to_vec(p.candidate()).unwrap());
    assert_eq!(p.id(), preview(&f.song, c).unwrap().id());
    assert_eq!(p.summary().changed_cells, 0);
    let mut c = f.config();
    c.fields.velocity = true;
    c.velocity_scale = 0.5;
    let p = preview(&f.song, c).unwrap();
    assert_eq!(p.candidate(), &f.song);
    assert_eq!(
        cell(p.candidate(), 1, 0).delay.unwrap().to_bits(),
        f.cell(1, 0).delay.unwrap().to_bits()
    );
}
#[test]
fn masks_are_independent_exact_scopes_and_unspecified_velocity_never_materialized() {
    let mut f = Fixture::new();
    f.put(
        1,
        0,
        Cell {
            velocity: Some(0.4),
            pan: Some(-0.2),
            delay: Some(0.125),
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 3.0,
                }),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    f.put(
        2,
        0,
        Cell {
            note: Note::Note(62),
            ..Default::default()
        },
    );
    f.put(1, 1, note(50, 2));
    f.put(5, 0, note(70, 1));
    let mut c = f.config();
    c.scope.row_start = 1;
    c.scope.row_end = 3;
    c.fields.notes = true;
    c.fields.velocity = true;
    c.transpose_semitones = 12;
    c.velocity_scale = 0.5;
    c.nudge_frames = 1000;
    let p = preview(&f.song, c.clone()).unwrap();
    assert_eq!(cell(p.candidate(), 1, 0).note, Note::Note(72));
    assert_eq!(cell(p.candidate(), 2, 0).note, Note::Note(74));
    assert_eq!(
        cell(p.candidate(), 1, 0)
            .effects
            .glide
            .as_ref()
            .unwrap()
            .target,
        72
    );
    assert_eq!(cell(p.candidate(), 1, 0).velocity, Some(0.2));
    assert_eq!(cell(p.candidate(), 2, 0).velocity, None);
    assert_eq!(cell(p.candidate(), 1, 0).delay, Some(0.125));
    assert_eq!(cell(p.candidate(), 1, 1), f.cell(1, 1));
    assert_eq!(cell(p.candidate(), 5, 0), f.cell(5, 0));
    c.fields.notes = false;
    c.fields.velocity = false;
    c.fields.glide_targets = true;
    let p = preview(&f.song, c).unwrap();
    assert_eq!(cell(p.candidate(), 1, 0).note, Note::Note(60));
    assert_eq!(
        cell(p.candidate(), 1, 0)
            .effects
            .glide
            .as_ref()
            .unwrap()
            .target,
        84
    );
    assert_eq!(cell(p.candidate(), 2, 0), f.cell(2, 0));
}
#[test]
fn transpose_rejects_kit_selectors_unknown_incoming_and_unmapped_voice_targets() {
    let mut f = Fixture::new();
    f.kit();
    f.put(0, 0, note(36, 1));
    let mut c = f.config();
    c.fields.notes = true;
    c.transpose_semitones = 1;
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("drum selectors"));
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
                glide: Some(Glide {
                    target: 60,
                    semitones_per_second: 2.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    c.fields.notes = false;
    c.fields.glide_targets = true;
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("kit glide"));
    f.put(0, 0, note(60, 2));
    f.put(
        1,
        0,
        Cell {
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    assert_eq!(
        cell(preview(&f.song, c.clone()).unwrap().candidate(), 2, 0)
            .effects
            .glide
            .as_ref()
            .unwrap()
            .target,
        61
    );
    f.put(
        1,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    assert!(preview(&f.song, c).unwrap_err().contains("locally proven"));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    f.put(
        1,
        0,
        Cell {
            note: Note::Note(60),
            ..Default::default()
        },
    );
    let mut c = f.config();
    c.fields.notes = true;
    c.transpose_semitones = 1;
    assert!(preview(&f.song, c).unwrap_err().contains("locally proven"));
}
#[test]
fn pitch_and_velocity_ranges_reject_without_clamp_and_extremes_are_valid() {
    let mut f = Fixture::new();
    f.put(
        0,
        0,
        Cell {
            velocity: Some(0.4),
            ..note(127, 1)
        },
    );
    let mut c = f.config();
    c.fields.notes = true;
    c.transpose_semitones = 1;
    assert!(preview(&f.song, c.clone()).is_err());
    c.transpose_semitones = -48;
    assert_eq!(
        cell(preview(&f.song, c.clone()).unwrap().candidate(), 0, 0).note,
        Note::Note(79)
    );
    f.put(
        0,
        0,
        Cell {
            velocity: Some(0.4),
            ..note(0, 1)
        },
    );
    assert!(preview(&f.song, c.clone()).is_err());
    c.transpose_semitones = 48;
    assert_eq!(
        cell(preview(&f.song, c).unwrap().candidate(), 0, 0).note,
        Note::Note(48)
    );
    let mut c = f.config();
    c.fields.velocity = true;
    c.velocity_scale = 4.0;
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("no clamping"));
    c.velocity_scale = 0.0;
    assert_eq!(
        cell(preview(&f.song, c).unwrap().candidate(), 0, 0).velocity,
        Some(0.0)
    );
}
#[test]
fn nudge_moves_whole_cell_and_all_row_fx_by_identical_output_frames() {
    let mut f = Fixture::new();
    f.song.bpm = 137.0;
    f.song.lines_per_beat = 7;
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.2),
            velocity: Some(0.3),
            pan: Some(0.1),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.1,
                }),
                release: Some(0.8),
                offset: Some(10),
                direction: Some(Direction::Reverse),
                glide: Some(Glide {
                    target: 64,
                    semitones_per_second: 12.0,
                }),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 71;
    let p = preview(&f.song, c).unwrap();
    let before = events(&f.song);
    let after = events(p.candidate());
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(after.iter()) {
        assert_eq!(b.frame, a.frame + 71);
        assert_eq!(a.command, b.command);
        assert_eq!(a.instrument, b.instrument);
    }
    assert_eq!(cell(p.candidate(), 1, 0).velocity, Some(0.3));
    assert_eq!(
        cell(p.candidate(), 1, 0).effects.retrigger,
        f.cell(1, 0).effects.retrigger
    );
    assert_eq!(cell(p.candidate(), 1, 0).effects.offset, Some(10));
}
#[test]
fn exact_timing_covers_repeated_prefixes_aliases_and_project_float_roundtrip() {
    let mut f = Fixture::new();
    f.song.bpm = 137.123456789;
    f.song.lines_per_beat = 7;
    f.put(
        2,
        0,
        Cell {
            delay: Some(0.3141592653589793),
            effects: StepEffects {
                cut: Some(0.8),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    f.song.patterns.insert(PatternId(2), pattern(3));
    f.song.patterns.insert(PatternId(3), pattern(8));
    f.song
        .set_track_alias(
            PatternId(3),
            TrackId(2),
            Some(TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            }),
        )
        .unwrap();
    f.song.order = vec![occurrence(2), occurrence(1), occurrence(3), occurrence(1)];
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = -31;
    let p = preview(&f.song, c).unwrap();
    assert_eq!(p.summary().affected_occurrences, vec![1, 2, 3]);
    assert_eq!(
        p.summary().affected_aliases,
        vec![TrackAlias {
            pattern: PatternId(3),
            track: TrackId(2)
        }]
    );
    for (a, b) in events(&f.song).iter().zip(events(p.candidate()).iter()) {
        assert_eq!(b.frame, a.frame - 31);
        assert_eq!(a.command, b.command);
    }
    let path = f.dir.join("transformed.json");
    aqua_pattern_core::project::save(
        p.candidate(),
        &path,
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    let (reopened, _) = aqua_pattern_core::project::load(&path).unwrap();
    assert_eq!(&reopened, p.candidate());
    assert_eq!(events(&reopened), events(p.candidate()));
    assert_eq!(pcm(&reopened), pcm(p.candidate()));
}
#[test]
fn row_carry_stays_in_selection_and_cannot_overwrite_scalar_or_overtake_off() {
    let mut f = Fixture::new();
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.9),
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 1200;
    let p = preview(&f.song, c.clone()).unwrap();
    assert_eq!(p.changes()[0].to_row, 2);
    assert_eq!(cell(p.candidate(), 2, 0).instrument, Some(InstrumentId(1)));
    c.scope.row_end = 2;
    assert!(preview(&f.song, c.clone()).is_err());
    c.scope.row_end = 8;
    f.put(
        2,
        0,
        Cell {
            velocity: Some(0.2),
            ..Default::default()
        },
    );
    c.scope.row_start = 1;
    c.scope.row_end = 2;
    assert!(preview(&f.song, c.clone()).is_err());
    // A long move over an unselected event is rejected even when its destination is empty.
    c.scope.row_end = 8;
    c.fields.timing = true;
    c.nudge_frames = 24000;
    c.scope.columns = vec![0];
    f.put(
        2,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    // Scope includes OFF, so both move in order; restrict OFF by putting it after scope instead.
    c.scope.row_end = 2;
    assert!(preview(&f.song, c).is_err());
}
#[test]
fn unrepresentable_fx_crossing_rows_rejects_instead_of_clamping_or_dropping() {
    let mut f = Fixture::new();
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.2),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.7,
                }),
                cut: Some(0.95),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 1200;
    assert!(preview(&f.song, c).unwrap_err().contains("row-local FX"));
}
#[test]
fn swing_is_integer_displacement_of_original_odd_bins_without_grid_snap() {
    let mut f = Fixture::new();
    f.song.bpm = 170.0;
    f.song.lines_per_beat = 32;
    for row in 0..8 {
        f.put(
            row,
            0,
            Cell {
                delay: Some(0.125),
                effects: StepEffects {
                    cut: Some(0.5),
                    ..Default::default()
                },
                ..note(60, 1)
            },
        );
    }
    let mut c = f.config();
    c.fields.timing = true;
    c.swing = Some(TransformSwing {
        division: Subdivision::OneTwentyEighth,
        amount_percent: 25,
    });
    let p = preview(&f.song, c).unwrap();
    assert_eq!(p.summary().swing_frames, 132);
    let before = events(&f.song);
    let after = events(p.candidate());
    for (a, b) in before.iter().zip(&after) {
        let row = a.position.unwrap().1;
        assert_eq!(b.frame, a.frame + if row % 2 == 1 { 132 } else { 0 });
    }
    for row in [0, 2, 4, 6] {
        assert_eq!(cell(p.candidate(), row, 0), f.cell(row, 0));
    }
}
#[test]
fn phrase_trigger_pitch_is_supported_but_nonzero_timing_is_explicitly_rejected() {
    let mut f = Fixture::new();
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Phrase".into(),
            instrument: InstrumentId(1),
            rows: 2,
            lines_per_beat: 4,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([(0, BTreeMap::from([(0, note(60, 1))]))]),
        },
    );
    f.put(
        1,
        0,
        Cell {
            phrase: Some(PhraseTrigger {
                phrase: PhraseId(1),
                cycles: 1,
            }),
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.notes = true;
    c.transpose_semitones = 12;
    assert_eq!(
        cell(preview(&f.song, c.clone()).unwrap().candidate(), 1, 0).note,
        Note::Note(72)
    );
    c.fields.timing = true;
    c.nudge_frames = 1;
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("phrase triggers"));
    c.nudge_frames = 0;
    c.transpose_semitones = 0;
    assert_eq!(preview(&f.song, c).unwrap().candidate(), &f.song);
}
#[test]
fn alias_authority_stale_base_and_config_bounds_are_explicit_and_atomic() {
    let mut f = Fixture::new();
    f.put(1, 0, note(60, 1));
    let mut c = f.config();
    c.fields.notes = true;
    c.transpose_semitones = 12;
    let p = preview(&f.song, c.clone()).unwrap();
    assert_eq!(f.cell(1, 0).note, Note::Note(60));
    let mut changed = f.song.clone();
    changed.title.push('x');
    assert!(p.commit(&changed).is_err());
    assert_eq!(p.commit(&f.song).unwrap(), *p.candidate());
    f.song.patterns.insert(PatternId(2), pattern(8));
    f.song
        .set_track_alias(
            PatternId(2),
            TrackId(2),
            Some(TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            }),
        )
        .unwrap();
    c.scope.pattern = PatternId(2);
    c.scope.track = TrackId(2);
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("read-only"));
    c = f.config();
    c.velocity_scale = f32::NAN;
    assert!(preview(&f.song, c.clone()).is_err());
    c = f.config();
    c.scope.columns = vec![0, 0];
    assert!(preview(&f.song, c).is_err());
}
#[test]
fn preview_is_pure_and_shifted_audio_has_exact_new_onset_and_no_new_dsp() {
    let mut f = Fixture::new();
    f.put(
        0,
        0,
        Cell {
            effects: StepEffects {
                cut: Some(0.1),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 100;
    let p = preview(&f.song, c).unwrap();
    let before = pcm(&f.song);
    let after = pcm(p.candidate());
    assert!(after[..200].iter().all(|v| *v == 0.));
    assert_eq!(&after[200..1400], &before[..1200]);
    let mut c = f.config();
    c.fields.notes = true;
    c.transpose_semitones = 12;
    let p = preview(&f.song, c).unwrap();
    let mut manual = f.song.clone();
    manual
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
    assert_eq!(pcm(p.candidate()), pcm(&manual));
    assert_ne!(pcm(p.candidate()), before);
}

#[test]
fn vacated_chain_moves_are_atomic_but_occupied_residuals_and_swing_overtaking_reject() {
    let mut f = Fixture::new();
    f.put(1, 0, note(60, 1));
    f.put(
        2,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 6000;
    let p = preview(&f.song, c).unwrap();
    assert_eq!(
        p.changes()
            .iter()
            .map(|c| (c.from_row, c.to_row))
            .collect::<Vec<_>>(),
        vec![(1, 2), (2, 3)]
    );
    assert_eq!(cell(p.candidate(), 2, 0).note, Note::Note(60));
    assert_eq!(cell(p.candidate(), 3, 0).note, Note::Off);
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.9),
            ..note(60, 1)
        },
    );
    f.put(
        2,
        0,
        Cell {
            velocity: Some(0.3),
            ..Default::default()
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 600;
    assert!(preview(&f.song, c).unwrap_err().contains("collision"));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 32;
    f.song.lines_per_beat = 32;
    f.put(
        15,
        0,
        Cell {
            delay: Some(0.9),
            ..note(60, 1)
        },
    );
    f.put(
        16,
        0,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let mut c = f.config();
    c.scope.row_end = 32;
    c.fields.timing = true;
    c.swing = Some(TransformSwing {
        division: Subdivision::Sixteenth,
        amount_percent: 50,
    });
    assert!(preview(&f.song, c).unwrap_err().contains("overtakes"));
}
#[test]
fn subdivision_swing_amounts_are_exact_frame_quantized_and_even_bins_are_bit_identical() {
    let mut f = Fixture::new();
    f.song.bpm = 170.0;
    f.song.lines_per_beat = 32;
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 32;
    for (division, row, frames) in [
        (Subdivision::Sixteenth, 8, 1059),
        (Subdivision::ThirtySecond, 4, 529),
        (Subdivision::SixtyFourth, 2, 265),
        (Subdivision::OneTwentyEighth, 1, 132),
    ] {
        f.song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .clear();
        f.put(
            0,
            0,
            Cell {
                delay: Some(f64::from_bits(1)),
                effects: StepEffects {
                    cut: Some(0.01),
                    ..Default::default()
                },
                ..note(60, 1)
            },
        );
        f.put(row, 0, note(60, 1));
        let mut c = f.config();
        c.scope.row_end = 32;
        c.fields.timing = true;
        c.swing = Some(TransformSwing {
            division,
            amount_percent: 25,
        });
        let p = preview(&f.song, c).unwrap();
        assert_eq!(p.summary().swing_frames, frames);
        assert_eq!(cell(p.candidate(), 0, 0), f.cell(0, 0));
        let before = events(&f.song)
            .into_iter()
            .find(|e| e.position == Some((0, row)) && matches!(e.command, Command::On { .. }))
            .unwrap();
        let after = events(p.candidate())
            .into_iter()
            .find(|e| matches!(e.command, Command::On { voice: 2, .. }))
            .unwrap();
        assert_eq!(after.frame, before.frame + u64::from(frames));
    }
}
#[test]
fn fractional_bpm_prefix_nudges_shift_every_row_local_command_exactly() {
    let mut f = Fixture::new();
    f.put(
        2,
        0,
        Cell {
            delay: Some(0.3141592653589793),
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.1,
                }),
                cut: Some(0.75),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    f.song.patterns.insert(PatternId(2), pattern(17));
    f.song.order = vec![occurrence(2), occurrence(1), occurrence(1)];
    for bpm in [70.0, 137.123456789, 170.0, 399.72241498959056] {
        for lpb in [1, 3, 7, 128, 256] {
            for delta in [-1, 1] {
                f.song.bpm = bpm;
                f.song.lines_per_beat = lpb;
                let mut c = f.config();
                c.fields.timing = true;
                c.nudge_frames = delta;
                let p = preview(&f.song, c).unwrap();
                let before = events(&f.song);
                let after = events(p.candidate());
                assert_eq!(before.len(), after.len());
                for (a, b) in before.iter().zip(&after) {
                    assert_eq!(
                        i128::from(b.frame),
                        i128::from(a.frame) + i128::from(delta),
                        "BPM{bpm} LPB{lpb}"
                    );
                    assert_eq!(a.command, b.command);
                }
                let json = serde_json::to_vec(p.candidate()).unwrap();
                let decoded: Song = serde_json::from_slice(&json).unwrap();
                assert_eq!(&decoded, p.candidate());
                assert_eq!(events(&decoded), after);
            }
        }
    }
}
#[test]
fn explicit_kit_velocity_scales_pcm_without_changing_selector_or_native_pitch() {
    let mut f = Fixture::new();
    f.kit();
    f.put(
        0,
        0,
        Cell {
            velocity: Some(0.4),
            ..note(36, 1)
        },
    );
    let mut c = f.config();
    c.fields.velocity = true;
    c.velocity_scale = 0.5;
    let p = preview(&f.song, c).unwrap();
    assert_eq!(cell(p.candidate(), 0, 0).note, Note::Note(36));
    let before = pcm(&f.song);
    let after = pcm(p.candidate());
    for (a, b) in before[..2000].iter().zip(&after[..2000]) {
        assert_eq!(*a, *b * 2.0);
    }
}
#[test]
fn bounded_selection_and_timing_context_product_reject_before_large_expansion() {
    let mut f = Fixture::new();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 4097;
    for row in 0..4097 {
        f.put(row, 0, note(60, 1));
    }
    let mut c = f.config();
    c.scope.row_end = 4097;
    assert!(preview(&f.song, c)
        .unwrap_err()
        .contains("4096 stored cells"));
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 8;
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .clear();
    for row in 0..8 {
        f.put(
            row,
            0,
            Cell {
                delay: Some(0.1),
                effects: StepEffects {
                    retrigger: Some(Retrigger {
                        repeats: 16,
                        spacing: 0.01,
                    }),
                    cut: Some(0.5),
                    ..Default::default()
                },
                ..note(60, 1)
            },
        );
    }
    f.song.order = vec![occurrence(1); 512];
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 1;
    assert!(preview(&f.song, c)
        .unwrap_err()
        .contains("65536 event-by-use"));
}
#[test]
fn pure_preview_does_not_read_sources_or_expand_inherited_selection_fields() {
    let mut f = Fixture::new();
    f.put(0, 0, note(60, 1));
    f.put(
        2,
        0,
        Cell {
            instrument: Some(InstrumentId(2)),
            velocity: Some(0.3),
            ..Default::default()
        },
    );
    f.put(
        3,
        0,
        Cell {
            note: Note::Note(64),
            ..Default::default()
        },
    );
    let mut c = f.config();
    c.scope.row_start = 2;
    c.scope.row_end = 5;
    c.fields.timing = true;
    c.nudge_frames = 300;
    let p = preview(&f.song, c.clone()).unwrap();
    let starts: Vec<_> = events(p.candidate())
        .into_iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .collect();
    assert_eq!(starts[0].instrument, InstrumentId(1));
    assert_eq!(starts[1].instrument, InstrumentId(2));
    assert_eq!(cell(p.candidate(), 3, 0).instrument, None);
    std::fs::remove_file(&f.song.samples[&SampleId(1)].path).unwrap();
    assert!(preview(&f.song, c).is_ok());
}

#[test]
fn ideal_scope_and_fx_boundaries_cannot_be_squeezed_back_by_ulp_search() {
    let mut f = Fixture::new();
    f.put(1, 0, note(60, 1));
    let mut c = f.config();
    c.scope.row_end = 2;
    c.fields.timing = true;
    c.nudge_frames = 6000;
    assert!(preview(&f.song, c.clone())
        .unwrap_err()
        .contains("no boundary clamping"));
    c.nudge_frames = -6001;
    c.scope.row_end = 8;
    assert!(preview(&f.song, c).is_err());
    f.put(
        1,
        0,
        Cell {
            delay: Some(0.0),
            effects: StepEffects {
                cut: Some(0.5),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 3000;
    assert!(preview(&f.song, c).unwrap_err().contains("row-local FX"));
    f.put(
        1,
        0,
        Cell {
            effects: StepEffects {
                retrigger: Some(Retrigger {
                    repeats: 1,
                    spacing: 0.5,
                }),
                ..Default::default()
            },
            ..note(60, 1)
        },
    );
    let mut c = f.config();
    c.fields.timing = true;
    c.nudge_frames = 3000;
    assert!(preview(&f.song, c).unwrap_err().contains("row-local FX"));
}
