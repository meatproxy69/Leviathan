use aqua_pattern_core::{
    generate_sample,
    jobs::{Completed, Jobs, Request},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn dir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("leviathan-jobs-{}-{name}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    p
}
fn finish(jobs: &mut Jobs) -> Result<Completed, String> {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(r) = jobs.poll() {
            return r;
        }
        assert!(Instant::now() < end, "worker hung");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn background_load_export_and_overlap_protection() {
    let d = dir("export");
    let sample = d.join("sample.wav");
    generate_sample(&sample).unwrap();
    let mut jobs = Jobs::default();
    jobs.start(Request::Load(sample.clone())).unwrap();
    assert!(jobs.busy());
    assert!(jobs.start(Request::Load(sample.clone())).is_err());
    match finish(&mut jobs).unwrap() {
        Completed::Loaded(info) => {
            assert_eq!(info.frames, 48000);
            assert_eq!(info.channels, 2);
        }
        _ => panic!("wrong result"),
    }
    assert!(!jobs.busy());
    let output = d.join("render.wav");
    jobs.start(Request::Export {
        sample: sample.clone(),
        output: output.clone(),
        bpm: 120.0,
        bars: 2,
    })
    .unwrap();
    match finish(&mut jobs).unwrap() {
        Completed::Exported { stats, .. } => {
            assert_eq!(stats.frames, 216000);
            assert!(stats.peak > 0.0);
        }
        _ => panic!("wrong result"),
    }
    assert!(output.with_extension("events.json").exists());
    let before = std::fs::read(&output).unwrap();
    jobs.start(Request::Export {
        sample,
        output: output.clone(),
        bpm: 120.0,
        bars: 2,
    })
    .unwrap();
    assert!(finish(&mut jobs).is_err());
    assert_eq!(before, std::fs::read(output).unwrap());
    std::fs::remove_dir_all(d).unwrap();
}
#[test]
fn worker_errors_recover_and_preserve_existing_manifest() {
    let d = dir("errors");
    let mut jobs = Jobs::default();
    jobs.start(Request::Load(d.join("missing.wav"))).unwrap();
    assert!(finish(&mut jobs).is_err());
    assert!(!jobs.busy());
    let sample = d.join("sample.wav");
    generate_sample(&sample).unwrap();
    let output = d.join("render.wav");
    let json = output.with_extension("events.json");
    std::fs::write(&json, b"keep").unwrap();
    jobs.start(Request::Export {
        sample: sample.clone(),
        output: output.clone(),
        bpm: 120.0,
        bars: 2,
    })
    .unwrap();
    assert!(finish(&mut jobs).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read(&json).unwrap(), b"keep");
    jobs.start(Request::Export {
        sample,
        output: d.join("bad.wav"),
        bpm: f64::NAN,
        bars: 2,
    })
    .unwrap();
    assert!(finish(&mut jobs).is_err());
    assert!(!d.join("bad.events.json").exists());
    std::fs::remove_dir_all(d).unwrap();
}
