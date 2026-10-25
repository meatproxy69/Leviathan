# Alpha 1 validation

Verified on Linux x86_64 with Rust/Cargo 1.99.1:

- 139 all-feature tests passed, including the runtime-lease collision/cleanup regression.
- 135 headless tests passed.
- All-target/all-feature Clippy with warnings denied and formatting passed.
- Optimized all-feature/all-bin build passed.
- The staged source tree independently passed the same 139 all-feature tests.
- Native release binary opened/exported outside the checkout and exited normally.
- Repeated, CLI and native exports were byte-identical for a locally generated
  synthetic integration fixture; the fixture/audio are not included in the release.
- Two bounded host playback/seek/loop/stop passes reported zero underruns and clean
  shutdown. The new user-runtime lease cleaned up correctly.
- The earlier all-ten-milestone workflow audit covers editing, phrases/FX, aliases,
  mutes/sections, independent variation, generation, groove/transpose, variable
  pattern lengths, history/save/open, rendering, snapshot streaming, loops/seek/Stop.

This is not exhaustive manual widget/platform testing. Speaker audibility, hardware
latency and power-loss/disk-full fault injection were not measured. The Linux
binary's newest referenced glibc symbol is GLIBC_2.43; it was run on glibc 2.44.
Source archives allow building against another supported system's libraries.

Release packaging uses an explicit allowlist. No audio files, sample-backed
projects, private sample builder, recordings, screenshots of local sample paths,
local MCP/agent configuration, credentials, caches, build trees or internal handoff
archives are included. Synthetic fixture generation code remains available in
source and the optional song_demo utility, so tests/examples are reproducible.
