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
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: song_demo NEW_DIRECTORY")?,
    );
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
                    ..Cell::default()
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
