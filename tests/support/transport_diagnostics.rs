//! Shared device-free example/test harness, NOT a scheduler or production backend.
use aqua_pattern_core::{
    sample_asset,
    song::*,
    song_render::{Command, Engine},
    transport::{Producer, Shared},
    RATE, TAIL,
};
use serde::Serialize;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell as Counter,
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
const MAX_NORMAL_FRAMES: u64 = 1_000_000;
const MAX_PUMPS: usize = 256;
const MAX_FILLS: usize = 20_000;
const CONCURRENT_COMMANDS: usize = 16;
const WATCHDOG: Duration = Duration::from_secs(20);

thread_local! { static MEMORY: Counter<Option<[u64; 3]>> = const { Counter::new(None) }; }
struct CountingAllocator;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
fn count(kind: usize) {
    let _ = MEMORY.try_with(|slot| {
        if let Some(mut values) = slot.get() {
            values[kind] += 1;
            slot.set(Some(values));
        }
    });
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(0);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(0);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(1);
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(2);
        unsafe { System.dealloc(ptr, layout) }
    }
}
fn require(value: bool, message: &str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(message.into())
    }
}

#[derive(Default)]
struct Measurements {
    times: Vec<u64>,
    memory: [u64; 3],
    samples: u64,
}
#[derive(Serialize)]
pub struct Timings {
    samples: usize,
    min_ns: u64,
    median_ns: u64,
    p95_ns: u64,
    max_ns: u64,
}
impl Measurements {
    fn new() -> Self {
        MEMORY.with(|slot| slot.set(None)); // Prewarm TLS outside the measured call.
        Self {
            times: Vec::with_capacity(MAX_FILLS),
            ..Self::default()
        }
    }
    fn fill(&mut self, shared: &Shared, output: &mut [f32]) -> Result<()> {
        require(
            self.times.len() < MAX_FILLS,
            "callback measurement budget exhausted (incomplete)",
        )?;
        let begin = Instant::now();
        MEMORY.with(|slot| slot.set(Some([0; 3])));
        shared.fill(output);
        let memory = MEMORY.with(|slot| slot.replace(None).unwrap());
        let elapsed = begin.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        // Clocks, vector growth, inspection and reporting are outside allocator counting.
        self.times.push(elapsed);
        for (total, value) in self.memory.iter_mut().zip(memory) {
            *total += value;
        }
        self.samples += output.len() as u64;
        require(
            memory == [0; 3],
            "Shared::fill allocated, reallocated or deallocated",
        )?;
        require(
            output.iter().all(|x| x.is_finite() && x.abs() <= 1.0),
            "callback returned invalid PCM",
        )
    }
    fn timing(&mut self) -> Timings {
        summarize(&mut self.times)
    }
}
fn summarize(values: &mut [u64]) -> Timings {
    values.sort_unstable();
    if values.is_empty() {
        return Timings {
            samples: 0,
            min_ns: 0,
            median_ns: 0,
            p95_ns: 0,
            max_ns: 0,
        };
    }
    let n = values.len();
    Timings {
        samples: n,
        min_ns: values[0],
        median_ns: values[n / 2],
        p95_ns: values[((n - 1) * 95) / 100],
        max_ns: values[n - 1],
    }
}
#[derive(Serialize)]
pub struct ParityCase {
    fixture: String,
    callback_frames: Vec<usize>,
    starts: usize,
    musical_frames: u64,
    printed_frames: u64,
    callbacks: u64,
    callback_sample_values: u64,
    end_padding_frames: u64,
    above_threshold_frames: u64,
    pub underrun_frames: u64,
    pub exact_pcm: bool,
    callback_timing: Timings,
    worker_refill_timing: Timings,
}
#[derive(Serialize)]
pub struct Concurrent {
    pub completion: String,
    pub commands_completed: usize,
    post_command_ready_observations: usize,
    pub pump_calls: usize,
    emitted_frame_upper_bound: usize,
    callbacks: usize,
    callback_sample_values: u64,
    underrun_frames: u64,
    callback_timing: Timings,
    worker_refill_timing: Timings,
    artificial_consumer_pause_us: u64,
}
#[derive(Serialize)]
pub struct Report {
    schema: u32,
    complete: bool,
    scope: String,
    pub physical_device_opened: bool,
    rustc_on_runtime_path: String,
    build_debug_assertions: bool,
    compiled_os: String,
    compiled_arch: String,
    cpu_runtime_procfs: String,
    output_sample_rate: u32,
    queue_capacity_frames: usize,
    readiness_frames: usize,
    production_preferred_device_buffer_frames: usize,
    limits: BTreeMap<&'static str, u64>,
    pub normal_matrix_frames: u64,
    pub callback_memory: [u64; 3],
    callback_memory_labels: [&'static str; 3],
    pub parity: Vec<ParityCase>,
    guards: Vec<String>,
    guard_measurements: BTreeMap<String, u64>,
    pub concurrent: Concurrent,
    authored_source_sha256: Vec<String>,
    warnings: Vec<String>,
}

fn note(inst: u64, pitch: u8, velocity: f32) -> Cell {
    Cell {
        note: Note::Note(pitch),
        instrument: Some(InstrumentId(inst)),
        velocity: Some(velocity),
        pan: Some(0.0),
        ..Cell::default()
    }
}
fn put(song: &mut Song, row: u32, column: u16, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(row)
        .or_default()
        .insert(column, cell);
}
fn base(root: &Path) -> Result<Song> {
    let mut song = example_song();
    song.title = "Authored device-free transport diagnostics".into();
    song.bpm = 137.0;
    song.lines_per_beat = 7;
    song.track_order = vec![TrackId(1)];
    song.tracks = BTreeMap::from([(
        TrackId(1),
        Track {
            name: "Probe".into(),
            note_columns: 64,
        },
    )]);
    song.instruments.clear();
    song.patterns = BTreeMap::from([(
        PatternId(1),
        Pattern {
            name: "Four rows".into(),
            rows: 4,
            tracks: BTreeMap::new(),
            track_aliases: BTreeMap::new(),
            generation: None,
        },
    )]);
    song.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: BTreeSet::new(),
        section: None,
    }];
    for (id, rate, channels) in [(1, 48000, 1), (2, 44100, 2), (3, 48000, 1)] {
        let path = root.join(format!("authored-{id}.wav"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let mut writer = hound::WavWriter::new(
            file,
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .map_err(|e| e.to_string())?;
        for frame in 0..4096 {
            for channel in 0..channels {
                // Quiet sources remain safe even during existing gain smoothing.
                // Source3 is reserved for the deliberate clipping/error fixture.
                let level = if id == 3 { 0.2 } else { 0.008 };
                let value = (level + (frame % 61) as f32 * 0.00002)
                    * if channel == 0 { 1.0 } else { -0.375 };
                writer.write_sample(value).map_err(|e| e.to_string())?;
            }
        }
        writer.finalize().map_err(|e| e.to_string())?;
        song.samples
            .insert(SampleId(id), sample_asset::import(&path)?.asset);
        song.regions.insert(
            RegionId(id),
            SampleRegion {
                name: format!("Native region {id}"),
                sample: SampleId(id),
                start_frame: 0,
                end_frame: 4096,
                root_note: 60,
                amp_envelope: Some(AmpEnvelope {
                    attack_ms: 2,
                    decay_ms: 3,
                    sustain_percent: 75,
                    release_ms: 30,
                }),
                forward_loop: (id == 1).then_some(ForwardLoop {
                    start_frame: 32,
                    end_frame: 2048,
                }),
            },
        );
        song.instruments.insert(
            InstrumentId(id),
            Instrument {
                name: format!("Region {id}"),
                kind: InstrumentKind::Pitched,
                sample: None,
                region: Some(RegionId(id)),
            },
        );
    }
    Ok(song)
}
fn dense(mut song: Song) -> Result<Song> {
    for col in 0..64 {
        put(&mut song, 0, col, note(1, 60 + (col % 2) as u8 * 12, 0.01));
        put(
            &mut song,
            2,
            col,
            Cell {
                note: Note::Off,
                ..Cell::default()
            },
        );
    }
    song.validate()?;
    Ok(song)
}
fn mixed(mut song: Song) -> Result<Song> {
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 8;
    song.drum_kits.insert(
        DrumKitId(1),
        DrumKitInstrument {
            name: "RR".into(),
            pads: BTreeMap::from([(
                PadId(1),
                DrumPad {
                    slot: 0,
                    name: "Authored pad".into(),
                    note: 36,
                    region: None,
                    velocity_zones: vec![VelocityZone {
                        min_velocity: 0,
                        max_velocity: 127,
                        alternatives: vec![
                            PadLayer {
                                id: PadLayerId(9),
                                region: RegionId(1),
                            },
                            PadLayer {
                                id: PadLayerId(2),
                                region: RegionId(2),
                            },
                        ],
                    }],
                    root_note: 60,
                    tune_semitones: 0,
                    gain: 0.25,
                    pan: 0.0,
                    mode: PadMode::Gated,
                    new_note: PadNewNote::ReleasePrevious,
                    choke_group: Some(1),
                },
            )]),
        },
    );
    song.instruments.insert(
        InstrumentId(3),
        Instrument {
            name: "Kit".into(),
            kind: InstrumentKind::DrumKit { kit: DrumKitId(1) },
            sample: None,
            region: None,
        },
    );
    song.multisamples.insert(
        MultisampleId(1),
        MultisampleInstrument {
            name: "Keys".into(),
            zones: [
                (KeyZoneId(17), 0, 63, 1, 60),
                (KeyZoneId(3), 64, 127, 2, 72),
            ]
            .into_iter()
            .map(|(id, lo, hi, region, root)| {
                (
                    id,
                    KeyZone {
                        name: format!("Key{region}"),
                        note_min: lo,
                        note_max: hi,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(region),
                        root_note: root,
                    },
                )
            })
            .collect(),
        },
    );
    song.instruments.insert(
        InstrumentId(4),
        Instrument {
            name: "Keys".into(),
            kind: InstrumentKind::Multisample {
                map: MultisampleId(1),
            },
            sample: None,
            region: None,
        },
    );
    song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "Finite phrase".into(),
            instrument: InstrumentId(1),
            rows: 2,
            lines_per_beat: 14,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([
                (0, BTreeMap::from([(0, note(1, 60, 0.5))])),
                (1, BTreeMap::from([(0, note(1, 72, 0.5))])),
            ]),
        },
    );
    put(&mut song, 0, 0, note(1, 60, 0.04));
    put(
        &mut song,
        1,
        0,
        Cell {
            velocity: Some(0.02),
            effects: StepEffects {
                glide: Some(Glide {
                    target: 72,
                    semitones_per_second: 12.0,
                }),
                ..StepEffects::default()
            },
            ..Cell::default()
        },
    );
    for row in 0..4 {
        put(&mut song, row, 1, note(3, 36, 0.2));
        put(
            &mut song,
            row,
            2,
            note(4, if row % 2 == 0 { 60 } else { 72 }, 0.04),
        );
    }
    let mut trigger = note(1, 60, 0.04);
    trigger.phrase = Some(PhraseTrigger {
        phrase: PhraseId(1),
        cycles: 2,
    });
    put(&mut song, 0, 3, trigger);
    let mut fx = note(2, 60, 0.04);
    fx.delay = Some(0.125);
    fx.effects = StepEffects {
        retrigger: Some(Retrigger {
            repeats: 2,
            spacing: 0.25,
        }),
        offset: Some(32),
        direction: Some(Direction::Reverse),
        ..StepEffects::default()
    };
    put(&mut song, 1, 4, fx);
    for col in [0, 3, 4] {
        put(
            &mut song,
            3,
            col,
            Cell {
                note: Note::Off,
                ..Cell::default()
            },
        );
    }
    song.tracks.insert(
        TrackId(2),
        Track {
            name: "Alias".into(),
            note_columns: 8,
        },
    );
    song.track_order.push(TrackId(2));
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .track_aliases
        .insert(
            TrackId(2),
            TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            },
        );
    song.order.push(Occurrence {
        pattern: PatternId(1),
        muted_tracks: BTreeSet::from([TrackId(2)]),
        section: None,
    });
    song.validate()?;
    Ok(song)
}
fn engine(song: &Song) -> Result<Engine> {
    let engine = Engine::new(song, None)?;
    let starts = engine
        .manifest
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .count();
    require(
        starts <= 256 && engine.manifest.musical_frames <= 96_000,
        "authored fixture exceeded start/frame budget",
    )?;
    // Independent integer arithmetic for ordinary row-start events, not a second evaluator.
    for event in &engine.manifest.events {
        if matches!(event.command, Command::On { .. })
            && event.phrase.is_none()
            && event.column <= 2
        {
            if let Some((occ, row)) = event.position {
                let denominator = 137u64 * 7;
                let wanted =
                    ((occ as u64 * 4 + row as u64) * 2_880_000 + denominator / 2) / denominator;
                require(
                    event.frame == wanted,
                    "independent authored row-frame oracle mismatch",
                )?;
            }
        }
    }
    Ok(engine)
}
fn rendered(engine: &mut Engine, end: u64) -> Result<Vec<f32>> {
    let mut result = Vec::new();
    let mut scratch = [0.0; 1024];
    while engine.position < end {
        let count = engine.block(&mut scratch, end)?;
        result.extend_from_slice(&scratch[..count * 2]);
    }
    Ok(result)
}
fn parity(
    song: &Song,
    name: &str,
    sizes: &[usize],
    expected: &[f32],
    memory: &mut [u64; 3],
) -> Result<ParityCase> {
    let state = Arc::new(Shared::default());
    let mut producer = Producer::new(song, None, state.clone())?;
    let end = producer.engine.manifest.musical_frames + TAIL;
    let starts = producer
        .engine
        .manifest
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .count();
    let mut output = vec![0.0; sizes.iter().max().unwrap() * 2];
    let mut got = Vec::with_capacity(expected.len() + 2048);
    let mut stats = Measurements::new();
    let mut pump_times = Vec::new();
    let mut calls = 0;
    while !state.has_ended() {
        require(calls < MAX_FILLS, "parity callback budget exhausted")?;
        let start = Instant::now();
        producer.pump_until_buffered()?;
        pump_times.push(start.elapsed().as_nanos() as u64);
        let n = sizes[calls % sizes.len()] * 2;
        stats.fill(&state, &mut output[..n])?;
        got.extend_from_slice(&output[..n]);
        calls += 1;
    }
    require(
        got.len() >= expected.len(),
        "natural end truncated expected PCM",
    )?;
    require(
        got[..expected.len()]
            .iter()
            .zip(expected)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "streamed PCM differs from offline Engine bits",
    )?;
    require(
        got[expected.len()..].iter().all(|x| *x == 0.0),
        "natural end padding is not silence",
    )?;
    require(
        state.underruns.load(Ordering::Relaxed) == 0,
        "prefilled parity run unexpectedly underflowed",
    )?;
    require(
        state.position() == end - 1,
        "natural end playhead is not last consumed frame",
    )?;
    let above_threshold = expected
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|frame| frame.iter().any(|sample| sample.abs() > 0.000001))
        .count() as u64;
    require(
        above_threshold > 0,
        "authored PCM fixture unexpectedly contains no above-threshold frame",
    )?;
    require(
        state.audible_frames.load(Ordering::Relaxed) == above_threshold,
        "above1e-6 frame counter differs from exact reference PCM threshold",
    )?;
    require(
        state.callbacks.load(Ordering::Relaxed) == calls as u64,
        "callback counter differs from actual fill calls",
    )?;
    for (a, b) in memory.iter_mut().zip(stats.memory) {
        *a += b;
    }
    Ok(ParityCase {
        fixture: name.into(),
        callback_frames: sizes.to_vec(),
        starts,
        musical_frames: producer.engine.manifest.musical_frames,
        printed_frames: end,
        callbacks: calls as u64,
        callback_sample_values: stats.samples,
        end_padding_frames: stats.samples / 2 - end,
        above_threshold_frames: above_threshold,
        underrun_frames: state.underruns.load(Ordering::Relaxed),
        exact_pcm: true,
        callback_timing: stats.timing(),
        worker_refill_timing: summarize(&mut pump_times),
    })
}

fn guards(song: &Song, memory: &mut [u64; 3]) -> Result<(Vec<String>, BTreeMap<String, u64>)> {
    let state = Arc::new(Shared::default());
    let mut stats = Measurements::new();
    let mut odd = [1.0; 19];
    stats.fill(&state, &mut odd)?;
    require(
        odd == [0.0; 19] && state.underruns.load(Ordering::Relaxed) == 0,
        "not-ready silence counted as underflow",
    )?;
    stats.fill(&state, &mut [])?;
    let mut producer = Producer::new(song, None, state.clone())?;
    producer.pump_until_buffered()?;
    let produced = producer.engine.position;
    require(produced <= 2048, "initial queue exceeded2048")?;
    let mut drain = vec![1.0; 8193];
    stats.fill(&state, &mut drain)?;
    require(drain[8192] == 0.0, "odd trailing sample is not zero")?;
    require(
        state.underruns.load(Ordering::Relaxed) == 4096 - produced,
        "READY starvation did not count exact missing frames",
    )?;
    let position = state.position();
    let missing = state.underruns.load(Ordering::Relaxed);
    stats.fill(&state, &mut odd)?;
    require(
        state.position() == position
            && state.underruns.load(Ordering::Relaxed) == missing + 9
            && odd == [0.0; 19],
        "starved playhead/silence/counter mismatch",
    )?;
    producer.pump_until_buffered()?;
    let mut resumed = [0.0; 256];
    stats.fill(&state, &mut resumed)?;
    let mut reference = Engine::new(song, None)?;
    let end = reference.manifest.musical_frames + TAIL;
    let all = rendered(&mut reference, end)?;
    require(
        resumed
            .iter()
            .zip(&all[produced as usize * 2..])
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "resume skipped/replayed PCM after starvation",
    )?;
    state.seek(1000);
    stats.fill(&state, &mut odd)?;
    require(odd == [0.0; 19], "seek generation leaked queued audio")?;
    producer.pump_until_buffered()?;
    reference.cold_seek(1000)?;
    let cold = rendered(&mut reference, end)?;
    stats.fill(&state, &mut resumed)?;
    require(
        resumed
            .iter()
            .zip(&cold)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "cold seek differs from same compiled-manifest reset",
    )?;
    state.set_loop(true, producer.engine.manifest.musical_frames);
    let origin = state.position();
    producer.pump_until_buffered()?;
    reference.cold_seek(origin)?;
    let musical = reference.manifest.musical_frames;
    let mut expected = rendered(&mut reference, musical)?;
    reference.cold_seek(0)?;
    expected.extend(rendered(&mut reference, musical)?);
    let mut got = Vec::new();
    let mut block = [0.0; 256];
    while got.len() < expected.len() {
        producer.pump_until_buffered()?;
        stats.fill(&state, &mut block)?;
        got.extend(block);
    }
    require(
        got[..expected.len()]
            .iter()
            .zip(&expected)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "loop did not reuse compiled choices with cold-reset voices",
    )?;
    state.set_loop(false, musical);
    producer.pump_until_buffered()?;
    state.stop();
    let before = state.underruns.load(Ordering::Relaxed);
    stats.fill(&state, &mut odd)?;
    require(
        odd == [0.0; 19] && state.underruns.load(Ordering::Relaxed) == before,
        "terminal silence counted as underflow",
    )?;
    state.device_failed();
    stats.fill(&state, &mut odd)?;
    require(odd == [0.0; 19], "device failure returned PCM")?;
    for (a, b) in memory.iter_mut().zip(stats.memory) {
        *a += b;
    }
    let measured = BTreeMap::from([
        ("initial_queued_frames".into(), produced),
        ("first_starvation_requested_frames".into(), 4096),
        ("first_missing_frames".into(), 4096 - produced),
        ("second_starvation_missing_frames".into(), 9),
        ("stalled_last_consumed_frame".into(), position),
        ("resume_first_frame".into(), produced),
        ("total_guard_callbacks".into(), stats.times.len() as u64),
        ("total_guard_sample_values".into(), stats.samples),
    ]);
    Ok((
        vec![
            "not-ready/empty/odd samples".into(),
            "exact READY underrun frames and stalled playhead".into(),
            "resume has no replay/skip".into(),
            "same compiled-manifest cold seek".into(),
            "loop wrap/loop toggle/stop/device-failure".into(),
        ],
        measured,
    ))
}

fn finish<T>(handle: thread::JoinHandle<T>, deadline: Instant) -> Result<T> {
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    if !handle.is_finished() {
        drop(handle);
        return Err(
            "thread completion watchdog expired; handle detached, never unconditionally joined"
                .into(),
        );
    }
    handle
        .join()
        .map_err(|_| "diagnostic actor panicked".into())
}
fn barriers(song: &Song, memory: &mut [u64; 3]) -> Result<()> {
    let state = Arc::new(Shared::default());
    let worker_state = state.clone();
    let owned = song.clone();
    let (request_tx, request_rx) = mpsc::sync_channel::<()>(1);
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    let deadline = Instant::now() + WATCHDOG;
    let worker = thread::spawn(move || -> Result<()> {
        let mut producer = match Producer::new(&owned, None, worker_state.clone()) {
            Ok(producer) => producer,
            Err(error) => {
                let _ = ack_tx.send(Err(error.clone()));
                worker_state.stop();
                return Err(error);
            }
        };
        while request_rx.recv().is_ok() {
            let result = producer.pump_until_buffered();
            let failed = result.is_err();
            if ack_tx.send(result).is_err() {
                break;
            }
            if failed {
                worker_state.stop();
                break;
            }
        }
        Ok(())
    });
    let result = (|| -> Result<()> {
        let mut reference = Engine::new(song, None)?;
        let mut stats = Measurements::new();
        let mut out = [0.0; 256];
        for target in [0, 1000, reference.manifest.musical_frames - 1, 0] {
            state.seek(target);
            request_tx
                .send(())
                .map_err(|_| "barrier worker disconnected")?;
            ack_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| "barrier producer timeout")??;
            reference.cold_seek(target)?;
            let expected = rendered(&mut reference, target + 128)?;
            stats.fill(&state, &mut out)?;
            require(
                out.iter()
                    .zip(expected)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "barrier-driven producer/callback PCM mismatch",
            )?;
        }
        for (a, b) in memory.iter_mut().zip(stats.memory) {
            *a += b;
        }
        Ok(())
    })();
    state.stop();
    drop(request_tx);
    let joined = finish(worker, deadline)?;
    result?;
    joined
}
fn short_loop(base: &Song, memory: &mut [u64; 3]) -> Result<()> {
    let mut song = base.clone();
    song.bpm = 400.0;
    song.lines_per_beat = 256;
    song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1;
    put(&mut song, 0, 0, note(1, 60, 0.02));
    song.validate()?;
    let mut reference = Engine::new(&song, None)?;
    require(
        reference.manifest.musical_frames == 28,
        "independent shortest-loop frame oracle",
    )?;
    let cycle = rendered(&mut reference, 28)?;
    let expected = cycle.repeat(20);
    let state = Arc::new(Shared::default());
    state.set_loop(true, u64::MAX);
    let mut producer = Producer::new(&song, None, state.clone())?;
    let mut stats = Measurements::new();
    let mut got = Vec::new();
    let mut out = [0.0; 256];
    while got.len() < expected.len() {
        producer.pump_until_buffered()?;
        stats.fill(&state, &mut out)?;
        got.extend(out);
    }
    require(
        got[..expected.len()]
            .iter()
            .zip(expected)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "short-loop refill/cold-wrap mismatch",
    )?;
    require(
        state.underruns.load(Ordering::Relaxed) == 0,
        "short-loop prefill underflow",
    )?;
    state.stop();
    for (a, b) in memory.iter_mut().zip(stats.memory) {
        *a += b;
    }
    Ok(())
}
fn render_failure(base: &Song, memory: &mut [u64; 3]) -> Result<()> {
    let mut song = base.clone();
    song.regions.get_mut(&RegionId(3)).unwrap().amp_envelope = None;
    for col in 0..64 {
        put(&mut song, 0, col, note(3, 60, 1.0));
    }
    let state = Arc::new(Shared::default());
    let mut producer = Producer::new(&song, None, state.clone())?;
    let mut stats = Measurements::new();
    let mut out = [0.0; 1024];
    let mut failed = None;
    for _ in 0..8 {
        match producer.pump_until_buffered() {
            Ok(()) => stats.fill(&state, &mut out)?,
            Err(error) => {
                failed = Some(error);
                break;
            }
        }
    }
    require(
        failed.is_some_and(|error| error.contains("clipping")) && state.is_running(),
        "expected clipping error; public Producer does not auto-terminate Shared",
    )?;
    // Explicit HARNESS action, not a physical callback/device error delivery.
    state.device_failed();
    stats.fill(&state, &mut out)?;
    require(
        out == [0.0; 1024] && !state.is_running(),
        "explicit harness failure shutdown failed",
    )?;
    for (a, b) in memory.iter_mut().zip(stats.memory) {
        *a += b;
    }
    Ok(())
}
fn concurrent(mut song: Song, memory: &mut [u64; 3], pump_budget: usize) -> Result<Concurrent> {
    require(
        (1..=MAX_PUMPS).contains(&pump_budget),
        "invalid concurrent pump budget",
    )?;
    // At most eight simultaneous ordinary looping voices, separate from dense64 parity.
    song.track_order = vec![TrackId(1)];
    song.tracks.retain(|id, _| *id == TrackId(1));
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 8;
    song.order.truncate(1);
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .track_aliases
        .clear();
    for col in 0..8 {
        put(&mut song, 0, col, note(1, 60, 0.02));
    }
    song.validate()?;
    let state = Arc::new(Shared::default());
    state.set_loop(true, u64::MAX);
    let ready_calls = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + WATCHDOG;
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_state = state.clone();
    let worker = thread::spawn(move || -> Result<(usize, Vec<u64>)> {
        let result = (|| -> Result<(usize, Vec<u64>)> {
            let mut producer = Producer::new(&song, None, worker_state.clone())?;
            let musical = producer.engine.manifest.musical_frames;
            let mut times = Vec::with_capacity(MAX_PUMPS);
            let mut last = u64::MAX;
            let initial = Instant::now();
            producer.pump_until_buffered()?;
            times.push(initial.elapsed().as_nanos() as u64);
            tx.send(musical)
                .map_err(|_| "initialization receiver closed")?;
            while worker_state.is_running() {
                if Instant::now() >= deadline {
                    worker_state.stop();
                    return Err("producer watchdog expired (incomplete)".into());
                }
                let callbacks = worker_state.callbacks.load(Ordering::Relaxed);
                if callbacks != last || !worker_state.is_ready() {
                    if times.len() >= pump_budget {
                        worker_state.stop();
                        return Err("producer refill budget exhausted (incomplete)".into());
                    }
                    let start = Instant::now();
                    producer.pump_until_buffered()?;
                    times.push(start.elapsed().as_nanos() as u64);
                    last = callbacks;
                }
                thread::sleep(Duration::from_micros(100));
            }
            Ok((times.len(), times))
        })();
        if result.is_err() {
            worker_state.stop();
        }
        result
    });
    let musical = match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(n) => n,
        Err(_) => {
            state.stop();
            drop(worker);
            return Err("producer initialization timed out/failed".into());
        }
    };
    let consumer_state = state.clone();
    let consumer_ready = ready_calls.clone();
    let consumer = thread::spawn(move || -> Result<Measurements> {
        let result = (|| -> Result<Measurements> {
            let mut stats = Measurements::new();
            let mut output = [0.0; 2048];
            while consumer_state.is_running() {
                if Instant::now() >= deadline {
                    consumer_state.stop();
                    return Err("consumer watchdog expired (incomplete)".into());
                }
                if stats.times.len() >= MAX_FILLS {
                    consumer_state.stop();
                    return Err("consumer fill budget exhausted (incomplete)".into());
                }
                let n = [128, 256, 512, 37][stats.times.len() % 4] * 2;
                let ready = consumer_state.is_ready();
                stats.fill(&consumer_state, &mut output[..n])?;
                if ready && consumer_state.is_ready() {
                    consumer_ready.fetch_add(1, Ordering::SeqCst);
                }
                thread::sleep(Duration::from_micros(100)); // Artificial pacing, not a device clock.
            }
            stats.fill(&consumer_state, &mut output)?;
            require(output == [0.0; 2048], "post-stop callback not silent")?;
            Ok(stats)
        })();
        if result.is_err() {
            consumer_state.stop();
        }
        result
    });
    let mut commands = 0;
    let mut error = None;
    for index in 0..CONCURRENT_COMMANDS {
        match index % 4 {
            0 => state.seek(0),
            1 => state.seek(137),
            2 => state.set_loop(true, musical),
            _ => state.seek(musical / 2),
        }
        let target = ready_calls.load(Ordering::SeqCst) + 2;
        while ready_calls.load(Ordering::SeqCst) < target
            && state.is_running()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_micros(100));
        }
        if !state.is_running() || Instant::now() >= deadline {
            error = Some("concurrent run ended before prescribed command sequence (incomplete)");
            break;
        }
        commands += 1;
    }
    state.stop();
    let producer_result = finish(worker, deadline)?;
    let consumer_result = finish(consumer, deadline)?;
    let mut stats = consumer_result?;
    let (pumps, mut pump_times) = producer_result?;
    if let Some(error) = error {
        return Err(error.into());
    }
    require(
        commands == CONCURRENT_COMMANDS,
        "not every concurrent control completed",
    )?;
    for (a, b) in memory.iter_mut().zip(stats.memory) {
        *a += b;
    }
    Ok(Concurrent {
        completion: "intended_stop".into(),
        commands_completed: commands,
        post_command_ready_observations: commands * 2,
        pump_calls: pumps,
        emitted_frame_upper_bound: pumps * (2048 + 511),
        callbacks: stats.times.len(),
        callback_sample_values: stats.samples,
        underrun_frames: state.underruns.load(Ordering::Relaxed),
        callback_timing: stats.timing(),
        worker_refill_timing: summarize(&mut pump_times),
        artificial_consumer_pause_us: 100,
    })
}

pub fn run(root: &Path) -> Result<Report> {
    require(root.is_absolute(), "diagnostic directory must be absolute")?;
    fs::create_dir(root).map_err(|e| format!("create NEW diagnostic directory: {e}"))?;
    let result = run_created(root);
    if let Err(error) = &result {
        let failure = serde_json::json!({"schema":1,"complete":false,"physical_device_opened":false,"error":error,"created_evidence_retained":true});
        if let Err(write_error) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("failure.json"))
            .and_then(|mut file| file.write_all(failure.to_string().as_bytes()))
        {
            return Err(format!(
                "{error}; could not write owned failure receipt: {write_error}"
            ));
        }
    }
    result
}
fn run_created(root: &Path) -> Result<Report> {
    let base = base(root)?;
    let dense = dense(base.clone())?;
    let mixed = mixed(base.clone())?;
    let mut cases = Vec::new();
    let mut memory = [0; 3];
    let mut normal_frames = 0;
    for (name, song) in [
        ("dense64", &dense),
        ("regions-kit-RR-keys-phrase-FX-alias-mute", &mixed),
    ] {
        let mut engine = engine(song)?;
        let end = engine.manifest.musical_frames + TAIL;
        if name == "dense64" {
            require(
                engine
                    .manifest
                    .events
                    .iter()
                    .filter(|e| matches!(e.command, Command::On { .. }))
                    .count()
                    == 64,
                "dense fixture is not64 simultaneous starts",
            )?;
        }
        let expected = rendered(&mut engine, end)?;
        for sizes in [
            &[128][..],
            &[256][..],
            &[512][..],
            &[1, 37, 127, 129, 255, 511, 513, 1024][..],
        ] {
            normal_frames += end;
            require(
                normal_frames <= MAX_NORMAL_FRAMES,
                "normal matrix frame budget exceeded",
            )?;
            cases.push(parity(song, name, sizes, &expected, &mut memory)?);
        }
    }
    let (mut guards, guard_measurements) = guards(&mixed, &mut memory)?;
    barriers(&mixed, &mut memory)?;
    guards.push("barrier-driven separate producer, exact cold-seek PCM".into());
    short_loop(&base, &mut memory)?;
    guards.push("28-frame loop cold-reset/refill exactness".into());
    render_failure(&base, &mut memory)?;
    guards.push("Engine error followed by explicit harness device_failed shutdown".into());
    let refused = concurrent(base.clone(), &mut memory, 1);
    require(
        matches!(refused, Err(ref e) if e.contains("producer refill budget exhausted")),
        "deliberately insufficient producer budget was not reported as incomplete",
    )?;
    guards.push("one-pump concurrent budget explicitly refuses incomplete sequence".into());
    let concurrent = concurrent(base.clone(), &mut memory, MAX_PUMPS)?;
    for asset in base.samples.values() {
        sample_asset::verify_identity(asset, &asset.path)?;
    }
    let compiler = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unavailable (record command environment separately)".into());
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "not reported on this platform".into());
    let report=Report{schema:1,complete:true,scope:"DEVICE-FREE authored transport diagnostics, not physical realtime acceptance".into(),physical_device_opened:false,rustc_on_runtime_path:compiler,build_debug_assertions:cfg!(debug_assertions),compiled_os:std::env::consts::OS.into(),compiled_arch:std::env::consts::ARCH.into(),cpu_runtime_procfs:cpu,output_sample_rate:RATE,queue_capacity_frames:2048,readiness_frames:1024,production_preferred_device_buffer_frames:512,limits:BTreeMap::from([("normal_matrix_frames",MAX_NORMAL_FRAMES),("fixture_starts",256),("fixture_musical_frames",96_000),("concurrent_pump_calls",MAX_PUMPS as u64),("concurrent_fill_calls",MAX_FILLS as u64),("concurrent_controls",CONCURRENT_COMMANDS as u64),("concurrent_voices",8),("watchdog_milliseconds",WATCHDOG.as_millis() as u64)]),normal_matrix_frames:normal_frames,callback_memory:memory,callback_memory_labels:["alloc_calls","realloc_calls","dealloc_calls"],parity:cases,guards,guard_measurements,concurrent,authored_source_sha256:base.samples.values().map(|a|a.sha256.clone()).collect(),warnings:vec![
        "No CPAL/device access; no measured hardware latency, device xruns, audibility, ten-minute sustain or realtime guarantee. Queue sizes are buffering facts only.".into(),
        "rustc_on_runtime_path is runtime PATH metadata, not proof of the build compiler; build_debug_assertions is not a full optimization profile. The external build receipt must bind command, toolchain, profile and executable hash.".into(),
        "PCM parity uses the SAME existing Engine; independent integer authored row-frame checks are narrower scheduling guards, not an independent DSP oracle.".into(),
        "Cold seek clears active voices but retains compiled later attribute/RR/zone/phrase choices. Loop reuses the same compiled schedule; no isolated-occurrence reevaluation.".into(),
        "Timing distributions include harness TLS counter overhead and cloud OS scheduling; clocks/reporting are outside allocation counting. No timing threshold is an acceptance assertion.".into(),
        "Zero callback alloc/realloc/dealloc does not prove absence of locks/syscalls, queue contention, deadline misses or device xruns. True-concurrent waveform order is intentionally not asserted; post-command ready observations are bounded liveness acknowledgements, not proof of a uniquely ordered waveform.".into(),
        "callbacks counts every fill including empty/not-ready calls; underruns counts missing stereo frames only while READY. Existing audible_frames counts frames above1e-6 sample amplitude, not perceptual audibility. Producer errors require an explicit harness stop/device_failed action; no real device failure delivery is exercised.".into(),
        "Concurrent actor work is bounded; intended command completion is required. Budget exhaustion or watchdog exit is a failed/incomplete run, not a shortened success. Run the executable under an external timeout.".into(),
    ]};
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    require(bytes.len() <= 1024 * 1024, "diagnostic report exceeds1MiB")?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("report.json"))
        .and_then(|mut f| f.write_all(&bytes))
        .map_err(|e| e.to_string())?;
    Ok(report)
}
