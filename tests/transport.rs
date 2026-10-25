use aqua_pattern_core::{
    generate_sample,
    song::*,
    song_render,
    transport::{location, Producer, Shared},
};
use std::{
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
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-m5-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("sample.wav");
        generate_sample(&path).unwrap();
        let mut song = example_song();
        for i in song.instruments.values_mut() {
            i.sample = Some(path.clone());
        }
        for p in song.patterns.values_mut() {
            p.rows = 4;
            p.tracks.clear();
        }
        song.patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(0)
            .or_default()
            .insert(
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(0.3),
                    ..Default::default()
                },
            );
        Self { dir, song }
    }
    fn stream(&self) -> (Arc<Shared>, Producer) {
        let s = Arc::new(Shared::default());
        let p = Producer::new(&self.song, None, s.clone()).unwrap();
        (s, p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn prefill(p: &mut Producer) {
    for _ in 0..8 {
        p.pump().unwrap();
    }
}
#[test]
fn streamed_pcm_matches_offline_and_natural_end_is_silent() {
    let f = Fixture::new();
    let output = f.dir.join("offline.wav");
    let (stats, _) = song_render::export(&f.song, None, &output).unwrap();
    let expected: Vec<f32> = hound::WavReader::open(output)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    let (s, mut p) = f.stream();
    let mut got = Vec::new();
    for _ in 0..1000 {
        prefill(&mut p);
        let mut block = [0.0; 512];
        s.fill(&mut block);
        got.extend_from_slice(&block);
        if s.ended.load(Ordering::Relaxed) {
            break;
        }
    }
    assert!(s.ended.load(Ordering::Relaxed));
    assert_eq!(&got[..stats.frames as usize * 2], expected);
    assert!(got[stats.frames as usize * 2..].iter().all(|v| *v == 0.0));
    let mut silence = [1.0; 512];
    s.fill(&mut silence);
    assert!(silence.iter().all(|v| *v == 0.0));
}
#[test]
fn stop_rejects_buffered_audio_and_repeated_sessions_are_independent() {
    let f = Fixture::new();
    for _ in 0..3 {
        let (s, mut p) = f.stream();
        prefill(&mut p);
        let mut data = [0.0; 512];
        s.fill(&mut data);
        assert!(data.iter().any(|v| *v != 0.0));
        s.stop();
        data.fill(1.0);
        s.fill(&mut data);
        assert!(data.iter().all(|v| *v == 0.0));
        assert!(s.quit.load(Ordering::Relaxed));
    }
}
#[test]
fn cold_seek_drops_earlier_voices_and_latest_command_wins() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    prefill(&mut p);
    s.seek(12000);
    let mut data = [1.0; 512];
    s.fill(&mut data);
    assert!(data.iter().all(|v| *v == 0.0));
    prefill(&mut p);
    s.fill(&mut data);
    assert!(data.iter().all(|v| *v == 0.0));
    assert_eq!(s.position.load(Ordering::Relaxed), 12255);
    s.seek(0);
    s.seek(24000);
    s.seek(6000);
    prefill(&mut p);
    s.fill(&mut data);
    assert!(data.iter().all(|v| *v == 0.0));
    assert_eq!(s.position.load(Ordering::Relaxed), 6255);
    s.seek(0);
    prefill(&mut p);
    s.fill(&mut data);
    assert!(data.iter().any(|v| *v != 0.0));
}
#[test]
fn whole_song_loop_restarts_deterministically_without_tail() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    s.looping.store(true, Ordering::SeqCst);
    let frames = p.engine.manifest.musical_frames as usize;
    let mut got = Vec::new();
    while got.len() < frames * 4 {
        prefill(&mut p);
        let mut block = [0.0; 512];
        s.fill(&mut block);
        got.extend_from_slice(&block);
    }
    assert_eq!(&got[..frames * 2], &got[frames * 2..frames * 4]);
    assert!(!s.ended.load(Ordering::Relaxed));
    assert_eq!(s.underruns.load(Ordering::Relaxed), 0);
    s.set_loop(false, frames as u64);
    for _ in 0..1000 {
        prefill(&mut p);
        s.fill(&mut [0.0; 512]);
        if s.ended.load(Ordering::Relaxed) {
            break;
        }
    }
    assert!(s.ended.load(Ordering::Relaxed));
}
#[test]
fn seek_invalidates_queued_end_and_playhead_is_separate() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    let end = p.engine.manifest.musical_frames + p.engine.manifest.tail_frames;
    while s.position.load(Ordering::Relaxed) + 1024 < end {
        prefill(&mut p);
        s.fill(&mut [0.0; 512]);
    }
    s.seek(0);
    prefill(&mut p);
    s.fill(&mut [0.0; 512]);
    assert!(s.running.load(Ordering::Relaxed));
    assert!(!s.ended.load(Ordering::Relaxed));
    assert_eq!(location(&f.song, 24000), (1, 0));
    assert_eq!(location(&f.song, 48000), (2, 0));
}
#[test]
fn underflow_is_silence_and_is_counted_not_stale_replay() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    prefill(&mut p);
    let mut out = vec![0.0; 8192];
    s.fill(&mut out);
    assert!(s.underruns.load(Ordering::Relaxed) > 0);
    assert!(out[4096..].iter().all(|v| *v == 0.0));
}

#[test]
fn device_failure_silences_queued_audio_until_a_new_session() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    prefill(&mut p);
    s.device_failed();
    s.seek(0);
    prefill(&mut p);
    let mut out = [1.0; 512];
    s.fill(&mut out);
    assert!(out.iter().all(|v| *v == 0.0));
    assert!(s.device_error.load(Ordering::SeqCst));
    assert!(!s.running.load(Ordering::SeqCst));
    assert!(!s.ready.load(Ordering::SeqCst));
    assert_eq!(s.audible_frames.load(Ordering::Relaxed), 0);
    let (next, mut producer) = f.stream();
    prefill(&mut producer);
    next.fill(&mut out);
    assert!(out.iter().any(|v| *v != 0.0));
    assert!(!next.device_error.load(Ordering::SeqCst));
}

#[test]
fn fractional_tempo_playhead_changes_on_the_rendered_row_boundary() {
    let mut song = example_song();
    song.bpm = 137.3;
    song.lines_per_beat = 7;
    let per_row = 48000.0 * 60.0 / (song.bpm * song.lines_per_beat as f64);
    let mut absolute_row = 0;
    for (occurrence, order) in song.order.iter().enumerate() {
        for row in 0..song.patterns[&order.pattern].rows {
            let frame = (absolute_row as f64 * per_row).round() as u64;
            assert_eq!(location(&song, frame), (occurrence, row));
            let next = ((absolute_row + 1) as f64 * per_row).round() as u64;
            assert_eq!(location(&song, next - 1), (occurrence, row));
            absolute_row += 1;
        }
    }
}
