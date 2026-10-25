//! Original generated five-part demo; separate assets from test fixtures.
use aqua_pattern_core::{
    pattern::{manifest_from_pattern, time_base},
    render, Result, RATE,
};
use pattrns::prelude::*;
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
            .ok_or("usage: musical_demo NEW_DIRECTORY")?,
    );
    fs::create_dir(&dir)?;
    let time = time_base(120.0, 4)?;
    let mut parts = Vec::new();
    parts.push((
        "kick",
        time.every_nth_beat(1.0)
            .emit(new_note_emitter((Note::C4, None, 0.7))),
    ));
    parts.push((
        "hat",
        time.every_nth_eighth(1.0)
            .emit(new_note_sequence_emitter(vec![
                new_note((Note::C4, None, 0.4, 0.5)),
                new_note((Note::C4, None, 0.22, -0.5)),
            ])),
    ));
    let bass = [
        Note::C2,
        Note::C2,
        Note::Gs1,
        Note::Gs1,
        Note::Ds2,
        Note::Ds2,
        Note::As1,
        Note::As1,
    ]
    .into_iter()
    .flat_map(|n| [new_note((n, None, 0.55, -0.1)), new_note(Note::OFF)])
    .collect();
    parts.push((
        "bass",
        time.every_nth_beat(1.0)
            .emit(new_note_sequence_emitter(bass)),
    ));
    let chords = vec![
        [Note::C4, Note::Ds4, Note::G4],
        [Note::Gs3, Note::C4, Note::Ds4],
        [Note::Ds4, Note::G4, Note::As4],
        [Note::As3, Note::D4, Note::F4],
    ]
    .into_iter()
    .map(|chord| {
        chord
            .into_iter()
            .enumerate()
            .map(|(i, n)| new_note((n, None, 0.3, (i as f32 - 1.0) * 0.65)))
            .collect()
    })
    .collect();
    parts.push((
        "chords",
        time.every_nth_bar(1.0)
            .emit(new_polyphonic_note_sequence_emitter(chords)),
    ));
    let lead = [
        Note::G4,
        Note::Ds4,
        Note::C5,
        Note::As4,
        Note::Gs4,
        Note::G4,
        Note::Ds4,
        Note::F4,
    ]
    .into_iter()
    .flat_map(|n| [new_note((n, None, 0.35, 0.25)), new_note(Note::OFF)])
    .collect();
    parts.push((
        "lead",
        time.every_nth_eighth(1.0)
            .emit(new_note_sequence_emitter(lead)),
    ));
    let mut mix = Vec::<f32>::new();
    for (name, mut pattern) in parts {
        let input = dir.join(format!("{name}-sample.wav"));
        sample(&input, name)?;
        let manifest = manifest_from_pattern(&mut pattern, 120.0, 4)?;
        let output = dir.join(format!("{name}.wav"));
        let stats = render(&input, &output, &manifest, 512)?;
        serde_json::to_writer_pretty(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(format!("{name}.events.json")))?,
            &manifest,
        )?;
        let data: Vec<f32> = hound::WavReader::open(output)?
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()?;
        if mix.is_empty() {
            mix.resize(data.len(), 0.0);
        }
        if data.len() != mix.len() {
            return Err("stem lengths differ".into());
        }
        for (out, value) in mix.iter_mut().zip(data) {
            *out += value * 0.75;
        }
        println!("{name}: {}", serde_json::to_string(&stats)?);
    }
    let mut writer = hound::WavWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("aqua-five-parts.wav"))?,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut peak = 0.0_f32;
    for x in mix.iter().copied() {
        if !x.is_finite() || x.abs() > 1.0 {
            return Err("invalid mix amplitude".into());
        }
        peak = peak.max(x.abs());
        writer.write_sample(x)?;
    }
    writer.finalize()?;
    println!("mix: {} frames; peak {peak}", mix.len() / 2);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("demo failed: {e}");
        std::process::exit(1);
    }
}
