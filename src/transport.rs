//! Incremental worker scheduling with a bounded lock-free PCM handoff.
//! Callback only copies frames and accesses atomics: no DSP, files, locks or allocation.
use crate::{
    editor::DocumentOwner,
    meter_windows::{MeterReport, MeterWindows},
    meters::{MeterLayout, REPORT_CAPACITY},
    song::Song,
    song_render::Engine,
    track_mix::{MixLayout, MixSnapshot},
    RATE,
};
use crossbeam_queue::ArrayQueue;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
const CAPACITY: usize = 2048;
const FRAME_MASK: u64 = u32::MAX as u64;
const GENERATION_STEP: u64 = 1 << 32;
const GENERATION_MASK: u64 = (1 << 60) - GENERATION_STEP;
const LOOP: u64 = 1 << 60;
const READY: u64 = 1 << 61;
const ENDED: u64 = 1 << 62;
const STOPPED: u64 = 1 << 63;
const TERMINAL: u64 = ENDED | STOPPED;
#[derive(Clone, Copy)]
struct Packet {
    command: u64,
    frame: u64,
    audio: [f32; 2],
    end: bool,
}
/// Latest complete mix state, never an ordered automation event. The opaque core
/// snapshot carries its Engine routing/layout identity and validated coefficients.
struct MixUpdate {
    revision: u64,
    snapshot: MixSnapshot,
}
struct MixOwnerBinding {
    owner: DocumentOwner,
    song: Song,
}
struct WorkerMixBinding {
    song: Arc<Song>,
    layout: MixLayout,
}
/// Request acceptance and worker enqueue are deliberately separate observations.
/// Applied means the first nonzero target-ramp block was wholly enqueued, not
/// that the 128-frame ramp completed. Neither revision acknowledges callback
/// consumption, hardware output or hearing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveMixStatus {
    pub accepted_revision: u64,
    pub applied_revision: u64,
    pub available: bool,
}
/// Opaque read grant for one exact owner/content/session. Acquire on a document
/// revision/session transition, not by serializing the Song on every paint.
#[derive(Debug, Clone)]
pub struct MeterReader {
    session: Arc<()>,
    owner: DocumentOwner,
}
/// At most four newly received reports. UI display coalescing is distinct from
/// dropped_reports (only bounded worker-queue evictions); none means no complete
/// window, never a measurement of zero. A generation change invalidates cached
/// reports immediately, including before the worker observes a manual seek.
#[derive(Debug, Clone)]
pub struct MeterPoll {
    pub generation: u64,
    pub dropped_reports: u64,
    pub layout: Option<Arc<MeterLayout>>,
    pub reports: Vec<MeterReport>,
    pub disabled_reason: Option<String>,
}
#[derive(Clone, Copy)]
struct MeterPacket {
    command: u64,
    report: MeterReport,
}
/// A session has one producer and one serial audio callback. Public state atomics
/// are diagnostic snapshots, not a command interface: use the methods below for
/// transitions and generation-consistent position/readiness/state observations.
pub struct Shared {
    queue: ArrayQueue<Packet>,
    // Orthogonal to `command`: changing mix never flushes queued PCM or seeks.
    // Only the UI/control submit path locks. Worker takes at most one snapshot
    // immediately before a renderable block; fill() never touches these fields.
    mix_mailbox: ArrayQueue<MixUpdate>,
    mix_owner: OnceLock<MixOwnerBinding>,
    mix_worker: OnceLock<Result<WorkerMixBinding, String>>,
    mix_submission_revision: Mutex<u64>,
    mix_accepted_revision: AtomicU64,
    mix_applied_revision: AtomicU64,
    mix_revoked: AtomicBool,
    mix_render_complete: AtomicBool,
    // Meter state is worker/control-only. Callback never reads or reclaims it.
    meter_session: Arc<()>,
    meter_source: OnceLock<Result<Arc<Song>, String>>,
    meter_layout: OnceLock<Arc<MeterLayout>>,
    meter_disabled: OnceLock<String>,
    meter_queue: ArrayQueue<MeterPacket>,
    meter_dropped: AtomicU64,
    // Target frame, generation, loop, readiness and terminal state transition
    // together. An old worker/callback cannot publish readiness or end a new seek.
    command: AtomicU64,
    consumed: AtomicU64,
    seek_limit: AtomicU64,
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
            mix_mailbox: ArrayQueue::new(1),
            mix_owner: OnceLock::new(),
            mix_worker: OnceLock::new(),
            mix_submission_revision: Mutex::new(0),
            mix_accepted_revision: AtomicU64::new(0),
            mix_applied_revision: AtomicU64::new(0),
            mix_revoked: AtomicBool::new(false),
            mix_render_complete: AtomicBool::new(false),
            meter_session: Arc::new(()),
            meter_source: OnceLock::new(),
            meter_layout: OnceLock::new(),
            meter_disabled: OnceLock::new(),
            meter_queue: ArrayQueue::new(REPORT_CAPACITY),
            meter_dropped: AtomicU64::new(0),
            command: AtomicU64::new(GENERATION_STEP),
            consumed: AtomicU64::new(0),
            seek_limit: AtomicU64::new(FRAME_MASK),
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
    fn check_live_mix_running(&self) -> Result<(), String> {
        if self.mix_revoked.load(Ordering::SeqCst) {
            return Err("Live mix control was revoked; start Song playback again".into());
        }
        let command = self.command.load(Ordering::SeqCst);
        if command & ENDED != 0 {
            return Err("Song playback ended; no live mix request was submitted".into());
        }
        if command & STOPPED != 0 {
            return Err(
                "Transport is stopped or stopping; no live mix request was submitted".into(),
            );
        }
        if self.mix_render_complete.load(Ordering::SeqCst) {
            return Err(
                "All Song audio is already rendered; no live mix request was submitted".into(),
            );
        }
        Ok(())
    }
    /// Bind once to the document that started this Song session. Do not call for
    /// audition. Binding can precede worker loading; submissions still require a
    /// successful Engine layout and an exact match to its playing content.
    pub fn bind_live_mix(&self, owner: DocumentOwner, song: &Song) -> Result<(), String> {
        self.check_live_mix_running()?;
        crate::project::check_serialized_size(song)?;
        if self.mix_owner.get().is_some() {
            return Err("This transport session already has a live mix owner".into());
        }
        if let Some(Ok(worker)) = self.mix_worker.get() {
            if !crate::track_mix::same_playing_content(song, &worker.song)? {
                return Err("Live mix binding does not match the playing Song content".into());
            }
        }
        let binding = MixOwnerBinding {
            owner,
            song: song.clone(),
        };
        self.check_live_mix_running()?;
        self.mix_owner
            .set(binding)
            .map_err(|_| "This transport session already has a live mix owner".to_string())?;
        // A racing stop/revocation never resets the terminal or revoked state.
        self.check_live_mix_running()
    }
    /// Permanently revoke this session's live-control grant (e.g. New/Open).
    /// Existing audio/ramp is not rewound. In-flight work may already have been
    /// rendered; pending updates are logically invalid immediately and reclaimed
    /// on the control/worker thread, never inside the audio callback.
    pub fn revoke_live_mix(&self) {
        self.mix_revoked.store(true, Ordering::SeqCst);
        let _ = self.mix_mailbox.pop();
    }
    /// Submit one immutable FULL snapshot. Newer accepted requests replace a
    /// pending older request; intermediate revisions need not reach the worker.
    /// Document commit is separate and is not rolled back on runtime refusal.
    pub fn submit_live_mix(&self, owner: &DocumentOwner, song: &Song) -> Result<u64, String> {
        self.check_live_mix_running()?;
        let binding = self
            .mix_owner
            .get()
            .ok_or("Live mix is unbound (audition does not support Song mix control)")?;
        if !binding.owner.same_owner(owner) {
            return Err("Live mix belongs to another document; start Song playback again".into());
        }
        let worker = self
            .mix_worker
            .get()
            .ok_or("Song playback is still loading; live mix was not submitted")?
            .as_ref()
            .map_err(Clone::clone)?;
        if !crate::track_mix::same_playing_content(song, &binding.song)?
            || !crate::track_mix::same_playing_content(song, &worker.song)?
        {
            return Err(
                "Playing content changed beyond track mix; start Song playback again".into(),
            );
        }
        let snapshot = worker.layout.snapshot(&song.track_mix)?;
        // Serialize only submitting control threads so a late older submission
        // can never overwrite the newest revision. Worker/callback never lock.
        let mut revision = self
            .mix_submission_revision
            .lock()
            .map_err(|_| "Live mix submission state is unavailable".to_string())?;
        self.check_live_mix_running()?;
        let next = revision
            .checked_add(1)
            .ok_or("Live mix revision exhausted; start Song playback again")?;
        self.mix_accepted_revision.store(next, Ordering::SeqCst);
        let _ = self.mix_mailbox.force_push(MixUpdate {
            revision: next,
            snapshot,
        });
        *revision = next;
        // If termination won the race, the accepted historical request remains
        // observable but must not be represented as a current delivery promise.
        if self.check_live_mix_running().is_err() {
            let _ = self.mix_mailbox.pop();
        }
        Ok(next)
    }
    pub fn live_mix_status(&self) -> LiveMixStatus {
        // Read the worker observation first: it cannot exceed a later observed
        // accepted revision even when a new request arrives between these loads.
        let applied_revision = self.mix_applied_revision.load(Ordering::SeqCst);
        LiveMixStatus {
            accepted_revision: self.mix_accepted_revision.load(Ordering::SeqCst),
            applied_revision,
            available: self.is_running()
                && !self.mix_revoked.load(Ordering::SeqCst)
                && !self.mix_render_complete.load(Ordering::SeqCst)
                && self.mix_owner.get().is_some()
                && self.mix_worker.get().is_some_and(Result::is_ok),
        }
    }
    /// Read authorization is separate from observation and never starts playback,
    /// enables audition, submits mix, or mutates the document. A reader must be
    /// reacquired after the caller's document revision changes; exact content
    /// excludes only track_mix, just like the playing Song binding.
    pub fn meter_reader(&self, owner: &DocumentOwner, song: &Song) -> Result<MeterReader, String> {
        let binding = self
            .mix_owner
            .get()
            .ok_or("Meters are unavailable for unbound/audition playback")?;
        if !binding.owner.same_owner(owner) {
            return Err("Meters belong to another document".into());
        }
        if !crate::track_mix::same_playing_content(song, &binding.song)? {
            return Err(
                "Playing content changed; meter readings belong to the previous Song snapshot"
                    .into(),
            );
        }
        if let Some(source) = self.meter_source.get() {
            let source = source.as_ref().map_err(Clone::clone)?;
            if !crate::track_mix::same_playing_content(song, source)? {
                return Err("Meter binding does not match the worker Song content".into());
            }
        }
        let reader = MeterReader {
            session: self.meter_session.clone(),
            owner: owner.clone(),
        };
        self.check_meter_reader(&reader)?;
        Ok(reader)
    }
    fn check_meter_reader(&self, reader: &MeterReader) -> Result<u64, String> {
        if !Arc::ptr_eq(&reader.session, &self.meter_session)
            || !self
                .mix_owner
                .get()
                .is_some_and(|binding| binding.owner.same_owner(&reader.owner))
        {
            return Err("Meter reader belongs to another owner or playback session".into());
        }
        let command = self.command.load(Ordering::SeqCst) & !READY;
        if command & TERMINAL != 0 || self.mix_revoked.load(Ordering::SeqCst) {
            return Err("Meter session is stopped, ended, failed or revoked".into());
        }
        Ok(command)
    }
    /// Cheap bounded worker-report read; no callback, device or heard position is
    /// measured. Single UI consumer is expected. Any command race discards this
    /// poll's old-generation reports before returning the new generation tag.
    /// A subsequent command can stale an already returned historical snapshot;
    /// callers invalidate cached reports on their next generation/status read.
    pub fn poll_meters(&self, reader: &MeterReader) -> Result<MeterPoll, String> {
        self.check_meter_reader(reader)?;
        let mut packets = [None; REPORT_CAPACITY];
        for slot in &mut packets {
            let Some(packet) = self.meter_queue.pop() else {
                break;
            };
            *slot = Some(packet);
        }
        self.finish_meter_poll(reader, packets)
    }
    fn finish_meter_poll(
        &self,
        reader: &MeterReader,
        packets: [Option<MeterPacket>; REPORT_CAPACITY],
    ) -> Result<MeterPoll, String> {
        let command = self.check_meter_reader(reader)?;
        let mut reports = Vec::with_capacity(REPORT_CAPACITY);
        if self.meter_disabled.get().is_none() {
            // Filter at final validation, not before draining. A new-generation
            // packet arriving during a racing poll remains useful; obsolete
            // packets can never be represented as current-generation data.
            for packet in packets.into_iter().flatten() {
                if packet.command == command {
                    reports.push(packet.report);
                }
            }
        }
        let disabled_reason = self.meter_disabled.get().cloned();
        if disabled_reason.is_some() {
            reports.clear();
        }
        Ok(MeterPoll {
            generation: (command & GENERATION_MASK) / GENERATION_STEP,
            dropped_reports: self.meter_dropped.load(Ordering::SeqCst),
            layout: self.meter_layout.get().cloned(),
            reports,
            disabled_reason,
        })
    }
    pub fn is_running(&self) -> bool {
        self.command.load(Ordering::SeqCst) & TERMINAL == 0
    }
    pub fn is_ready(&self) -> bool {
        self.command.load(Ordering::SeqCst) & (TERMINAL | READY) == READY
    }
    pub fn has_ended(&self) -> bool {
        self.command.load(Ordering::SeqCst) & ENDED != 0
    }
    /// Last consumed frame, or the cold-seek target before that generation plays.
    /// An in-flight old callback cannot overwrite a newer seek's playhead.
    pub fn position(&self) -> u64 {
        let before = self.command.load(Ordering::SeqCst);
        let consumed = self.consumed.load(Ordering::SeqCst);
        let after = self.command.load(Ordering::SeqCst);
        if before & GENERATION_MASK == after & GENERATION_MASK
            && consumed & GENERATION_MASK == after & GENERATION_MASK
        {
            consumed & FRAME_MASK
        } else {
            (after & FRAME_MASK).min(self.seek_limit.load(Ordering::SeqCst))
        }
    }
    fn change_command(&self, frame: u64, looping: Option<bool>) {
        let frame = frame.min(self.seek_limit.load(Ordering::SeqCst));
        let result = self
            .command
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                if v & TERMINAL != 0 {
                    return None;
                }
                let loop_flag = looping.map_or(v & LOOP, |enabled| if enabled { LOOP } else { 0 });
                Some(((v.wrapping_add(GENERATION_STEP)) & GENERATION_MASK) | loop_flag | frame)
            });
        if result.is_ok() {
            self.ready.store(false, Ordering::SeqCst);
            self.position.store(frame, Ordering::Relaxed);
            if let Some(looping) = looping {
                self.looping.store(looping, Ordering::SeqCst);
            }
        }
    }
    /// Cold seek, clamped to the musical region once the producer is initialized.
    /// A terminal session cannot be restarted by seek; create a new session.
    pub fn seek(&self, frame: u64) {
        self.change_command(frame, None);
    }
    pub fn stop(&self) {
        self.command
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(
                    (v.wrapping_add(GENERATION_STEP) & GENERATION_MASK)
                        | (v & (LOOP | ENDED))
                        | STOPPED,
                )
            })
            .unwrap();
        self.running.store(false, Ordering::SeqCst);
        self.ready.store(false, Ordering::SeqCst);
        self.quit.store(true, Ordering::SeqCst);
        self.position.store(0, Ordering::Relaxed);
    }
    fn finish_worker(&self) {
        self.command.fetch_or(STOPPED, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        self.ready.store(false, Ordering::SeqCst);
    }
    /// Device failure is terminal for this session; queued PCM must remain silent.
    pub fn device_failed(&self) {
        self.device_error.store(true, Ordering::SeqCst);
        self.finish_worker();
        self.quit.store(true, Ordering::SeqCst);
    }
    /// Toggle loop with a cold restart at the last consumed frame. Loop policy and
    /// generation change atomically, invalidating buffered old end/tail decisions.
    pub fn set_loop(&self, looping: bool, musical_frames: u64) {
        self.change_command(
            self.position().min(musical_frames.saturating_sub(1)),
            Some(looping),
        );
    }
    fn mark_ready(&self, command: u64) {
        if self
            .command
            .compare_exchange(command, command | READY, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.ready.store(true, Ordering::SeqCst);
        }
    }
    fn finish_naturally(&self, command: u64) -> bool {
        if self
            .command
            .compare_exchange(
                command | READY,
                command | ENDED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return false;
        }
        self.running.store(false, Ordering::SeqCst);
        self.ready.store(false, Ordering::SeqCst);
        self.ended.store(true, Ordering::SeqCst);
        true
    }
    /// Fill one callback buffer. Do not invoke concurrently with another fill.
    /// A command observed during this callback silences the entire in-flight
    /// buffer; already returned/submitted hardware buffers cannot be retracted.
    pub fn fill(&self, output: &mut [f32]) {
        output.fill(0.0);
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        let command = self.command.load(Ordering::SeqCst);
        if command & (TERMINAL | READY) != READY {
            return;
        }
        let mut stale_budget = CAPACITY;
        let mut audible = 0;
        for index in 0..output.len() / 2 {
            loop {
                if self.command.load(Ordering::SeqCst) != command {
                    output.fill(0.0);
                    return;
                }
                let Some(packet) = self.queue.pop() else {
                    self.underruns.fetch_add(1, Ordering::Relaxed);
                    break;
                };
                if packet.command != command & !READY {
                    stale_budget -= 1;
                    if stale_budget == 0 {
                        return;
                    }
                    continue;
                }
                if packet.end {
                    if self.finish_naturally(packet.command) {
                        self.audible_frames.fetch_add(audible, Ordering::Relaxed);
                    } else {
                        output.fill(0.0);
                    }
                    return;
                }
                output[index * 2..index * 2 + 2].copy_from_slice(&packet.audio);
                self.consumed.store(
                    (packet.command & GENERATION_MASK) | packet.frame,
                    Ordering::SeqCst,
                );
                self.position.store(packet.frame, Ordering::Relaxed);
                if packet.audio.iter().any(|v| v.abs() > 0.000001) {
                    audible += 1;
                }
                break;
            }
        }
        if self.command.load(Ordering::SeqCst) != command {
            output.fill(0.0);
        } else {
            self.audible_frames.fetch_add(audible, Ordering::Relaxed);
        }
    }
}
pub struct Producer {
    pub engine: Engine,
    shared: Arc<Shared>,
    command: u64,
    end_queued: bool,
    mix_taken_revision: u64,
    mix_pending_ack: Option<u64>,
    meter_windows: Option<MeterWindows>,
    meter_started: bool,
    meter_emitted: u64,
}
impl Producer {
    pub fn new(song: &Song, base: Option<&Path>, shared: Arc<Shared>) -> Result<Self, String> {
        let engine = Engine::new(song, base)?;
        // Reuse one bounded immutable source clone for mix and telemetry. An
        // unsupported optional observation must never impose a playback limit.
        let meter_source =
            crate::project::check_serialized_size(song).map(|_| Arc::new(song.clone()));
        // A neutral legacy Song may exceed live-mix limits and must still play.
        // Only this optional control binding is unavailable in that case.
        let mix_worker = (|| {
            let layout = engine.mix_layout().ok_or_else(|| {
                engine
                    .mix_unavailable()
                    .unwrap_or("Live mix layout is unavailable")
                    .to_string()
            })?;
            Ok(WorkerMixBinding {
                song: meter_source.as_ref().map_err(Clone::clone)?.clone(),
                layout: layout.clone(),
            })
        })();
        shared
            .mix_worker
            .set(mix_worker)
            .map_err(|_| "A transport session supports exactly one producer".to_string())?;
        let _ = shared.meter_source.set(meter_source);
        let limit = engine.manifest.musical_frames - 1;
        shared.seek_limit.store(limit, Ordering::SeqCst);
        // A seek may arrive while samples are loading, before the limit is known.
        shared
            .command
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some((v & !FRAME_MASK) | (v & FRAME_MASK).min(limit))
            })
            .unwrap();
        Ok(Self {
            engine,
            shared,
            command: 0,
            end_queued: false,
            mix_taken_revision: 0,
            mix_pending_ack: None,
            meter_windows: None,
            meter_started: false,
            meter_emitted: 0,
        })
    }
    /// One bounded render step. Mix snapshot reclamation and cold-reset work stay
    /// on this worker; the audio callback remains an exact PCM-copy path.
    pub fn pump(&mut self) -> Result<(), String> {
        self.checked_pump_step().map(|_| ())
    }
    /// Refill before yielding the worker, even if events/short loops split audio
    /// into tiny blocks. Work is bounded to CAPACITY + 511 emitted frames, never
    /// an unbounded loop chasing a concurrently draining callback.
    pub fn pump_until_buffered(&mut self) -> Result<(), String> {
        let mut emitted = 0;
        while emitted < CAPACITY {
            let count = self.checked_pump_step()?;
            if count == 0 {
                break;
            }
            emitted += count;
        }
        Ok(())
    }
    fn discard_pending_mix(&mut self) {
        let _ = self.shared.mix_mailbox.pop();
        self.mix_pending_ack = None;
    }
    fn checked_pump_step(&mut self) -> Result<usize, String> {
        let result = self.pump_step();
        if result.is_err() {
            // Preserve the public Producer contract: errors are returned, while
            // the host/harness owns terminal transport shutdown. Live control is
            // nevertheless revoked and pending snapshots can never be applied.
            self.shared.revoke_live_mix();
            self.mix_pending_ack = None;
            self.disable_meter_observation(
                "Worker error discarded pending meter observations".into(),
            );
        }
        result
    }
    fn clear_meter_reports(&self) {
        for _ in 0..REPORT_CAPACITY {
            if self.shared.meter_queue.pop().is_none() {
                break;
            }
        }
    }
    fn disable_meter_observation(&mut self, reason: String) {
        let _ = self.shared.meter_disabled.set(reason);
        self.meter_windows = None;
        self.engine.disable_meters();
        self.clear_meter_reports();
    }
    fn prepare_meter_observation(&mut self) {
        if self.meter_started || self.shared.meter_disabled.get().is_some() {
            return;
        }
        let Some(binding) = self.shared.mix_owner.get() else {
            return;
        };
        if self.shared.mix_revoked.load(Ordering::SeqCst) {
            return;
        }
        let valid = self
            .shared
            .meter_source
            .get()
            .ok_or("Worker meter source is unavailable".to_string())
            .and_then(|source| source.as_ref().map_err(Clone::clone))
            .and_then(|source| crate::track_mix::same_playing_content(&binding.song, source))
            .and_then(|same| {
                if same {
                    Ok(())
                } else {
                    Err("Meter owner does not match the worker Song".into())
                }
            });
        if let Err(error) = valid {
            self.disable_meter_observation(error);
            return;
        }
        let layout = Arc::new(self.engine.enable_meters().clone());
        let windows = MeterWindows::new(&layout).and_then(|mut windows| {
            windows.start_at(self.meter_emitted)?;
            Ok(windows)
        });
        match windows {
            Ok(windows) => {
                self.meter_windows = Some(windows);
                self.meter_started = true;
                let _ = self.shared.meter_layout.set(layout);
            }
            Err(error) => self.disable_meter_observation(error.to_string()),
        }
    }
    fn observe_enqueued_block(&mut self, command: u64, start: u64, count: usize) {
        if count == 0 || self.shared.meter_disabled.get().is_some() {
            return;
        }
        // Linearization is after WHOLE enqueue. A command arriving later makes
        // every report stale to poll_meters; it never turns into current data.
        if self.shared.command.load(Ordering::SeqCst) & !READY != command
            || self.shared.mix_revoked.load(Ordering::SeqCst)
        {
            if let Some(windows) = &mut self.meter_windows {
                windows.reset();
            }
            return;
        }
        // Count even before owner binding: late observation must retain the
        // generation-local clock, including automatic loops before the bind.
        let Some(next) = self.meter_emitted.checked_add(count as u64) else {
            self.disable_meter_observation(
                "Meter emitted counter exhausted; audio continues".into(),
            );
            return;
        };
        self.meter_emitted = next;
        if self.meter_windows.is_none() {
            return;
        }
        let Some(block) = self.engine.meter_block() else {
            self.disable_meter_observation("Successful block has no meter observation".into());
            return;
        };
        if block.frames() != count || block.song_start() != start {
            self.disable_meter_observation(
                "Meter observation does not match the whole enqueued block".into(),
            );
            return;
        }
        let shared = &self.shared;
        let mut drop_overflow = false;
        let result = self.meter_windows.as_mut().unwrap().push_block(
            block,
            self.mix_taken_revision,
            |report| {
                if shared
                    .meter_queue
                    .force_push(MeterPacket { command, report })
                    .is_some()
                {
                    let dropped = shared.meter_dropped.load(Ordering::SeqCst);
                    if let Some(next) = dropped.checked_add(1) {
                        shared.meter_dropped.store(next, Ordering::SeqCst);
                    } else {
                        drop_overflow = true;
                    }
                }
            },
        );
        if let Err(error) = result {
            self.disable_meter_observation(error.to_string());
        } else if drop_overflow {
            self.disable_meter_observation(
                "Meter queue-drop counter exhausted; audio continues".into(),
            );
        }
    }
    fn take_live_mix(&mut self) -> Result<(), String> {
        if self.shared.check_live_mix_running().is_err() {
            self.discard_pending_mix();
            return Ok(());
        }
        // Exactly one pop, only after proving the next step can render audio.
        if let Some(update) = self.shared.mix_mailbox.pop() {
            if self.shared.check_live_mix_running().is_err() {
                self.mix_pending_ack = None;
                return Ok(());
            }
            if update.revision <= self.mix_taken_revision {
                return Ok(());
            }
            // Layout-bound core validation is repeated on the Engine; no caller
            // can manufacture a coefficient array for a different routing graph.
            self.engine.apply_mix_snapshot(&update.snapshot)?;
            self.mix_taken_revision = update.revision;
            self.mix_pending_ack = Some(update.revision);
        }
        Ok(())
    }
    fn acknowledge_live_mix(&mut self, command: u64, count: usize) {
        if count == 0 {
            return;
        }
        // Called only AFTER every produced frame was successfully enqueued.
        // A racing command must not acknowledge a block from an obsolete seek.
        // This is a historical observation linearized at matching-command/session
        // validation; a later Stop can retain this revision while unavailable.
        if self.shared.command.load(Ordering::SeqCst) & !READY != command
            || self.shared.check_live_mix_running().is_err()
        {
            return;
        }
        if let Some(revision) = self.mix_pending_ack.take() {
            self.shared
                .mix_applied_revision
                .store(revision, Ordering::SeqCst);
        }
    }
    fn enqueue_rendered_block(
        &mut self,
        command: u64,
        start: u64,
        data: &[f32],
        count: usize,
    ) -> Result<(), String> {
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
        self.acknowledge_live_mix(command, count);
        self.observe_enqueued_block(command, start, count);
        Ok(())
    }
    fn pump_step(&mut self) -> Result<usize, String> {
        let command = self.shared.command.load(Ordering::SeqCst) & !READY;
        if command & TERMINAL != 0 {
            self.meter_windows = None;
            self.engine.disable_meters();
            self.clear_meter_reports();
            self.discard_pending_mix();
            return Ok(0);
        }
        if self.shared.mix_revoked.load(Ordering::SeqCst) && self.meter_started {
            self.meter_windows = None;
            self.engine.disable_meters();
            self.clear_meter_reports();
        }
        if command != self.command {
            self.meter_emitted = 0;
            if self.command != 0 && (command & GENERATION_MASK) <= (self.command & GENERATION_MASK)
            {
                self.disable_meter_observation(
                    "Transport generation wrapped; telemetry disabled without stopping audio"
                        .into(),
                );
            }
            if let Some(windows) = &mut self.meter_windows {
                windows.reset();
            }
            self.clear_meter_reports();
            // Only this worker pushes; a callback may also pop. Bound cleanup even
            // when obsolete packets fill the queue before a rapid command change.
            for _ in 0..CAPACITY {
                if self.shared.queue.pop().is_none() {
                    break;
                }
            }
            self.engine
                .cold_seek((command & FRAME_MASK).min(self.engine.manifest.musical_frames - 1))?;
            self.command = command;
            self.end_queued = false;
            self.shared
                .mix_render_complete
                .store(false, Ordering::SeqCst);
        }
        if self.end_queued || self.shared.queue.len() > CAPACITY - 512 {
            return Ok(0);
        }
        let looping = command & LOOP != 0;
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
                self.shared
                    .mix_render_complete
                    .store(true, Ordering::SeqCst);
                self.discard_pending_mix();
                // Live meters deliberately omit the final incomplete (<50ms)
                // window, while already published full windows remain readable
                // only until the callback commits the natural terminal state.
                if let Some(windows) = &mut self.meter_windows {
                    windows.reset();
                }
                self.end_queued = true;
                self.shared.mark_ready(command);
                return Ok(1);
            }
        }
        self.prepare_meter_observation();
        self.take_live_mix()?;
        let start = self.engine.position;
        let mut data = [0.0f32; 1024];
        let count = self.engine.block(&mut data, end)?;
        self.enqueue_rendered_block(command, start, &data, count)?;
        if self.shared.queue.len() >= 1024 {
            self.shared.mark_ready(command);
        }
        Ok(count)
    }
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.shared.mix_revoked.store(true, Ordering::SeqCst);
        self.discard_pending_mix();
        self.clear_meter_reports();
    }
}
/// Convert consumed audio frame to immutable snapshot occurrence/row, not edit cursor.
pub fn location(song: &Song, frame: u64) -> (usize, u32) {
    // Search the evaluator's actual rounded boundaries. An inverse division
    // misclassifies exact half-frame ties (round(112.5) is 113, not 112).
    let per_row = RATE as f64 * 60.0 / (song.bpm * song.lines_per_beat as f64);
    if !per_row.is_finite() || per_row <= 0.0 || song.order.is_empty() {
        return (0, 0);
    }
    let mut total = 0u64;
    for occurrence in &song.order {
        let Some(pattern) = song.patterns.get(&occurrence.pattern) else {
            return (0, 0);
        };
        total = total.saturating_add(pattern.rows as u64);
    }
    if total == 0 {
        return (0, 0);
    }
    let (mut low, mut high) = (0, total);
    while low < high {
        let middle = low + (high - low) / 2;
        if (middle as f64 * per_row).round() as u64 <= frame {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let row = low.saturating_sub(1);
    let mut start = 0;
    for (index, occurrence) in song.order.iter().enumerate() {
        let rows = song.patterns[&occurrence.pattern].rows as u64;
        if row < start + rows {
            return (index, (row - start) as u32);
        }
        start += rows;
    }
    (0, 0)
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
            self.start_with_loop(song, base, false)
        }
        pub fn start_with_loop(
            &mut self,
            song: Song,
            base: Option<PathBuf>,
            looping: bool,
        ) -> Result<(), String> {
            if self.busy() {
                return Err("Transport is active or still stopping".into());
            }
            song.validate()?;
            if song.order.is_empty() {
                return Err("Song order is empty".into());
            }
            let shared = Arc::new(Shared::default());
            if looping {
                shared.set_loop(true, u64::MAX);
            }
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
                    state.finish_worker();
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
        let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".transport.lock");
        let _file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|e| {
                format!(
                    "Transport lease unavailable (another Leviathan instance or stale lease): {e}"
                )
            })?;
        let _lease = Lease(lock);
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
        producer.pump_until_buffered()?;
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
        while !state.quit.load(Ordering::SeqCst) && state.is_running() {
            producer.pump_until_buffered()?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_end_commit_cannot_finish_a_new_seek_or_loop() {
        for loop_change in [false, true] {
            let state = Shared::default();
            let old = state.command.load(Ordering::SeqCst);
            state.mark_ready(old);
            // The callback already popped and validated the old end marker.
            assert_eq!(state.command.load(Ordering::SeqCst), old | READY);
            if loop_change {
                state.set_loop(true, 100);
            } else {
                state.seek(75);
            }
            // Resume exactly at the old callback's terminal commit.
            assert!(!state.finish_naturally(old));
            assert!(state.is_running());
            assert!(!state.has_ended());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn delayed_worker_ready_cannot_publish_for_seek_stop_or_device_failure() {
        for transition in 0..3 {
            let state = Shared::default();
            let old = state.command.load(Ordering::SeqCst);
            match transition {
                0 => state.seek(75),
                1 => state.stop(),
                _ => state.device_failed(),
            }
            // Resume the worker after it rendered or queued an end marker.
            state.mark_ready(old);
            assert!(!state.is_ready());
            assert!(!state.finish_naturally(old));
            assert_eq!(state.is_running(), transition == 0);
            assert!(!state.has_ended());
        }
    }

    #[test]
    fn delayed_old_progress_cannot_overwrite_new_seek_position() {
        let state = Shared::default();
        let old = state.command.load(Ordering::SeqCst);
        state.seek(12345);
        // Resume the old callback after the UI has committed its new seek.
        state
            .consumed
            .store((old & GENERATION_MASK) | 42, Ordering::SeqCst);
        state.position.store(42, Ordering::Relaxed);
        assert_eq!(state.position(), 12345);
        state.stop();
        state
            .consumed
            .store((old & GENERATION_MASK) | 43, Ordering::SeqCst);
        assert_eq!(state.position(), 0);
    }

    #[test]
    fn stale_queue_cleanup_is_bounded_and_never_outputs_old_pcm() {
        let state = Shared::default();
        let old = state.command.load(Ordering::SeqCst);
        for frame in 0..CAPACITY {
            assert!(state
                .queue
                .push(Packet {
                    command: old,
                    frame: frame as u64,
                    audio: [0.5; 2],
                    end: false,
                })
                .is_ok());
        }
        state.seek(500);
        state.mark_ready(state.command.load(Ordering::SeqCst));
        let mut output = [1.0; 3];
        state.fill(&mut output);
        assert_eq!(output, [0.0; 3]);
        assert!(state.queue.is_empty());
        assert_eq!(state.position(), 500);
        assert_eq!(state.audible_frames.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn stale_diagnostic_flags_do_not_resurrect_a_terminal_session() {
        let state = Shared::default();
        state.stop();
        state.running.store(true, Ordering::SeqCst);
        state.ready.store(true, Ordering::SeqCst);
        state.seek(90);
        state.set_loop(true, 100);
        assert!(!state.is_running());
        assert!(!state.is_ready());
        assert_eq!(state.position(), 0);
        let mut output = [1.0; 9];
        state.fill(&mut output);
        assert_eq!(output, [0.0; 9]);
    }
}

#[cfg(test)]
mod live_mix_tests {
    use super::*;
    use crate::{editor::Editor, song::*, track_mix::TrackMix};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub(super) struct TestDocument {
        editor: Editor,
        dir: std::path::PathBuf,
    }
    impl std::ops::Deref for TestDocument {
        type Target = Editor;
        fn deref(&self) -> &Editor {
            &self.editor
        }
    }
    impl Drop for TestDocument {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    pub(super) fn session() -> (TestDocument, Arc<Shared>, Producer) {
        let dir = std::env::temp_dir().join(format!(
            "transport-mix-private-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("authored-tiny.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..64 {
            wav.write_sample(0.1f32).unwrap();
        }
        wav.finalize().unwrap();
        let mut editor = Editor::default();
        editor.song.order.truncate(1);
        for pattern in editor.song.patterns.values_mut() {
            pattern.tracks.clear();
            pattern.rows = 1;
        }
        for instrument in editor.song.instruments.values_mut() {
            instrument.sample = Some(path.clone());
        }
        editor
            .song
            .patterns
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
                    velocity: Some(0.1),
                    ..Default::default()
                },
            );
        let shared = Arc::new(Shared::default());
        shared
            .bind_live_mix(editor.document_owner(), &editor.song)
            .unwrap();
        let producer = Producer::new(&editor.song, None, shared.clone()).unwrap();
        (TestDocument { editor, dir }, shared, producer)
    }
    fn changed(editor: &Editor, gain: f32) -> Song {
        let mut song = editor.song.clone();
        song.track_mix.insert(
            TrackId(1),
            TrackMix {
                gain,
                ..Default::default()
            },
        );
        song
    }
    fn submit(editor: &Editor, shared: &Shared, gain: f32) -> u64 {
        shared
            .submit_live_mix(&editor.document_owner(), &changed(editor, gain))
            .unwrap()
    }
    pub(super) fn drain(shared: &Shared) {
        while shared.queue.pop().is_some() {}
    }

    #[test]
    fn checked_revision_exhaustion_preserves_earlier_pending_request() {
        let (editor, shared, _producer) = session();
        assert_eq!(submit(&editor, &shared, 0.5), 1);
        *shared.mix_submission_revision.lock().unwrap() = u64::MAX;
        let error = shared
            .submit_live_mix(&editor.document_owner(), &changed(&editor, 0.25))
            .unwrap_err();
        assert!(error.contains("exhausted"));
        assert_eq!(shared.live_mix_status().accepted_revision, 1);
        assert_eq!(shared.mix_mailbox.pop().unwrap().revision, 1);
    }

    #[test]
    fn stale_internal_revision_does_not_replace_the_worker_target() {
        let (editor, shared, mut producer) = session();
        submit(&editor, &shared, 0.5);
        submit(&editor, &shared, 0.25);
        producer.pump().unwrap();
        assert_eq!(shared.live_mix_status().applied_revision, 2);
        let snapshot = producer
            .engine
            .mix_layout()
            .unwrap()
            .snapshot(&changed(&editor, 0.75).track_mix)
            .unwrap();
        assert!(shared
            .mix_mailbox
            .push(MixUpdate {
                revision: 1,
                snapshot
            })
            .is_ok());
        producer.pump().unwrap();
        assert_eq!(producer.mix_taken_revision, 2);
        assert_eq!(shared.live_mix_status().applied_revision, 2);
    }

    #[test]
    fn wrong_layout_snapshot_fails_closed_without_acknowledgement() {
        let (editor, shared, mut producer) = session();
        let mut different = editor.song.clone();
        different.title.push_str(" different content");
        let other = Engine::new(&different, None).unwrap();
        let snapshot = other
            .mix_layout()
            .unwrap()
            .snapshot(&changed(&editor, 0.5).track_mix)
            .unwrap();
        assert!(shared
            .mix_mailbox
            .push(MixUpdate {
                revision: 1,
                snapshot
            })
            .is_ok());
        assert!(producer.pump().is_err());
        // Public Producer returns errors; its host/harness owns terminal stop.
        assert!(shared.is_running());
        assert!(!shared.live_mix_status().available);
        assert_eq!(shared.live_mix_status().applied_revision, 0);
        assert!(shared.mix_mailbox.is_empty());
        assert_eq!(producer.engine.position, 0);
    }

    #[test]
    fn zero_or_partially_enqueued_blocks_never_acknowledge() {
        let (editor, shared, mut producer) = session();
        submit(&editor, &shared, 0.5);
        producer.take_live_mix().unwrap();
        let command = shared.command.load(Ordering::SeqCst) & !READY;
        producer.enqueue_rendered_block(command, 0, &[], 0).unwrap();
        assert_eq!(shared.live_mix_status().applied_revision, 0);
        for frame in 0..CAPACITY - 1 {
            assert!(shared
                .queue
                .push(Packet {
                    command,
                    frame: frame as u64,
                    audio: [0.0; 2],
                    end: false,
                })
                .is_ok());
        }
        assert!(producer
            .enqueue_rendered_block(command, 0, &[0.1; 4], 2)
            .is_err());
        assert_eq!(shared.queue.len(), CAPACITY);
        assert_eq!(shared.live_mix_status().applied_revision, 0);
        assert_eq!(producer.mix_pending_ack, Some(1));
    }

    #[test]
    fn seek_or_loop_before_enqueue_validation_defers_ack_until_new_generation() {
        for looping in [false, true] {
            let (editor, shared, mut producer) = session();
            submit(&editor, &shared, 0.5);
            producer.take_live_mix().unwrap();
            let old = shared.command.load(Ordering::SeqCst) & !READY;
            if looping {
                shared.set_loop(true, producer.engine.manifest.musical_frames);
            } else {
                shared.seek(10);
            }
            producer
                .enqueue_rendered_block(old, 0, &[0.1; 2], 1)
                .unwrap();
            assert_eq!(shared.live_mix_status().applied_revision, 0);
            assert_eq!(producer.mix_pending_ack, Some(1));
            producer.pump().unwrap();
            assert_eq!(shared.live_mix_status().applied_revision, 1);
            assert_eq!(producer.mix_pending_ack, None);
        }
    }

    #[test]
    fn terminal_or_revocation_before_enqueue_validation_never_acknowledges() {
        for terminal in 0..4 {
            let (editor, shared, mut producer) = session();
            submit(&editor, &shared, 0.5);
            producer.take_live_mix().unwrap();
            let old = shared.command.load(Ordering::SeqCst) & !READY;
            match terminal {
                0 => shared.stop(),
                1 => shared.device_failed(),
                2 => {
                    shared.mark_ready(old);
                    assert!(shared.finish_naturally(old));
                }
                _ => shared.revoke_live_mix(),
            }
            producer
                .enqueue_rendered_block(old, 0, &[0.1; 2], 1)
                .unwrap();
            assert_eq!(shared.live_mix_status().applied_revision, 0);
            assert!(!shared.live_mix_status().available);
            producer.pump().unwrap();
            assert_eq!(shared.live_mix_status().applied_revision, 0);
            assert!(shared.mix_mailbox.is_empty());
        }
    }

    #[test]
    fn older_successful_ack_does_not_clear_newer_pending_request() {
        let (editor, shared, mut producer) = session();
        submit(&editor, &shared, 0.75);
        producer.take_live_mix().unwrap();
        assert_eq!(submit(&editor, &shared, 0.25), 2);
        let command = shared.command.load(Ordering::SeqCst) & !READY;
        producer
            .enqueue_rendered_block(command, 0, &[0.1; 2], 1)
            .unwrap();
        assert_eq!(shared.live_mix_status().applied_revision, 1);
        assert_eq!(shared.mix_mailbox.len(), 1);
        producer.take_live_mix().unwrap();
        assert_eq!(producer.mix_pending_ack, Some(2));
        producer
            .enqueue_rendered_block(command, 1, &[0.1; 2], 1)
            .unwrap();
        assert_eq!(shared.live_mix_status().applied_revision, 2);
        shared.stop();
        let status = shared.live_mix_status();
        assert_eq!(status.applied_revision, 2); // Historical enqueue only.
        assert!(!status.available);
    }

    #[test]
    fn end_marker_cannot_ack_a_pending_mix_and_stops_new_submissions() {
        let (editor, shared, mut producer) = session();
        producer.pump().unwrap();
        drain(&shared);
        submit(&editor, &shared, 0.5);
        producer.engine.position =
            producer.engine.manifest.musical_frames + producer.engine.manifest.tail_frames;
        producer.pump().unwrap();
        assert!(producer.end_queued);
        assert!(shared.mix_mailbox.is_empty());
        assert_eq!(shared.live_mix_status().applied_revision, 0);
        assert!(!shared.live_mix_status().available);
        assert!(shared
            .submit_live_mix(&editor.document_owner(), &changed(&editor, 0.25))
            .unwrap_err()
            .contains("already rendered"));
        shared.seek(0);
        producer.pump().unwrap();
        assert!(shared.live_mix_status().available);
        assert_eq!(submit(&editor, &shared, 0.25), 2);
    }

    #[test]
    fn pending_target_is_not_taken_when_pcm_queue_cannot_render() {
        let (editor, shared, mut producer) = session();
        producer.pump_until_buffered().unwrap();
        let position = producer.engine.position;
        submit(&editor, &shared, 0.5);
        producer.pump().unwrap();
        assert_eq!(producer.engine.position, position);
        assert_eq!(shared.mix_mailbox.len(), 1);
        assert_eq!(shared.live_mix_status().applied_revision, 0);
        assert_eq!(producer.mix_taken_revision, 0);
    }

    #[test]
    fn dropped_producer_revokes_and_discards_pending_control() {
        let (editor, shared, producer) = session();
        submit(&editor, &shared, 0.5);
        drop(producer);
        assert!(!shared.live_mix_status().available);
        assert!(shared.mix_mailbox.is_empty());
        assert!(shared
            .submit_live_mix(&editor.document_owner(), &changed(&editor, 0.25))
            .unwrap_err()
            .contains("revoked"));
    }
    #[test]
    fn clipping_error_revokes_mix_then_existing_host_finish_stops_shared() {
        let (editor, shared, mut producer) = session();
        submit(&editor, &shared, 0.5);
        // Authored fixture plus an explicit synthetic DSP fault, not device IO.
        producer.engine.manifest.master_gain = 10_000.0;
        let error = producer.pump().unwrap_err();
        assert!(error.contains("clipping"));
        assert!(shared.is_running()); // Preserve the public Producer contract.
        assert!(!shared.live_mix_status().available);
        assert_eq!(shared.live_mix_status().accepted_revision, 1);
        assert_eq!(shared.live_mix_status().applied_revision, 0);
        assert!(shared.mix_mailbox.is_empty());
        assert_eq!(producer.mix_pending_ack, None);
        assert!(shared
            .submit_live_mix(&editor.document_owner(), &changed(&editor, 0.25))
            .unwrap_err()
            .contains("revoked"));
        // The existing host wrapper's post-run action owns terminal shutdown.
        shared.finish_worker();
        assert!(!shared.is_running());
        assert!(!shared.is_ready());
        let mut output = [1.0; 32];
        shared.fill(&mut output);
        assert_eq!(output, [0.0; 32]);
        producer.pump().unwrap();
        assert_eq!(shared.live_mix_status().applied_revision, 0);
    }
}

#[cfg(test)]
mod meter_tests {
    use super::live_mix_tests::{drain, session};
    use super::*;
    use crate::{editor::Editor, song::TrackId, track_mix::TrackMix};

    fn advance(producer: &mut Producer, frames: u64) {
        while producer.engine.position < frames {
            drain(&producer.shared);
            producer.pump().unwrap();
        }
    }
    #[test]
    fn meter_reads_are_owner_content_and_session_bound_without_commands() {
        let (editor, shared, mut producer) = session();
        let command = shared.command.load(Ordering::SeqCst);
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        assert_eq!(shared.command.load(Ordering::SeqCst), command);
        assert_eq!(shared.live_mix_status().accepted_revision, 0);
        let another = Editor::default();
        assert!(shared
            .meter_reader(&another.document_owner(), &editor.song)
            .is_err());
        let mut changed = editor.song.clone();
        changed.title.push_str(" different");
        assert!(shared
            .meter_reader(&editor.document_owner(), &changed)
            .is_err());
        changed = editor.song.clone();
        changed.track_mix.insert(
            TrackId(1),
            TrackMix {
                gain: 0.5,
                ..Default::default()
            },
        );
        assert!(shared
            .meter_reader(&editor.document_owner(), &changed)
            .is_ok());
        let other_shared = Arc::new(Shared::default());
        other_shared
            .bind_live_mix(editor.document_owner(), &editor.song)
            .unwrap();
        assert!(other_shared.poll_meters(&reader).is_err());
        advance(&mut producer, 2560);
        let poll = shared.poll_meters(&reader).unwrap();
        assert_eq!(poll.reports.len(), 1);
        assert_eq!(poll.layout.unwrap().tracks.len(), 1);
        assert_eq!(shared.live_mix_status().accepted_revision, 0);
    }
    #[test]
    fn meter_only_whole_enqueued_frames_complete_exact_windows_before_callback_consumption() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 2048);
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        advance(&mut producer, 2560);
        let poll = shared.poll_meters(&reader).unwrap();
        let report = poll.reports[0];
        assert_eq!(
            (report.emitted_start, report.emitted_end_exclusive),
            (0, 2400)
        );
        assert_eq!(
            (
                report.first_song_frame,
                report.last_song_frame_end_exclusive
            ),
            (0, 2400)
        );
        assert_eq!(report.wrap_count, 0);
        assert!(!report.partial);
        assert!(report.master_peak.iter().all(|peak| *peak > 0.0));
        assert_eq!(report.track_peaks(), &[Some(report.master_peak)]);
        assert_eq!(shared.position(), 0); // Worker report is not consumption.
        assert_eq!(shared.callbacks.load(Ordering::Relaxed), 0);
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
    }
    #[test]
    fn meter_zero_partial_and_obsolete_enqueues_do_not_publish() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 2048);
        drain(&shared);
        let command = shared.command.load(Ordering::SeqCst) & !READY;
        producer
            .enqueue_rendered_block(command, producer.engine.position, &[], 0)
            .unwrap();
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        let start = producer.engine.position;
        let mut data = [0.0; 1024];
        let count = producer.engine.block(&mut data, 10_000).unwrap();
        for frame in 0..CAPACITY - 1 {
            shared
                .queue
                .push(Packet {
                    command,
                    frame: frame as u64,
                    audio: [0.0; 2],
                    end: false,
                })
                .ok()
                .unwrap();
        }
        assert!(producer
            .enqueue_rendered_block(command, start, &data, count)
            .is_err());
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        drain(&shared);
        shared.seek(100);
        producer
            .enqueue_rendered_block(command, start, &data, count)
            .unwrap();
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        advance(&mut producer, 3000);
        let poll = shared.poll_meters(&reader).unwrap();
        assert_eq!(poll.reports.len(), 1);
        assert_eq!(poll.reports[0].emitted_start, 0);
        assert_eq!(poll.reports[0].first_song_frame, 100);
    }
    #[test]
    fn meter_seek_invalidates_published_and_pending_windows_before_worker_runs() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 3000);
        let old_generation = shared.poll_meters(&reader).unwrap().generation;
        advance(&mut producer, 5000);
        shared.seek(111);
        let poll = shared.poll_meters(&reader).unwrap();
        assert!(poll.generation > old_generation);
        assert!(poll.reports.is_empty());
        // A delayed obsolete worker publication is also rejected by its tag.
        let old = producer.command;
        let mut report = None;
        producer
            .meter_windows
            .as_mut()
            .unwrap()
            .finish_partial(|value| report = Some(value))
            .unwrap();
        shared
            .meter_queue
            .push(MeterPacket {
                command: old,
                report: report.unwrap(),
            })
            .ok()
            .unwrap();
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        producer.pump().unwrap();
        advance(&mut producer, 3000);
        let report = shared.poll_meters(&reader).unwrap().reports[0];
        assert_eq!(report.emitted_start, 0);
        assert_eq!(report.first_song_frame, 111);
    }
    #[test]
    fn meter_racing_poll_keeps_new_generation_packets_and_rejects_old_ones() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 2560);
        shared.check_meter_reader(&reader).unwrap(); // Poll begins in old generation.
        let old = shared.meter_queue.pop().unwrap();
        shared.seek(111);
        producer.pump().unwrap();
        advance(&mut producer, 3000);
        let new = shared.meter_queue.pop().unwrap();
        let poll = shared
            .finish_meter_poll(&reader, [Some(old), Some(new), None, None])
            .unwrap();
        assert_eq!(poll.reports.len(), 1);
        assert_eq!(poll.reports[0].first_song_frame, 111);
        assert_eq!(
            poll.generation,
            (new.command & GENERATION_MASK) / GENERATION_STEP
        );
        shared.stop();
        assert!(shared
            .finish_meter_poll(&reader, [Some(new), None, None, None])
            .is_err());
    }
    #[test]
    fn meter_queue_is_bounded_and_reports_evictions_separately_from_ui_reads() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 14_400);
        assert_eq!(shared.meter_queue.len(), 4);
        let poll = shared.poll_meters(&reader).unwrap();
        assert_eq!(poll.reports.len(), 4);
        assert_eq!(poll.dropped_reports, 2);
        assert_eq!(
            poll.reports
                .iter()
                .map(|r| r.report_index)
                .collect::<Vec<_>>(),
            vec![3, 4, 5, 6]
        );
        let poll = shared.poll_meters(&reader).unwrap();
        assert_eq!(poll.dropped_reports, 2);
        assert!(poll.reports.is_empty());
    }
    #[test]
    fn meter_revision_span_uses_worker_target_not_just_latest_accepted() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 2048);
        let mut target = editor.song.clone();
        target.track_mix.insert(
            TrackId(1),
            TrackMix {
                gain: 0.5,
                ..Default::default()
            },
        );
        assert_eq!(
            shared
                .submit_live_mix(&editor.document_owner(), &target)
                .unwrap(),
            1
        );
        target.track_mix.get_mut(&TrackId(1)).unwrap().gain = 0.25;
        assert_eq!(
            shared
                .submit_live_mix(&editor.document_owner(), &target)
                .unwrap(),
            2
        );
        advance(&mut producer, 2560);
        let report = shared.poll_meters(&reader).unwrap().reports[0];
        assert_eq!(
            (report.first_mix_revision, report.last_mix_revision),
            (0, 2)
        );
        assert_eq!(shared.live_mix_status().applied_revision, 2);
    }
    #[test]
    fn meter_terminal_revoked_and_device_error_reads_clear_without_callback_work() {
        for action in 0..4 {
            let (editor, shared, mut producer) = session();
            let reader = shared
                .meter_reader(&editor.document_owner(), &editor.song)
                .unwrap();
            advance(&mut producer, 2560);
            match action {
                0 => shared.stop(),
                1 => shared.device_failed(),
                2 => shared.revoke_live_mix(),
                _ => {
                    shared.mark_ready(producer.command);
                    assert!(shared.finish_naturally(producer.command));
                }
            }
            assert!(shared.poll_meters(&reader).is_err());
            producer.pump().unwrap();
            assert!(shared.meter_queue.is_empty());
            assert!(producer.engine.meter_block().is_none());
        }
    }
    #[test]
    fn meter_counter_overflow_disables_only_telemetry_and_audio_keeps_advancing() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 9600);
        assert_eq!(shared.meter_queue.len(), 4);
        shared.meter_dropped.store(u64::MAX, Ordering::SeqCst);
        advance(&mut producer, 12_000);
        let poll = shared.poll_meters(&reader).unwrap();
        assert!(poll.disabled_reason.unwrap().contains("exhausted"));
        assert!(poll.reports.is_empty());
        assert!(shared.is_running());
        let before = producer.engine.position;
        drain(&shared);
        producer.pump().unwrap();
        assert!(producer.engine.position > before);
        assert!(producer.engine.meter_layout().is_none());
    }
    #[test]
    fn meter_command_generation_wrap_disables_only_telemetry() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 2560);
        producer.command = GENERATION_MASK;
        shared.command.store(GENERATION_MASK, Ordering::SeqCst);
        shared.seek(0);
        producer.pump().unwrap();
        assert!(shared
            .poll_meters(&reader)
            .unwrap()
            .disabled_reason
            .unwrap()
            .contains("generation wrapped"));
        assert!(shared.is_running());
        assert!(producer.engine.position > 0);
    }
    #[test]
    fn meter_seek_after_queued_natural_end_starts_fresh_windows_without_resurrecting_old_partial() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        while !producer.end_queued {
            drain(&shared);
            producer.pump().unwrap();
        }
        assert!(shared.is_running()); // End marker has not been consumed.
        shared.seek(321);
        let poll = shared.poll_meters(&reader).unwrap();
        assert!(poll.reports.is_empty());
        producer.pump().unwrap();
        advance(&mut producer, 3000);
        let report = shared.poll_meters(&reader).unwrap().reports[0];
        assert_eq!(report.observation_start_emitted, 0);
        assert_eq!(
            (
                report.emitted_start,
                report.first_song_frame,
                report.frames()
            ),
            (0, 321, 2400)
        );
        assert!(!report.partial);
    }
    #[test]
    fn meter_emitted_overflow_disables_observation_without_failing_render() {
        let (editor, shared, mut producer) = session();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 2560);
        producer.meter_emitted = u64::MAX;
        let before = producer.engine.position;
        drain(&shared);
        producer.pump().unwrap();
        assert!(producer.engine.position > before);
        assert!(shared.is_running());
        let poll = shared.poll_meters(&reader).unwrap();
        assert!(poll.reports.is_empty());
        assert!(poll
            .disabled_reason
            .unwrap()
            .contains("emitted counter exhausted"));
        producer.meter_emitted = 123;
        drain(&shared);
        producer.pump().unwrap();
        assert_eq!(producer.meter_emitted, 123); // Disabled means no clock retry.
    }
    #[test]
    fn meter_loading_reader_cannot_expose_mismatched_worker_content() {
        let (editor, _bound, _producer) = session();
        let shared = Arc::new(Shared::default());
        shared
            .bind_live_mix(editor.document_owner(), &editor.song)
            .unwrap();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        let mut different = editor.song.clone();
        different.title.push_str(" not the bound snapshot");
        let mut producer = Producer::new(&different, None, shared.clone()).unwrap();
        producer.pump_until_buffered().unwrap();
        let poll = shared.poll_meters(&reader).unwrap();
        assert!(poll.reports.is_empty());
        assert!(poll.layout.is_none());
        assert!(poll.disabled_reason.unwrap().contains("does not match"));
        assert!(shared.is_running());
        assert!(shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .is_err());
    }
    #[test]
    fn meter_late_binding_preserves_generation_clock_and_omits_incomplete_prefix() {
        let (editor, _bound, _producer) = session();
        let shared = Arc::new(Shared::default());
        let mut producer = Producer::new(&editor.song, None, shared.clone()).unwrap();
        producer.pump_until_buffered().unwrap();
        assert_eq!(producer.meter_emitted, 2048);
        assert!(producer.engine.meter_layout().is_none());
        shared
            .bind_live_mix(editor.document_owner(), &editor.song)
            .unwrap();
        let reader = shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .unwrap();
        advance(&mut producer, 4608);
        assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
        advance(&mut producer, 5120);
        let poll = shared.poll_meters(&reader).unwrap();
        assert_eq!(poll.reports.len(), 1);
        let report = poll.reports[0];
        assert_eq!(report.observation_start_emitted, 2048);
        assert_eq!(
            (
                report.report_index,
                report.emitted_start,
                report.emitted_end_exclusive
            ),
            (2, 2400, 4800)
        );
        assert_eq!(
            (
                report.first_song_frame,
                report.last_song_frame_end_exclusive
            ),
            (2400, 4800)
        );
        assert_eq!(poll.dropped_reports, 0); // Omitted prefix is not queue loss.
    }
    #[test]
    fn meter_loop_before_late_binding_uses_emitted_clock_not_wrapped_song_position() {
        let (editor, _bound, _producer) = session();
        let mut song = editor.song.clone();
        song.bpm = 400.0;
        song.lines_per_beat = 256;
        let shared = Arc::new(Shared::default());
        shared.set_loop(true, u64::MAX);
        let mut producer = Producer::new(&song, None, shared.clone()).unwrap();
        producer.pump_until_buffered().unwrap();
        let origin = producer.meter_emitted;
        let musical = producer.engine.manifest.musical_frames;
        // Short-loop priming reserves the next worst-case 512-frame block;
        // it need not fill all 2048 slots (28-frame blocks prime 1540 here).
        assert!(origin >= 1024);
        assert!(producer.engine.position <= musical);
        assert!(producer.engine.position < origin);
        shared
            .bind_live_mix(editor.document_owner(), &song)
            .unwrap();
        let reader = shared
            .meter_reader(&editor.document_owner(), &song)
            .unwrap();
        let first = origin.div_ceil(2400) * 2400;
        while producer.meter_emitted < first + 2400 {
            drain(&shared);
            producer.pump().unwrap();
        }
        let report = shared.poll_meters(&reader).unwrap().reports[0];
        assert_eq!(report.observation_start_emitted, origin);
        assert_eq!(
            (report.emitted_start, report.emitted_end_exclusive),
            (first, first + 2400)
        );
        assert_eq!(report.first_song_frame, first % musical);
        assert_eq!(
            report.last_song_frame_end_exclusive,
            (first + 2399) % musical + 1
        );
        assert_eq!(
            report.wrap_count,
            (first + 2399) / musical - first / musical
        );
    }
    #[test]
    fn meter_unbound_audition_never_enables_observation_or_exposes_data() {
        let (editor, _bound, _producer) = session();
        let shared = Arc::new(Shared::default());
        let mut producer = Producer::new(&editor.song, None, shared.clone()).unwrap();
        producer.pump_until_buffered().unwrap();
        assert!(producer.engine.meter_layout().is_none());
        assert!(shared
            .meter_reader(&editor.document_owner(), &editor.song)
            .is_err());
        assert!(shared.meter_queue.is_empty());
    }
}
