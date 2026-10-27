//! Incremental worker scheduling with a bounded lock-free PCM handoff.
//! Callback only copies frames and accesses atomics: no DSP, files, locks or allocation.
use crate::{song::Song, song_render::Engine, RATE};
use crossbeam_queue::ArrayQueue;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
const CAPACITY: usize = 2048;
#[derive(Clone, Copy)]
struct Packet {
    command: u64,
    frame: u64,
    audio: [f32; 2],
    end: bool,
}
pub struct Shared {
    queue: ArrayQueue<Packet>,
    command: AtomicU64,
    pub running: AtomicBool,
    pub ready: AtomicBool,
    pub looping: AtomicBool,
    pub quit: AtomicBool,
    pub ended: AtomicBool,
    pub device_error: AtomicBool,
    pub position: AtomicU64,
    pub underruns: AtomicU64,
    pub callbacks: AtomicU64,
    pub audible_frames: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            queue: ArrayQueue::new(CAPACITY),
            command: AtomicU64::new(1 << 32),
            running: AtomicBool::new(true),
            ready: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            device_error: AtomicBool::new(false),
            position: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            audible_frames: AtomicU64::new(0),
        }
    }
}
impl Shared {
    pub fn seek(&self, frame: u64) {
        self.ready.store(false, Ordering::SeqCst);
        self.command
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some((((v >> 32).wrapping_add(1)) << 32) | (frame & 0xffff_ffff))
            })
            .unwrap();
        self.position.store(frame, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.quit.store(true, Ordering::SeqCst);
        self.seek(0);
    }
    /// A device failure must silence subsequent callbacks, including queued PCM.
    pub fn device_failed(&self) {
        self.device_error.store(true, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        self.ready.store(false, Ordering::SeqCst);
    }
    /// Toggle loop with a cold restart at the last consumed frame. Discards buffered
    /// old end/tail decisions, rather than letting a stale end marker stop a new loop.
    pub fn set_loop(&self, looping: bool, musical_frames: u64) {
        self.looping.store(looping, Ordering::SeqCst);
        self.seek(
            self.position
                .load(Ordering::Relaxed)
                .min(musical_frames - 1),
        );
    }
    pub fn fill(&self, output: &mut [f32]) {
        output.fill(0.0);
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        if !self.running.load(Ordering::SeqCst) || !self.ready.load(Ordering::SeqCst) {
            return;
        }
        let mut stale_budget = CAPACITY;
        for frame in output.as_chunks_mut::<2>().0 {
            if !self.running.load(Ordering::SeqCst) || !self.ready.load(Ordering::SeqCst) {
                break;
            }
            loop {
                let Some(packet) = self.queue.pop() else {
                    self.underruns.fetch_add(1, Ordering::Relaxed);
                    break;
                };
                if packet.command != self.command.load(Ordering::SeqCst) {
                    if stale_budget == 0 {
                        return;
                    }
                    stale_budget -= 1;
                    continue;
                }
                if packet.end {
                    self.running.store(false, Ordering::SeqCst);
                    self.ended.store(true, Ordering::Relaxed);
                    return;
                }
                // Recheck immediately before writing; previously submitted hardware
                // buffers cannot be retracted by Stop.
                if !self.running.load(Ordering::SeqCst) {
                    return;
                }
                frame.copy_from_slice(&packet.audio);
                self.position.store(packet.frame, Ordering::Relaxed);
                if packet.audio.iter().any(|v| v.abs() > 0.000001) {
                    self.audible_frames.fetch_add(1, Ordering::Relaxed);
                }
                break;
            }
        }
    }
}
pub struct Producer {
    pub engine: Engine,
    shared: Arc<Shared>,
    command: u64,
    end_queued: bool,
}
impl Producer {
    pub fn new(song: &Song, base: Option<&Path>, shared: Arc<Shared>) -> Result<Self, String> {
        Ok(Self {
            engine: Engine::new(song, base)?,
            shared,
            command: 0,
            end_queued: false,
        })
    }
    /// One bounded render step. Allocations only when cold-resetting on this worker.
    pub fn pump(&mut self) -> Result<(), String> {
        if !self.shared.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let command = self.shared.command.load(Ordering::SeqCst);
        if command != self.command {
            while self.shared.queue.pop().is_some() {}
            self.engine
                .cold_seek((command & 0xffff_ffff).min(self.engine.manifest.musical_frames - 1))?;
            self.command = command;
            self.end_queued = false;
        }
        if self.end_queued || self.shared.queue.len() > CAPACITY - 512 {
            return Ok(());
        }
        let looping = self.shared.looping.load(Ordering::SeqCst);
        let end = self.engine.manifest.musical_frames
            + if looping {
                0
            } else {
                self.engine.manifest.tail_frames
            };
        if self.engine.position >= end {
            if looping {
                self.engine.cold_seek(0)?;
            } else {
                self.shared
                    .queue
                    .push(Packet {
                        command,
                        frame: end,
                        audio: [0.0; 2],
                        end: true,
                    })
                    .map_err(|_| "Transport queue unexpectedly full")?;
                self.end_queued = true;
                self.shared.ready.store(true, Ordering::SeqCst);
                return Ok(());
            }
        }
        let start = self.engine.position;
        let mut data = [0.0f32; 1024];
        let count = self.engine.block(&mut data, end)?;
        for i in 0..count {
            self.shared
                .queue
                .push(Packet {
                    command,
                    frame: start + i as u64,
                    audio: [data[i * 2], data[i * 2 + 1]],
                    end: false,
                })
                .map_err(|_| "Transport queue unexpectedly full")?;
        }
        if self.shared.queue.len() >= 1024 && self.shared.command.load(Ordering::SeqCst) == command
        {
            self.shared.ready.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}
/// Convert consumed audio frame to immutable snapshot occurrence/row, not edit cursor.
pub fn location(song: &Song, frame: u64) -> (usize, u32) {
    let per_row = RATE as f64 * 60.0 / (song.bpm * song.lines_per_beat as f64);
    let mut row = (frame as f64 / per_row).floor() as u64;
    // Match the evaluator's nearest-frame boundaries, including rows rounded down.
    if ((row + 1) as f64 * per_row).round() as u64 <= frame {
        row += 1;
    }
    let mut start = 0;
    for (index, o) in song.order.iter().enumerate() {
        let rows = song.patterns[&o.pattern].rows as u64;
        if row < start + rows {
            return (index, (row - start) as u32);
        }
        start += rows;
    }
    let index = song.order.len() - 1;
    (index, song.patterns[&song.order[index].pattern].rows - 1)
}
#[cfg(feature = "realtime")]
pub mod host {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::{
        fs::{self, OpenOptions},
        path::PathBuf,
        sync::mpsc::{self, Receiver},
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };
    struct Lease(PathBuf);
    impl Lease {
        fn acquire_in(directory: PathBuf) -> Result<Self, String> {
            fs::create_dir_all(&directory)
                .map_err(|e| format!("Cannot create transport runtime directory: {e}"))?;
            let lock = directory.join("transport.lock");
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .map_err(|e| {
                    format!(
                        "Transport lease unavailable at {} (another instance or stale lease): {e}",
                        lock.display()
                    )
                })?;
            Ok(Self(lock))
        }
        fn acquire() -> Result<Self, String> {
            #[cfg(target_os = "windows")]
            let base = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
                .unwrap_or_else(std::env::temp_dir)
                .join("Leviathan");

            #[cfg(not(target_os = "windows"))]
            let base = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
                .unwrap_or_else(std::env::temp_dir)
                .join("leviathan");

            Self::acquire_in(base)
        }
    }

    impl Drop for Lease {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    #[derive(Default)]
    pub struct Transport {
        pub shared: Option<Arc<Shared>>,
        pub snapshot: Option<Song>,
        worker: Option<JoinHandle<()>>,
        result: Option<Receiver<Result<(), String>>>,
        pub error: Option<String>,
    }
    impl Transport {
        pub fn busy(&self) -> bool {
            self.worker.is_some()
        }
        pub fn start(&mut self, song: Song, base: Option<PathBuf>) -> Result<(), String> {
            if self.busy() {
                return Err("Transport is active or still stopping".into());
            }
            song.validate()?;
            if song.order.is_empty() {
                return Err("Song order is empty".into());
            }
            let shared = Arc::new(Shared::default());
            let state = shared.clone();
            let snapshot = song.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let worker = thread::Builder::new()
                .name("leviathan-transport".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(song, base, state.clone())
                    }))
                    .unwrap_or_else(|_| Err("Transport worker panicked".into()));
                    state.running.store(false, Ordering::SeqCst);
                    let _ = tx.send(result);
                })
                .map_err(|e| e.to_string())?;
            self.shared = Some(shared);
            self.snapshot = Some(snapshot);
            self.worker = Some(worker);
            self.result = Some(rx);
            self.error = None;
            Ok(())
        }
        pub fn stop(&self) {
            if let Some(s) = &self.shared {
                s.stop();
            }
        }
        pub fn poll(&mut self) {
            if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
                let _ = self.worker.take().unwrap().join();
                if let Some(rx) = self.result.take() {
                    match rx.try_recv() {
                        Ok(Err(e)) => self.error = Some(e),
                        Err(_) => self.error = Some("Transport worker disconnected".into()),
                        _ => (),
                    }
                }
            }
        }
    }
    impl Drop for Transport {
        fn drop(&mut self) {
            self.stop(); /* thread owns stream; never join a loading worker on UI */
        }
    }
    fn run(song: Song, base: Option<PathBuf>, state: Arc<Shared>) -> Result<(), String> {
        let _lease = Lease::acquire()?;
        let mut producer = Producer::new(&song, base.as_deref(), state.clone())?;
        if state.quit.load(Ordering::SeqCst) {
            return Ok(());
        }
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No default output device")?;
        let supported = device
            .supported_output_configs()
            .map_err(|e| e.to_string())?
            .find(|c| {
                c.channels() == 2
                    && c.sample_format() == cpal::SampleFormat::F32
                    && c.min_sample_rate() <= RATE
                    && c.max_sample_rate() >= RATE
            })
            .ok_or(
                "Default device does not expose 48 kHz stereo float32; settings were not changed",
            )?
            .with_sample_rate(RATE);
        let mut config = supported.config();
        if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
            config.buffer_size = cpal::BufferSize::Fixed(512u32.clamp(*min, *max));
        }
        for _ in 0..8 {
            producer.pump()?;
        }
        let callback_state = state.clone();
        let error_state = state.clone();
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| callback_state.fill(data),
                move |_| error_state.device_failed(),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        let mut previous = 0;
        let mut progressed = Instant::now();
        while !state.quit.load(Ordering::SeqCst) && state.running.load(Ordering::SeqCst) {
            producer.pump()?;
            let callbacks = state.callbacks.load(Ordering::Relaxed);
            if callbacks != previous {
                previous = callbacks;
                progressed = Instant::now();
            }
            if progressed.elapsed() > Duration::from_secs(2) {
                return Err("Audio callback stopped advancing".into());
            }
            thread::sleep(Duration::from_millis(1));
        }
        drop(stream);
        if state.device_error.load(Ordering::SeqCst) {
            Err("Audio device reported a stream error; playback stopped".into())
        } else {
            Ok(())
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn runtime_lease_guards_collision_and_cleans_its_own_file() {
            let directory = std::env::temp_dir()
                .join(format!("leviathan-release-lease-{}", std::process::id()));
            let lease = Lease::acquire_in(directory.clone()).unwrap();
            assert!(Lease::acquire_in(directory.clone()).is_err());
            assert!(directory.join("transport.lock").is_file());
            drop(lease);
            assert!(!directory.join("transport.lock").exists());
            drop(Lease::acquire_in(directory.clone()).unwrap());
            fs::remove_dir(directory).unwrap();
        }
    }
}
