# Leviathan cloud handoff — 2026-10-24

> Historical handoff plus continuation notes. Current build semantics supersede
> the original `desktop` → `realtime` edge below: `desktop` is editor/offline-only;
> `desktop-audio` enables the full host application. See
> [CLOUD-VALIDATION.md](CLOUD-VALIDATION.md) for current cloud checks, actual EGL
> framebuffer evidence and outstanding OS-window/physical-audio acceptance limits.

User requested stopping local development for cost and continuing in cloud.
No next milestone was started. Current milestone 5 is IN PROGRESS, not complete.
No commands remain running from this agent; test/probe/native instances exited.
Existing user sessions and unsaved edits were not closed or altered.

## Exact checkpoint / immediate compile issue

The last verified revision passed 53 tests (`cargo test --locked --offline --all-features`),
Clippy with warnings denied, and host/native transport probes. After that, a small UI
polish script partially wrote src/desktop.rs and aborted on a text-match assertion
before writing src/tracker_ui.rs. CURRENT SOURCE DOES NOT COMPILE with desktop:
`no field playhead on type TrackerUi`, at desktop.rs approximately lines 601/616.
Complete the intended `pub playhead: Option<(usize,u32)>` field in TrackerUi (it derives
Default), and optional row highlighting, OR remove the newly added desktop playhead
assignment block. Do this in cloud. No source fix or further tests ran after stop request.
The intended device_failed helper and extra test in that interrupted script did NOT land.
The prior working target/debug binary remains local but is excluded from this archive.

## Milestone 5 implemented before interruption

- src/song_render.rs now exposes Engine shared by offline export and streaming.
  It preloads sample buffers once, emits <=512-frame chunks split at event boundaries,
  and rebuilds samplers on cold seek without rereading files. Offline demo WAV/manifest
  were compared byte-for-byte to milestone 4 and remained identical.
- src/transport.rs uses a worker to generate PCM incrementally, not a rendered-file
  preview. Bounded crossbeam ArrayQueue: 2048 stereo frames, initial ready threshold
  1024. Callback copies PCM/atomics only: no UI locks, file I/O or allocation.
  Generation-tagged commands discard obsolete buffers after seek/stop. Underflow
  emits silence, counts missing frames and stalls musical progress (no catch-up).
- CPAL opens only the default device with supported stereo float32 48 kHz and a
  supported fixed 512-frame stream buffer when available; no system settings changed.
  Device errors stop playback; a two-second no-callback watchdog reports failure.
- Project-local .transport.lock (create_new, removed by worker RAII) prevents competing
  Leviathan audio instances. A stale lock is refused, never automatically bypassed.
- Play/Stop/whole-song Loop/Seek to edit row; immutable start snapshot until restart.
  Natural end includes tail; loops restart at frame 0 without tail and kill old voices.
  Cold seek skips earlier notes (no voice reconstruction), but later notes have compiled
  instrument/attribute memory. Loop toggle also cold-seeks current consumed position.
- Follow shows a read-only playback-snapshot viewport without mutating editor cursor.
  Turn Follow off to edit. A separate editor-row playhead highlight was the interrupted
  polishing change. Current desktop partial edit also collapses audio tools initially
  and removes obsolete 'No live playback' wording from export controls.
- Close stops transport and waits asynchronously for worker/device cleanup, alongside
  existing Save/Discard/Cancel. Repeated starts wait until prior worker has finished.
- Added src/bin/transport_probe.rs and six tests in tests/transport.rs.

## Last verified results

53 tests passed: prior 47 plus six transport tests (streamed/offline PCM equality,
natural end, stop silence/repeated sessions, cold seek/latest command, deterministic
loops, queued-end invalidation/playhead, underflow silence/counting).
Clippy passed after changing chunks_exact_mut to as_chunks_mut; before partial UI edit.
Host probe (12-second bound, two ~1.6-second passes):
  pass 0: 128 callbacks, 65535 nonzero frames, 0 underrun frames, clean stop.
  pass 1: 148 callbacks, 75775 nonzero frames, 0 underrun frames, clean stop.
This proves nonzero PCM reached the device callback, NOT speaker audibility or measured
end-to-end latency. 2048 frames = 42.7 ms maximum software queue; not a latency claim.
Native --transport-smoke captured actual playing/follow UI and exited 0, lease removed.
Evidence included: m5-host-probe.log, m5-native-smoke.log, leviathan-transport-m5.png.

## Cloud setup / continuation

Cargo.toml/lock pin approved pattrns Git rev
8505bfe09111d4123a202af5222466f55021db9f and crates.io phonic 0.16.0.
New DIRECT deps are existing cached versions: crossbeam-queue=0.3.14 and optional
cpal=0.17.3. desktop now enables realtime. Dependency/license docs still need milestone
5 refresh (packages already existed transitively in DEPENDENCIES.tsv).
No caches, binaries, system files, secrets, audio files or source vendor checkouts included.

In cloud, use a fresh project-local CARGO_HOME and fetch only the pinned official sources:
  export CARGO_HOME="$PWD/.cargo-home"
  cargo fetch --locked
  cargo test --locked             # no desktop/host system dependency required
  cargo test --locked --all-features
  cargo clippy --locked --all-targets --all-features -- -D warnings
All-features needs the platform CPAL/GUI system libraries; report missing dependencies,
do not install/change local Aqua settings. Core tests generate samples in temp folders.
Regenerate demo (NEW output directory; create artifacts parent if needed):
  cargo run --locked --bin song_demo -- artifacts/new-song-demo
  python3 scripts/verify_song_demo.py artifacts/new-song-demo
Render saved song:
  cargo run --locked --bin render_song -- artifacts/new-song-demo/first-current.leviathan.json artifacts/new-render.wav
Reserve physical device/UI checks for a later essential Aqua integration pass.
Do not start milestone 6 before finishing/reporting milestone 5. Refresh roadmap/README
and inventory, inspect race/stop/seek/error behavior, finish missing UI edit, rerun tests.

## Cloud continuation result (supersedes immediate compile issue above)

The missing playhead field is now present; tracker highlights the playback row.
Seek/highlight require full document-snapshot equality. Added device-failure helper
and regression, and corrected/tested fractional row-boundary mapping. Core tests:
53 passed; core Clippy and format checks passed; regenerated demo independently
verified. See roadmap for paths and exact all-feature blocker (`jack.pc` missing).
No user computer calls or global changes. Milestone 5 remains verification-pending.

Cloud command environment (project root):
  export CARGO_HOME="$PWD/.cargo-home" RUSTUP_HOME="$PWD/.toolchain/rustup"
  export PATH="$PWD/.cargo-home/bin:$PATH"
The official stable Rust installed here is 1.99.1. Do not include `.toolchain`,
`.cargo-home`, `.system-deps` or `target` when packaging the source checkpoint.
