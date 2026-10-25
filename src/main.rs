use aqua_pattern_core::*;
use std::{fs::OpenOptions, path::PathBuf};
fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut sample = None;
    let mut output = None;
    let mut bpm = 120.0;
    let mut bars = 2;
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!("aqua-pattern-core --sample FILE --bpm 120 --bars 2 --output NEW.wav\naqua-pattern-core --generate-sample NEW.wav\nUses pinned pattrns sequencing and phonic sample playback. Refuses existing output files.");
            return Ok(());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--generate-sample" => {
                if args.next().is_some() {
                    return Err("generate-sample must be used alone".into());
                }
                return generate_sample(&PathBuf::from(value));
            }
            "--sample" => sample = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--bpm" => bpm = value.parse()?,
            "--bars" => bars = value.parse()?,
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    let manifest = pattern_manifest(bpm, bars)?;
    let sample = sample.ok_or("--sample is required")?;
    let output = output.ok_or("--output is required")?;
    let manifest_path = output.with_extension("events.json");
    if manifest_path.exists() {
        return Err("manifest already exists".into());
    }
    let stats = render(&sample, &output, &manifest, 512)?;
    serde_json::to_writer_pretty(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(manifest_path)?,
        &manifest,
    )?;
    println!("{}", serde_json::to_string(&stats)?);
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
