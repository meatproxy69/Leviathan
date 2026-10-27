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
// Count allocations only on the thread executing the callback under test, so
// parallel fixture/sample setup does not contaminate the real-time-path check.
thread_local! {
    static CALLBACK_ALLOCATIONS: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
}
struct TestAllocator;
#[global_allocator]
static ALLOCATOR: TestAllocator = TestAllocator;
fn record_allocation() {
    let _ = CALLBACK_ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}
unsafe impl std::alloc::GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation();
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record_allocation();
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
fn without_callback_allocations(run: impl FnOnce()) {
    CALLBACK_ALLOCATIONS.with(|count| count.set(Some(0)));
    run();
    let count = CALLBACK_ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(count, 0, "audio callback allocated");
}
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
    assert!(!s.is_running());
    assert!(!s.is_ready());
    assert!(s.has_ended());
    assert_eq!(s.position(), stats.frames - 1);
    s.seek(0);
    s.set_loop(true, stats.frames);
    prefill(&mut p);
    assert!(!s.is_running());
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
    assert_eq!(s.position(), 12255);
    s.seek(0);
    s.seek(24000);
    s.seek(6000);
    prefill(&mut p);
    s.fill(&mut data);
    assert!(data.iter().all(|v| *v == 0.0));
    assert_eq!(s.position(), 6255);
    s.seek(0);
    prefill(&mut p);
    s.fill(&mut data);
    assert!(data.iter().any(|v| *v != 0.0));
}
#[test]
fn whole_song_loop_restarts_deterministically_without_tail() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    s.set_loop(true, u64::MAX);
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
    while s.position() + 1024 < end {
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
fn device_failure_silences_buffered_audio_and_stops_production() {
    let f = Fixture::new();
    let (s, mut p) = f.stream();
    prefill(&mut p);
    s.device_failed();
    let position = p.engine.position;
    p.pump().unwrap();
    assert_eq!(p.engine.position, position);
    let mut output = [1.0; 512];
    s.fill(&mut output);
    assert!(output.iter().all(|v| *v == 0.0));
    assert!(s.device_error.load(Ordering::SeqCst));
    assert!(s.quit.load(Ordering::SeqCst));
    assert!(!s.ended.load(Ordering::SeqCst));
}

#[test]
fn playhead_matches_rounded_fractional_row_boundaries() {
    let mut song = example_song();
    song.bpm = 137.0;
    song.lines_per_beat = 7;
    let row_frames = 48000.0 * 60.0 / (song.bpm * song.lines_per_beat as f64);
    for row in 1..32 {
        let frame = (row as f64 * row_frames).round() as u64;
        assert_eq!(location(&song, frame - 1), (0, row - 1));
        assert_eq!(location(&song, frame), (0, row));
    }
}

#[test]
fn seek_saturates_instead_of_wrapping_and_terminal_sessions_do_not_restart() {
    let f = Fixture::new();
    let (state, mut producer) = f.stream();
    let last = producer.engine.manifest.musical_frames - 1;
    for requested in [u32::MAX as u64 + 1, u64::MAX] {
        state.seek(requested);
        assert_eq!(state.position(), last);
        prefill(&mut producer);
        state.fill(&mut [0.0; 2]);
        assert_eq!(state.position(), last);
    }
    state.stop();
    state.seek(500);
    state.set_loop(true, last + 1);
    prefill(&mut producer);
    assert!(!state.is_running());
    assert!(!state.is_ready());
    assert_eq!(state.position(), 0);
}

#[test]
fn startup_loop_policy_applies_before_first_short_song_frame() {
    let mut f = Fixture::new();
    f.song.bpm = 400.0;
    f.song.lines_per_beat = 256;
    f.song.order.truncate(1);
    for pattern in f.song.patterns.values_mut() {
        pattern.rows = 1;
    }
    let state = Arc::new(Shared::default());
    state.set_loop(true, u64::MAX);
    let mut producer = Producer::new(&f.song, None, state.clone()).unwrap();
    let frames = producer.engine.manifest.musical_frames as usize;
    assert_eq!(frames, 28);
    // Startup must fill past tiny event/loop boundaries before yielding.
    producer.pump_until_buffered().unwrap();
    assert!(state.is_ready());
    let mut output = vec![0.0; frames * 4];
    state.fill(&mut output);
    assert!(output.iter().any(|sample| *sample != 0.0));
    assert_eq!(&output[..frames * 2], &output[frames * 2..]);
    assert!(state.is_running());
    assert!(!state.has_ended());
    assert_eq!(state.underruns.load(Ordering::Relaxed), 0);
    for _ in 0..100 {
        producer.pump_until_buffered().unwrap();
        state.fill(&mut [0.0; 1024]);
    }
    assert_eq!(state.underruns.load(Ordering::Relaxed), 0);
    assert!(state.is_running());
}

#[test]
fn callback_handles_empty_odd_and_starved_buffers_without_progress_replay() {
    let f = Fixture::new();
    let (state, mut producer) = f.stream();
    prefill(&mut producer);
    state.fill(&mut []);
    let mut odd = [1.0; 5];
    state.fill(&mut odd);
    assert_eq!(odd[4], 0.0);
    let mut drain = vec![0.0; 8192];
    state.fill(&mut drain);
    let stalled = state.position();
    let missing = state.underruns.load(Ordering::Relaxed);
    let mut empty = [1.0; 19];
    state.fill(&mut empty);
    assert_eq!(empty, [0.0; 19]);
    assert_eq!(state.position(), stalled);
    assert_eq!(state.underruns.load(Ordering::Relaxed), missing + 9);
}

#[test]
fn playhead_exact_half_frame_ties_match_forward_evaluator_boundaries() {
    let mut song = example_song();
    for (bpm, lines_per_beat) in [(400.0, 128), (400.0, 256), (137.0, 7)] {
        song.bpm = bpm;
        song.lines_per_beat = lines_per_beat;
        let per_row = 48000.0 * 60.0 / (bpm * lines_per_beat as f64);
        let mut absolute_row = 0;
        for (occurrence, order) in song.order.iter().enumerate() {
            for row in 0..song.patterns[&order.pattern].rows {
                let frame = (absolute_row as f64 * per_row).round() as u64;
                assert_eq!(location(&song, frame), (occurrence, row));
                if row > 0 {
                    assert_eq!(location(&song, frame - 1), (occurrence, row - 1));
                }
                absolute_row += 1;
            }
        }
    }
    assert_eq!(location(&song, u64::MAX), (2, 31));
    song.order.clear();
    assert_eq!(location(&song, 0), (0, 0));
}

#[test]
fn full_queue_stalls_production_and_seek_discards_it_without_skipping_new_audio() {
    let f = Fixture::new();
    let (state, mut producer) = f.stream();
    prefill(&mut producer);
    let full_position = producer.engine.position;
    assert!(full_position <= 2048);
    for _ in 0..1000 {
        producer.pump().unwrap();
    }
    assert_eq!(producer.engine.position, full_position);
    state.seek(0);
    prefill(&mut producer);
    let mut restarted = [0.0; 1024];
    state.fill(&mut restarted);
    let (fresh, mut fresh_producer) = f.stream();
    prefill(&mut fresh_producer);
    let mut expected = [0.0; 1024];
    fresh.fill(&mut expected);
    assert_eq!(restarted, expected);
    assert_eq!(state.position(), 511);
    assert_eq!(state.underruns.load(Ordering::Relaxed), 0);
}

#[test]
fn seek_requested_before_producer_loading_is_clamped_when_song_limit_is_known() {
    let f = Fixture::new();
    let state = Arc::new(Shared::default());
    state.seek(u64::MAX);
    let mut producer = Producer::new(&f.song, None, state.clone()).unwrap();
    let last = producer.engine.manifest.musical_frames - 1;
    assert_eq!(state.position(), last);
    prefill(&mut producer);
    state.fill(&mut [0.0; 2]);
    assert_eq!(state.position(), last);
}

#[test]
fn callback_allocates_nothing_during_play_underflow_seek_stop_and_failure() {
    let f = Fixture::new();
    let (state, mut producer) = f.stream();
    producer.pump_until_buffered().unwrap();
    let mut output = [0.0; 8193];
    without_callback_allocations(|| state.fill(&mut output));
    assert!(state.underruns.load(Ordering::Relaxed) > 0);
    state.seek(100);
    without_callback_allocations(|| state.fill(&mut output));
    producer.pump_until_buffered().unwrap();
    without_callback_allocations(|| state.fill(&mut output));
    state.stop();
    without_callback_allocations(|| state.fill(&mut output));
    let (failed, mut producer) = f.stream();
    producer.pump_until_buffered().unwrap();
    failed.device_failed();
    without_callback_allocations(|| failed.fill(&mut output));
}
