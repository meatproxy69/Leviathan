//! Explicitly authorized, bounded, output-only diagnostic. Not an app entrypoint.
//! No input device, GUI, volume/settings change, user media, or automatic retry.
#[cfg(not(feature = "realtime"))]
fn main() {
    eprintln!("No audio opened: this diagnostic requires an explicit realtime build and --authorized-output-check NEW_ABSOLUTE_DIRECTORY");
    std::process::exit(2);
}

#[cfg(feature = "realtime")]
fn main() {
    if let Err(error) = diagnostic::main() {
        eprintln!("host_track_mix_probe: {error}");
        std::process::exit(1);
    }
}

#[cfg(feature = "realtime")]
mod diagnostic {
    use aqua_pattern_core::{
        editor::Editor,
        sample_asset,
        song::*,
        track_mix::{self, TrackMix, TrackMixConfig},
        transport::{host::Transport, Shared},
        RATE,
    };
    use serde::Serialize;
    use serde_json::{json, Value};
    use std::{
        fs::{self, OpenOptions},
        path::{Path, PathBuf},
        sync::{atomic::Ordering, Arc},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const PASSES: usize = 2;
    const PASS_MS: u64 = 1800;
    const CLEANUP_MS: u64 = 2000;
    const SOURCE_FRAMES: u64 = 96_000;
    const MAX_OBSERVATIONS: usize = 1024;

    #[derive(Clone, Debug, Serialize)]
    struct Observation {
        elapsed_us: u64,
        accepted: u64,
        enqueued: u64,
        available: bool,
        consumed_position: u64,
        callbacks: u64,
        above_threshold_frames: u64,
        software_missing_frames: u64,
        running: bool,
        ready: bool,
        ended: bool,
        device_error: bool,
    }
    fn observe(state: &Shared, start: Instant) -> Observation {
        let mix = state.live_mix_status();
        Observation {
            elapsed_us: start.elapsed().as_micros() as u64,
            accepted: mix.accepted_revision,
            enqueued: mix.applied_revision,
            available: mix.available,
            consumed_position: state.position(),
            callbacks: state.callbacks.load(Ordering::Relaxed),
            above_threshold_frames: state.audible_frames.load(Ordering::Relaxed),
            software_missing_frames: state.underruns.load(Ordering::Relaxed),
            running: state.is_running(),
            ready: state.is_ready(),
            ended: state.has_ended(),
            device_error: state.device_error.load(Ordering::Relaxed),
        }
    }
    fn json_new(path: &Path, value: &Value) -> Result<(), String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("Create {}: {e}", path.display()))?;
        serde_json::to_writer_pretty(file, value).map_err(|e| e.to_string())
    }
    fn absent(path: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(format!("Inspect {}: {e}", path.display())),
        }
    }
    fn triangle(phase: u64) -> f32 {
        let phase = (phase % 256) as i32;
        (if phase < 128 { phase - 64 } else { 192 - phase }) as f32
    }
    fn write_source(path: &Path) -> Result<Value, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let mut wav = hound::WavWriter::new(
            file,
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .map_err(|e| e.to_string())?;
        let mut peaks = [0.0f32; 2];
        for frame in 0..SOURCE_FRAMES {
            // Binary-exact base/envelope arithmetic followed by one f32 ×0.1.
            // The authored signal is zero from frame 73,728 (1.536 s) onward.
            let envelope = ((frame.min(256) as f32) / 256.0)
                .min((73_728u64.saturating_sub(frame).min(4096) as f32) / 4096.0);
            let unattenuated = [triangle(frame) / 512.0, triangle(frame + 64) / 1024.0];
            for channel in 0..2 {
                let quiet = (unattenuated[channel] * envelope) * 0.1f32;
                peaks[channel] = peaks[channel].max(quiet.abs());
                wav.write_sample(quiet).map_err(|e| e.to_string())?;
            }
        }
        wav.finalize().map_err(|e| e.to_string())?;
        let imported = sample_asset::import(path)?;
        Ok(json!({
            "filename":"authored-quiet.wav", "sha256": imported.asset.sha256,
            "frames": imported.asset.frames, "sample_rate": imported.asset.sample_rate,
            "channels": imported.asset.channels, "sample_format":"IEEE float32",
            "waveform":"integer_triangle_quiet_v1", "attenuation_f32":0.1f32,
            "attenuation_db_nominal":-20, "peaks":peaks,
            "base_peak_bounds":[0.125,0.0625], "zero_from_frame":73728,
            "derivation":"q(p)=p-64 for p<128, else192-p; p=frame mod256. L=q(frame)/512; R=q(frame+64)/1024; envelope=min(min(frame,256)/256,min(saturating_sub(73728,frame),4096)/4096); sample=f32(f32(base*envelope)*f32(0.1)).",
            "provenance":"Authored here; no private audio or recording. External verifier must independently reconstruct every f32 sample and hash this WAV."
        }))
    }
    fn song(path: &Path) -> Song {
        let mut song = example_song();
        song.title = "Explicit quiet host track-mix diagnostic".into();
        song.bpm = 120.0;
        song.lines_per_beat = 4;
        song.tracks.retain(|id, _| *id == TrackId(1));
        song.track_order = vec![TrackId(1)];
        song.instruments.retain(|id, _| *id == InstrumentId(1));
        song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(path.to_owned());
        song.patterns.retain(|id, _| *id == PatternId(1));
        let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
        pattern.rows = 16; // Two seconds musical span, unchanged existing tail.
        pattern.tracks.clear();
        pattern
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
                    velocity: Some(0.2),
                    ..Default::default()
                },
            );
        song.order.truncate(1);
        song.track_mix.clear();
        song
    }
    fn target(song: &Song, index: usize) -> Song {
        let mut target = song.clone();
        target.track_mix.insert(
            TrackId(1),
            TrackMix {
                gain: if index == 0 { 0.5 } else { 0.25 },
                balance: if index == 0 { -0.5 } else { 0.5 },
                mute: false,
                solo: false,
            },
        );
        target
    }
    fn delta(before: &Observation, after: &Observation) -> Value {
        json!({
            "elapsed_us":after.elapsed_us.saturating_sub(before.elapsed_us),
            "consumed_frames":after.consumed_position.saturating_sub(before.consumed_position),
            "callbacks":after.callbacks.saturating_sub(before.callbacks),
            "above_threshold_frames":after.above_threshold_frames.saturating_sub(before.above_threshold_frames),
            "software_missing_frames":after.software_missing_frames.saturating_sub(before.software_missing_frames),
        })
    }
    fn run_pass(pass: usize, original: &Song, lease: &Path) -> Result<Value, String> {
        if !absent(lease)? {
            return Err("Transport lease exists; no output started and no lease removed".into());
        }
        let mut editor = Editor::default();
        editor.replace_document(original.clone())?;
        let owner = editor.document_owner();
        let model = track_mix::preview(original, TrackMixConfig::from_song(original))?;
        let layout = model
            .layout()
            .ok_or("Model-derived diagnostic layout unavailable")?;
        let model_manifest = aqua_pattern_core::song_render::evaluate(original, None)?;
        let targets = [target(original, 0), target(original, 1)];
        let target_hashes = [
            track_mix::content_sha256(&targets[0])?,
            track_mix::content_sha256(&targets[1])?,
        ];
        let mut player = Transport::default();
        let start = Instant::now(); // Loading consumes the unconditional Play budget.
        let start_result = player.start(original.clone(), None);
        let mut failures = Vec::<String>::new();
        let mut observations = Vec::<Observation>::new();
        let mut updates = Vec::<Value>::new();
        let mut returned = Vec::<u64>::new();
        let mut enqueued = [false; 2];
        let shared: Option<Arc<Shared>> = player.shared.clone();
        if let Err(error) = start_result {
            failures.push(format!("Start: {error}"));
        }
        if let Some(state) = &shared {
            if let Err(error) = state.bind_live_mix(owner.clone(), original) {
                failures.push(format!("Owner/content binding: {error}"));
            }
            while failures.is_empty() && start.elapsed() < Duration::from_millis(PASS_MS) {
                player.poll();
                if let Some(error) = &player.error {
                    failures.push(format!("Host: {error}"));
                    break;
                }
                let now = observe(state, start);
                if now.elapsed_us >= PASS_MS * 1000 {
                    break;
                }
                if now.enqueued > now.accepted {
                    failures.push("Enqueued revision exceeded accepted revision".into());
                }
                if !now.running || now.device_error {
                    failures.push("Transport became terminal before requested Stop".into());
                }
                if let Some(previous) = observations.last() {
                    if now.accepted < previous.accepted
                        || now.enqueued < previous.enqueued
                        || now.consumed_position < previous.consumed_position
                        || now.callbacks < previous.callbacks
                    {
                        failures.push(
                            "Revision/position/callback monotonicity failed before Stop".into(),
                        );
                    }
                }
                for index in 0..returned.len() {
                    if !enqueued[index] && now.enqueued == returned[index] {
                        updates[index]["first_enqueued_observation"] = json!(now);
                        enqueued[index] = true;
                    }
                }
                if observations.len() >= MAX_OBSERVATIONS {
                    failures.push("Bounded observation budget exhausted".into());
                    break;
                }
                observations.push(now.clone());
                if !failures.is_empty() {
                    break;
                }
                let index = returned.len();
                if index < 2
                    && now.available
                    && now.consumed_position >= [12_000, 36_000][index]
                    && (index == 0 || enqueued[0])
                {
                    let changed = &targets[index];
                    let content_ok = player.snapshot.as_ref().is_some_and(|snapshot| {
                        track_mix::same_playing_content(snapshot, changed).unwrap_or(false)
                    });
                    if !content_ok || !editor.owns_document(&owner) {
                        failures.push("Owner/exact playing content changed".into());
                        break;
                    }
                    let before = observe(state, start);
                    if before.elapsed_us >= PASS_MS * 1000 {
                        break;
                    }
                    let submit = state.submit_live_mix(&owner, changed);
                    let after = observe(state, start);
                    if after.elapsed_us >= PASS_MS * 1000 {
                        failures.push(
                            "Mix submission observation exceeded the unconditional Play budget"
                                .into(),
                        );
                    }
                    match submit {
                        Ok(revision) => {
                            updates.push(json!({"full_settings":changed.track_mix,
                                "owner_is_current":true, "exact_playing_content_matches":true,
                                "content_sha256":target_hashes[index],
                                "model_layout_id":layout.id(), "returned_accepted_revision":revision,
                                "before_submit":before,"after_submit":after,"submit_observation_delta":delta(&before,&after),
                                "first_enqueued_observation":if after.enqueued==revision {json!(after)} else {Value::Null}}));
                            enqueued[index] = after.enqueued == revision;
                            returned.push(revision);
                            if revision != index as u64 + 1
                                || after.accepted != revision
                                || after.enqueued > revision
                            {
                                failures.push("Expected exactly accepted revisions1 then2 with enqueued≤accepted".into());
                            }
                        }
                        Err(error) => {
                            updates.push(json!({"full_settings":changed.track_mix,
                                "owner_is_current":true,"exact_playing_content_matches":true,
                                "content_sha256":target_hashes[index],"model_layout_id":layout.id(),
                                "returned_accepted_revision":Value::Null,"submit_error":error,
                                "before_submit":before,"after_submit":after,
                                "submit_observation_delta":delta(&before,&after),
                                "first_enqueued_observation":Value::Null}));
                            failures.push(format!("Mix request{} refused: {error}", index + 1));
                            break;
                        }
                    }
                }
                let remaining = Duration::from_millis(PASS_MS).saturating_sub(start.elapsed());
                std::thread::sleep(Duration::from_millis(5).min(remaining));
            }
        } else if failures.is_empty() {
            failures.push("Start returned no Shared state".into());
        }
        let stop_requested_us = start.elapsed().as_micros() as u64;
        player.stop(); // Unconditional, even if milestones or owner checks failed.
        let cleanup_start = Instant::now();
        while player.busy() && cleanup_start.elapsed() < Duration::from_millis(CLEANUP_MS) {
            player.poll();
            if player.busy() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        player.poll();
        let cleanup_elapsed_us = cleanup_start.elapsed().as_micros() as u64;
        if cleanup_elapsed_us > CLEANUP_MS * 1000 {
            failures.push("Cleanup observation exceeded its2s refusal threshold; preserve evidence without retry".into());
        }
        let final_observation = shared.as_ref().map(|state| observe(state, start));
        let lease_absent = match absent(lease) {
            Ok(value) => value,
            Err(error) => {
                failures.push(error);
                false
            }
        };
        if returned != [1, 2] || enqueued != [true, true] {
            failures.push(
                "INCOMPLETE: both accepted/enqueued milestones were not observed before Stop"
                    .into(),
            );
        }
        if player.busy() || !lease_absent {
            failures.push("Cleanup incomplete: worker or lease remains; do not retry or remove lease automatically".into());
        }
        if let Some(error) = &player.error {
            failures.push(format!("Final host error: {error}"));
        }
        if let Some(final_state) = &final_observation {
            if final_state.callbacks == 0 || final_state.above_threshold_frames == 0 {
                failures.push("No advancing callback/nonzero above-threshold PCM evidence".into());
            }
            if final_state.software_missing_frames != 0 {
                failures.push(
                    "Nonzero software missing-frame count; not a hardware-xrun measurement".into(),
                );
            }
            if final_state.running
                || final_state.available
                || final_state.enqueued != 2
                || final_state.accepted != 2
            {
                failures.push(
                    "Final stopped/live-revision state did not meet the diagnostic contract".into(),
                );
            }
        }
        if stop_requested_us > 2_000_000 {
            failures.push("Stop observation exceeded2s; no deadline guarantee or retry".into());
        }
        Ok(
            json!({"pass":pass,"complete":failures.is_empty(),"failures":failures,
            "play_requested_elapsed_us":0,"stop_requested_elapsed_us":stop_requested_us,
            "cleanup_elapsed_us":cleanup_elapsed_us,
            "final_busy":player.busy(),"final_host_error":player.error.as_deref(),"lease_absent":lease_absent,
            "owner_identity":"Fresh opaque Editor owner for this pass; numeric/raw identity is not exposed",
            "content_sha256":layout.content_sha256(),"model_layout_id":layout.id(),
            "playback_scalars":{"configured_note_velocity":0.2,"metadata_evaluator_master_gain":model_manifest.master_gain,
                "scope":"Separate from source ×0.1 attenuation; not hardware amplitude or listening measurement"},
            "layout_identity_scope":"Pure model preview only; worker validates internally but public host API does not expose its layout ID",
            "command_generation":"Not exposed; harness issues no seek or loop commands. Monotonic pre-Stop position is observed, not a direct generation measurement",
            "updates":updates,"observations":observations,"final_observation":final_observation}),
        )
    }
    pub fn main() -> Result<(), String> {
        let args: Vec<_> = std::env::args_os().collect();
        if args.len() != 3 || args[1] != "--authorized-output-check" {
            return Err("No audio opened. Usage: host_track_mix_probe --authorized-output-check NEW_ABSOLUTE_DIRECTORY".into());
        }
        let output = PathBuf::from(&args[2]);
        if !output.is_absolute() {
            return Err("Output directory must be absolute; no audio opened".into());
        }
        fs::create_dir(&output).map_err(|e| format!("Output must be a NEW directory: {e}"))?;
        let output = output.canonicalize().map_err(|e| e.to_string())?;
        let lease = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".transport.lock");
        json_new(
            &output.join("started.json"),
            &json!({"complete":false,
            "started_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e|e.to_string())?.as_millis(),
            "executable":std::env::current_exe().map_err(|e|e.to_string())?,
            "arguments":args.iter().map(|s|s.to_string_lossy().into_owned()).collect::<Vec<_>>(),"candidate_manifest_dir":env!("CARGO_MANIFEST_DIR"),
            "note":"If no final report exists, this run is INCOMPLETE/FAIL. A watchdog exit is not success."}),
        )?;
        let source_path = output.join("authored-quiet.wav");
        let result = (|| -> Result<Value, String> {
            if !absent(&lease)? {
                return Err(
                    "Pre-existing lease: no output started; do not delete or retry automatically"
                        .into(),
                );
            }
            let source = write_source(&source_path)?;
            let song = song(&source_path);
            song.validate()?;
            json_new(
                &output.join("authored-song.json"),
                &serde_json::to_value(&song).map_err(|e| e.to_string())?,
            )?;
            let mut passes = Vec::new();
            for pass in 1..=PASSES {
                let report = match run_pass(pass, &song, &lease) {
                    Ok(report) => report,
                    Err(error) => json!({"pass":pass,"complete":false,"pre_start_failure":error}),
                };
                let passed = report["complete"] == true;
                passes.push(report);
                if !passed {
                    break;
                } // Never automatically repeat a failed pass.
            }
            let after = sample_asset::import(&source_path);
            let source_unchanged = after
                .as_ref()
                .is_ok_and(|after| source["sha256"] == after.asset.sha256);
            let source_final_error = after.err();
            let complete = passes.len() == PASSES
                && passes.iter().all(|p| p["complete"] == true)
                && source_unchanged;
            Ok(
                json!({"diagnostic_schema":1,"complete":complete,"source":source,"source_unchanged":source_unchanged,"source_final_error":source_final_error,
                "passes":passes,"limits":{"passes_max":PASSES,"play_stop_target_ms":PASS_MS,"new_submission_deadline_ms":PASS_MS,
                    "observed_stop_refusal_ms":2000,"cleanup_budget_ms":CLEANUP_MS,
                "outer_watchdog_seconds":12,"max_poll_observations_per_pass":MAX_OBSERVATIONS},
                "output_contract":{"enforced_by_existing_host_source":{"sample_rate":48000,"channels":2,"sample_format":"float32"},
                    "actual_hardware_format_device":"Not independently exposed/observed by this public API diagnostic"},
                "scope":"Output only. No microphone, recording, GUI, system-volume/settings changes, installs, user audio, automatic pass retries or stale-lease removal.",
                "interpretation":"Accepted and worker-enqueued revisions are distinct. Enqueued acknowledges a nonempty rendered block wholly queued, not ramp completion, callback consumption or hearing. Callback counters and PCM threshold are not speaker audibility. Software missing frames are not hardware xruns.2048-frame queue capacity is not end-to-end latency. The1.8s monotonic Play budget is the Stop target and new-submission deadline, not a measured hard bound: scheduler overshoot is reported and observed Stop requests beyond2.0s fail. Durations are diagnostic chronology, not latency/deadline guarantees."}),
            )
        })();
        let report = match result {
            Ok(report) => report,
            Err(error) => json!({"complete":false,"failure":error}),
        };
        json_new(&output.join("report.json"), &report)?;
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|e| e.to_string())?
        );
        if report["complete"] != true {
            return Err(
                "Diagnostic FAILED/INCOMPLETE; preserve evidence, do not retry automatically"
                    .into(),
            );
        }
        Ok(())
    }
}
