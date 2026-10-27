//! Device-free live-mix handoff tests. No audio device or host output is opened.
use aqua_pattern_core::{
    editor::Editor,
    song::*,
    song_render::Engine,
    track_mix::TrackMix,
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
    fn target(&self, gain: f32, balance: f32) -> Song {
        let mut song = self.editor.song.clone();
        for track in [TrackId(1), TrackId(2)] {
            song.track_mix.insert(
                track,
                TrackMix {
                    gain,
                    balance,
                    ..Default::default()
                },
            );
        }
        song
    }
    fn submit(&self, shared: &Shared, song: &Song) -> u64 {
        shared
            .submit_live_mix(&self.editor.document_owner(), song)
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn apply(engine: &mut Engine, target: &Song) {
    let snapshot = engine
        .mix_layout()
        .unwrap()
        .snapshot(&target.track_mix)
        .unwrap();
    engine.apply_mix_snapshot(&snapshot).unwrap();
}
fn render(engine: &mut Engine, frames: usize, looping: bool) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    let mut done = 0;
    while done < frames {
        let end = engine.manifest.musical_frames
            + if looping {
                0
            } else {
                engine.manifest.tail_frames
            };
        if engine.position == end && looping {
            engine.cold_seek(0).unwrap();
        }
        let count = engine.block(&mut output[done * 2..], end).unwrap();
        assert_ne!(count, 0);
        done += count;
    }
    output
}

#[test]
fn latest_full_snapshot_coalesces_without_flushing_or_seeking() {
    let fixture = Fixture::new(false, false);
    let (shared, mut producer) = fixture.start(false);
    for gain in [0.75, 0.5, 0.25] {
        fixture.submit(&shared, &fixture.target(gain, 0.5));
    }
    assert_eq!(shared.live_mix_status().accepted_revision, 3);
    assert_eq!(shared.live_mix_status().applied_revision, 0);
    let mut reference = Engine::new(&fixture.editor.song, None).unwrap();
    apply(&mut reference, &fixture.target(0.25, 0.5));
    let expected = render(&mut reference, 2048, false);
    producer.pump_until_buffered().unwrap();
    assert_eq!(shared.live_mix_status().applied_revision, 3);
    assert_eq!(producer.engine.position, 2048);
    assert_eq!(shared.position(), 0); // Enqueued is not consumed.
    let mut output = vec![0.0; 4096];
    fill_without_memory(&shared, &mut output);
    assert_eq!(output, expected);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
}

#[test]
fn already_queued_pcm_is_retained_and_only_next_rendered_block_ramps() {
    let fixture = Fixture::new(false, false);
    let (shared, mut producer) = fixture.start(false);
    let mut reference = Engine::new(&fixture.editor.song, None).unwrap();
    let old = render(&mut reference, 2048, false);
    producer.pump_until_buffered().unwrap();
    let target = fixture.target(0.25, -0.75);
    fixture.submit(&shared, &target);
    producer.pump().unwrap();
    assert_eq!(producer.engine.position, 2048);
    assert_eq!(shared.live_mix_status().applied_revision, 0);
    assert!(shared.is_ready());
    let mut first = vec![0.0; 1024];
    fill_without_memory(&shared, &mut first);
    assert_eq!(first, old[..1024]);
    producer.pump().unwrap();
    assert_eq!(producer.engine.position, 2560);
    assert_eq!(shared.live_mix_status().applied_revision, 1);
    apply(&mut reference, &target);
    let new = render(&mut reference, 512, false);
    let mut rest = vec![0.0; 4096];
    fill_without_memory(&shared, &mut rest);
    assert_eq!(rest[..3072], old[1024..]);
    assert_eq!(rest[3072..], new);
    assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
}

#[test]
fn invalid_legacy_group_and_invalid_values_do_not_replace_valid_pending_target() {
    let fixture = Fixture::new(true, false);
    let (shared, mut producer) = fixture.start(false);
    let valid = fixture.target(0.5, 0.25);
    assert_eq!(fixture.submit(&shared, &valid), 1);
    for failure in 0..5 {
        let mut invalid = valid.clone();
        let mix = invalid.track_mix.get_mut(&TrackId(1)).unwrap();
        match failure {
            0 => mix.mute = true,
            1 => mix.solo = true,
            2 => mix.gain = f32::NAN,
            3 => mix.balance = 1.01,
            _ => {
                invalid.track_mix.insert(TrackId(999), TrackMix::default());
            }
        }
        assert!(shared
            .submit_live_mix(&fixture.editor.document_owner(), &invalid)
            .is_err());
        assert_eq!(shared.live_mix_status().accepted_revision, 1);
    }
    producer.pump_until_buffered().unwrap();
    let mut reference = Engine::new(&fixture.editor.song, None).unwrap();
    apply(&mut reference, &valid);
    let expected = render(&mut reference, 2048, false);
    let mut output = vec![0.0; 4096];
    fill_without_memory(&shared, &mut output);
    assert_eq!(output, expected);
    assert_eq!(shared.live_mix_status().applied_revision, 1);
}

#[test]
fn loading_owner_content_and_identical_document_replacement_refuse_separately() {
    let mut fixture = Fixture::new(false, false);
    let shared = Arc::new(Shared::default());
    let original_owner = fixture.editor.document_owner();
    shared
        .bind_live_mix(original_owner.clone(), &fixture.editor.song)
        .unwrap();
    let target = fixture.target(0.5, 0.0);
    assert!(shared
        .submit_live_mix(&original_owner, &target)
        .unwrap_err()
        .contains("loading"));
    let _producer = Producer::new(&fixture.editor.song, None, shared.clone()).unwrap();
    let foreign = Editor::default().document_owner();
    assert!(shared
        .submit_live_mix(&foreign, &target)
        .unwrap_err()
        .contains("another document"));
    let mut changed = target.clone();
    changed.title.push('!');
    assert!(shared
        .submit_live_mix(&original_owner, &changed)
        .unwrap_err()
        .contains("content changed"));
    assert_eq!(shared.live_mix_status().accepted_revision, 0);
    fixture
        .editor
        .replace_document(fixture.editor.song.clone())
        .unwrap();
    assert!(shared
        .submit_live_mix(&fixture.editor.document_owner(), &target)
        .unwrap_err()
        .contains("another document"));
    shared.revoke_live_mix();
    assert!(shared
        .submit_live_mix(&original_owner, &target)
        .unwrap_err()
        .contains("revoked"));
    assert!(shared
        .bind_live_mix(fixture.editor.document_owner(), &target)
        .is_err());
    assert!(!shared.live_mix_status().available);
}

#[test]
fn unbound_audition_style_session_cannot_receive_song_mix() {
    let fixture = Fixture::new(false, false);
    let shared = Arc::new(Shared::default());
    let mut producer = Producer::new(&fixture.editor.song, None, shared.clone()).unwrap();
    assert!(shared
        .submit_live_mix(&fixture.editor.document_owner(), &fixture.target(0.5, 0.0))
        .unwrap_err()
        .contains("unbound"));
    producer.pump_until_buffered().unwrap();
    assert!(!shared.live_mix_status().available);
    let mut output = [0.0; 1024];
    fill_without_memory(&shared, &mut output);
    assert!(output.iter().any(|sample| *sample != 0.0));
}

#[test]
fn short_loop_and_explicit_seek_preserve_in_progress_ramp_by_emitted_frames() {
    let fixture = Fixture::new(false, true);
    let (shared, mut producer) = fixture.start(true);
    let target = fixture.target(0.0, 0.0);
    fixture.submit(&shared, &target);
    let mut reference = Engine::new(&fixture.editor.song, None).unwrap();
    assert_eq!(reference.manifest.musical_frames, 28);
    apply(&mut reference, &target);
    // One loop is shorter than the 128 emitted-frame ramp.
    producer.pump().unwrap();
    let _discarded = render(&mut reference, 28, true);
    assert_eq!(producer.engine.position, 28);
    assert_eq!(shared.live_mix_status().applied_revision, 1);
    shared.seek(0);
    reference.cold_seek(0).unwrap();
    producer.pump_until_buffered().unwrap();
    let expected = render(&mut reference, 1024, true);
    let mut output = vec![0.0; 2048];
    fill_without_memory(&shared, &mut output);
    assert_eq!(output, expected);
    assert!(output[..64].iter().any(|sample| *sample != 0.0));
    assert!(output[200..].iter().all(|sample| *sample == 0.0));
    assert!(shared.is_running());
}

#[test]
fn concurrent_submitters_preserve_monotonic_latest_snapshot_and_bounded_mailbox() {
    let fixture = Fixture::new(false, false);
    let (shared, mut producer) = fixture.start(false);
    let mut threads = Vec::new();
    for index in 1..=8 {
        let shared = shared.clone();
        let owner = fixture.editor.document_owner();
        let target = fixture.target(index as f32 / 16.0, 0.0);
        threads.push(std::thread::spawn(move || {
            let revision = shared.submit_live_mix(&owner, &target).unwrap();
            (revision, target)
        }));
    }
    let (_, latest) = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .max_by_key(|(revision, _)| *revision)
        .unwrap();
    assert_eq!(shared.live_mix_status().accepted_revision, 8);
    assert_eq!(shared.live_mix_status().applied_revision, 0);
    producer.pump_until_buffered().unwrap();
    assert_eq!(shared.live_mix_status().applied_revision, 8);
    let mut reference = Engine::new(&fixture.editor.song, None).unwrap();
    apply(&mut reference, &latest);
    let expected = render(&mut reference, 2048, false);
    let mut output = vec![0.0; 4096];
    fill_without_memory(&shared, &mut output);
    assert_eq!(output, expected);
}

#[test]
fn stop_and_device_error_discard_pending_and_callback_does_no_memory_work() {
    for device_error in [false, true] {
        let fixture = Fixture::new(false, false);
        let (shared, mut producer) = fixture.start(false);
        producer.pump_until_buffered().unwrap();
        fixture.submit(&shared, &fixture.target(0.25, 1.0));
        if device_error {
            shared.device_failed();
        } else {
            shared.stop();
        }
        producer.pump().unwrap();
        let status = shared.live_mix_status();
        assert_eq!(status.accepted_revision, 1);
        assert_eq!(status.applied_revision, 0);
        assert!(!status.available);
        let mut output = [1.0; 1024];
        fill_without_memory(&shared, &mut output);
        assert_eq!(output, [0.0; 1024]);
        assert!(shared
            .submit_live_mix(&fixture.editor.document_owner(), &fixture.target(0.5, 0.0))
            .is_err());
    }
}

#[test]
fn exact_signed_zero_path_spelling_and_other_session_owner_are_not_authority() {
    let mut fixture = Fixture::new(false, false);
    fixture
        .editor
        .song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .pan = Some(0.0);
    let (shared, _producer) = fixture.start(false);
    let target = fixture.target(0.5, 0.0);
    let mut signed_zero = target.clone();
    signed_zero
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .pan = Some(-0.0);
    // Ordinary float equality misses this exact content change.
    assert_eq!(target, signed_zero);
    assert!(shared
        .submit_live_mix(&fixture.editor.document_owner(), &signed_zero)
        .is_err());
    let mut spelling = target.clone();
    spelling
        .instruments
        .get_mut(&InstrumentId(1))
        .unwrap()
        .sample = Some(fixture.dir.join("./authored-stereo.wav"));
    assert_eq!(target, spelling); // Path equality also normalizes this spelling.
    assert!(shared
        .submit_live_mix(&fixture.editor.document_owner(), &spelling)
        .is_err());
    let mut other = Editor::default();
    other.replace_document(fixture.editor.song.clone()).unwrap();
    let other_shared = Arc::new(Shared::default());
    other_shared
        .bind_live_mix(other.document_owner(), &other.song)
        .unwrap();
    let _other_producer = Producer::new(&other.song, None, other_shared.clone()).unwrap();
    assert!(other_shared
        .submit_live_mix(&fixture.editor.document_owner(), &target)
        .unwrap_err()
        .contains("another document"));
    assert_eq!(shared.live_mix_status().accepted_revision, 0);
    assert_eq!(other_shared.live_mix_status().accepted_revision, 0);
    assert_eq!(
        other_shared
            .submit_live_mix(&other.document_owner(), &target)
            .unwrap(),
        1
    );
    assert_eq!(shared.live_mix_status().accepted_revision, 0);
}

#[test]
fn natural_end_after_live_mix_keeps_callback_memory_free_and_cannot_restart() {
    let fixture = Fixture::new(false, true);
    let (shared, mut producer) = fixture.start(false);
    fixture.submit(&shared, &fixture.target(0.5, 0.0));
    for _ in 0..100 {
        producer.pump_until_buffered().unwrap();
        fill_without_memory(&shared, &mut [0.0; 1024]);
        if shared.has_ended() {
            break;
        }
    }
    assert!(shared.has_ended());
    assert_eq!(shared.live_mix_status().accepted_revision, 1);
    assert_eq!(shared.live_mix_status().applied_revision, 1);
    assert!(!shared.live_mix_status().available);
    assert!(shared
        .submit_live_mix(&fixture.editor.document_owner(), &fixture.target(0.25, 0.0))
        .unwrap_err()
        .contains("ended"));
    shared.seek(0);
    shared.set_loop(true, producer.engine.manifest.musical_frames);
    producer.pump().unwrap();
    let mut silence = [1.0; 64];
    fill_without_memory(&shared, &mut silence);
    assert_eq!(silence, [0.0; 64]);
    assert!(shared.has_ended());
}
