//! Device-free read-only meter publication tests. No audio device or host output.
use aqua_pattern_core::{
    editor::Editor,
    meter_windows::MeterReport,
    meters::TrackMeterAvailability,
    song::*,
    transport::{Producer, Shared},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

thread_local! {
    static CALLBACK_MEMORY: std::cell::Cell<Option<[usize; 3]>> = const {
        std::cell::Cell::new(None)
    };
}
struct CallbackAllocator;
#[global_allocator]
static ALLOCATOR: CallbackAllocator = CallbackAllocator;
fn memory_event(kind: usize) {
    let _ = CALLBACK_MEMORY.try_with(|counter| {
        if let Some(mut counts) = counter.get() {
            counts[kind] += 1;
            counter.set(Some(counts));
        }
    });
}
unsafe impl std::alloc::GlobalAlloc for CallbackAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        memory_event(0);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        memory_event(0);
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        memory_event(1);
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        memory_event(2);
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
fn fill_without_memory(shared: &Shared, output: &mut [f32]) {
    CALLBACK_MEMORY.with(|counter| counter.set(Some([0; 3])));
    shared.fill(output);
    let memory = CALLBACK_MEMORY.with(|counter| counter.replace(None).unwrap());
    assert_eq!(memory, [0; 3], "callback alloc/realloc/dealloc");
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    editor: Editor,
}
impl Fixture {
    fn new(shared_legacy: bool, tiny: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "transport-mix-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("authored-stereo.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..24_000 {
            // Deterministic asymmetric source makes channel swap and restart visible.
            wav.write_sample(0.2 + (frame % 31) as f32 * 0.001).unwrap();
            wav.write_sample(-0.3 + (frame % 47) as f32 * 0.001)
                .unwrap();
        }
        wav.finalize().unwrap();
        let mut editor = Editor::default();
        editor.song.order.truncate(1);
        if tiny {
            editor.song.bpm = 400.0;
            editor.song.lines_per_beat = 256;
        }
        for instrument in editor.song.instruments.values_mut() {
            instrument.sample = Some(path.clone());
        }
        for pattern in editor.song.patterns.values_mut() {
            pattern.rows = if tiny { 1 } else { 8 };
            pattern.tracks.clear();
        }
        for track in 1..=if shared_legacy { 2 } else { 1 } {
            editor
                .song
                .patterns
                .get_mut(&PatternId(1))
                .unwrap()
                .tracks
                .entry(TrackId(track))
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
        }
        Self { dir, editor }
    }
    fn start(&self, looping: bool) -> (Arc<Shared>, Producer) {
        let shared = Arc::new(Shared::default());
        shared
            .bind_live_mix(self.editor.document_owner(), &self.editor.song)
            .unwrap();
        if looping {
            shared.set_loop(true, u64::MAX);
        }
        let producer = Producer::new(&self.editor.song, None, shared.clone()).unwrap();
        (shared, producer)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn peak(pcm: &[f32]) -> [f32; 2] {
    let mut result = [0.0f32; 2];
    for frame in pcm.as_chunks::<2>().0 {
        for (peak, sample) in result.iter_mut().zip(frame) {
            *peak = peak.max(sample.abs());
        }
    }
    result
}
fn collect(
    fixture: &Fixture,
    partition: &[usize],
    frames: usize,
    bound: bool,
) -> (Vec<f32>, Vec<MeterReport>) {
    let shared = Arc::new(Shared::default());
    if bound {
        shared
            .bind_live_mix(fixture.editor.document_owner(), &fixture.editor.song)
            .unwrap();
    }
    let mut producer = Producer::new(&fixture.editor.song, None, shared.clone()).unwrap();
    let reader = bound.then(|| {
        shared
            .meter_reader(&fixture.editor.document_owner(), &fixture.editor.song)
            .unwrap()
    });
    let mut pcm = Vec::new();
    let mut reports = Vec::new();
    let mut i = 0;
    while pcm.len() / 2 < frames {
        producer.pump_until_buffered().unwrap();
        if let Some(reader) = &reader {
            let poll = shared.poll_meters(reader).unwrap();
            assert_eq!(poll.dropped_reports, 0);
            assert!(poll.disabled_reason.is_none());
            reports.extend(poll.reports);
        }
        let count = partition[i % partition.len()].min(frames - pcm.len() / 2);
        let mut block = vec![0.0; count * 2];
        fill_without_memory(&shared, &mut block);
        pcm.extend(block);
        i += 1;
    }
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    reports.retain(|report| report.emitted_end_exclusive as usize <= frames);
    shared.stop();
    let mut stopped = [1.0; 17];
    fill_without_memory(&shared, &mut stopped);
    assert_eq!(stopped, [0.0; 17]);
    (pcm, reports)
}
#[test]
fn meter_observation_never_changes_pcm_and_peaks_equal_actual_queued_output() {
    let fixture = Fixture::new(false, false);
    let frames = 19_200;
    let (plain, empty) = collect(&fixture, &[512], frames, false);
    assert!(empty.is_empty());
    let (observed, reports) = collect(&fixture, &[512], frames, true);
    assert_eq!(
        plain.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        observed.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(reports.len(), 8);
    for report in &reports {
        let pcm =
            &observed[report.emitted_start as usize * 2..report.emitted_end_exclusive as usize * 2];
        assert_eq!(
            report.master_peak.map(f32::to_bits),
            peak(pcm).map(f32::to_bits)
        );
        assert_eq!(report.track_peaks(), &[Some(report.master_peak)]);
        assert_eq!(report.observation_start_emitted, 0);
        assert!(!report.partial);
    }
}
#[test]
fn meter_windows_ignore_callback_partition_and_keep_raw_values_identical() {
    let fixture = Fixture::new(false, false);
    let (a_pcm, a) = collect(&fixture, &[512], 19_200, true);
    let (b_pcm, b) = collect(&fixture, &[1, 3, 127, 17, 511, 1024], 19_200, true);
    assert_eq!(
        a_pcm.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b_pcm.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
}
#[test]
fn meter_shared_legacy_tracks_are_wholly_unavailable_but_master_is_actual_pcm() {
    let fixture = Fixture::new(true, false);
    let (shared, mut producer) = fixture.start(false);
    let reader = shared
        .meter_reader(&fixture.editor.document_owner(), &fixture.editor.song)
        .unwrap();
    let mut pcm = Vec::new();
    let mut reports = Vec::new();
    for _ in 0..12 {
        producer.pump_until_buffered().unwrap();
        let poll = shared.poll_meters(&reader).unwrap();
        let layout = poll.layout.unwrap();
        assert_eq!(layout.tracks.len(), 2);
        assert!(layout.tracks.iter().all(|row| matches!(
            row.availability,
            TrackMeterAvailability::SharedLegacy { .. }
        )));
        reports.extend(poll.reports);
        let mut block = [0.0; 1024];
        fill_without_memory(&shared, &mut block);
        pcm.extend(block);
    }
    for report in reports
        .iter()
        .filter(|r| r.emitted_end_exclusive as usize <= pcm.len() / 2)
    {
        assert_eq!(report.track_peaks(), &[None, None]);
        assert_eq!(
            report.master_peak,
            peak(
                &pcm[report.emitted_start as usize * 2..report.emitted_end_exclusive as usize * 2]
            )
        );
    }
    assert!(reports.iter().any(|report| report.master_peak[0] > 0.0));
}
#[test]
fn meter_automatic_loop_is_monotonic_with_truthful_song_endpoint_and_wrap_tags() {
    let fixture = Fixture::new(false, true);
    let (shared, mut producer) = fixture.start(true);
    let reader = shared
        .meter_reader(&fixture.editor.document_owner(), &fixture.editor.song)
        .unwrap();
    let musical = producer.engine.manifest.musical_frames;
    let mut reports = Vec::new();
    let mut pcm = Vec::new();
    let mut generation = None;
    for _ in 0..20 {
        producer.pump_until_buffered().unwrap();
        let poll = shared.poll_meters(&reader).unwrap();
        assert_eq!(*generation.get_or_insert(poll.generation), poll.generation);
        reports.extend(poll.reports);
        let mut block = [0.0; 1024];
        fill_without_memory(&shared, &mut block);
        pcm.extend(block);
    }
    assert!(reports.len() >= 4);
    for report in reports
        .iter()
        .filter(|r| r.emitted_end_exclusive as usize <= pcm.len() / 2)
    {
        assert_eq!(report.first_song_frame, report.emitted_start % musical);
        assert_eq!(
            report.last_song_frame_end_exclusive,
            (report.emitted_end_exclusive - 1) % musical + 1
        );
        assert_eq!(
            report.wrap_count,
            (report.emitted_end_exclusive - 1) / musical - report.emitted_start / musical
        );
        assert_eq!(
            report.master_peak,
            peak(
                &pcm[report.emitted_start as usize * 2..report.emitted_end_exclusive as usize * 2]
            )
        );
    }
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
}
#[test]
fn meter_natural_end_omits_partial_and_terminal_poll_refuses_stale_reports() {
    let fixture = Fixture::new(false, true);
    let (shared, mut producer) = fixture.start(false);
    let reader = shared
        .meter_reader(&fixture.editor.document_owner(), &fixture.editor.song)
        .unwrap();
    let total = producer.engine.manifest.musical_frames + producer.engine.manifest.tail_frames;
    assert_ne!(total % 2400, 0);
    let mut reports = Vec::new();
    while shared.is_running() {
        producer.pump_until_buffered().unwrap();
        reports.extend(shared.poll_meters(&reader).unwrap().reports);
        fill_without_memory(&shared, &mut [0.0; 1024]);
    }
    assert!(shared.has_ended());
    assert!(shared.poll_meters(&reader).is_err());
    assert_eq!(reports.len() as u64, total / 2400);
    assert!(reports.iter().all(|r| r.frames() == 2400 && !r.partial));
    assert_eq!(
        reports.last().unwrap().emitted_end_exclusive,
        total / 2400 * 2400
    );
    producer.pump().unwrap();
    assert!(producer.engine.meter_layout().is_none());
}
#[test]
fn meter_over64_neutral_contributors_keep_master_without_imposing_playback_limit() {
    let fixture = Fixture::new(false, false);
    // Existing legacy phonic voices begin their volume ramp at gain 1, so note
    // velocity alone cannot quiet 65 simultaneous starts. Attenuate only this
    // authored source, without changing accepted sampler/Engine behavior.
    let path = fixture.editor.song.instruments[&InstrumentId(1)]
        .sample
        .as_ref()
        .unwrap();
    let mut wav = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..24_000 {
        wav.write_sample((0.2 + (frame % 31) as f32 * 0.001) * 0.01)
            .unwrap();
        wav.write_sample((-0.3 + (frame % 47) as f32 * 0.001) * 0.01)
            .unwrap();
    }
    wav.finalize().unwrap();
    let mut song = fixture.editor.song.clone();
    for id in 1..=65 {
        song.tracks.insert(
            TrackId(id),
            Track {
                name: format!("T{id}"),
                note_columns: 1,
            },
        );
        if !song.track_order.contains(&TrackId(id)) {
            song.track_order.push(TrackId(id));
        }
        song.patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(id))
            .or_default()
            .entry(0)
            .or_default()
            .insert(
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(0.01),
                    ..Default::default()
                },
            );
    }
    let shared = Arc::new(Shared::default());
    shared
        .bind_live_mix(fixture.editor.document_owner(), &song)
        .unwrap();
    let mut producer = Producer::new(&song, None, shared.clone()).unwrap();
    let reader = shared
        .meter_reader(&fixture.editor.document_owner(), &song)
        .unwrap();
    let mut reports = Vec::new();
    for _ in 0..8 {
        producer.pump_until_buffered().unwrap();
        let poll = shared.poll_meters(&reader).unwrap();
        let layout = poll.layout.unwrap();
        assert!(layout.tracks.is_empty());
        assert!(layout.tracks_unavailable.is_some());
        reports.extend(poll.reports);
        fill_without_memory(&shared, &mut [0.0; 1024]);
    }
    assert!(!reports.is_empty());
    assert!(reports
        .iter()
        .all(|r| r.track_peaks().is_empty() && r.master_peak[0] > 0.0));
    assert!(shared.is_running());
}
