# Leviathan stack audit

Canonical project: `/home/raine/Leviathan`. Audit scope is the chosen phonic +
pattrns engine and the requested native shell, not every repository from earlier
research. No global packages, services, security settings, accounts or plugin
folders were changed/scanned. Original workspace copy remains a fallback and has
not been updated with this new shell.

Current cloud checkpoint (2026-10-24): **89 core / 98 desktop tests passed**;
formatting, all-target capture-build Clippy, native editor binary build and six
real EGL/Glow framebuffer captures passed on unchanged hashed source. Physical
audio, OS-window/input integration and full-app Follow remain unverified.
Reproduction/evidence: [CLOUD-VALIDATION.md](CLOUD-VALIDATION.md).

| Component | Actual state | Evidence / limitation |
|---|---|---|
| phonic 0.16.0 | Integrated | Sampler, note queues, stereo offline source rendering; AGPL-3.0 |
| pattrns 0.9.3 | Integrated | Approved Git revision 8505bfe09111d4123a202af5222466f55021db9f; player and scripting disabled; AGPL-3.0 |
| Typed event bridge | Implemented/tested | Pitches, rests, OFF, retrigger, chords, delay, timestamps, queue errors |
| WAV writer, hound 3.5.1 | Integrated | Synchronous finalized output avoids reproduced phonic WAV stop deadlock |
| serde / serde_json | Integrated | Deterministic event manifests |
| CPAL 0.17.3 | Optional direct transport dependency | Enabled by `realtime` or `desktop-audio`, not `desktop`; prior host callback/clean-close probe passed; current physical host checks blocked; no measured latency |
| Symphonia / rubato | phonic transitive dependencies | Audio decoding/resampling are available through phonic; generated WAV fixtures tested, not a complete codec/sample-rate matrix |
| egui / eframe 0.33.3 | Integrated native shell | Editable song tracker, order/matrix selection, guarded keyboard editing; legacy motif sample/export tools remain separate |
| image 0.25.10 | Desktop-only support dependency | PNG capture of the app's own rendered framebuffer for visual QA |
| Background job controller | Implemented/tested | One load/export at a time, error recovery, no-clobber output, worker panic propagation; close waits for job completion |
| Original musical demo | Implemented/tested | Five generated parts and deterministic WAV stems/mix |
| Full-pattern real-time transport | Implemented; current host verification pending | Immutable song snapshot, bounded PCM queue, Play/Stop/Loop/Seek/Follow with `desktop-audio`; explicitly unavailable in editor-only `desktop` builds |
| MIDI I/O, VST/CLAP hosting | Missing / future scope | No plugin support claimed; no plugins discovered or loaded |
| AFEC / sample discovery | Not installed; future component | Not required to run the selected engine or this shell |
| Pattern editing and document rendering | Implemented/tested | Saved song/order to routed offline audio; note/OFF, independent attributes, repeat/clone, cross-boundary voice state and snapshot export |
| Routing/mixer UI, automation | Missing | No effect or routing controls claimed |
| Project persistence and undo/redo | Implemented/tested | Validated v1 documents, atomic saves with confirmation, missing-asset errors, guarded transitions and 64-entry history |
| AI orchestration, recording | Missing | This is a functional desktop foundation, not a complete DAW |

All chosen engine dependencies have been obtained. The broader DAW capability list
is not complete. `DEPENDENCIES.tsv` inventories all **422 locked package
identities** (including the local crate), with license declarations/previous
verification notes. Optional and foreign-target packages are not all compiled;
a package appearing there is not evidence of a finished product feature.

### Dependency inventory refresh — 2026-10-24

The optional experimental onset-v2 implementation adds the following packages;
v1 remains the default detector. Versions/licenses below were read from exact
locked offline Cargo metadata, not inferred from repository descriptions:

| Package | Version | Declared license |
|---|---|---|
| realfft | 3.5.0 | MIT |
| rustfft | 6.4.1 | MIT OR Apache-2.0 |
| primal-check | 0.3.4 | MIT OR Apache-2.0 |
| strength_reduce | 0.2.4 | MIT OR Apache-2.0 |
| transpose | 0.2.3 | MIT OR Apache-2.0 |

`num-complex 0.4.6` (MIT OR Apache-2.0) was already present. Inventory count
increased from 417 to 422; all prior license fields, including the independently
verified pattrns license note, were preserved. Name/version/source identities
match `Cargo.lock` exactly, with no missing or obsolete entries.

Reproduction command (no build or download):

```sh
cargo metadata --locked --offline --all-features \
  --filter-platform x86_64-unknown-linux-gnu --format-version 1
```

This freshly inspected **266 Linux all-feature packages**, including all five
additions. The remaining **156 foreign-target entries** retain prior inventory
license fields; their licenses were not freshly revalidated. Unfiltered offline
metadata was unavailable because `android-activity 0.6.1` is not cached; no
network fallback or package download was performed. Full locked identity
coverage was separately checked against every `Cargo.lock` package. See
`evidence/onset-v2-integration/dependency-inventory.json` for hashes and scope.


## Validation — historical initial shell stage

The following 13-test/native results predate milestones 2–5. They do not validate
the current cloud source. See [CLOUD-VALIDATION.md](CLOUD-VALIDATION.md) for the
current feature matrix, reproducible checks and explicit host/OS limitations.

- `cargo test --locked --offline --all-features`: 13 passed, 0 failed (6 bridge,
  5 core, 2 worker-controller tests).
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`: passed.
- Native Aqua desktop smoke: app launches, background-loads the generated sample,
  background-exports through the same UI handlers/controller, captures its own
  framebuffer and closes normally, bounded at 25 seconds. Captured status confirms
  216,000 frames and peak 0.173. WAV/JSON match verified CLI artifacts byte-for-byte.
- Actual screenshots inspected; compact-height layout revised and recaptured as
  `evidence/leviathan-shell-final.png`. Event rows are visible at Aqua's desktop size.
- This is automated app-level interaction, not a manual mouse/keyboard test of every
  widget. Path loading tested; OS drag-and-drop and close-during-job behavior are
  implemented but not separately exercised by the native smoke test.

Milestone 2 update: 25 tests pass; Clippy clean. See `SEQUENCER-ROADMAP.md` for current evidence and limits. Validation above records the earlier shell stage.

Milestone 3 update: 36 tests pass; Clippy and formatting checks pass. Native save/open roundtrip verified and final UI inspected. Current details: `SEQUENCER-ROADMAP.md`; transcript: `evidence/m3-verification.log`. No song rendering yet.

Historical milestone 4 update: 47 tests passed; Clippy/formatting clean. Main UI assigns instrument samples and exports the current Song snapshot. Saved four-part demo and CLI/UI exports were byte-identical; actual UI capture inspected. See `SEQUENCER-ROADMAP.md` for exact evaluator semantics and `evidence/m4-independent-verification.json` for independent WAV checks. Live transport was not yet implemented at that stage.

Milestone 5 cloud update: crossbeam-queue 0.3.14 is a direct dependency (MIT OR Apache-2.0); CPAL 0.17.3 is an optional direct dependency (Apache-2.0). Existing inventory entries remain accurate. See roadmap for verification limits.
