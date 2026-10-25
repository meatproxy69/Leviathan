//! Original generated audio + saved editable song; independent of test fixtures.
use aqua_pattern_core::{
    project::{self, SaveMode},
    song::*,
    song_render, Result, RATE,
};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};
fn sample(path: &Path, kind: &str) -> Result<()> {
    let mut w = hound::WavWriter::new(
        OpenOptions::new().write(true).create_new(true).open(path)?,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut rng = 0x12345678_u32;
    for i in 0..RATE * 2 {
        let t = i as f64 / RATE as f64;
        let phase = std::f64::consts::TAU * 261.625565 * t;
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let noise = (rng as f64 / u32::MAX as f64) * 2.0 - 1.0;
        let attack = (t / 0.005).min(1.0);
        let x = match kind {
            "kick" => {
                (std::f64::consts::TAU * (48.0 * t + 70.0 * 0.025 * (1.0 - (-t / 0.025).exp())))
                    .sin()
                    * (-t * 18.0).exp()
                    * 0.4
            }
            "hat" => noise * (-t * 65.0).exp() * 0.13,
            "bass" => (phase.sin() + 0.2 * (phase * 2.0).sin()) * attack * (-t * 1.5).exp() * 0.22,
            "chords" => {
                (phase.sin() + 0.25 * (phase * 3.0).sin()) * attack * (-t * 1.8).exp() * 0.16
            }
            _ => (phase.sin() + 0.3 * (phase * 2.003).sin()) * attack * (-t * 5.0).exp() * 0.22,
        };
        for gain in [1.0, if kind == "lead" { 0.85 } else { 1.0 }] {
            w.write_sample((x * gain) as f32)?;
        }
    }
    w.finalize()?;
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty()
        || args.len() > 2
        || (args.len() == 2
            && !matches!(
                args[1].as_str(),
                "--effects" | "--phrases" | "--arrangement" | "--groove" | "--generate"
            ))
    {
        return Err(
            "usage: song_demo NEW_DIRECTORY [--effects | --phrases | --arrangement | --groove | --generate]"
                .into(),
        );
    }
    let effects = args.get(1).is_some_and(|mode| mode == "--effects");
    let generated = args.get(1).is_some_and(|mode| mode == "--generate");
    let groove = args.get(1).is_some_and(|mode| mode == "--groove") || generated;
    let arrangement = args.get(1).is_some_and(|mode| mode == "--arrangement") || groove;
    let phrases = args.get(1).is_some_and(|mode| mode == "--phrases") || arrangement;
    let dir = PathBuf::from(&args[0]);
    fs::create_dir(&dir)?;
    let dir = dir.canonicalize()?;
    let mut song = example_song();
    song.title = "Leviathan • First current".into();
    for (n, kind) in [(1, "kick"), (2, "hat"), (3, "bass"), (4, "lead")] {
        let path = dir.join(format!("{kind}.wav"));
        sample(&path, kind)?;
        song.instruments.get_mut(&InstrumentId(n)).unwrap().sample = Some(path);
        song.instruments.get_mut(&InstrumentId(n)).unwrap().name = kind.into();
        song.tracks.get_mut(&TrackId(n)).unwrap().name = kind.into();
    }
    for (id, pattern) in &mut song.patterns {
        pattern.tracks.clear();
        pattern.rows = if id.0 == 1 { 32 } else { 16 };
        for row in 0..pattern.rows {
            for (track, step) in [(1, 4), (2, 2), (3, 8), (4, 4)] {
                if row % step != 0 {
                    continue;
                }
                let pitch = match track {
                    3 => {
                        if id.0 == 1 {
                            36
                        } else {
                            44
                        }
                    }
                    4 => [60, 63, 67, 70][(row / 4) as usize % 4] + if id.0 == 2 { 3 } else { 0 },
                    _ => 60,
                };
                let cell = Cell {
                    note: Note::Note(pitch),
                    instrument: Some(InstrumentId(track)),
                    velocity: Some(if track == 2 && row % 4 != 0 {
                        0.25
                    } else {
                        0.5
                    }),
                    pan: Some(if track == 2 {
                        0.4
                    } else if track == 4 {
                        -0.3
                    } else {
                        0.0
                    }),
                    delay: if track == 2 && row % 4 != 0 {
                        Some(0.08)
                    } else {
                        None
                    },
                    ..Default::default()
                };
                pattern
                    .tracks
                    .entry(TrackId(track))
                    .or_default()
                    .entry(row)
                    .or_default()
                    .insert(0, cell);
            }
        }
    }
    if effects {
        song.title = "Leviathan • Step currents".into();
        for pattern in song.patterns.values_mut() {
            // Generated, editable examples of all five effects in both A/B patterns.
            pattern
                .tracks
                .get_mut(&TrackId(1))
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .effects = StepEffects {
                retrigger: Some(2),
                cut: Some(0.75),
                ..Default::default()
            };
            pattern
                .tracks
                .get_mut(&TrackId(2))
                .unwrap()
                .get_mut(&2)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .effects = StepEffects {
                offset: Some(0.3),
                reverse: true,
                ..Default::default()
            };
            pattern
                .tracks
                .get_mut(&TrackId(3))
                .unwrap()
                .get_mut(&8)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .effects = StepEffects {
                glide: Some(24.0),
                ..Default::default()
            };
            pattern
                .tracks
                .get_mut(&TrackId(4))
                .unwrap()
                .get_mut(&4)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .effects = StepEffects {
                reverse: true,
                offset: Some(0.125),
                ..Default::default()
            };
            pattern
                .tracks
                .get_mut(&TrackId(4))
                .unwrap()
                .get_mut(&8)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .effects = StepEffects {
                glide: Some(36.0),
                ..Default::default()
            };
        }
    }
    if phrases {
        song.title = "Leviathan • Phrase currents".into();
        for owner in [1, 3, 4] {
            let root = if owner == 3 { 48 } else { 60 };
            let mut phrase = Phrase {
                name: format!("{} phrase", song.instruments[&InstrumentId(owner)].name),
                rows: if owner == 1 { 4 } else { 8 },
                lines_per_beat: 8,
                note_columns: if owner == 4 { 2 } else { 1 },
                root_note: root,
                ..Default::default()
            };
            for row in (0..phrase.rows).step_by(2) {
                let pitch = if owner == 1 {
                    root
                } else {
                    root + [0, 7, 12, 7][(row / 2) as usize]
                };
                let mut cell = Cell {
                    note: Note::Note(pitch),
                    velocity: Some(if owner == 4 { 0.4 } else { 0.5 }),
                    ..Default::default()
                };
                if owner == 3 && row == 2 {
                    cell.effects.glide = Some(24.0);
                }
                if owner == 1 && row == 2 {
                    cell.effects.retrigger = Some(2);
                    cell.effects.cut = Some(0.8);
                }
                phrase.cells.entry(row).or_default().insert(0, cell);
            }
            if owner == 4 {
                phrase.cells.entry(1).or_default().insert(
                    1,
                    Cell {
                        note: Note::Note(72),
                        velocity: Some(0.25),
                        pan: Some(0.4),
                        effects: StepEffects {
                            reverse: true,
                            offset: Some(0.5),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                );
                phrase.cells.entry(6).or_default().insert(
                    1,
                    Cell {
                        note: Note::Off,
                        ..Default::default()
                    },
                );
            }
            song.instruments
                .get_mut(&InstrumentId(owner))
                .unwrap()
                .phrases
                .insert(PhraseId(1), phrase);
            for pattern in song.patterns.values_mut() {
                for columns in pattern
                    .tracks
                    .get_mut(&TrackId(owner))
                    .unwrap()
                    .values_mut()
                {
                    for cell in columns.values_mut() {
                        if matches!(cell.note, Note::Note(_)) {
                            cell.phrase = Some(PhraseId(1));
                        }
                    }
                }
            }
        }
    }
    if arrangement {
        // Equal-sized source/destination blocks keep alias timing explicit.
        song.clone_pattern(PatternId(1), PatternId(3))?;
        let p = song.patterns.get_mut(&PatternId(3)).unwrap();
        p.name = "A shared rhythm".into();
        for track in [TrackId(1), TrackId(2)] {
            p.tracks.remove(&track);
            p.aliases.insert(
                track,
                BlockRef {
                    pattern: PatternId(1),
                    track,
                },
            );
        }
        song.order[2].pattern = PatternId(3);
        song.order[0].section = Some(Section {
            id: SectionId(1),
            name: "Verse".into(),
        });
        song.order[1].section = Some(Section {
            id: SectionId(2),
            name: "Bridge".into(),
        });
        song.order[1].muted_tracks.insert(TrackId(4));
        song.order[2].section = Some(Section {
            id: SectionId(3),
            name: "Return".into(),
        });
        // Demonstrate a true independent variation without extending demo duration.
        let mut editor = aqua_pattern_core::editor::Editor::default();
        editor.replace_document(song)?;
        editor.edit_mode = true;
        editor.select(2, TrackId(3))?;
        editor.append_section(true)?;
        let copy = editor.pattern_id();
        song = editor.song;
        song.order.pop();
        song.order[2].pattern = copy;
        song.order[2].section.as_mut().unwrap().name = "Return variation".into();
        let cell = song
            .patterns
            .get_mut(&copy)
            .unwrap()
            .tracks
            .get_mut(&TrackId(3))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap();
        cell.note = Note::Note(43);
        // Keep an alias in the active variation, and local independent bass/lead.
        let p = song.patterns.get_mut(&copy).unwrap();
        p.tracks.remove(&TrackId(1));
        p.aliases.insert(
            TrackId(1),
            BlockRef {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
    }
    if groove {
        use aqua_pattern_core::{
            editor::Editor,
            transform::{Scope, Transform},
        };
        let mut editor = Editor::default();
        editor.replace_document(song)?;
        editor.edit_mode = true;
        let preview = editor.preview_transform(
            Scope::Song,
            0..0,
            &Transform::Groove(vec![0.0, 0.0, 0.25, 0.0]),
        )?;
        editor.apply_transform(preview)?;
        editor.select(2, TrackId(3))?;
        let preview = editor.preview_transform(
            Scope::Track,
            0..editor.pattern().rows,
            &Transform::Transpose(2),
        )?;
        editor.apply_transform(preview)?;
        editor.select(0, TrackId(2))?;
        let preview = editor.preview_transform(
            Scope::Track,
            0..editor.pattern().rows,
            &Transform::ScaleVelocity(0.8),
        )?;
        editor.apply_transform(preview)?;
        song = editor.song;
        song.title = "Leviathan • Committed groove".into();
    }
    if generated {
        use aqua_pattern_core::{
            editor::Editor,
            generation::{self, Recipe, WriteMode},
        };
        let mut editor = Editor::default();
        editor.replace_document(song)?;
        editor.edit_mode = true;
        editor.select(2, TrackId(4))?;
        editor.row = 0;
        editor.column = 0;
        let recipe = Recipe {
            rows: 32,
            cycle_steps: 8,
            hits: 5,
            probability: 0.75,
            seed: 42,
            sequence: generation::parse_sequence("60,67 63,70 off,off 67,74 . 70,77")?,
            instrument: InstrumentId(4),
            velocity: 0.25,
            pan: 0.15,
            ..Default::default()
        };
        let preview =
            editor.preview_generation("Seeded return chords", &recipe, WriteMode::Replace)?;
        editor.apply_generation(preview)?;
        let other =
            editor.preview_generation("Seeded return chords", &recipe, WriteMode::Replace)?;
        assert_eq!(other.summary.changed_cells, 0);
        song = editor.song;
        song.title = "Leviathan • Seeded editable current".into();
    }
    let project_path = dir.join("first-current.leviathan.json");
    project::save(&song, &project_path, SaveMode::CreateNew)?;
    let (song, _) = project::load(&project_path)?;
    for name in ["first-current.wav", "first-current-repeat.wav"] {
        let (stats, m) = song_render::export(&song, None, &dir.join(name))?;
        println!(
            "{name}: {} frames, peak {}, {} events",
            stats.frames,
            stats.peak,
            m.events.len()
        );
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
