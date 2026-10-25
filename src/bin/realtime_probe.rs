use aqua_pattern_core::{dispatch, Event, Result};
use phonic::{
    generators::Sampler,
    outputs::{CpalOutput, CpalOutputConfig},
    GeneratorPlaybackOptions, OutputDevice,
};
fn run() -> Result<()> {
    let sample = std::env::args()
        .nth(1)
        .ok_or("provide generated sample path")?;
    let mut output = CpalOutput::open_with_config(CpalOutputConfig {
        sample_rate: Some(48000),
        ..Default::default()
    })?;
    let sampler = Sampler::from_file(
        sample,
        GeneratorPlaybackOptions::default(),
        output.channel_count(),
        output.sample_rate(),
    )?;
    dispatch(
        &sampler,
        &Event::NoteOn {
            id: 1,
            midi_note: 60,
            velocity: 0.2,
            pan: 0.0,
        },
    )?;
    output.play(Box::new(sampler));
    output.resume();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let samples = output.sample_position();
    output.stop();
    output.close();
    if samples == 0 {
        return Err("audio callback did not advance".into());
    }
    println!("callback samples advanced: {samples}; audibility and latency not measured");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("real-time probe failed: {e}");
        std::process::exit(1);
    }
}
