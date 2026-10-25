use aqua_pattern_core::*;
use phonic::{generators::Sampler, Generator, GeneratorPlaybackOptions};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "aqua-core-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn audio(path: &std::path::Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(|x| x.unwrap())
        .collect()
}
#[test]
fn exact_grid_and_deterministic_manifest() {
    let m = pattern_manifest(120.0, 2).unwrap();
    assert_eq!(m.musical_frames, 192000);
    assert_eq!(m.events[2].frame, 6000);
    assert_eq!(m.events[8].frame, 24000);
    assert_eq!(m.events[1].frame, 3000);
    assert_eq!(m.events.len(), 65);
    assert_eq!(
        serde_json::to_vec(&m).unwrap(),
        serde_json::to_vec(&pattern_manifest(120.0, 2).unwrap()).unwrap()
    );
    let fractional = pattern_manifest(137.0, 256).unwrap();
    assert_eq!(
        fractional.musical_frames,
        (256.0_f64 * 4.0 * 48000.0 * 60.0 / 137.0).round() as u64
    );
    for bpm in [0.0, -1.0, f64::NAN, f64::INFINITY, 401.0] {
        assert!(pattern_manifest(bpm, 2).is_err());
    }
    assert!(pattern_manifest(120.0, 0).is_err());
}
#[test]
fn stereo_audio_exact_length_and_repeatability() {
    let s = Sandbox::new();
    let sample = s.path("sample.wav");
    generate_sample(&sample).unwrap();
    let m = pattern_manifest(120.0, 2).unwrap();
    let a = s.path("a.wav");
    let b = s.path("b.wav");
    let stats = render(&sample, &a, &m, 512).unwrap();
    render(&sample, &b, &m, 512).unwrap();
    assert_eq!(stats.frames, 216000);
    assert!(stats.peak > 0.01 && stats.peak < 1.0);
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let reader = hound::WavReader::open(&a).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 48000);
    let samples = audio(&a);
    assert_eq!(samples.len(), 432000);
    assert!(samples.iter().all(|x| x.is_finite() && x.abs() < 1.0));
    assert!(samples
        .as_chunks::<2>()
        .0
        .iter()
        .any(|x| (x[0] - x[1]).abs() > 0.01));
    assert!(samples[samples.len() - 2000..]
        .iter()
        .all(|x| x.abs() < 1e-6));
    assert!(render(&sample, &a, &m, 512).is_err()); // never clobber
}
#[test]
fn note_off_changes_sound_and_queue_errors_propagate() {
    let s = Sandbox::new();
    let sample = s.path("sample.wav");
    generate_sample(&sample).unwrap();
    let mut m = pattern_manifest(120.0, 1).unwrap();
    m.events = vec![
        TimedEvent {
            frame: 0,
            event: Event::NoteOn {
                id: 1,
                midi_note: 60,
                velocity: 0.5,
                pan: 0.0,
            },
        },
        TimedEvent {
            frame: 3000,
            event: Event::NoteOff { id: 1 },
        },
    ];
    render(&sample, &s.path("off.wav"), &m, 512).unwrap();
    m.events.pop();
    render(&sample, &s.path("on.wav"), &m, 512).unwrap();
    let off = audio(&s.path("off.wav"));
    let on = audio(&s.path("on.wav"));
    let energy = |xs: &[f32]| {
        xs[12000..24000]
            .iter()
            .map(|x| (*x as f64).powi(2))
            .sum::<f64>()
    };
    assert!(energy(&off) < energy(&on) * 0.01);
    let sampler =
        Sampler::from_file(&sample, GeneratorPlaybackOptions::default(), 2, RATE).unwrap();
    for _ in 0..sampler.playback_message_queue().capacity() {
        dispatch(&sampler, &Event::AllNotesOff).unwrap();
    }
    assert!(dispatch(&sampler, &Event::AllNotesOff)
        .unwrap_err()
        .to_string()
        .contains("queue is full"));
}
fn bounded(args: &[&str]) -> std::process::ExitStatus {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aqua-pattern-core"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("CLI exceeded five seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn cli_exits_cleanly_and_reports_bad_inputs() {
    let s = Sandbox::new();
    let sample = s.path("sample.wav");
    generate_sample(&sample).unwrap();
    assert!(bounded(&[
        "--sample",
        sample.to_str().unwrap(),
        "--bpm",
        "120",
        "--bars",
        "2",
        "--output",
        s.path("cli.wav").to_str().unwrap()
    ])
    .success());
    assert!(!bounded(&[
        "--sample",
        "missing.wav",
        "--output",
        s.path("missing.wav").to_str().unwrap()
    ])
    .success());
    assert!(!bounded(&[
        "--sample",
        sample.to_str().unwrap(),
        "--bpm",
        "NaN",
        "--output",
        s.path("bad.wav").to_str().unwrap()
    ])
    .success());
    assert!(!s.path("missing.wav").exists());
    assert!(!s.path("bad.wav").exists());
}
#[test]
fn event_boundary_is_sample_accurate_across_render_blocks() {
    let s = Sandbox::new();
    let sample = s.path("sample.wav");
    generate_sample(&sample).unwrap();
    let mut m = pattern_manifest(120.0, 1).unwrap();
    m.events = vec![
        TimedEvent {
            frame: 6000,
            event: Event::NoteOn {
                id: 1,
                midi_note: 60,
                velocity: 0.5,
                pan: 0.0,
            },
        },
        TimedEvent {
            frame: 9000,
            event: Event::NoteOff { id: 1 },
        },
    ];
    render(&sample, &s.path("a.wav"), &m, 512).unwrap();
    render(&sample, &s.path("b.wav"), &m, 127).unwrap();
    let a = audio(&s.path("a.wav"));
    let b = audio(&s.path("b.wav"));
    assert!(a[..12000].iter().all(|x| *x == 0.0));
    assert!(a[12000..13000].iter().any(|x| x.abs() > 0.01));
    let max_delta = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0_f32, f32::max);
    eprintln!("cross-block maximum sample delta: {max_delta}");
    assert!(
        max_delta < 0.001,
        "cross-block maximum sample delta: {max_delta}"
    );
}
