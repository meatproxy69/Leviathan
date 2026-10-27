# Leviathan sequencer implementation checklist

Implement one milestone at a time and verify it before proceeding. Renoise-inspired;
no claim of Renoise command fidelity or XRNS compatibility.

- [x] 1. Song content model: tracks, instruments, variable-length patterns, ordered occurrences.
- [x] 2. Central editable tracker: note/OFF/instrument/velocity/pan/delay, keyboard navigation, explicit Edit Mode and edit-step; useful vertical order list and track-block matrix foundation where feasible.
- [x] 3. Save/open and undo/redo.
- [x] 4. Document-driven A-B-A rendering, repeat/clone and boundary-safe voices.
- [ ] 5. Audible transport: play/stop/loop/playhead and defined seek behavior.
- [x] 6. Tested per-step retrigger/cut/offset/reverse/glide (cloud implementation accepted; physical-host integration remains separate).
- [x] 7. Bounded instrument phrases and triggering (cloud implementation accepted; no recursive nesting).
- [x] 8. Matrix, bounded track aliases and per-occurrence mute variations (cloud gate accepted).
- [x] 9. Baked groove and scoped transformations (bounded cloud gate accepted).
- [x] 10. Reproducible pattrns generation committed to editable patterns (one bounded seeded Euclidean preset; cloud gate accepted).

## Current status — 2026-10-25 UTC

**Read-only sample-peak meters v1 are accepted (bounded cloud scope):**
[receipt](evidence/judge-meters-final-20261025/judge-receipt.json). **980 core and
1,222 desktop tests pass freshly**, with a separate **300-test ui-capture library
run**, 203 MCP tests, **64 tools / 55 typed operations** and 19 byte-exact WAV/events
reference pairs (18 retained plus the authored effective-neutral pair). The gate
validates 38 fresh captures: 36 workflows and two zero-difference default images,
with 16 direct critical-frame views, 14 input observations and 64 poll deliveries
whose reported peaks have independent numerical coverage.

Optional worker observation measures the actual final stereo master sample peak
and available track subtotals after track balance and fixed master gain. It never
feeds measurements back into audio. Fixed 2,400-emitted-frame windows are worker
render-ahead observations, **not RMS, loudness, true peak, device consumption or
heard output**. Shared legacy ownership makes the whole affected track unavailable;
unsupported neutral per-track layouts retain an explicit master-only result, not a truncated
first-64-track total. The contributing-track cap is separate from the native
Mix table limit of 64 defined tracks. Finite peaks above unity are retained, not
clamped or labeled device clipping. Missing/unavailable is not measured zero. A bounded four-report
queue exposes evictions; session/generation checks reject obsolete reports. Live
reports omit incomplete leading/trailing windows and identify loop wraps and mix
revision ranges. Bars alone use display hold/decay; exact raw reports remain
inspectable/copyable, including a stable inspection snapshot and Follow latest.

Separate headless offline analysis uses explicit 48 kHz Song-frame bounds: start
aligned to 2,400, end aligned or the exact natural musical-plus-tail terminal, and
at most 120,000 processed frames. It advances full context from frame zero rather
than cold-seeking/cropping. Every full-compiled used source must be registered and
assigned as a region, including later notes beyond the measured prefix. Used
absolute paths are authorized before existing verified loading; unused source
**files** are not opened/byte-verified, while all model metadata remains validated.
Cancellation, clipping or failure returns no completed report. No Song/schema,
history, cursor, revision, save bytes, PCM file or automatic gain is changed;
MCP is offline analysis/inspection, not GUI transport IPC. Inspecting a retained
result does not reverify source bytes (`source_identity_verified_now:false`).

[The exact meter candidate](evidence/judge-meters-final-20261025/host-compile/independent-review.json)
passed isolated locked/offline desktop-audio compile/link with **123 source /
139 payload hashes before and after**. Nothing was executed; earlier host output
smokes remain separate historical evidence. Full M5, physical hearing, latency,
deadlines, hardware xruns, long-run and native GUI/device validation remain open.
Effects, buses, stems, automation, recording and MIDI are not added by meters.

### Preceding bounded track-balance acceptance

**Preceding bounded acceptance:** optional persisted track gain/balance/mute/solo,
[receipt](evidence/judge-track-mix-final-20261025/judge-receipt.json).
The **914-test completed core phase is explicitly reused as source-equivalent**:
the sole final2 desktop test-hook cfg correction is proved excluded from that
no-default build. Fresh corrected **1,148 desktop tests / 52 targets** and a
separate **265-test ui-capture library run** pass, with 191 MCP tests, 62 tools /
55 operations and 19 byte-exact WAV/events pairs (18 retained plus one authored
effective-neutral pair). Sixty fresh captures comprise 58 workflows plus two
official defaults; 22 critical frames were directly viewed. The 112 exact input
observations are 104 numeric texts, six path texts and two sliders. Fifty successful
preview replays use 485 wire requests; 14 nonempty device-free PCM records and two
GUI WAV/events pairs are retained. The Mix affordance intentionally changes 2,252
pixels in each default image, not zero.

One exact owner/base/revision-guarded full-map Apply stores at most 64 settings in
one Undo, without cursor movement. Live submission is separate and never rolls
back valid saved settings on refusal. Native full-table editing supports ≤64
defined tracks; larger documents show all configured entries read-only and permit
explicit Preview/Apply neutral Reset. Shared legacy sampler owners require equal
effective left/right targets. Neutral rendering preserves prior PCM; changed live
targets ramp over 128 emitted frames without retracting queued audio. These are
balance/mute/solo controls, not a complete mixer, meters, FX, stems or automation.

Candidate1's separately authorized two-pass host smoke supports bounded callback/
enqueue observations only (145/168 callbacks, 73,687 above-threshold copied PCM
frames each, zero software missing frames, revisions 1/2 separately accepted and
enqueued). Corrected candidate2 passed exact 117-source / 136-payload compile/link
only; neither binary was executed. Do not relabel historical runtime as final-source
runtime or infer speaker/coefficient PCM, ramp completion at a device, latency,
deadlines, hardware xruns, GUI or long-run proof. **Full physical M5 remains open.**

### Preceding rectangular editing acceptance

**Latest bounded cloud acceptance:** same-document rectangular whole-cell editing,
[receipt](evidence/judge-cell-blocks-final-20261025/judge-receipt.json).
**852 core / 1,074 desktop all-target tests**, plus a separate **240-test ui-capture
library run**, 182 MCP integration tests, 60 tools / 54 operations and 18 byte-exact
WAV/events reference pairs pass: 17 retained compatibility fixtures plus one
independently hand-authored expected post-edit reference. The latter deliberately
differs from the unedited Song, not an audio-equivalence claim. Fifty fresh captures cover 48 workflows plus two official
default views, with 20 direct critical-frame reviews. The new Cells toolbar entry
intentionally changes 145,991 pixels at 900px and 2,895 at 1180px; these are not
unchanged baselines. Copy is read-only, including resolved aliases; Paste Replace/
Clear require local destinations; Paste keeps the original logical TrackId. Exact
owner/revision/base guards protect one-Undo commits. Holes differ from explicit
stored default cells. Clipboard ownership survives edits/Save/Undo/Redo but successful New/Open revokes it.

Common history identity now handles signed zero and lexical paths exactly, refuses
comparison errors before history changes, and never falsely marks an unverifiable
Undo/Redo state clean. Registered source paths remain lexically immutable. No
source IO, persistent clipboard schema, OS-backed/cross-document/multi-track cell clipboard,
ID remapping, inherited-memory repair or audio-equivalence promise is added.
The exact 111-source / 127-payload candidate passed isolated locked/offline
`desktop-audio` compile/link only, not launch or audio/GUI testing. Engine/transport
and dependencies remain unchanged. **Full physical M5 stays unchecked.**

### Preceding reviewed marker-bank acceptance

**Preceding bounded cloud acceptance:** reviewed marker-to-drum-bank v1,
[receipt](evidence/judge-marker-bank-final-20261024/judge-receipt.json).
**811 core / 1,022 desktop all-target tests**, plus a separate **227-test ui-capture
library run**, 175 MCP integration tests, 56 tools / 54 typed operations and
17 byte-exact WAV/events
goldens pass. The 46 native captures comprise 44 workflow frames and two unchanged
full-image baselines. Exact sparse accepted intervals become fresh ordinary region,
kit and instrument metadata, not notes, generated music or classified drum roles.
Create preserves existing mappings, markers/locks, clock, patterns/order and cursor;
selected-source fresh hashing precedes one exact-base/revision-guarded Undo.
No source BPM is required, and source audio is never rewritten.

The exact 107-build-source / 123-payload candidate also passed an isolated
locked/offline desktop-audio compile/link. It was not launched: no new GUI/audio,
actual bank/audition-device, speaker, latency/deadline or hardware-xrun acceptance.
**Full M5 remains unchecked.** This is a one-time reviewed allocation; automatic
analysis/role acceptance, multi-bank paging and broader sampler/grammar remain open.

### Preceding audition acceptance

**Preceding bounded cloud acceptance:** exclusive region/pad Audition v1 final2,
using one immutable synthetic Song snapshot and the existing Engine/Transport.
No document commit, persistent schema, tracker cursor movement or Undo consumption;
Song playback and audition are exclusive, not mixed performance voices. Region
previews may use an unsaved imported draft; pad previews use committed mapping.
Fixed input 1 / bin 127, fresh RR0, one ON with a two-second gate plus existing 500ms
tail are bounded semantics, not a general gate/performance editor. Used sources
must be absolute model paths; saved relative paths are resolved by the existing
project loader, never guessed from process CWD. No MIDI/extras or new engine.

Final2 receipt: [evidence/judge-audition-final2-20261024/judge-receipt.json](evidence/judge-audition-final2-20261024/judge-receipt.json).
**780 core / 981 desktop all-target tests**, plus a separate **215-test ui-capture
library run**, 169 real-process MCP tests, 54 tools / 54 typed operations and
16 byte-exact WAV/events goldens pass. There are 58 main frames (56 workflow frames
plus two unchanged full-image baselines), and two additional unregistered-source/
Read-Mode frames. The initial same-frame focus-loss/regain gate was rejected;
final2 fixes active/preparing cancellation and proves no automatic restart.
The exact corrected source passed a separate isolated desktop-audio compile/link
with before/after 103-source and 119-payload identity. It was not launched: this
is not actual audition-device, native OS-window/input or listening acceptance.
The earlier candidate1 compile remains superseded historical evidence.

**Separate partial host gate now passed:** [evidence/judge-aqua-host-smoke-20261024/receipt.json](evidence/judge-aqua-host-smoke-20261024/receipt.json) records four short
authorized isolated Aqua CPAL/PipeWire output sessions at 48 kHz stereo float32,
148 callbacks each, above-threshold PCM and zero software missing frames.
Seek/loop observations are not sample-exact PCM proof. Fresh post-build hashes
match the accepted transport checkpoint's 1,115 payloads; independent reconstruction
matches four authored WAV hashes from 768,000 values ×0.1. No historical pre-build/
pre-execution hash log or runtime attestation exists. Speakers, latency/deadlines,
hardware xruns, long runs, natural end and native GUI remain unverified; **M5 stays
unchecked**. This new evidence does not rewrite any earlier sealed receipt.

The preceding accepted cloud checkpoint added **device-free transport stress diagnostics**:
`evidence/judge-transport-stress-final-20261024/judge-receipt.json`
(**756 core / 944 desktop tests**, plus a separate **202-test ui-capture library
run**; 53 MCP tools / 54 typed operations unchanged). Three additive diagnostic
files extend tests/examples; all 96 prior build-source files, production
Engine/transport and dependencies remain unchanged. Five fresh CLI runs cover
40 bit-exact same-Engine PCM cases and 80 acknowledged controls / at least 160
post-command ready observations. Sixteen compatibility goldens and only two fresh
unchanged default frames pass, not a new transport UI workflow gate. No measured
fill allocation/reallocation/deallocation occurred; locks/syscalls/deadlines and
physical audibility/latency/xruns are not certified. The earlier artificially paced
concurrent run's 769 missing frames remain disclosed. **Physical M5 stays open;
no host action or runtime improvement is claimed.**

The preceding accepted cloud build added **reviewed-role seeded THIN v1**:
`evidence/judge-thin-final2-20261024/judge-receipt.json`
(**754 core / 942 desktop tests**, plus a separate **202-test ui-capture library
run**; 53 MCP tools / 54 typed operations). This is a bounded whole ordinary-cell
deletion operator with actual pinned pattrns gates, current all-use human-role
protection, exact retained-ON/control proof and one guarded Undo. It does not prove
unchanged release/choke/tails or full audio, genre coherence or a complete grammar.
Unknown/Mixed/stale/unreviewed uses remain protected; even 0% requires a fully
model-evaluable assigned-source base. Core is IO-free; MCP pathname preflight remains.
The initial UI gate was explicitly rejected and superseded; final2 contains fresh
corrected exact-record reachability and the complete rerun. Device-free transport
verification is now accepted above; further physical verification requires separate explicit permission.
**Physical M5 remains open; no host action is authorized by these cloud results.**

The preceding accepted cloud build added **native slice features and separately
reviewed roles**:
`evidence/judge-slice-catalog-final-20261024/judge-receipt.json`
(**720 core / 894 desktop tests**, plus a separate **188-test ui-capture library
run**). Native acoustic descriptors and uncalibrated handcrafted role hypotheses
remain advisory. User-reviewed role/articulation is separate explicit metadata,
not an automatic mapping/grammar or playback change. Source/bounds edits retain
stale historical snapshots; current-match is model equality, not a file check or
scientific attestation. Analyze/Store verify source bytes; metadata inspections and
pure core/native manual-role edits do not require media IO. MCP manual batches keep
normal all-reference pathname preflight. Original B3.4 stays PARTIAL, with broad
recognition/bleed/genre/listening acceptance still open.

The preceding accepted tempo build added **timing-point tempo / pulse-phase
hypotheses**:
`evidence/judge-tempo-final-20261024/judge-receipt.json`
(**685 core / 846 desktop tests**, plus a separate **175-test ui-capture library
run**). It fits bounded declared marker/cache timing points, exposes multiple
heuristic candidates/beat-unit ambiguity, and permits only an explicit guarded
BPM-only edit of a same-source marker set. No automatic tempo, phase, marker or
Song-clock application occurs. Marker-only evidence is historical without source-byte
hashing/decoding; normal MCP pathname/root preflight remains. Cached adapters
freshly authorize/hash at Analyze and Use. Native cached max-frames is
16,777,216 versus MCP's source-rate × 15, so effective config SHA and analysis/Use
IDs are not promised identical. B3.2 stays PARTIAL: downbeat, meter, loop-length,
automatic role recognition and musical-quality validation remain unfinished.

The preceding scoped render/resample gate remains historical at
`evidence/judge-resample-final-20261024/judge-receipt.json`
(649 core / 799 desktop tests). It prints one cold occurrence to a new verified
pre-master-compensated WAV, then separately registers fresh references with all-
source/storage preflight and guarded Undo. Publication is Unix-only; used legacy
paths require import/region assignment. Artifacts survive failed/stale registration
and Undo. This is not contextual cropping, stems, a race-free path sandbox or full
original sampler M5 completion. Exclusive melodic keyzones remain historical at
`evidence/judge-keyzones-final-20261024/judge-receipt.json` (611 core / 749 desktop).
Reusable accepted-marker groove timing transfer remains historical at
`evidence/judge-groove-final-20261024/judge-receipt.json` (580 core / 708 desktop):
timing-only baked applications, not automatic inference or full renderability proof.
The earlier drum velocity-zone/deterministic round-robin gate remains historical at
`evidence/judge-velocity-rr-final-20261024/judge-receipt.json` (538 core / 660 desktop).
The earlier opt-in sampler v1 envelope/forward loop gate remains historical at
`evidence/judge-sampler-v1-final-20261024/judge-receipt.json` (505 core / 622 desktop).
The intervening UI polish is accepted at
`evidence/judge-ui-polish-final2-20261024/judge-receipt.json` (581 desktop tests);
the M10 receipt remains historical at
`evidence/judge-generation-final2-20261024/judge-receipt.json` (469 core / 572 desktop).
Bounded baked groove/scoped transforms were
accepted in `evidence/judge-transforms-final-20261024/judge-receipt.json`.
Track aliases/mutes were accepted in `evidence/judge-matrix-final-20261024/judge-receipt.json`.
The guarded drum-step overlay was accepted earlier at
`evidence/judge-drum-steps-final-20261024/judge-receipt.json`. Accepted intervening
work includes [instrument phrases](evidence/judge-m7-final-20261024/judge-receipt.json),
[verified source regions](evidence/judge-slice-foundation-final2-20261024/judge-receipt.json),
[editable break compilation](evidence/judge-compiler-final-20261024/judge-receipt.json),
[optional experimental onset-v2 and marker locks](evidence/judge-onset-v2-final-20261024/judge-receipt.json),
and [single-layer drum kits](evidence/judge-kit-final2-20261024/judge-receipt.json).
The original roadmap's **track aliases and occurrence mute variations are now
accepted within the bounded cloud scope**. Bounded groove/scoped transforms are
also accepted, as is the single bounded seeded Euclidean generation preset.
The opt-in sampler v1, bounded drum velocity-zone/round-robin and reusable explicit
groove-timing subsets, exclusive melodic keyzone mapping, scoped render-to-asset,
timing-point tempo hypotheses and reviewed native slice metadata are accepted.
This does not complete broader composer, scripting, full multisample-sampler,
recursive rendering, groove-analysis or runtime-groove plans. M5 physical-host/
listening remains open.

These current-status corrections do not alter any sealed historical receipt or
its reviewed document hashes. The chronological entries below describe their
original review snapshots; later acceptance notes supersede their stale statuses.

The complete original sampler/Break Compiler plans, remaining features and explicit
user exclusions are tracked separately in [FEATURE-STATUS.md](FEATURE-STATUS.md).

## Historical M6 accepted checkpoint — 2026-10-24 UTC

The final independent [judge receipt](evidence/judge-integrated-final-20261024/judge-receipt.json)
accepts **M6 implementation, editor/storage, tested egui/EGL UI, and the isolated
MCP interface**. The unchanged-source aggregate passed **155 core tests** and
**174 desktop tests**, all-target UI-capture Clippy with warnings denied, format,
desktop/MCP builds, independent numerical PCM and JSON-RPC probes, and eight
final-source EGL/Glow captures. See the receipt for exact boundaries and hashes.

**M5 physical-host acceptance remains open:** the cloud's physical-audio build is
blocked by missing `jack.pc`; speaker audibility, device latency, OS-window/input
integration, and live GUI/transport control through MCP are not accepted claims.
MCP is a separate headless document session, supports the pinned 2025-11-25 stdio
profile, and was not registered or deployed to a host. No user-PC changes occurred.

Earlier entries below are chronological implementation history. Their statements
that M6 was "not started", incomplete, or lacked hard cut/offset/reverse are
**superseded by the sample-control adapter and this final receipt**. M6 acceptance
does not close the separate M5 host gate or claim Renoise/sampler-quality parity.

## Milestone 1 acceptance criteria

- Stable typed nonzero track/instrument/pattern IDs, independent of names/order.
- Pattern pool with variable row counts and ordered pattern references.
- Explicit Empty/Note/Off; independently optional instrument, velocity, pan and delay.
- Finite/range validation, sparse row/column bounds and referential integrity.
- Four-track fixture, 32/64-row patterns and A-B-A order.
- Repeat shares content; clone deep-copies under a new ID without modifying order.
- Serde round-trip preserves content and IDs using existing pinned dependencies.
- No mutable playback state, effects or note-lifetime policy embedded in content.
- Existing CLI, GUI and audio tests continue passing.

## Model contract and limitations

`src/song.rs` is an independent content module. Public fields are draft data;
call `Song::validate` before accepting a document. Serde decoding itself does not
validate. IDs are assigned by callers and must remain stable; map keys define
identity. Reorder tracks through `track_order`, not by changing IDs. Asset paths
are optional unresolved references and are not checked on disk. Sparse missing
cells represent empty content. Empty projects are valid.

Pitch is MIDI 0..127, velocity 0..1, pan -1..1 and delay a fraction of a row
in [0,1). BPM 20..400, LPB 1..256, rows 1..65536 and columns 1..256 are current
product limits, not Renoise limits. Optional instrument stays distinct from an
explicit instrument even on a pitched note. This preserves the guide's omitted-
instrument/ghost-note representation but does not implement its envelope behavior.

At milestone 1 completion, no editor, file save/open, scheduler, voice-lifetime rules,
effects, phrase runtime, UI integration or song rendering had been added. Existing CLI/GUI continue
using their existing pattrns motif. No new dependencies or system changes.

## Verification

Completed on Aqua, 2026-10-23, from `/home/raine/Leviathan`:

```sh
CARGO_HOME="$PWD/.cargo-home" cargo test --locked --offline --all-features
CARGO_HOME="$PWD/.cargo-home" cargo clippy --locked --offline --all-targets --all-features -- -D warnings
```

Results: **19 passed, 0 failed** (6 bridge, 5 core, 2 background-job,
6 new song-model integration tests). Clippy passed with warnings denied across
all targets/features. Tests cover JSON round-trip/IDs, repeat vs independent clone,
atomic failure of clone/repeat, independent optional fields with Empty/Note/Off,
dangling references, track ordering, invalid/nonfinite attributes, sparse bounds,
unused-pattern validation and schema rejection. Existing tests continue exercising
CLI errors/bounded shutdown and deterministic audio/manifest rendering.

No new GUI smoke test was run for this model-only change; no visual or new playback
capability is claimed. No dependencies changed. No outstanding blocker.

## Milestone 2 scope

Milestone 2: bind an editable tracker to this document with explicit Edit Mode,
keyboard note/OFF entry, independent attributes, selection/navigation and edit-step.
Prioritize the central tracker and vertical sequencer/matrix over browser, mixer or
cosmetic work. Show independently optional instrument values, vertical song positions
and track blocks. Keep repeat/reference and clone operations clear; aliases follow
as a distinct content-sharing feature. Eventual local FX, track FX, global commands
and phrase effects need distinct semantics, with no fake enabled controls.
Do not silently claim song playback until the later rendering/transport milestones.

## Grid layout clarification from user references

The tracker editor runs time downward through rows. Matrix orientation is a UI
choice: the references show both song positions across/tracks down and song
positions down/tracks across. The initial vertical order-list layout does not
require supporting both orientations immediately.

A matrix block represents one track's pattern content at a song occurrence,
not an individual note or audio clip. Connect selection as occurrence -> track
block -> editor of the referenced shared pattern content. Distinguish repeated
references from independent clones; future aliases and occurrence mute indicators
need real model/playback semantics before exposing working controls. Older
screenshots are layout references, not authoritative current effect syntax.

## Milestone 2 completion — 2026-10-23

Acceptance criteria completed:

- Central editable tracker backed by `Song`, all five displayed fields independently
  editable (note/OFF, instrument, velocity, pan, row-fraction delay).
- Edit Mode guards content changes, including repeat/clone and row resizing.
- Keyboard grid focus, note entry, OFF, field/cell clearing, bounded row/field/column
  navigation, edit-step and octave. Cursor follows keyboard movement via scrolling.
- Vertical order list and track-block matrix select the referenced pattern/track;
  repeat appends shared content and clone appends an independent copy.
- Variable row counts; shrink refuses to discard occupied rows. Edits validate before
  committing, so invalid values leave content intact. Edit cursor is not a playhead.
- Existing export tools are explicitly labeled legacy fixed motif; tracker edits are
  not rendered or played. Dirty-close confirmation avoids accidental edit loss.

Verification: `cargo test --locked --offline --all-features`: **25 passed, 0 failed**
(6 editor, 6 song, 6 bridge, 5 core, 2 worker tests). All-target/all-feature Clippy
passed with `-D warnings`. Desktop build passed. New tests exercise mutation guards,
independent attributes, invalid-edit atomicity, safe resize, selection, repeat/clone,
bounds and edit-step; a headless egui input test types a path into an actual TextEdit
and confirms it does not change the song, then verifies focused-grid note input.

A separate native `--tracker-smoke` instance edited A and checked its repeated
occurrence, captured the real framebuffer and exited normally within 20 seconds.
Inspected capture: `evidence/leviathan-tracker-m2-final.png`. An initial capture
revealed a layout issue; it was corrected before this final capture. This is not
manual coverage of every widget. Existing sessions were not closed or replaced.
A separate legacy `--smoke-test` instance loaded/exported normally within 25 seconds;
`artifacts/m2-legacy-check.wav` and `.events.json` compare byte-for-byte equal to
`artifacts/pattrns-demo.wav` and its manifest.

Known limitations: in-memory editing only; no save/open or undo yet; no song audio,
playhead, effects, aliases, mutes, order rearrangement or multi-cell selection.
Numeric field input uses a selected-field form and decimal normalized values, not
Renoise hexadecimal digit entry. The fixture provides two note columns per track;
the wide grid scrolls horizontally. No new dependencies/system changes or blockers.

Next milestone: **3 — save/open and undo/redo**, maintaining the central editor and
connected order/matrix as the primary work surfaces. Stop here for milestone report.

## Milestone 3 completion — 2026-10-24

- Versioned `leviathan-song` JSON envelope v1 with song schema 1. Load validates
  every reference/range, rejects unknown struct fields/unsupported versions and
  leaves the current document, history and active path intact on failure.
- Project reads/writes limited to 8 MiB. Sample paths are absolute in memory;
  save writes relative paths for assets inside the destination project directory,
  absolute paths otherwise. Relative references load against the project directory,
  never the process working directory. Save As preserves asset targets. None is an
  intentionally unassigned instrument; a referenced missing/unreadable/non-file
  asset fails with instrument ID and path. Audio decoding is not part of project I/O.
- Saves validate and serialize before creating a unique sibling temporary file,
  write and sync it, then publish atomically. New files use no-clobber hard linking;
  replacement requires explicit confirmation of an existing byte snapshot and a
  final comparison before rename. Symlink project targets are refused. Temporary
  files are cleaned up on handled failures. Parent-directory sync failure after
  commit is reported as a durability warning, not a falsely failed pre-commit save.
- New/Open/Close use Save / Discard / Cancel. Save-before-open has a separate current-
  song destination so the requested input file is not accidentally used as the save
  destination. Failed saves do not clear dirty state or complete the pending action.
- Up to 64 undo snapshots; redo is invalidated after a divergent content edit.
  Saved-state comparison restores correct dirty status across undo/redo. Navigation
  and no-op edits add no history. Note/field apply, clear, resize, repeat, clone and
  occurrence moves are individual transactions. Row count now uses an explicit
  Set rows button, avoiding one transaction per drag frame. New/Open clear history.
- Project bar provides New/Open/Save-as and Undo/Redo. Ctrl+N/O/S and Ctrl+Z/Y or
  Ctrl+Shift+Z only act on song/project while the tracker grid has focus. Text
  entry keeps its own shortcuts. Order Move up/down is undoable. Empty valid
  documents show an empty-state view rather than crashing the tracker.

Verification: **36 tests passed, 0 failed** (10 project/session/history, 7 editor,
6 song, 6 bridge, 5 core, 2 worker). Full Clippy with warnings denied and formatting
checks passed. Transcript: `evidence/m3-verification.log`.

Native separate-instance `--project-smoke` verified edit/save, saved-state undo/redo,
new/open roundtrip, then captured and closed normally within 20 seconds. Inspected
final screenshot: `evidence/leviathan-project-m3-final.png`; created test project:
`artifacts/m3-project-final.json`. Narrow desktop tiling exposed crowded file
controls; those were corrected and recaptured. Existing sessions were untouched.
Legacy native export still matches the earlier WAV/JSON byte-for-byte:
`artifacts/m3-legacy-check.wav` and `.events.json` vs `artifacts/pattrns-demo.*`.

Limits: no automatic asset copying/embedding, relinking UI or sample assignment UI;
project I/O is synchronous and may pause on slow storage despite the size limit.
History uses bounded-count whole-song snapshots, not a byte-budgeted diff journal,
and is not persisted. Concurrent external writers are checked before replacement
but there is no cross-process locking/merge protocol. No power-loss/disk-full fault
injection or manual coverage of every dialog is claimed. Song rendering/playback
remains milestone 4/5. No dependencies/system settings changed; no blockers.

Next milestone: **4 — document-driven A-B-A rendering, repeat/clone and boundary-
safe voices**. Milestone 4 was not started in this turn.

## Milestone 4 completion — 2026-10-24

Implemented document-driven offline evaluation/rendering in `src/song_render.rs`.
Main UI exports an immutable validated Song clone via the existing one-job background
controller. Editing, saving or opening another document after the click cannot
change that snapshot. Referenced samples decode into samplers at worker start;
external file changes before decoding can affect a run. No audio rendering runs on
the UI thread. Legacy fixed-motif CLI/export tools remain compatible and separate.

Instrument assignment is available by ID and file path in the main view, guarded by
Edit Mode and undoable. Paths canonicalize; relative render references require the
saved project directory. Missing/unassigned used instruments and undecodable assets
produce errors. New WAV and event JSON paths use create-new reservations; existing
files are never replaced. Handled render failures clean only newly created outputs.
This output pair is not a crash-atomic multi-file transaction.

### Evaluator v1 semantics

- MIDI pitches are used directly, without the pattrns octave-offset convention.
  State is per (track ID, note column), preserved across every ordered occurrence.
- Explicit instrument changes the column's selection. Omission reuses its previous
  selection; a pitched note before any selection is an error. Instrument-only rows
  select future notes without changing the currently sounding voice's instrument.
- Velocity and pan are independently optional, latched per column (initial 1 and 0).
  On empty-note or OFF rows, supplied values also address the currently active voice.
  Empty rows sustain. A pitched note releases its predecessor and starts a new voice;
  omission of instrument does NOT promise Renoise ghost-note/envelope preservation.
- OFF releases the active voice but retains column selection/attributes. Explicit
  OFF, replacement and song-end releases are distinguished in the event manifest.
  No automatic releases occur at pattern boundaries. Natural sample exhaustion is
  allowed. Song end releases active notes, renders a fixed 0.5-second tail, then
  drains AllNotesOff before dropping samplers. Longer tails are not preserved.
- Delay offsets the entire populated cell by a fraction of a row. Frames come from
  absolute cumulative row positions using f64 and nearest-frame rounding, not sums
  of rounded row lengths. Delayed events rounding to song end clamp to its final
  musical frame. Ties sort by absolute row, displayed track order and column;
  within a cell replacement OFF precedes ON. No LPB/BPM change commands or ticks yet.
- One phonic sampler per used instrument; unique voice IDs and enough slots for all
  its note triggers prevent voice stealing. Stable instrument-order summation uses
  fixed master gain 0.5, no normalization/limiter. Nonfinite/clipping audio fails.
- Current bounded offline limits: ten minutes, 32768 populated cells, 2048 triggers;
  stereo 48 kHz float32, 512-frame maximum processing blocks split at event boundaries.
  This is Leviathan semantics, not a Renoise playback interpreter or XRNS support.

### Verification and deliverables

**47 tests passed, 0 failed**: 11 new song-render/assignment tests plus all 36 prior
tests. All-target/all-feature Clippy with warnings denied and formatting checks pass.
Transcript: `evidence/m4-verification.log`. Coverage includes sustained audio across
boundaries, explicit OFF vs end, instrument/track routing, parameter-only volume
changing audio, omitted values, shared A and independent B, fractional BPM/LPB and
late delay, save/reopen byte identity, immutable worker snapshot, missing/invalid
assets, empty song, no-clobber, clipping cleanup and undoable assignment.

`artifacts/m4-song-demo/first-current.leviathan.json` is an editable saved four-track
song with generated kick/hat/bass/lead samples, A=32 rows, B=16, A-B-A order, BPM 120,
LPB 4. No copyrighted downloads. `first-current.wav` and `first-current-repeat.wav`
are byte-identical, as are their manifests. Each has 504000 frames (480000 musical
+ 24000 tail), 10.5 seconds, peak 0.2141743302345276 and 180 events. Independent Python
RIFF parsing handles IEEE float WAVE_FORMAT_EXTENSIBLE, verifies channels/rate,
finite nonzero nonclipping output and stereo difference. Report:
`evidence/m4-independent-verification.json`; script: `scripts/verify_song_demo.py`.
The initial independent checker assumed a basic format tag; it was corrected to
handle the standard extensible header before producing the successful report.

Native `--song-smoke` opened the saved project and exported using the main UI worker,
captured `evidence/leviathan-song-export-m4.png`, and exited within 25 seconds. The
actual screenshot was inspected. UI WAV/JSON (`artifacts/m4-ui-song.*`) and document
CLI WAV/JSON (`artifacts/m4-cli-song.*`) both equal the demo byte-for-byte. No existing
session was closed. No new dependencies, source approvals or settings changes.

Next: **milestone 5 — audible transport**. Not started. No live playback, latency,
seek, loops, per-step effects, phrases, aliases or plugin support claimed. Song BPM
and LPB are stored/evaluated but do not yet have dedicated editor controls. There
is no unresolved blocker for this milestone.

## Milestone 5 cloud continuation — 2026-10-24

**Historical M5 checkpoint: desktop verification was pending and M6 had not started.
Superseded by the current accepted checkpoint above; the physical-host gate remains open.**

The transferred revision's missing `TrackerUi.playhead` field is restored with a
yellow playback-row highlight, separate from the edit cursor. Highlight and seek
require the editor document to match the immutable playback snapshot, preventing
misleading reused-ID positions after New/Open or timing edits. Fractional-tempo
playhead mapping now matches the evaluator's rounded frame boundaries. Device
failure clears readiness, stops production and requests worker shutdown; queued
PCM remains silent. Added regressions for device failure and rounded boundaries.

Incremental worker rendering, bounded PCM queue, snapshot Follow, cold seek,
whole-song loop, asynchronous stop/close and project-local lease behavior are
recorded in `CLOUD-HANDOFF.md` and README. Stop/seek review confirms generation tags
reject previously queued commands; hardware-submitted frames cannot be recalled.
This is not a proof of all atomic interleavings or an end-to-end latency measurement.

Verified in this cloud workspace using project-local official Rust/Cargo 1.99.1:

- `cargo test --locked`: **53 passed, zero failed**, including eight transport tests.
  Desktop-gated tests are excluded. Log: `evidence/m5-cloud-core.log`.
- `cargo clippy --locked --all-targets -- -D warnings`: passed (core features).
  Log: `evidence/m5-cloud-clippy.log`. `cargo fmt -- --check`: passed.
- Generated `artifacts/m5-cloud-song-demo`; independent Python WAV/manifest check
  passed: 504000 frames, 10.5 seconds, peak 0.2141743302345276, 180 events,
  byte-identical repeated exports. `evidence/m5-cloud-demo-verification.json`.
- `cargo test --locked --all-features`: **blocked**, before desktop crate checking,
  by missing `jack.pc` (`jack-sys` build). ALSA/X11/Wayland development metadata is
  also absent. `evidence/m5-cloud-all-features.log`. All-feature Clippy and native
  smoke tests have not run against this revision.

A download-only `apt-get download libasound2-dev libwayland-dev libx11-dev` attempt
found no package entries and reported a denied read of
`/etc/apt/apt.conf.d/80-applied-apt-retries`; no retry or permissions bypass was
performed. No system package install, settings change, user-computer call or push.
Official Rust and approved pinned Cargo sources downloaded successfully into
`.toolchain`/`.cargo-home`; these caches are not deliverable source artifacts.

Historical pre-interruption desktop evidence: 53 all-feature tests, Clippy and
host/native probes passed; see handoff. That evidence is not a final-revision
pass. Milestone 5 remains unchecked until an authorized environment with the
required development libraries verifies desktop compilation and tests.

## Milestone 6, supported step-command subset — 2026-10-24

**Historical M6a subset; superseded by the accepted M6b adapter below.** The pinned phonic 0.16.0 sampler
provides per-voice NoteOn/NoteOff and SetSpeed, but no per-voice sample offset,
reverse direction, or immediate hard-kill command. At this subchunk those three were deferred;
timed release is not presented as a substitute for hard cut. No phrases, new
backend dependency, or user-computer/system changes are part of this subchunk.

`Cell.effects: StepEffects` contains optional typed retrigger, timed release and
glide. Missing effects load as empty; empty effects are omitted when saving, so
existing no-effect schema-1 project content remains unchanged. Project envelope
version and song schema remain 1. This means old files open in this new build,
not that older binaries can understand new effect fields (they reject them).
Effect-bearing evaluated manifests use schema 2 / `leviathan-song-v2`; no-effect
manifests retain schema 1 / `leviathan-song-v1` and unchanged output bytes.

### Exact supported semantics

- **Retrigger:** `Retrigger { repeats, spacing }`, with 1..=16 **additional** note
  starts and a finite positive spacing in fractions of a row. A pitched cell is
  required. The starts occur at `delay + k*spacing`, k=1..=repeats; the last must
  be strictly less than 1. Each releases the preceding voice and starts the same
  MIDI pitch from the beginning with the column's resolved instrument, volume
  and pan. Release tails may overlap. There is no implicit retrigger memory on
  subsequent cells, no ticks, and no Renoise command-code compatibility claim.
- **Timed release:** `release` is an absolute fraction of its source row in [0,1),
  at or after the cell delay and every retrigger. It emits NoteOff to the active
  voice and keeps instrument/volume/pan selection. Empty-column release is a
  deterministic no-op; a naturally exhausted sample also need not produce audio.
  OFF plus timed release is rejected. phonic's configured 50 ms exponential
  de-click reaches approximately 99% attenuation at 50 ms, not immediate silence;
  completion occurs later and is detected at a processing boundary. It is **not
  hard cut**. Decimal-equal times tolerate f64 roundoff, and the scheduled release
  is clamped to the final start fraction so half-frame rounding cannot reverse
  their order.
- **Glide:** `Glide { target, semitones_per_second }` uses absolute MIDI target
  0..127 and finite rate 0.01..=120 semitones per second, not a duration or row
  fraction. The backend changes speed in 64-frame updates. Glide addresses the
  currently active/newly triggered voice, even if an instrument-only selection
  chooses a different instrument for the future. A glide on a cell without an
  active logical voice is an evaluation error; glide with OFF is rejected. A
  pitched cell retriggers normally, then glides; it is not legato/ghost-note
  behavior. Each retrigger restarts its own glide. New notes without a glide do
  not inherit the previous glide target/rate.
- Timing is computed from absolute cumulative row fractions and rounded once
  to frames. Delay affects the cell and all retrigger starts; release is absolute
  within the row, not an additional delay. Ties sort by absolute source row,
  displayed track order, column, then base cell/retrigger index/release. Each
  retrigger's OFF precedes ON, which precedes its Glide. Extremely close fractions
  may intentionally collapse to the same frame. Late final-row events clamp to
  the last musical frame, before the song-end release.
- Existing bounds remain 32768 populated cells, ten minutes, and 2048 total note
  starts **including retriggers**. Expansion adds a 65536-step bound. Unsupported
  effect fields, nonfinite/out-of-range parameters, incompatible note forms, and
  commands extending beyond their row are rejected, not silently ignored.
- Offline export and streaming use the same evaluated manifest and Engine. Cold
  seek still reconstructs no earlier voices; a later compiled retrigger starts
  independently with resolved instrument/attributes. No new device-side work.

### Core evidence for this subchunk

`tests/effects.rs` has 15 passing tests covering actual retrigger sample restarts,
actual glide pitch/rate in both directions and later-note reset, exponential
release tails, delayed/tied and final-frame ordering, decimal half-frame edge
regression, column/instrument isolation, no-active cases, cold seek into pending
retriggers, 2048-expanded-trigger limits, validation failures/unsupported fields,
exact streaming/export PCM and manifest equality, retrigger-overlap clipping
cleanup, extreme glide targets/rates, and glide musical-PCM identity across
1/37/512-frame processing partitions. Core aggregate log:
`evidence/m6-core-effects.log`; core Clippy log:
`evidence/m6-core-effects-clippy.log`. UI/project tests are recorded separately.

Re-render of the unchanged saved demo is byte-identical to the M5 baseline:
`artifacts/m6-no-effects-check.wav` SHA-256
`2bf974106c3ec99b04902c813137f75a23de3ebb7e811ae88a093c077f66b8e3`, and its event
JSON SHA-256 `7314e1ef67f842c4c518b3e19452dfab928549b9ba9168d19aad18014d20fc92`.
At this historical subchunk, full M6 remained unchecked pending hard-cut/offset/reverse
implementation and verification; the later adapter and final receipt supersede that status. Physical output and native UI require the already-documented later
integration environment; these core checks do not establish speaker audibility.

## Milestone 6, sample-control adapter subchunk — 2026-10-24

The bounded implementation now includes **real hard cut, source-frame offset,
and forward/reverse direction** in addition to retrigger, timed release and glide.
This supersedes the deferred status in the preceding M6a subchunk. The final
aggregate judge receipt accepts this implementation in its tested cloud scope;
this is not a production sampler-quality or Renoise-compatibility claim.

### New persisted commands and timing

`StepEffects` adds optional `cut: f64`, `offset: u64`, and
`direction: Direction::{Forward, Reverse}`. Missing values default to none; absent
new fields are omitted on serialization, preserving existing M6a JSON. Schema-1
projects still load. Manifests using any of these commands use schema 3 /
`leviathan-song-v3`; ordinary and M6a-only schedules keep their previous versions.

- **Hard cut** is an absolute fraction of the row in [0,1), at or after the delayed
  start and final retrigger. It is abrupt at its rounded frame and **may click**.
  It targets the current logical column voice, not earlier replacement/retrigger
  release tails or other columns/tracks. Without an active voice it is a no-op.
  It cannot combine with OFF or timed release in one cell. Decimal-equal times
  use the same tolerance and final-start clamp as timed release. A zero-time cut
  can silence a new voice before it emits any PCM.
- **Offset** is a zero-based **source sample frame**, not output frames, scalar
  interleaved samples, seconds or a percentage. The model bounds it to u32::MAX;
  worker preflight also requires it to be less than the decoded real frame count.
  On a pitched note it selects the initial position. On an empty-note cell it
  seeks the active logical voice without retriggering or changing pitch/direction.
- **Direction** on a new note defaults forward. Reverse without an explicit offset
  starts at the last real source frame; the decoder's extra zero guard frame is
  never treated as musical content. An explicit offset is an absolute source
  position in either direction. On an empty-note cell a direction command flips
  the sign of traversal while preserving the next-unread fractional cursor: no
  implicit jump to an endpoint. If both offset and direction occur, seek is applied
  first, then direction. Offset/direction with OFF or without an active logical
  voice is rejected. Natural sample exhaustion does not create a new voice; later
  offset/direction cannot revive one, including exactly at an event/block boundary.
- Offset/direction are not latched into later notes. Retriggers restart the original
  cell's offset and direction. Initial source controls follow ON and precede Glide;
  hard cut follows same-frame retriggers. Existing displayed-track/column/source-row
  tie ordering and final-musical-frame clamp remain in force.

### Explicit adapter scope and quality limits

`src/song_render/controlled.rs` is an opt-in worker-side adapter over immutable
preloaded buffers. Voices needing only hard cut use isolated one-voice **phonic**
samplers; their PCM is hard-muted at the event boundary. A voice that uses offset
or direction uses the linear-interpolation adapter **from its original ON**, even
when its first source control occurs on a later row. Thus PCM before that later
command can also differ from phonic's resampler. There is no claim of anti-aliasing
or filter-quality parity. Other voices continue through the existing phonic pool.

The source-control adapter supports mono (duplicated to stereo) and frame-aligned
stereo, source rates 1..=384000, finite decoded samples, and at most 64 MiB combined
source-control PCM (including decoder guards). Encoded controlled files also have
a 64 MiB preflight bound. Source-rate conversion and MIDI pitch jointly advance a
fractional source cursor; interpolation does not swap stereo channels in reverse.
Gain/pan use the existing phonic smoothing helpers. Glide retains the documented
rate and 64-frame speed-update semantics. Timed release uses the same 50 ms/99%
exponential attenuation convention, with deterministic per-frame completion in
this adapter rather than phonic's processing-boundary completion.

**Embedded sample loops are rejected for offset/direction voices before export
outputs are created**, rather than silently changing a loop into a one-shot.
Ordinary and cut-only phonic voices continue honoring their existing embedded-loop
behavior. No sample-loop editing or direction-aware embedded loops are claimed.
All voice slots are bounded by the existing 2048-total-start limit, allocated on
the worker, and reset on cold seek/whole-song loops. Source PCM is not reread on
cold reset. Callback code remains queue-copy/atomics only.

### Intentional zero-velocity correctness fix

Review found that phonic starts a new gain smoother at 1 even for velocity zero,
causing a brief audible transient. Zero-start phonic voices are now isolated and
masked while their source timeline continues advancing; a later positive Volume
command can reveal the correctly advanced source. The linear adapter initializes
zero gain directly. Expired silent samples are not restarted by a gain increase.
This intentionally corrects old zero-velocity renders; established nonzero M5 demo
WAV and manifest remain byte-identical (`artifacts/m6b-no-effects-check.*`).

### Core verification

`tests/sample_controls.rs` has **19 passing tests** for recognizable exact PCM:
mono/stereo offsets, last-real-frame reverse, 24/44.1/96 kHz input and MIDI pitch,
fractional cursor direction changes, active offset without retrigger, retrigger
cursor reset, exact hard cut and other-voice/older-tail isolation, decimal ties,
small-block partition equality, cold seek, reset loops/stop, zero callback
allocations, saved/reopened/exported/streamed equality, source/range/size/nonfinite
failures, clipping cleanup, embedded-loop rejection, and zero-start silence with
later gain raise. The earlier 15 M6a FX tests continue passing. Evidence:
`evidence/m6b-core-sample-controls.log`, `evidence/m6b-core-clippy.log`.

## Milestone 7 — bounded document instrument phrases, 2026-10-24

Implemented a reusable **document Phrase**, distinct from a `pattrns::Phrase`
generation object. No Renoise phrase/XRNS compatibility or recursive generation
is claimed. Acceptance remains subject to the integrated judge report.

### Content and persistence contract

`Song.phrases` is a stable-ID pool of `Phrase { name, instrument, rows,
lines_per_beat, note_columns, root_note, cells }`. A phrase is bound to one existing
instrument, has 1..1024 rows, LPB 1..256, 1..16 note columns and MIDI root 0..127.
Sparse internal cells reuse the normal note/velocity/pan/delay and six effect
fields. Their instrument must be omitted or match the bound instrument. Nested
phrase triggers are rejected, including in unused definitions.

A song cell's optional `PhraseTrigger { phrase, cycles }` explicitly invokes a
phrase. Cycles counts total iterations: 1 is one-shot, 2..64 is finite repetition.
The cell must contain a pitched note. That pitch supplies a semitone offset from
the phrase root; **both inner note pitches and glide targets** transpose and must
remain within MIDI 0..127. This changes individual sample pitches, not phrase row
spacing or duration. An omitted trigger instrument selects the phrase's bound
instrument regardless of earlier parent selection, and remembers it for later
ordinary omitted-instrument notes. An explicit mismatch is rejected.

The trigger accepts velocity/pan, delay, timed release and hard cut. Trigger-level
retrigger/glide/offset/direction are rejected rather than given ambiguous all-child
meaning; edit internal phrase cells for those operations. All six effects are
supported inside a phrase. Trigger velocity/pan are immutable instance snapshots:
inner local gain is multiplied by trigger gain, and local plus trigger pan is
clamped to [-1,1]. Local memory remains separate, so repeating cycles do not
compound gain. Later parent parameter-only volume/pan selects values for future
notes/triggers; it does not modify a running instance. Parent glide/offset/direction
against a running instance is an explicit evaluation error, not a silent no-op.

Missing phrase bank/trigger fields load empty and serialize omitted. Project
version/song schema remain 1. Atomic model/editor operations validate definition
replacement, cell edits and deletion; referenced removal, unsafe shrink, owner
mismatch and transposition invalidation are rejected without partial edits.

### Clock, instances and ownership

The phrase clock uses song BPM and its **own LPB**, independent of song LPB and
trigger pitch. Each event frame is rounded from the combined absolute unrounded
trigger origin plus phrase cycle/row/delay fraction, not from separately rounded
durations. Trigger delay shifts the instance origin. True events after song end
are dropped; only in-range rounding spill clamps to the final musical frame.
A late zero-offset initial child follows the already-clamped trigger frame.

Each trigger occurrence creates an independent instance with independent inner
column selection, gain/pan memory and voice IDs. Memory persists across that
instance's finite cycles, but never leaks to another instance or the parent.
Root song actions win same-frame ties before phrase child actions, so parent
OFF/replacement/cut cancels pending simultaneous child starts deterministically.
Internal cells retain their existing cell/retrigger/release/cut ordering.

- Parent OFF, replacement by a normal note, or another phrase trigger cancels
  future instance actions and releases only its currently active inner voices.
- Natural final-cycle end releases the remaining active inner voices. Earlier
  release tails are allowed; song end truncates future work and releases active
  voices into the existing fixed render tail.
- Parent hard cut kills **all voices and older tails owned by that instance**, not
  unrelated instances or song voices. This broader instance scope is deliberate;
  ordinary note cut still addresses just its current logical voice. Internal
  phrase-column cut retains ordinary single-voice semantics.
- Stop at exactly the trigger frame cancels initial children before they start.
  If no actual note starts anywhere, rendering retains the existing explicit
  "Song contains no note triggers" error; it does not invent a silent note/WAV.
- Cold seek reconstructs no past voices. Later compiled internal notes/retriggers
  still start with resolved instrument, transpose and snapshot attributes. Whole-
  song loops reset the Engine and instances. No song-wide note flush is used.

Bounds are enforced before large materialization: 128 definitions, 32768 total
stored phrase-definition cells, 256 trigger occurrences per evaluation, 65536
combined expanded root/child actions, and 2048 actual note starts including inner
retrigger starts. Expansion is clipped at musical end and finite cycles only;
the action budget is conservative even for actions later canceled by parent OFF.

### Shared rendering and evidence

`src/song_render/phrases.rs` compiles ownership-resolved events into the same
Engine used by offline export and streaming. Phrase schedules use manifest schema
4 / `leviathan-song-v4`; optional event provenance records phrase ID, instance,
inner column and cycle/row (owner-level stops omit cycle/row). Only an actually
ordered trigger selects this evaluator: unused definitions or unused-pattern
triggers leave the old evaluator, event ordering and manifest version unchanged.

`tests/phrases.rs` has **17 passing tests** covering real transposed PCM, unchanged
phrase duration, independent LPB/fractional rounding, concurrent instance memory,
noncompounding repeated gain, parent snapshot parameters, root-before-child OFF
and replacement, instance-tail cut isolation, all internal effects, song-end
truncation, empty/canceled cases, validation and expansion budgets, saved/reopened
and streamed/offline equality, cold seek, deterministic loops and Stop silence.
MCP/editor/UI verification is recorded separately. The judge's richer pre-M7
no-phrase fixture (all six M6 effects, fractional timing, active controls and zero
velocity) renders byte-identically as `artifacts/m7-no-phrase-baseline/after.*`
against its preserved `before.*` files.

Per the user's updated priorities, the next backend work is BPM-independent
sample slicing and editable slice-to-instrument mapping before matrix aliases or
polish. Transient detection, 70-to-170 BPM slicing, slice editing and gap/overlap/
choke policy are **not implemented or claimed by this phrase subchunk**. Current
immutable sample buffers and integer source-frame controls are reusable foundations,
not a completed slicing workflow.

M7 judge correction: trigger-cell timed stops bind their exact compiled instance. A hard cut later than a short phrase’s natural end still cuts that instance’s released tails; a late release is a no-op and neither command targets a replacement. Exact PCM regression verifies PhraseEnd at frame 94 and silence from scheduled cut frame 120. The phrase suite now has 17 passing tests.


## M8a — Verified immutable sample sources and non-destructive regions (cloud)

This follows the user's sampler/Break Compiler priority after M7, not a reset of
existing transport/tracker milestones. It reuses the existing Song, Editor,
frame scheduler, Engine and offline/streaming sinks. There is no second sequencer.

### Source identity and bounded WAV/FLAC import

`sample_asset::import` authorizes no paths itself: UI/MCP must apply their path
policy first, then run import on a worker. It returns `ImportedSample { asset,
buffer }`; the canonical absolute path and lowercase encoded-file SHA-256 identify
the **original bytes**, separately from native sample rate, channels and real
frame count. The original is never rewritten, normalized or globally resampled.
`load_verified` checks those bytes and decoded metadata before Engine reuse;
`verify_identity` provides storage's hash-only check. Event-only `evaluate` embeds
expected provenance but does not prove file identity; successful Engine creation/
render verifies used assets. Project storage separately verifies registered sources.

The direct `symphonia = 0.5.5` dependency exposes an already pinned transitive
package; no package/version change or new decoder stack. Unlike the legacy phonic
convenience decoder, this importer propagates packet corruption, truncation and
integrity errors instead of silently dropping bad packets. It checks declared
frames before allocating PCM, limits each packet to 65536 frames, checks actual
packet growth/format consistency and finite samples, and rejects missing/zero
frame counts. Therefore **legal unknown-length FLAC is not supported in this
bounded first importer**. WAV/FLAC only; AIFF/MP3/OGG are not advertised here.

Limits: 64 MiB encoded per source, mono/stereo, native rate 1..384000 Hz, at most
64 MiB decoded values including interpolation guards across distinct used source
buffers and controlled legacy buffers. Storage preflights registered-source files
with an additional 256 MiB aggregate encoded budget; Engine applies the same
limit to distinct used canonical source paths before any hash/decode. These are product limits,
not the full formats' limits. SHA-256 uses the standard FIPS-180-4 compression,
short/multiblock/million-byte vectors; the independent judge also compared 86
random/padding-boundary inputs with Python hashlib. It is content identity, not
an authentication/signature claim.

### Persistent regions and mapping

Additive schema-1 fields are omitted when unused: `Song.samples`, `Song.regions`,
and `Instrument.region`. Old sample paths still work; a raw instrument cannot
have both a legacy path and a region. Explicitly assigning a region clears the
legacy path; explicitly assigning a non-null legacy path clears the region.
A null legacy path does not silently clear an existing region; explicit unmapping
is available. Old documents load, but older binaries need not understand new
fields. There are at most 128 source records and 4096 regions.

`SampleAsset { path, sha256, sample_rate, channels, frames }` has immutable identity
under its SampleId. `SampleRegion { name, sample, start_frame, end_frame, root_note }`
stores integer SOURCE bounds **[start,end)**, with 0 <= start < end <= real source
frames and MIDI root 0..127. Atomic helpers register/remove sources, upsert/remove
regions and map instruments; dangling references (including unused regions) are
rejected. Region editing changes metadata only, never PCM. Source replacement
needs a new identity; relink UI is deferred.

All region voices share immutable decoded Arc buffers, deduplicated by encoded
SHA-256 after path/metadata verification. Within one Engine snapshot, a canonical
path/hash/native-metadata identity is read and hashed once even when 256 or more
instruments map to it. Distinct source paths are still verified before reusing
PCM. Unit counters prove 256 mapped instruments incur one read, hash and decode;
a distinct alias incurs another read/hash but not another decode, and aggregate
encoded-budget rejection performs none. Once loaded, PCM stays immutable even
if an external process changes the file; a new Engine rejects the changed source. They use the existing controlled linear
renderer from the original ON, with the interpolation window bounded to the
region: no neighboring slice sample is read. This is not production anti-alias
parity with phonic. Embedded source loops are not activated: these are explicitly
one-shot regions; ordinary legacy loop behavior remains unchanged.

Playback ratio is `source_rate / 48000 * 2^((note-root_note)/12)`; glide targets
use the same region root. **No target/source BPM ratio enters sample playback.**
Target BPM only places events on the existing sample-frame timeline. Offset stays
an **absolute original SOURCE frame**, not region-relative: out-of-region offsets
fail before output files are reserved. Forward defaults to start; reverse defaults
to end-1; retrigger restores that start/offset. Direction changes preserve the next
unread cursor. Exhausted voices cannot be revived by later offsets.

The existing linear cursor edge convention is preserved: forward interpolates
its final fractional position toward a virtual zero at end; reverse starts at
end-1 and stops below start, without a lower fractional guard tail. Consequently,
at 24k->48k a four-frame region renders eight forward frames but seven reverse
frames (a one-frame region renders two versus one). This is an explicit boundary
convention, not equal-duration reverse transparency or independent time stretching.

Existing zero-start silence, per-note volume/pan/pitch/glide, release and exact
hard cut remain available. Ordinary same-column replacement releases its previous
voice, multiple columns can overlap, and one-shot exhaustion leaves gaps. There
are **no new choke groups, configurable new-note/tail modes, crossfades, time
stretch, source-BPM inference, classification or seeded generation** in this
foundation. Those remain separate reviewable chunks.

### Evidence and remaining workflow scope

`tests/sample_regions.rs` currently has 17 real-PCM regressions covering WAV and
FLAC, 44.1/48/96k mono/stereo, hash/metadata changes, malformed/truncated headers,
NaN and file budgets, exact boundaries/offset/reverse/direction, root pitch/glide,
zero-start/retrigger/cut, clipping cleanup, old JSON omission/unused banks,
roundtrip, offline/stream equality, cold seek, loops and Stop. Three private Engine
unit tests prove shared Arc identity, bounded per-snapshot verification work,
distinct-alias verification and preflight rejection before IO-heavy work.
The checked-in tiny FLAC fixture is newly synthesized arithmetic stereo PCM,
not a sampled copyrighted recording; test execution needs no external encoder.

A fixture places source hits exactly 37800 frames apart at 44.1k (70 BPM), maps
four independent regions to editable tracker notes at 170 BPM, and checks both
target frame starts and every native-rate interpolated hit sample. Existing
bounded row-local retrigger represents 1/16, 1/32, 1/64 and 1/128 note rolls at
4 LPB; the test verifies all starts with one absolute beat-to-frame conversion.
This proves the core editable slicing/resequencing mechanism, not automatic
musical break understanding or listening quality. Transient suggestions and the
native waveform/editor/MCP integration have separate owners and evidence.

M8a no-region compatibility evidence: both the accepted M7 phrase-heavy fixture
(`artifacts/m8-no-region-baseline/after.*`) and the M6 all-effects fixture
(`artifacts/m7-no-phrase-baseline/after-m8.*`) compare byte-identically to their
preserved baseline WAVs and manifests. The only newly exposed dependency is the
already-locked Symphonia 0.5.5 package. Final integrated/UI/MCP receipts remain
separate; no physical audio-device or listening-test success is claimed in cloud.


M8a judge boundary correction: region voices and legacy **rate-converted
controlled** voices now derive source phase from an anchor and signed integer
output-frame count, multiplying native rate before division by 48000. This
prevents accumulated floating error from leaving an expired 44.1k voice just
below its end and allowing a boundary offset to revive it. Direction changes
preserve signed phase; explicit start/offset and actual glide-speed changes
re-anchor it. No broad epsilon truncates very-slow voices. Regressions cover
44.1k lengths 294/441/44100, nonzero starts, forward/reverse, exact final reverse
sample, late offsets and 1/37/512-frame partitions; legacy controlled paths have
matching exhaustion tests. This is an intentional fractional-rate correctness
change and may alter least-significant PCM rounding for those older controlled
voices. Ordinary phonic playback and 48k no-region controlled math are unchanged;
the accepted M6 and M7 golden fixtures remain the compatibility checks.

## M8b — accepted-marker break compiler (cloud gate PASS)

This is a bounded authoring layer over the existing Song/evaluator/Engine, **not
another sequencer, automatic beat detector or time stretcher**. It creates normal
editable pattern cells, a new track, one appended pattern occurrence, and one
region/instrument per accepted slice. Instrument allocation is not limited to the
four demo instruments. Independent pad-trigger UI and musical classification are
not part of this chunk.

### Persistent markers and explicit timing

`MarkerSetId` addresses a `MarkerSet { name, sample, source_bpm, markers }` in
`Song.marker_sets`. The additive map defaults to empty and is omitted when empty,
so older documents load without migration. As with other new fields, old binaries
are not promised to read new documents. At most 128 sets may each have 2..257
strictly increasing integer SOURCE-frame boundaries, including 0 and the exact
source end. Referenced source deletion is rejected, including unused marker sets.
`source_bpm` may be unknown (`None`) for editing, but compilation requires an
explicit finite 20..400 value. Beat zero is **file frame zero**. Neither tempo nor
downbeat is inferred from file name, duration, onset count or classifier output.

Analysis suggestions never overwrite manual boundaries. `suggest_marker_cleanup`
returns a separate review-only sorted proposal with reasons for duplicate,
near-start, near-end and too-close markers. It retains 0/end and never modifies
PCM or the persisted set. A 16-frame proposal at 44.1 kHz is only 0.363 ms after
start: choosing a 221-frame minimum (5 ms rounded up) proposes merging it with
zero, not deleting the source prefix. The user may accept, reject or edit the
proposal, or explicitly choose a shorter minimum. Compilation rejects any slice
shorter than its chosen minimum; there is no silent cleanup during generation.

`CompileTiming::SourceRhythm` maps each source marker to
`source_frame * source_bpm / (source_rate * 60)` beats. `Grid { division }` instead
places successive slices at a fixed named subdivision. Both are distinct from
sample playback: **the region root pitch/native sample-rate ratio is unchanged**.
Target BPM changes the GLOBAL Song clock, including any earlier ordered patterns;
LPB is unchanged, and preview/UI disclose that consequence. Patterns append rather
than replace the existing arrangement. Later marker edits do not rewrite earlier
compiled snapshots.

`CompileRoll { division, hits }` adds a fixed total of 2..16 starts per base hit,
including the base start. Subdivisions are exactly 1/16, 1/32, 1/64 or 1/128 of a
whole note (1/4, 1/8, 1/16 or 1/32 beat). The last roll start must precede the next
base hit/cycle end; otherwise preview rejects rather than spilling or overwriting.
Cycles are bounded to 1..16. Rolls are ordinary separate note cells, not hidden
runtime generation. Their spacing is not successive rounded-frame accumulation.

Compiler timing uses checked, reduced `u128` rational arithmetic. BPM values are
rationalized as their **exact stored binary f64 values**, not their shortest
printed decimal text. Integer 70/170 and named subdivisions remain exact. An
absolute source/grid/cycle position plus existing order prefix is rounded once
to an output sample frame. The compiler then encodes this frame in the existing
row + f64 delay model, moving by at most 32 adjacent representable row values to
resolve half-frame ties. If no in-pattern representation exists, preview returns
an error rather than moving an event to another frame. The shared evaluator and
legacy DSP are unchanged. The existing pinned serde_json dependency enables its
`float_roundtrip` parser feature: without it, ordinary generated f64 delays could
drift one ULP on reload (for example 0.0010582010582010583). No dependency version
or Cargo.lock bytes change. This is an intentional exact-state persistence fix,
not a special-case rounding of generated content. A regression pins source marker 147 at 44.1 kHz,
source 70 -> target 128, LPB 3 to frame **88**, including prefixed arrangements and
save/reopen preservation of corrected delay bits and actual PCM.

### Overlap, stopping and padding

* `Overlap` keeps each voice until natural exhaustion. Lane reuse waits for the
  rounded global ON frame plus the ceiling of its native output-frame duration;
  continuous beat duration alone is insufficient at fractional sample rates.
* `ReleasePrevious` emits a timed declick release for the previous generated
  voice, not the new voice. Its older release tails may overlap.
* `CutPrevious` hard-cuts the previous generated voice at the next hit. This is
  abrupt and can click; it is not a click-free choke-group implementation.

These policies only address generated voices. Existing unrelated voices are not
flushed. Source windows remain one-shot; adjacent slice PCM never leaks through
interpolation. Sparse-cell collisions allocate another bounded note column or
reject with a density error; no hit is silently overwritten. Up to 256 columns
are available. Stops use existing typed effects/OFF cells and remain editable.

With `pad_tails=true`, rounded musical end is at least the required integer native
end frame. Overlap and ReleasePrevious conservatively cover every generated
voice's native end (an older release can outlast a tiny final slice); CutPrevious
needs only the final voice's end. This may add conservative silence. Without
padding, normal SongEnd release applies and may alter unfinished native tails.
The export's 500 ms tail does not justify claiming the musical/loop boundary
preserves those tails. Preview reports sequence rows and padding rows separately.

### Preview, commit and bounds

`break_compiler::preview(&Song, &CompileConfig)` is pure and does no source IO.
Opaque `CompilePreview` retains exact base/candidate/config and a SHA-256 token,
with a bounded typed summary of generated IDs, counts, timing and warnings. Fresh
IDs use checked max+1 allocation, never overwrite or recycle an existing ID.
`preview.commit(current)` rejects a different base. Controllers additionally
check monotonic document revision, authorize source paths and verify immutable
source identity before publishing a commit. The Editor applies tempo, regions,
instruments, track, pattern and order as **one undoable transaction**. Preview is
non-mutating; edit/undo-to-same-content still invalidates a stale controller token.
No arbitrary serialized candidate Song is accepted from an MCP client.

Generation caps the whole arrangement conservatively at 2048 starts (including
existing phrase expansion), 4096 regions, 256 new-track columns, 65536 generated
pattern rows and ten minutes. Allocation/timing/density errors leave the original
Song untouched. Clipping is rejected by the existing render path, not silently
normalized. Source root note, per-note pitch/volume and existing effects remain
editable after commit. There is no random seed because this version makes no
random decisions; seeded mutations, choke groups, layers, stretch and drum labels
remain deferred.

### Evidence and limits

`tests/break_compiler.rs` currently has 14 regressions: a complete synthetic
four-beat 70-BPM source (151200 frames at 44.1 kHz, 3.42857 s) compiled to 170 BPM,
source-native pitched PCM, exact starts 0/16941/33882/50824, all four roll divisions,
explicit cleanup, unknown-tempo rejection, grid/source-rhythm distinction,
independent instrument allocation, sparse collision handling, exact old-voice
release/cut, conservative tail padding, stale/overflow/budget errors, global tempo
disclosure, fractional-rate lane reuse, native-ceiling padding and rational timing
roundtrips. Editor, project, MCP and actual native UI tests are separate evidence.
The 384 kHz/24025-source-frame and dense 44.1 kHz/26-frame-slice regressions were
added after independent judge PCM repros exposed premature release at boundaries.

Private user recordings are additional explicitly labeled manual-tempo hypotheses,
not proof of detected BPM, correct downbeat, perfect onset detection or drum type.
Their original bytes remain immutable and neither originals nor derived audio are
release fixtures. Waveform/PCM inspection does not establish subjective listening
quality. Cloud testing makes no physical audio-device or listening-test claim.


M8b independent final gate: **PASS** for the implemented compiler workflow.
Receipt: `evidence/judge-compiler-final-20261024/judge-receipt.json`.
273 core and 320 desktop tests passed; 540 exact stored-f64 rational timing cases
also preserved complete Song JSON state. Both independently reproduced native
padding/overlap edge PCM differences are now zero. Three accepted compatibility
fixtures reproduced byte-identical WAVs and manifests, including native 44.1 kHz
regions, phrases and all six effects. Source manifest SHA-256:
`a46dc2625d9983b4668db144e9cb4b9ad56d7614a40cb9817d013034c4148a8a`.
Evidence bundle SHA-256:
`d932b77f5e4c093344c705f046c19891f7897046377d3519d2077850c4bce495`.
This does not accept a physical-host/listening gate, pad bank, or staged onset-v2
prototype; those remain separate scope.

## Drum-kit base — sixteen single-layer pads (cloud review in progress)

This batch adds a real typed drum-kit instrument over the existing Song event
scheduler and shared offline/streaming Engine. It does not add a second pattern
format, sampler clock, MIDI/plugin host or physical pad-audition path.

### Persistent model and reference safety

`Song.drum_kits` is an additive default-empty/omitted map of nonzero stable
`DrumKitId` to `DrumKitInstrument { name, pads }`. There are at most 128 kits and
16 pads per kit. Each `PadId` is stable within its kit and independent of visual
slot or selector note. Slots 0..15 and MIDI selector notes 0..127 must be unique.
A pad stores name, optional RegionId, native `root_note`, integer
`tune_semitones`, gain, pan, OneShot/Gated mode, Overlap/ReleasePrevious new-note
policy and optional choke group 1..16. A kit may be empty or a pad unassigned
while being authored, but using an absent/unmapped selector is an error, never
silence or a fallback pitched sample. Kit/pad names are bounded to 256/128 bytes.

`Instrument.kind` defaults to Pitched and is omitted for all old instruments.
A kit uses the tagged representation `{"type":"DrumKit","data":{"kit":1}}`.
A kit assignment is exclusive with legacy sample/region fields. Explicit kit,
region or legacy-sample assignments switch kind deliberately; a null legacy path
preserves an existing kit. Normal empty new fields do not churn old project JSON.
Old schema-1 projects remain readable, not necessarily new projects in old builds.

Shared Song helpers atomically upsert/remove kits/pads and assign/unassign kits;
Editor and MCP use them rather than separate validation. References include
unused patterns, phrase owners and incoming column instrument memory. Ordered
patterns use their actual occurrence selection history, so an unused kit pattern
cannot reinterpret an otherwise pitched arrangement. Unused patterns use a
conservative set of observed incoming selections for their track/column; an
explicit instrument disambiguates an omitted one. Used kit arrangements cap this
validation at 65536 expanded ordered cells, above the existing render-cell cap.
Removing a referenced kit, pad selector, source or region is rejected atomically.
Renaming/reordering a stable pad does not change the selector or sample identity.

### Selector, native rate, tuning and controls

A tracker note selects a pad; it does **not** transpose sample playback. The pad's
root is its native pitch reference, replacing the region reference for this
voice. `tune_semitones` defaults to zero/omitted, ranges -48..48 and must leave
root+tune in MIDI 0..127. The emitted playback pitch is root+tune, giving
`source_rate / 48000 * 2^(tune/12)` source frames per output frame. No BPM ratio
enters this calculation. Pads reuse the bounded linear region interpolator, not
production anti-aliased resampling or time stretching; large upward tuning can
alias. Root alone is not a misleading tuning knob. Explicit
Glide targets remain absolute MIDI targets relative to the native root, without
adding tune a second time.

Pad gain (0..1) multiplies the resolved note/phrase gain; pad pan (-1..1) adds to
resolved pan and clamps. Initial kit gain/pan are applied immediately so a low or
zero pad gain cannot leak a gain-1 startup transient. This is kit-only behavior;
legacy pitched voice ramps are unchanged. Subsequent Volume/Pan controls use the
existing smoothing and recompute from column values plus fixed pad controls,
never compound across retriggers. Existing equal-power panning can raise one
channel; clipping still errors rather than normalizing. Offset/reverse/retrigger,
glide, timed release and hard cut use the actual selected region/source window.

### Voice lifetime, grouping and phrase ownership

Different pads can overlap even within one tracker column: automatic column
replacement/retrigger OFF does not choke a kit voice. The pad's explicit
`new_note=ReleasePrevious` instead declick-releases older held voices of that
same stable pad. Overlap keeps them independent. Gated means ordinary OFF ends
the current logical voice early; it does not enable sample looping. OneShot
ignores ordinary OFF while dropping the column's active control pointer, so the
sample continues naturally and later empty-cell controls do not retarget it.
Timed release and hard cut explicitly override either mode. A hard cut remains
abrupt and may click; the base pad policy does not expose hard-cut choking.

An arriving grouped hit declick-releases older held voices in the same group,
BEFORE its own ON. Grouping dominates the pad's Overlap policy and applies across
tracks and phrase instances of the **same InstrumentId**. Assigning one kit
definition to another instrument gives an independent choke scope. Already
released tails are not re-released or hard-cut; they finish their existing fade.
Natural exhausted logical voices tolerate harmless releases. Zero-gain hits still
start/advance and choke; they are not optimized away. Simultaneous order follows
the shared deterministic event order, never wall-clock/UI timing.

Kit-associated phrases initially require a neutral root trigger: selector-note
transposition is rejected rather than accidentally choosing another drum. Their
internal notes are ordinary pad selectors. Owner-level phrase stops enumerate
all owned kit voices, including older overlapping selectors: Gated PhraseEnd/OFF
releases all held voices, OneShot ordinary endings leave played tails, timed
release overrides both (including a trigger-owned release after the short phrase
clock has already ended), and parent hard cut kills every owned voice/tail without
touching another instrument/instance. Pending child scheduling still cancels by
existing phrase rules. SongEnd releases every remaining kit voice, not merely
those that remain a column's most recent active pointer. Cold seek, loop reset
and Stop still discard old voices; OneShot does not promise tail carry across a
transport reset. An unfinished tail may be truncated at the musical loop boundary;
kit mapping does not automatically extend Song rows.

### Shared Engine, resources and evidence

The existing evaluator assigns frames/voice IDs; a bounded kit-resolution pass
adds ownership-specific releases and resolves selectors. Manifest schema 6 is
used only when a kit voice actually starts. `pad_voices` records each voice's
instrument, kit, stable pad, slot, selector, gain/pan/tune/policies and verified
path plus source/region/native-root metadata. Legacy assets/regions and manifest
schema/bytes are unchanged when no kit starts.

Engine asset keys distinguish each kit instrument/pad instead of aliasing all
pads under one instrument key. Immutable decoded sources still share the existing
verified SHA-256/path cache and 64 MiB decoded / 256 MiB aggregate encoded budgets.
A unit counter proves sixteen mapped pads on one source incur one read, hash and
decode while retaining sixteen distinct asset mappings. All voices are allocated
on the worker before playback; callback code remains only bounded PCM copying and
atomics. Existing 2048-start/ten-minute limits remain, expanded kit events cap at
65536, and no voice stealing is introduced. Rendering failures clean up outputs.

`tests/drum_kits.rs` currently has 15 real-PCM/behavior regressions, including
sixteen distinct synthesized source files, native 24/44.1/96 kHz, +/-12 and +/-48 tuning,
1/37/512-frame partitions, immediate gain/pan and memory composition, OneShot vs
Gated, different-pad/same-pad overlap, zero-gain choke and shared-kit instrument
isolation, multi-hit phrase ends/override/hardcut, selected-source effects, exact
project roundtrip/offline-stream equality/cold seek/loops/Stop, mapping/reference
safety, unused kit/pattern transparency, voice budget and clipping cleanup.
These are synthesized test signals, not a claimed classified real drum dataset.

Layering, velocity zones, round-robin/random/probability selection, per-pad FX
chains, time stretching, live audition/recording and external MIDI/plugin hosting
are deferred. The optional 4x4 bank selects or explicitly inserts ordinary cells;
selection alone is not an audio trigger. Physical listening/host acceptance stays
separate from cloud PCM and framebuffer evidence.

Native first-kit workflow: Project > New creates an empty pattern with four
unassigned instruments; Pads can create a kit and explicitly map source regions.
For an existing populated song, the native/MCP `create_drum_kit_instrument` path
allocates a NEW instrument and empty kit together with checked max+1 IDs, one
validated commit and one Undo. Existing assignments, patterns and cursor are
preserved. Exhausted ID namespaces and invalid names leave the Song unchanged.
Assigning an empty kit over a used pitched instrument remains correctly rejected
until referenced selectors can be resolved; no existing notes are silently
cleared or reassigned to make the operation succeed.

### Post-review kit status — 2026-10-24, final2 PASS

The drum-kit base described above has now passed its independent cloud gate,
including the populated-song fresh-instrument workflow. Receipt:
`evidence/judge-kit-final2-20261024/judge-receipt.json`.
345 core and 410 desktop tests passed; 19 independent PCM probes passed;
three no-kit compatibility WAV/manifest pairs remained byte-identical.
The final binary also reproduced the private test-side thirteen-pad conversion's
WAV and manifest exactly, without changing the original recording. This remains
numerical evidence, not automatic slice classification, measured recording tempo
or listening validation. Source manifest SHA-256:
`6cd0d3bcff3b3792bcc00f8b0213936548bf0b48a8bdee0c9ad09912da47fb01`.

This status note was appended after sealing that receipt. Its historical reviewed
document hashes and evidence files are left untouched; subsequent documentation
changes belong to the next stage's review snapshot. The physical-host/listening
and deferred layering/audition/MIDI/plugin limitations above remain open.


## Original milestone 8 — track-block aliases and occurrence variations (in review)

This is the original sequencer matrix milestone, distinct from the intervening
M8a/M8b source-slicing/compiler labels. It adds content sharing and occurrence
mute variation, not automation, a mixer, DSP effects or another sequencer.

### Persistent content and explicit editing

`Pattern.track_aliases` maps a target `TrackId` to `TrackAlias { pattern, track }`.
`Occurrence.muted_tracks` is a set of track IDs local to that occurrence. Both
fields default empty and are omitted when empty, preserving old schema-1 project
JSON. New files with these fields are not promised readable by older binaries.
All links use stable IDs; aliases may cross tracks. Source and target must have
**identical row counts and note-column counts**, at every link: no implicit repeat,
padding, truncation, time scaling or column remapping. An aliased block cannot
also have local rows. Missing source/target IDs and cycles, including unused
patterns, are rejected. Iterative resolution permits at most 32 edges; the shared
inspection API returns the target-to-terminal chain. No recursive lookup is used.

Alias playback reads terminal cells into the target track/column runtime: source
content is shared, but instrument/volume/pan memory, active notes, phrase instances
and voice ownership are not. Source edits intentionally affect every alias. The
tracker displays resolved content read-only at an aliased target; normal cell,
FX, phrase-trigger and drum-step edits reject with a Make unique/Edit source
instruction. Edit source explicitly selects the terminal pattern/track; when the
source has no ordered occurrence it cannot be silently appended by navigation.
Make unique copies terminal content locally, then removes the link. Unlink removes
only the alias and leaves an empty local block. Whole-pattern Clone materializes
all aliases so the established independent-copy promise remains true.

Shared helpers validate a candidate before publication. Row/column resize and
source deletion cannot invalidate incoming links. Explicit and locally inherited
kit-selector references remain protected even in muted blocks. A muted note with
no local selection remains incoming-context dependent; unmute revalidates it. Complete occurrence records
(including mutes) move by the existing **swap** semantics. New repeat occurrences
are unmuted; legacy MCP `set_order` replaces the order with fresh unmuted records,
not an attempt to guess occurrence identity from repeated pattern IDs.

### Mute timing and lifetime

A muted occurrence suppresses that track's cells and parameter updates. At its
start frame, pending phrase work on that track is cancelled and all still-held
voices on that track receive a de-click release, including overlapping kit
OneShots whose ordinary column pointer was already dropped. This is **not hard
silence**: already-releasing tails continue and newly released notes fade. Other
tracks survive even when they use the same kit instrument. This does not add
track-level automation, hard mute, choke groups or effect routing.

Before-mute instrument/volume/pan memory remains available after unmute; values
inside the muted occurrence are not latched. A prior row's delayed action that
rounds exactly onto the mute boundary is suppressed **before** parameter memory
or kit choke resolution. Phrase children at that boundary do not start, and an
instance cancelled by mute does not resume on unmute. Row timing still uses the
single existing absolute sample-frame authority and source sample rate is unchanged.

Explicitly muting all ordered note/phrase triggers produces a finite silent
Engine/export over the unchanged musical duration plus existing export tail,
with no triggered assets or source reads. This is a narrow exception for muted
musical content; an ordinary blank Song still has the existing no-note error.
Project/MCP workspace path authorization can remain stricter and inspect registered
path metadata before Engine construction; it does not mean the Engine decodes
suppressed sources.

### Bounds and compatibility evidence plan

At most 4096 stored aliases, 65536 resolved alias cells and 65536 resolved alias
row containers are accepted, including unused links. Terminal size counts are
cached; empty sparse row containers cannot amplify unbounded validation work.
Ordered alias blocks, resolved alias cells/row containers across occurrences, and
occurrence mute entries each cap at 65536. Existing
32768 populated-cell, 65536 expanded-action/event, 2048-start and ten-minute render
limits remain. Compiler existing-start accounting resolves aliases and ignores
muted blocks; generation remains ordinary editable content.

Only ordered alias/mute use emits manifest schema 7, with bounded resolved-chain
and mute-frame provenance. Unused aliases do not change ordinary output/schema.
The same evaluator feeds offline export and streaming. The review includes exact
materialized-reference PCM, target-memory/phrase/kit isolation, rounded-boundary
choke prevention, all-muted missing-source silence, cold seek/loop/Stop, graph and
expansion bounds, persistence/undo and four previously accepted WAV/manifest
goldens. Acceptance is pending the independent final-source gate; this section
is implementation documentation, not a claim that that gate already passed.


### Post-review matrix status — 2026-10-24, PASS

The original milestone 8 alias/mute chunk above passed the independent final-source
gate: `evidence/judge-matrix-final-20261024/judge-receipt.json`. **392 core / 472
desktop tests** passed, including 66 real MCP subprocess tests. Independent review
covered 15 core groups, 48 fractional-clock cases, PCM partitions 1/37/512, 22
exact UI/MCP Song/manifest/matrix parity cases and 24 final-source EGL captures.
All four previously accepted WAV/manifest golden pairs remained byte-identical;
normal tracker crops changed zero pixels. The production Engine section,
controlled sampler and transport remain byte-identical to the prior accepted
chunk; no dependencies were added. Source manifest SHA-256:
`42267824989769fe2c77116002b82170b67290da1e08bea216dd5903ce4bf219`.

The checkbox/current-status corrections and this note were written **after** the
receipt was sealed. Its reviewed document snapshot and evidence hashes remain
unchanged; this current-document version belongs to subsequent review. Physical
host/listening remains open. Equal-geometry aliases, read-only targets, explicit
source/Make unique authority, de-click tails, context-dependent unknown muted
selection and separate stricter project/MCP path checks retain the limits above.
Original milestones 9 (groove/scoped transformations) and 10 (committed pattrns
generation) remain next, not implied by this acceptance.


## Original milestone 9 — scoped baked transforms (in review)

`transforms::preview` is a pure, bounded proposal over existing `Song` cells, not a
new sequencer, runtime/global groove state, DSP, random generator or asset reader.
It returns an opaque base-bound `TransformPreview`, exact candidate, deterministic
SHA-256 identity, effective config, bounded summary and before/after cell changes.
Editor/MCP commit additionally checks the captured monotonic document revision,
then applies one ordinary validated edit/Undo. An edited-then-undone document is
still stale. Identity transforms preserve the original Song and redo history.
The candidate cannot be supplied or substituted by an MCP client.

### Scope and independent field authority

A selection names one local PatternId/TrackId, half-open zero-based row interval
`[row_start,row_end)` and a strictly sorted unique nonempty list of note columns.
There is no implicit current-occurrence-only edit: shared pattern repeats and
track aliases are disclosed in the preview. Alias targets remain read-only;
explicitly edit their terminal source or Make unique first. Missing sparse cells
are not created just because a rectangle includes them.

Four independent masks select Notes, Glide targets, explicit stored Velocity, or
whole-event Timing. Parameters outside a mask are range-validated but ignored;
there is no requirement to clear draft controls when disabling a mask. Transpose
is -48..48 semitones. Note transpose requires an explicit or locally established
pitched instrument; it rejects kit selectors and unresolved incoming selection
rather than retargeting a pad. Pitched phrase trigger notes retain the existing
phrase-transposition semantics; kit phrase triggers do not transpose. Glide target
transpose requires a locally proven active pitched voice, not merely the latest
instrument selection. Instrument-only cells never reassign that active voice;
OFF/release/cut clears its logical handle. Kit/unknown/phrase-active glide targets
reject. Notes and Glide targets can deliberately be transformed independently.

Velocity scale is finite 0..4 and changes only existing `Some(value)` fields.
Omitted/inherited values remain omitted. Changing a selected explicit velocity
also changes column memory: later notes with omitted velocity may audibly inherit
the new gain, including beyond the selected rows. No automatic materialization or
restoration commands are inserted to pretend audible isolation. A result outside
0..1 rejects the whole preview; no normalization or clamping. Notes/targets outside MIDI 0..127 also
reject. Pan, source offset/direction, instrument references, other columns and
unselected cells retain their stored content unless their whole event is moved.
Pure preview does not decode/render audio or promise that increased polyphonic
gain will avoid the existing export clipping guard.

### Declared baked timing convention

Nudge is an integer displacement of -48000..48000 **output frames**, independent
of source sample rate. Optional swing uses the named whole-note divisions 1/16,
1/32, 1/64 or 1/128 and amount 0..50 percent. Its grid starts at pattern row zero,
classifies each original cell using its stored delay before nudge, and delays odd
bins. Exact bin classification is
`floor((row * division + floor(delay * division)) / (LPB * 4))`; power-of-two delay
scaling avoids losing a near-boundary fractional value in a large row sum.

Swing duration is computed from the exact stored-f64 BPM rational, then rounded
**once to an integer output-frame displacement**. That integer plus nudge is
added to each original scheduled frame. This preserves the original off-grid
frame phase; it is not continuous-time per-hit swing rounding, quantization,
Renoise compatibility or a grid snap. Even swing bins and net-zero displacement
preserve exact existing delay representation, including None, Some(0), subnormal
values and near-one fractions. A new preview of already transformed content is
a deliberately cumulative edit, not a hidden reversible runtime setting.

Timing moves the whole selected cell and adjusts its row/delay and row-local
release/cut fractions so its note/control, retriggers and timed stops retain the
same integer displacement in every current repeated/aliased occurrence. Retrigger
spacing, glide rate and other command values are not changed. An effect group
that cannot remain representable in one destination row rejects; no stop is
clamped, dropped or silently left at its old time. Nonzero timing of a phrase
trigger is initially rejected rather than claiming all child clocks shift;
phrase-trigger zero timing remains an exact no-op.

The timing proof applies at the captured Song BPM/LPB and current ordered/alias
uses. Later tempo or LPB edits rescale these ordinary baked row/delay values;
there is no persistent frame-offset object holding the displacement constant.
The ideal destination must lie inside the selected row interval, before encoding
correction. At most 32 adjacent f64 ULPs either side are tried, without crossing
the intended destination row. Corrections cannot squeeze an excluded endpoint or
out-of-row timed stop back into a near-one delay. Every current use must produce
the requested raw scheduled frames, or preview reports an explicit representation
error. This is a scheduled-frame proof, not a promise of unchanged actual event
counts: moving across an occurrence-mute boundary can intentionally change whether
a rounded event is suppressed by that existing mute policy.

Destinations must be unique. Moving into another selected cell's simultaneously
vacated slot is allowed when the whole move is injective and preserves order
(e.g. rows 1,2 become 2,3). Unvacated notes, scalar-only cells, OFF and explicit
empty entries cannot be overwritten or merged. No transformed event may overtake
another stored cell in its column, including unselected scalar/OFF cells; this
preserves the column's instrument/volume/pan memory sequence. Exact row-key order
is compared rather than adding a near-one f64 delay to a large row number.

### Bounds and acceptance scope

Selections cap at 4096 stored cells, affected ordered uses at 4096, ownership/order
scans at 65536 cells, and timing work at 65536 event-by-use checks plus an absolute
4194304 frame-comparison ceiling during bounded encoding search. Existing Song,
phrase/kit, alias and render resource limits still apply. Errors identify the
source row/column and collision destination where applicable; no partial edits
are published. The exact base/candidate/config SHA identity supports bounded
cached diff inspection; all file authorization remains outside this pure helper.

The native preview/commit window and MCP tools call this same module and Editor
guard. No new persistent transformation schema or evaluator path is added:
results are ordinary editable notes, scalar fields, effects, rows and delays.
Seeded humanize, stochastic variation, advanced scale/chord tools and runtime
global groove are deferred, with no enabled controls in this subset. The next
pattrns-generation stage is separate. Tests and independent review are underway;
this section is not a final gate PASS or a physical listening claim.

## Post-seal M9 acceptance note — 2026-10-24 08:06 UTC

The independent scoped-transform gate passed: 426 core / 518 desktop tests,
five byte-identical WAV/manifest goldens, 15 independent core groups / 220 clock
cases, and 28 actual-input EGL captures with 26 UI/MCP parity scenes. Receipt:
`evidence/judge-transforms-final-20261024/judge-receipt.json` (SHA-256
`633bb2e50b41acd8ae8894d45e1906f4ebfa06277ed1ea9f5160319a433745db`).
This is a later status correction, not a rewrite of the sealed reviewed-doc
snapshot. Runtime global groove, seeded humanize, and physical host/listening
remain outside that accepted bounded transform subset.

## M10 reproducible pattrns bake — bounded cloud acceptance

The independent final2 gate passed on unchanged code: **469 core / 572 desktop
tests**, including 110 desktop-library and 85 MCP subprocess tests, strict
Clippy/format checks, six compatibility goldens, and 32 reviewed real-input EGL
frames with 30 UI/MCP parity cases. The accepted MCP catalog has 31 tools and
44 typed batch operations. Receipt:
`evidence/judge-generation-final2-20261024/judge-receipt.json`.
M5 physical audio, native OS-window/input, latency/dropouts, and listening remain
open; cloud pixels and numerical output do not substitute for those gates.

A nonblocking viewport follow-up remains: toolbar Undo/Redo restores exact Song
content and the editor cursor but may leave that cursor offscreen after
Commit & show; grid-keyboard Undo/Redo already reveals a changed selection.
This checkpoint does not include the subsequent toolbar-scroll polish.

`generation::preview` offers **one bounded Euclidean preset**, not a general
composer, arbitrary code/Lua runner, ML model, or live opaque generator. Required
version is `1` (`pattrns-euclidean-bake-v1`). The caller supplies an explicit u64
seed, nonblank name (<=256 bytes), 1..16 four-beat bars, an existing instrument,
1..16 ordered `{note, velocity}` choices, and a Euclidean preset: 1..128 divisions
per bar, 1..steps hits, normalized rotation 0..steps-1, and finite probability
0..1. Notes/selectors are MIDI **12..127** for this initial bridge, velocity 0..1.
Every choice must map to an assigned source; kit choices are pad selectors, never
chromatic transposition. Song BPM/LPB are captured but **not changed**.

This executes the actual pinned `pattrns` revision
`8505bfe09111d4123a202af5222466f55021db9f`: its
`euclidean(pulses, steps, rotation)` API (whose parameter names use the opposite
hit/total naming), `ProbabilityGate::new(Some(seed))`, and fixed note-sequence
emitter. The gate is locked rand 0.9.5 / rand_xoshiro 0.7.0 Xoshiro256++ seeded by
`seed_from_u64`. It runs one continuous seeded stream across all bars; the choice
sequence advances **only on emitted hits**, not silent ticks. Probability 0/1
consumes no random draws in the pinned gate. No entropy seed is allowed. Zero-hit
results reject explicitly, never insert a fallback note or modify the document.

The pattrns clock is deliberately synthetic: 60 BPM, 1024 ticks/second, one
virtual beat per logical step. Exactly `bars * steps` events (including silent
ticks) are consumed, each at `step * 1024` with duration 1024. This is not the Song
clock and avoids its f32-BPM/truncated-time bridge. A raw SHA-256 covers compact
serde JSON ordered records `{time,duration,note}`, where `note` is null or
`{note,volume,panning,delay}` with original pattrns note number/f32 attributes.
Actual emitted content/timestamps are inspected, not merely labelled pattrns.

Each global step maps to exact rational `4 * step / steps` Song beats, plus the
existing arrangement row prefix. BPM means the **exact stored f64 value**, not
its decimal spelling. The existing bounded compiler encoder finds a row/f64-delay
that produces the required once-rounded 48 kHz frame, or rejects. It may not
cross the intended row. A model-only entry into the **same** row/phrase/kit/mute
evaluator validates the complete candidate's 2048-start/65536-action/ten-minute
limits and checks every generated ON frame and musical end. It skips asset path
canonicalization/existence checks only; all actually used existing and generated
instruments must still have model source assignments (a populated unassigned
demo must be assigned or cleared first). Normal evaluate/Engine/export retain
all prior source checks. No decoding, source hashing, PCM/clipping, or physical
host verification is claimed by pure preview. Reproducible PCM additionally
requires unchanged verified source content and playback configuration; historical
generation metadata alone is not a frozen audio asset.

Output is one fresh checked-max+1 PatternId, one fresh TrackId, and one appended
unmuted occurrence. Existing definitions, order records, tempo, and LPB remain
unchanged. Every generated cell explicitly stores note, instrument, velocity,
and pan 0. Lowest free column **within each output row** prevents collisions;
columns are reused in later rows (at most 128 needed, within normal 256 cap).
This is not a promise to import pattrns note durations or preserve its monophonic
lifetime: ordinary same-column replacement, pad overlap/gate/choke, and SongEnd
semantics govern playback. Extending SongEnd can extend prior held ordinary or
OneShot tails and permit previously end-truncated phrase child notes to execute;
using the same kit instrument can choke previous tracks.
These consequences are disclosed in the shared preview warnings.

Generation also preflights traversal work **before full Song validation**:
`(existing order.len + 1) * (existing track_order.len + 1)` is checked and capped
at 65536 prospective ordered track blocks. Separately, each of the total ordered
resolved sparse row-container visits and cell visits is capped at 65536,
including empty/default/muted entries and alias uses. Terminal source counts are
cached; alias resolution uses the existing bounded, checked resolver. Exact
candidate counts are rechecked after creation, so a one-hit plan pays for one
actual row/cell, not a hypothetical 2048-hit reserve. These conservative limits
are specific to this generator preview and do not change ordinary Song/model or
playback acceptance. Failures identify the exceeded block/row/cell budget and
leave the document/history untouched. They prevent a small empty arrangement
from multiplying into excessive validation/scheduling work.

Both base and candidate must pass the shared pure `project::check_serialized_size`
preflight: a bounded counting writer uses the same pretty Document wrapper as
project save, including escaping, without allocating an over-budget byte buffer.
The 8 MiB cap is conservative for captured absolute paths because saving may
shorten them relative to its destination. A shorter SaveAs destination might fit
a conservatively rejected draft; no silent truncation is performed. Relative
path drafts are counted as written and still need resolution before save; asset
and path validation remains the save/export responsibility. Failed generation
returns no candidate or partial commit.

The opaque preview includes the exact base/candidate, required version/config,
absolute generated-frame/cell records, bounded summary, raw-stream digest, and
historical provenance. Editor/MCP/UI guard the whole base and monotonic revision,
then append everything in **one undoable commit**. They never accept a caller's
serialized replacement candidate. Repeating preview does not mutate state;
stale/failed previews do not consume redo. The resulting cells are normally
editable, saveable, repeatable, clonable, and aliasable, with no generator required
at playback.

Pattern's additive optional `generation` field defaults to None and is omitted
for all old/no-generator documents (schema remains 1). It stores the original
recipe, versions, seed, BPM/LPB/prefix, raw count/hash, and output-content hash.
It is **historical evidence, not live reference authority**: ordinary edits,
resizes, cleared content, instrument deletion after removing real references,
and independent clones do not regenerate. `generation::content_matches` reports
whether current rows/aliases/tracks still match; names and provenance itself are
excluded. Clone copies history; removing the pattern removes its history. Later
tempo/LPB changes or new repeats/aliases do not promise the original absolute
frame proof. Unsupported preset versions, silent output, out-of-range choices,
ID exhaustion, unrepresentable timing, and aggregate budget failures reject
without clamping, overwriting, voice stealing, or partial document mutation.

## Sampler v1 — opt-in region amp envelope and forward loop (bounded cloud acceptance)

This bounded continuation implements part of the supplied sampler plan's melodic
sampler milestone, not a new engine or a complete multisample sampler. The final
independent cloud gate passed **505 core / 622 desktop tests**, including 124
desktop-library, 22 sampler and 92 MCP tests, strict formatting/core and UI Clippy,
seven exact compatibility goldens, 19 independent groups, 20 directly inspected
EGL frames, 18 exact UI/MCP parity cases (185 requests / 183 tool calls), and zero
changed baseline-grid pixels. Receipt:
`evidence/judge-sampler-v1-final-20261024/judge-receipt.json`.
The accepted M10 and UI-polish receipts remain immutable historical checkpoints;
**physical-host, OS-window/input and listening acceptance remain open**.

Historical wording notes in the sampler v1 snapshot, **resolved and visually
verified in the later velocity-zone/RR gate below**: after Reload,
selecting an existing region still says “Preview only / Apply to register” even
though the region metadata is committed/restored. An expanded explanatory sentence
can ambiguously subject timed release to OneShot behavior; the engine, tests and
Pad help instead correctly treat **timed release as an override of OneShot**.
Neither note changes stored content or playback semantics.

### Additive region settings and edit authority

`SampleRegion` gains optional `amp_envelope` and `forward_loop` fields. Both
`None` values are omitted from schema-1 JSON and render manifests. Old documents,
ordinary one-shot regions and legacy phonic sources keep their prior behavior.
Unused option-bearing definitions do not change unrelated audio or promote its
manifest. Manifest schema 8 is used when a rendered region or pad carries these
options; its source metadata includes the exact settings.

`AmpEnvelope { attack_ms, decay_ms, sustain_percent, release_ms }` uses integer
milliseconds for attack/decay (0..10000) and release (0..500), and an integer
sustain percentage (0..100). Explicit `AmpEnvelope::default()` is A0/D0/S100/R50;
it is NOT the same as `None`: its release is linear, whereas `None` keeps the
existing exponential backend de-click. Invalid values reject, never clamp.

`ForwardLoop { start_frame, end_frame }` is an absolute original-source interval
[start,end), at least two frames, wholly inside the parent region. Bounds remain
integer source frames, not output frames, seconds or normalized percentages.
Both settings are non-destructive metadata; original audio bytes remain unchanged.

`Song::set_region_playback(id, amp_envelope, forward_loop)` replaces only these
settings through one validated transaction. The Editor wrapper is protected and
undoable; UI/MCP use the same authority. All instruments/pads using that region
see the change. Existing UI/MCP name/root/bounds edits preserve the options;
shrinking bounds around an incompatible loop rejects. Full-value `upsert_region`
remains a full replacement, not an implicit merge of arbitrary caller data.

Model validation checks ranges, bounds and references, including unused regions.
Playback combinations are checked against actual evaluated voice ownership after
phrase, kit and occurrence-mute resolution. A stored document may therefore pass
metadata validation but fail playback because its loop voices use unsupported
source controls. Public evaluate/Engine/export reject such commands before audio
decode or output creation; existing pathname authorization/canonicalization can
precede that rejection. The private model-only evaluator applies the same check
without filesystem I/O. Applying settings is not an audio-renderability guarantee.

### Exact amplitude and release convention

The current engine is fixed at **48 kHz output**: one envelope millisecond means
48 output frames, independent of source sample rate, pitch, glide, BPM or LPB.
There is no claim of generalized variable-output-rate support. Envelope state is
independent per voice and multiplies the existing smoothed column/pad gain and
panning; it does not bypass velocity, pad gain, or future parameter controls.

For a held voice at zero-based output frame n, attack lasts A frames and decay D.
During attack the gain is n/A (frame 0 is zero when A>0). At frame A the decay
starts at 1 and reaches sustain S exactly at A+D. Zero attack skips directly to
decay; zero attack and decay start at S. Sustain is a linear amplitude percentage,
not decibels. Zero sustain/velocity does not skip source advancement or choke
policy. A new retrigger has its own fresh envelope and source-prefix state.

An OFF at frame T samples the envelope's next-frame gain E(T), then a release of
R>0 frames emits E(T)*(1-k/R) for k=0..R-1 and becomes exactly inactive at T+R.
R=0 stops at T and **may click**. OFF during attack/decay does not jump to sustain.
A repeated release never restarts or extends an existing release. For opted-in
envelope voices, Choke, PadReplaced and TrackMute use an explicit fixed 2400-frame
(50 ms) linear safety release instead of the user release; this is distinct from
a hard cut and from the older exponential policy. All other emitted OFF reasons,
including ordinary replacement/retrigger, timed release, phrase end and SongEnd,
use the configured release. Voices with no envelope retain their older de-click.

Existing ownership and filtering remain authoritative: OneShot pads ignore
ordinary OFF/PhraseEnd; Gated pads honor them; timed release/cut and track mute
retain their existing overrides. Already-releasing kit tails are not re-released.
A hard cut is exact zero at its event frame and keeps existing logical-voice versus
phrase-instance scope. An unlooped source can exhaust before its envelope finishes;
no synthetic post-source tail is invented. The maximum explicit release fits the
existing 24000-frame export tail, without lengthening old exports. Transport loop
restart and cold seek keep their previous cold-reset/tail-truncation semantics.

### Forward-loop scope and source phase

The source prefix from region start to loop start plays once, followed by repeated
forward playback of [loop_start,loop_end). The loop continues during release; the
source after loop_end is not played as a post-loop tail. Loop-only regions without
an amp envelope still terminate through the existing de-click at OFF/SongEnd.

Interpolation at the loop's last frame reads the loop's first frame, never a
neighboring slice, source guard frame or post-loop sample. Native-rate phase uses
an anchor and integer output-frame counter. Wrapping uses constant-time modulo,
even when a high pitch crosses many complete loops per output frame; looping adds
no events and no unbounded wrap iteration. Glide reanchors the continuous cursor
only when speed actually changes. A Forward direction control does not reset
phase or envelope. Existing source-rate/root pitch semantics remain unchanged.

This initial loop implementation rejects offset commands and Reverse direction
on an actual loop voice, including later active-voice controls after instrument
selection changes. Envelope-only voices retain existing offset/reverse behavior.
Forward playback, pitch/glide and retrigger are supported. There is no crossfade,
ping-pong/reverse loop, post-loop-tail mode, embedded-loop import, time stretching,
filter, keyzone, layered sampler or voice-stealing implementation in this chunk.
Discontinuous loop endpoints can click; linear interpolation is not an anti-alias
quality upgrade or a click-free claim.

Existing source verification/cache limits, 2048 starts, 65536 actions and ten-minute
musical limit remain. All voice state is prepared on the rendering worker; no new
callback allocation, filesystem work, locks or second scheduler are introduced.
The separate general evaluator traversal-amplification follow-up remains open;
this sampler change does not broaden M10's generator-only work-budget guarantee.

## Drum source v1 — velocity zones and deterministic round-robin (bounded cloud acceptance)

This is a bounded continuation of the original sampled drum-kit plan, not stacked
layers, a second sequencer or a random generator. The independent final cloud gate
passed **538 core / 660 desktop tests**, including 130 desktop-library, 18 velocity/
round-robin and 99 MCP tests; strict formatting/Clippy/builds; nine exact compatibility
goldens; 638 independent exact-f32-bin cases and four distinct native-source PCM
oracles with zero error; 19 retained sampler analytic groups; 26 directly inspected
EGL frames; 24 exact UI/MCP parity cases (211 requests / 207 tool calls); 31 tools /
46 typed operations; 65,536-mapping pagination stress; and zero changed baseline-grid
pixels at both sizes. Receipt:
`evidence/judge-velocity-rr-final-20261024/judge-receipt.json`.

Both prior sampler UI wording notes are visibly corrected: registered region
selection after Reload is labeled as committed, and ordinary OFF pad policy is
separate from the timed-release override. No new UI wording caveat was found in
this bounded gate. Prior sampler/UI-polish receipts and source archives remain
immutable historical snapshots. **Physical-host, OS-window/input and listening
acceptance remain open.** No random/stacked/crossfade/keyzone/full-sampler claim is
made; structural mapping caps do not replace the independent 8 MiB project-save
budget.

### Mapping, bounds and explicit source-mode changes

`DrumPad.velocity_zones` defaults to an omitted empty vector. Empty zones preserve
the existing optional single `region` mapping exactly. Nonempty zones are mutually
exclusive with that single region. A `VelocityZone { min_velocity, max_velocity,
alternatives }` has inclusive integer bins and an ordered vector of
`PadLayer { id: PadLayerId, region: RegionId }` alternatives. Layer IDs are nonzero
and unique across ALL zones within one pad, not globally across the Song; their
numeric order does not choose playback order. Alternative vector order does.

A populated map is a sorted, gapless, non-overlapping partition of all bins
0..127. There are at most 8 zones, 1..8 alternatives per zone, and 32 alternatives
total per pad. These limits are checked before flattening/clone/selection work.
Every alternative references an existing region, including unused alternatives
and unused kit definitions. Source/region deletion cannot create dangling mappings.
Duplicate region references under distinct layer IDs are permitted as explicit
repeated entries in the round-robin cycle. No velocity crossfade is implied.

`Song::set_pad_sources(kit, pad, region, zones)` and its protected Editor wrapper
replace BOTH source-mode fields in one validated, undoable transaction: Some
region plus [] is single-source; None plus complete zones is zoned; None plus []
is explicitly unassigned; both populated rejects. Clearing a used selector remains
subject to the existing reference checks. `DrumPad::has_source()` and
`source_regions()` are the shared assignment/inspection helpers for the grid,
Editor, generator, UI and MCP; none independently select a playback alternative.

Full-value `upsert_drum_pad` remains a full replacement, not a hidden merge.
Metadata UI drafts must retain their existing zones. The legacy MCP pad-upsert
operation does not have a zones argument: on a zoned pad, null region preserves
zones, while non-null region rejects with an instruction to use `set_pad_sources`
for the explicit switch. No legacy operation silently discards a zoned mapping.
All old single-region documents retain their existing behavior and omission bytes.

### Selection clock, counters and amplitude

The selection input is the actual incoming ON's stored f32 volume after column
memory and phrase gain snapshot composition, BEFORE pad gain or gain smoothing.
The bin is `round(127 * f64::from(input_velocity))`: f32 conversion and multiplication
by integer 127 are exact in f64, and nonnegative half ties round upward. At exactly
0.5 the bin is 64; adjacent f32 values below/above that boundary remain distinct.
This quantizes **selection only**: the audible amplitude remains the original
continuous f32 velocity multiplied by existing pad gain/envelope processing.
Changing pad gain never changes the zone. A later Volume command updates its
already selected voice and column memory; it does not change that voice's source.

Each InstrumentId/PadId/zone has an independent zero-based counter, initialized
once at full arrangement evaluation. Each unsuppressed ON chooses
`alternatives[counter % alternatives.len()]` and advances the counter, including
zero-velocity, retrigger and same-frame immediately-cut hits. OFF, Volume, Pan and
Glide do not advance counters. Muted/cancelled actions, including delayed prior-row
spills rounded onto mute entry, do not consume a choice. Existing same-frame event
order is preserved; no sorting by layer ID or separate timing clock is introduced.

Counters continue across note columns, tracks, repeated/aliased occurrences and
phrase instances sharing one instrument/pad/zone. Different InstrumentIds using
the same kit definition have independent cycles. Distinct velocity zones also
have independent cycles. Pattern/phrase boundaries do not reset them. This is
round-robin, not RNG: there is no seed input, stochastic mode or persisted counter.

Choices are compiled into the shared manifest before playback. Cold seek clears
old voices but retains the choices obtained by evaluating from the arrangement's
start, including ordinals consumed by earlier skipped ONs. Transport looping
replays the same compiled cycle rather than advancing an external round-robin
state between transport iterations. Save/reopen and equivalent snapshots select
the same sources; editing earlier hits may intentionally change later choices.

### One voice per hit and selected-source provenance

Every hit still has exactly one source and one logical voice. Existing pad root
note plus tune is authoritative for every alternative: a region's own root note
does not override the established kit pitch policy. Native source rate, region
bounds and optional region ADSR/forward-loop settings come from the chosen region.
There are no new per-layer gain, pan, tune, probability or modulation controls.
Pad gain/pan and ordinary effects retain their prior semantics.

New-note and choke ownership stays at InstrumentId/PadId/group level across
alternative changes, never at LayerId level. An old voice keeps its chosen source,
independent cursor/envelope and original release policy; a new hit does not mutate
old voices or reselect their source. Asset keys distinguish each selected layer
identity within its pad/instrument, while immutable PCM still shares the verified
source cache. No cross-voice envelope/loop state is shared.

Used zoned voices promote the manifest to schema 9 and include optional
`layer_selection` containing stable layer ID, exact input velocity, velocity bin,
zone bounds and zero-based round-robin index/ordinal. The ordinary per-voice region
metadata remains the actual selected region and its source identity/settings.
Unused zoned definitions and old single-region voices do not add this field or
change default manifest/audio bytes.

Unsupported loop+offset/reverse checks follow the actually selected region.
An unused loop alternative alone does not reject a hit, but any selected
incompatible alternative rejects the whole evaluation/render without fallback,
partial sound substitution or output creation. Normal pathname authorization can
precede that check; model-only evaluation remains free of asset I/O. All source
identity, memory, start/action, clipping and cleanup policies remain in effect.
This chunk adds no source-decoding work to the audio callback, no voice fanout,
stacked simultaneous layers, random choice, velocity crossfade or host audition.

## Groove timing v1 — reusable accepted-marker templates (bounded cloud accepted)

This is a bounded timing-only extension of the accepted shared-cell transform
pipeline, not completion of the original groove-analysis/composition plan.
`src/groove.rs` captures explicit accepted marker timing and bakes applications
into ordinary tracker cells. There is no decoder I/O, automatic beat/tempo/meter
inference, accent transfer, class labels, humanization, stretch, live global groove
or second sequencer. The broader unfinished inventory is in `FEATURE-STATUS.md`.

### Capture authority and frozen reusable storage

`Song.grooves` is an additive, default-empty/omitted map of nonzero `GrooveId` to
`GrooveTemplate`; old JSON stays unchanged. At most 64 templates, each with
1..256 anchors, are supported. A capture explicitly selects a persisted accepted
marker set, start/end marker indices, subdivision (1/16, 1/32, 1/64 or 1/128),
1..1024 cycle steps, name and version 1. The marker set must already contain an
explicit source BPM; no tempo is guessed from recording length or filename.

The window-start frame is the declared beat-zero origin. The end boundary is
always structural and excluded. The start boundary becomes a rhythm anchor only
with explicit `include_start_anchor`; the UI default is off. Every selected
internal boundary is an explicitly user-declared timing anchor, not a detector
claim of an isolated drum attack. An empty anchor set rejects without inventing
an attack at zero. A window may include trailing audio longer than the declared
cycle: cycle length is explicit, not inferred from file duration. All included
anchors must fit; no anchor is silently truncated, wrapped or merged.

Each raw delta is the accepted marker frame minus the window-start frame—not a
difference from a rounded nominal grid timestamp. Its exact beat position is
`delta * captured_source_bpm / (source_rate * 60)`. The grid step is the exact
rational `4 / subdivision_denominator`; nearest-slot ties round upward. Two
anchors in one slot, a before-origin/out-of-cycle anchor, or a nearest slot equal
to the excluded cycle end reject. Frozen integer deltas plus the exact stored
f64 BPM and native rate are the active timing authority; no approximate beat
float is persisted. The inspector's signed `offset_beats` is explicitly an
approximate display of exact beat position minus exact nominal slot position.

Provenance retains the source/marker IDs, declared SHA-256, source rate/frames,
marker-snapshot digest, selected indices/window and start policy. Capture does
not reverify source bytes: this is historical declared identity, not a fresh
hash/decoder receipt. Changing/removing the original markers or source does not
regenerate or invalidate a template. Source IDs are historical, not live asset
references. The template—including its name—is immutable under an existing ID;
recapture requires a fresh ID. Deleting a template never deletes already baked
cells. Low-level identical registration is a no-op, but capture preview itself
always requires a fresh nonzero ID. UI allocation uses shared checked max+1.

### Timing application and exact bounded representation

An apply selects one stored template and an explicit existing pattern, local
track, sorted note columns and half-open row interval. Aliased target blocks
remain read-only; Make unique or edit the actual source explicitly. The target
phase origin is `row_start`, and only ordinary `Note::Note` cells are eligible,
including kit selector notes without changing the selector. OFF/scalar cells are
not moved. The whole note cell travels with its instrument, velocity, pan and
supported effects; no missing field is materialized. Changed phrase-trigger
cells are rejected by the existing timing authority.

Classification uses each note's current phase and exact nearest grid tick, with
half ties upward. The explicit template cycle repeats across the selected span,
including a partial last cycle. Sparse unmatched slots leave cells byte-identical
and are counted in the review summary. Two matched notes in one column mapping
to the same global grid tick reject rather than merge. A matched grid anchor at
or beyond the exclusive selection end rejects rather than wrapping into the
prior row or another cycle.

Strength 0..100% **blends the existing phase toward the stored template phase**;
it is not an additive nudge. Thus 100% can straighten an already off-grid hit if
the stored offset is zero. Strength zero bypasses classification/encoding and
preserves exact `None`, tiny f64 delays, effects and sparse representation. A
zero-output-frame displacement also keeps the original cell representation.
Full strength avoids constructing a rational for the old phase, while partial
strength may reject tiny stored f64 fractions. More generally, exact fractional
BPM/delay/prefix combinations can exceed the checked u128 arithmetic range and
explicitly reject. Nothing is silently rounded to fit that limit.

Desired frames are computed with checked rationals at the captured song BPM/LPB.
Every changed cell must have the same integer-frame displacement across all
current ordered repeats and alias uses. Occurrence-dependent ±1-frame deltas
reject in v1 rather than weakening that proof. The existing transform encoder
then proves the same displacement for root/retrigger and row-local release/cut
commands, within its 32-ULP search bound. Ideal rational row bounds are checked
separately, preventing an out-of-range destination being squeezed into a
near-one delay. The encoded result must also retain its intended nearest grid
tick; an exact rounded audio frame that crosses a half-slot boundary rejects
rather than silently retargeting the cell on a later application. Some valid
musical requests are conservatively unrepresentable
and return actionable errors; 100% is not a promise that every transfer succeeds.

All occupied destination cells—including explicit defaults—and the complete
same-column scalar/OFF/instrument sequence remain protected. Simultaneously
vacated selected destinations are allowed only when mapping is injective and
order is retained. Every moved root/retrigger/row-local action preserves its
occurrence-mute eligibility in every affected track context; hidden suppression
or revival rejects. No collision merge, clamping, deletion or alias write occurs.

Ordinary evaluator policies are recompiled after the bake. Preserving stored
notes/pad mappings does **not** guarantee the same selected round-robin source:
changed cross-column/track interleaving can change RR choices, choke order,
phrase replacement timing and overlap. Moving an ordinary replacement note can
expose or cancel pending phrase children, including children near mute/end bounds.
The preview proves stored-cell/timing constraints, not unchanged expanded event
count or full renderability. Source assignment/decoding and expanded child/action
budgets are still checked by evaluation/export; pure authoring does not require
all sources assigned. The mute proof concerns the moved cell's scheduled commands,
not every existing phrase child. This is not a promise of identical audio except
for a time shift. Later song BPM/LPB edits rescale ordinary row/delay values;
there is no persistent absolute-frame offset or runtime template dependency.

### Preview, resources and remaining limits

Capture and apply each return an opaque exact-base candidate, config, identity
and bounded review summary; apply also returns exact before/after cell changes.
UI/MCP keep one pending Capture/Apply plan: a newer preview supersedes the older
one. Commit requires both current editor revision and exact captured Song equality,
then performs one validated undo transaction. No-op commits preserve redo; failed
previews/commits do not mutate the document. Captures/applies do not alter audio
files, tempo, note selection, velocities, kit definitions or unrelated stored cells.

Both previews check the exact pretty project size before expensive validation
and again before publishing the candidate (8 MiB encoding limit). The existing
bounded ordered-work preflight is reused here: at most 65536 ordered track blocks,
resolved row containers and cells, counting defaults/mutes. Apply additionally
bounds selected notes to 4096, relevant ordered uses to 4096, visited cells and
action/context checks to 65536, and inherits the transform encoder's bounded
comparison budget. These are groove-entrypoint constraints, not a claim that the
broader public evaluator's known traversal-hardening follow-up is complete.

### Independent bounded acceptance — 2026-10-24 UTC

The unchanged-code gate at
[`evidence/judge-groove-final-20261024/judge-receipt.json`](evidence/judge-groove-final-20261024/judge-receipt.json)
passed **580 core / 708 desktop tests**, including 136 desktop-library, 22 groove
and 110 real-process MCP tests; strict format/Clippy/builds; **10 exact compatibility
goldens**; 3,984 independent rational cases, 649 slot/idempotence cases and 13 guard
probes; four native PCM strength cases with zero error; and **38 directly reviewed
fresh EGL frames / 36 exact UI–MCP cases** (736 requests / 734 tool calls), plus
seven protocol guard cases (21 requests / 19 calls). Both baseline grid crops have
zero changed pixels. The shared catalog has **37 tools / 47 typed operations**.

This accepts only the timing-transfer contract above. The detailed conservative
representation/resource limits, historical-only provenance and RR/choke/phrase
consequences remain part of acceptance. It does not promise automatic musical
inference, full renderability, unchanged expanded-event counts or completion of the
broader original groove/composer workflow. **M5 physical output/listening remains
open.** Earlier receipts retain their own source snapshots and counts.

## Melodic multisample/keyzone v1 (bounded cloud accepted)

This is the original sampler plan's bounded **exclusive** note/velocity mapping
subset, not stacked layers, round-robin/random melodic selection, SFZ import,
release samples, key switches or a new synthesis engine. It shares the existing
Song clock, shared region references over immutable source audio, Engine and
streaming/export authority.
Physical keyboard/MIDI/audition and host audio acceptance remain separate.

### Persistent mapping and edit authority

`Song.multisamples` is a default-empty, omitted map of nonzero `MultisampleId` to
`MultisampleInstrument { name, zones }`. `zones` maps stable, local nonzero
`KeyZoneId` to `{ name, note_min, note_max, velocity_min, velocity_max, region,
root_note }`. Both ranges are inclusive MIDI-style 0..127. Root is also 0..127.
Limits are 64 maps, 128 zones/map and 4096 total zones; map names are nonblank and
at most 256 bytes, zone names at most 128. Caps precede overlap pair scans.

Two zones may share note ranges OR velocity ranges, but their rectangles may not
intersect in BOTH dimensions, including endpoints. There is no list priority or
ZoneId priority. Every region reference is validated, including unused maps and
zones; referenced regions/sources and assigned maps cannot be removed. Empty or
holey maps are valid authoring drafts. They do not imply silence or fallback:
every actual unsuppressed ON, including velocity zero, must find exactly one
zone or evaluation/export rejects before decoding/reserving output. Muted or
cancelled starts do not demand coverage. Coverage is a runtime event check, not
a promise made merely by storing or assigning a map. Editing/removing a zone can
therefore make existing musical content unrenderable until explicitly remapped.

`InstrumentKind::Multisample { map }` is additive. No-map JSON and unused-map
rendering retain their old representations. Maps are mutable shared definitions:
editing one affects every assigned instrument. A full-value copy under a fresh
map ID is independent mapping metadata but still shares region references and
immutable source audio; later shared-region edits affect every user.
There is no automatic copy-on-assign or new clone command. Fresh map+instrument
creation uses checked max+1 IDs and one atomic edit, preserving occupied content.
Whole-map replacement allows adjacent ranges to change in one transaction
without invalid intermediate overlaps. The new nullable assignment API detaches
only an existing Multisample on `None`; Pitched/DrumKit assignments are unchanged.
`Some` explicitly replaces the prior source kind and clears legacy path/region.
Existing assignment APIs retain their established behavior.

### Selection, pitch and lifetime

Selection uses the real emitted ON note after phrase transposition, and its
pre-source f32 velocity after column memory and phrase gain snapshot. The bin is
`round(f64(velocity) * 127)`, nonnegative half ties upward; this chooses a region
only and never quantizes audible amplitude. Zone IDs, map order, repeated
patterns and track order do not establish selection priority. There is no RNG
or round-robin counter. A retrigger is a new independent voice and resolves the
same deterministic inputs again; later Volume/Pan/Glide do not reselect its
source, even after a future instrument-only selection.

Unlike drum selector notes, these notes are actual MIDI pitches. Each zone's
explicit `root_note` overrides its region's native-reference root: native rate
is heard at that zone root, with playback ratio
`source_rate / 48000 * 2^((note - zone.root_note) / 12)`. Region root is never
applied a second time. Target BPM does not alter this ratio. Whole-note
transposition may intentionally select a different zone or expose a coverage
gap. An absolute Glide target instead changes pitch on the original selected
region/root, without remapping across key/velocity boundaries. The model keeps
legacy single-source routing/JSON checks distinct from melodic pitch semantics.

Region source bounds, ADSR and optional forward loop are copied into independent
per-voice state. Existing ordinary pitched-region gain/pan smoothing, zero-start
silence, column replacement/retrigger release, OFF, phrase-end/owned stop,
TimedRelease, hard cut, track mute and SongEnd behavior remain authoritative.
There are no new map-level choke, OneShot or NNA settings. Different note columns
can overlap; the same column still replaces through the existing release path.
Already releasing tails are not restarted, hard cut is abrupt, and the selected
region's natural exhaustion wins over a non-loop envelope tail. The existing
500 ms export tail and fixed 48 kHz output remain unchanged.

Offset bounds and loop+offset/Reverse rejection follow the actual chosen voice,
not a later selected instrument or an unused zone. Used incompatible commands
reject before decoding/output creation; public pathname authorization may occur
first. Only selected source files are loaded by Engine; project open/save retains
its stricter all-registered-source policy. These are bounded mono/stereo linear
interpolation voices, not a new high-quality antialiasing/resampling claim.

### Rendering identity and bounds

Used map voices add `Manifest.multisample_voices` with map/zone, actual input
note/velocity/bin, inclusive ranges and resolved region/root/hash/rate/settings.
Schema 10 is selected only when a map actually contributes a voice; unused
mapping metadata does not promote old manifests. Asset keys include instrument,
map and zone, so simultaneous voices from one instrument cannot cross-wire
roots or region settings. Verified decoded PCM remains shared by source identity,
with existing distinct-path verification and encoded/decoded budgets. Selection
and allocation occur during control-side compilation/construction, not callback
processing. Existing 2048-start/65536-action and arrangement limits are retained;
this mapping adds no extra voices, events, new voice stealing or second clock.
Cold seek keeps precompiled choices but does not revive skipped starts. Transport
loops replay the same compiled mapping choices and reset independent voice state.


### Independent bounded keyzone acceptance — 2026-10-24 UTC

The unchanged-code gate at
[`evidence/judge-keyzones-final-20261024/judge-receipt.json`](evidence/judge-keyzones-final-20261024/judge-receipt.json)
passed **611 core / 749 desktop tests**, including 147 desktop-library, 14 keyzone
and 118 real-process MCP tests; strict format/Clippy/builds and **11 exact
compatibility goldens**. Independent probes cover 2,307 rectangle/bin cases,
953 phrase-bin cases, 12 PCM groups / 48 cases, four extreme-root cases and
19 guards. **38 fresh EGL frames** were directly reviewed, with **36 exact UI–MCP
parity cases** (802 requests / 800 calls), plus 14 protocol workflows (39 requests /
37 calls). Both baseline grid crops have zero changed pixels. The shared catalog
has **38 tools / 52 typed operations**.

The accepted subset is the exclusive mapping contract above, including mutable
sharing, explicit fresh copy, coverage errors and selected-source pitch/lifetimes.
Structural caps remain separate from the 8 MiB save budget. The only nonblocking
capture note is a lingering waveform-help tooltip in the narrow Inspect view;
no data or interaction defect was found. The original sampler M2 remains **PARTIAL**:
full multisample-piano/editor/quality/import and broader overlap-layer workflows
are not claimed complete. **M5 physical host/listening remains open.**

## Scoped occurrence render/resample v1 — bounded cloud accepted

This bounded original-plan slice prints **one selected occurrence, all tracks** into
one new reusable asset through the existing Engine. It is not a mixer, stem
export, arbitrary row/track selection, format converter or new sequencer.
`resample::prepare` is pure; `render` is explicitly side-effectful; registration
is a separate protected, full-base/revision-guarded Editor transaction. No new
persistent recipe schema is introduced and no earlier receipt is rewritten.

### Cold scope, exact boundaries and gain

The captured occurrence retains its mutes and full alias/definition closure but
starts at frame zero with cold column/phrase/kit state, including restarting
phrase instances and deterministic drum round-robin counters. This is **not a crop of the
running arrangement**: no preceding voices, instrument/gain memory or outside
choke targets are imported. An unresolved incoming instrument fails with an
explicit cold-context error. Absolute rounding restarts at this new origin.
Normal phrase/kit/keyzone/retrigger/mute and SongEnd rules remain authoritative;
no phrase children start beyond the selected musical end. `MusicalOnly` writes
exactly the musical frames and can truncate/click. `ReleaseTail` adds exactly
24,000 output frames (500 ms at fixed 48 kHz), not silence detection or an
unbounded natural-tail promise. The whole newly registered region includes the
chosen printed tail; no tempo, beat origin, onset marker or loop is inferred.

The sole gain policy is explicitly **pre-master compensated float32 stereo**:
assert the Engine master is still 0.5 and write its actual PCM multiplied by 2.
This is not normalization, clipping, limiting or a global master change. Engine
nonfinite/clipping rejection remains in force. Stored values may exceed ±1 and
external consumers can clip them. The receipt reports post-master and asset
peaks separately. Native-root, unity-velocity, center-pan, no-added-envelope/loop
replay held for the full asset, with neutral effective replay track mix, reproduces
the captured output under this same master. Printing already bakes the captured
track mix; reapplying attenuation is not compensated. Arbitrary replay pitch,
velocity, pan, envelope, track mix, shorter lifetime or
additional voices do not have that equivalence. A printed release tail is already
baked: replaying it in the old shorter occurrence triggers a second release at
that earlier SongEnd. Hold through the entire printed region for neutral reuse.
Repeated neutral reuse is tested
against actual Engine PCM, not a mathematical signed-zero ideal.

### Immutable output, publication and history

Configuration is versioned and explicit: occurrence, nonblank name (at most 256
bytes), MIDI root 0..127, tail policy and an absolute `.wav` destination. Pure
preparation validates path syntax but performs no source/output pathname IO. Printing
requires an existing canonical non-symlink parent and a never-existing target,
including rejecting target symlinks. The worker authorizes declared source paths
before decode and the output before work and again immediately before publication.
Actually used legacy `Instrument.sample` paths are rejected with import/register
region guidance; they are not silently converted to different DSP. Used registered
paths must be absolute. Unused legacy metadata remains untouched. Existing MCP
root and project all-source policies remain stricter than selected Engine loading.

Printing uses a unique owned sibling temporary WAV, strict native importer/hash
verification and final candidate validation, then an atomic **no-clobber hard
link**, never overwrite-rename. No `.events.json` companion is created. A retained
open temporary-file handle plus Unix device/inode identity prevents cleanup from
mistaking a recycled pathname for worker ownership. Output-parent identity is
rechecked before publication and later verification. **Publication v1 is Unix-only**;
platforms without this stable identity implementation fail closed before source
decode/temp creation. Windows-safe publication remains unfinished, not a tested
host claim. Path rechecks are not a capability sandbox against malicious concurrent
filesystem replacement; later external mutation remains a TOCTOU limitation.
There is no extra parent-identity recheck immediately before temporary creation
after Engine construction. Adding that check is a defense-in-depth follow-up;
stronger openat/dirfd-rooted authority is separately scoped, not supplied by the
current pathname checks.

Cancellation before publication aborts and attempts owned-temp cleanup only.
Cleanup failure reports the owned temporary path; a foreign replacement is never
deleted. Once publication succeeds, subsequent cleanup/directory-sync issues are
warnings in a successful receipt, not errors concealing an existing artifact.
Published WAVs survive cancellation, stale registration, closing/forgetting the
result, New/Open, and Undo. Registration first runs a prospective project-source
preflight (off-thread in the native UI; synchronous in MCP) using the same Save
constraints: every legacy path must be absolute/readable and authorized; every registered source (including unused
sources and the new WAV) must retain its hash, fit 64 MiB individually and
256 MiB aggregate. Exact pretty encoding is rechecked without writing a project.
Thus a successful all-muted print can still fail registration because an unused
source is missing/changed or the added WAV exceeds the aggregate storage budget.
No access policy is widened and the artifact is retained. Registration verifies
the output parent/non-symlink status and reauthorizes all required sources,
then atomically adds fresh checked-max+1 Sample/Region/Pitched-Instrument IDs in
one Undo without changing patterns/order or inserting a note. Redo restores
references only; it does not recreate a removed file. There is no filesystem/history
transaction, automatic deletion, overwrite confirmation or generic cleanup tool.

### Scoped work limits and provenance

Both the captured base and prospective/final candidate must fit the exact pretty
8 MiB project encoding. Before expensive validation/evaluation, this entrypoint
uses the existing 65,536 ordered-block/row-container/cell-visit preflight, including
empty/default/muted content. Existing musical/action/source-bank limits remain.
Printed stereo float32 frames are capped at **8,388,096** (a conservative 4,096-byte
encoded-header reserve within 64 MiB, also allowing the decoder guard). Checked
`all evaluated starts × printed frames` must be at most **268,435,456**, counting
zero-gain/retrigger/phrase starts and the selected tail without estimating short
natural source duration. These are conservative work limits, not latency claims;
shorten/split the occurrence or omit the tail when rejected. They do not change
general Engine acceptance or solve broader evaluator amplification globally.

A transient bounded receipt records algorithm/config, original occurrence/pattern,
clock and frame counts, source SampleIds/verified identities, allocated IDs, base
Song digest, actual output SHA/encoded bytes and both peak domains. Only ordinary
verified SampleAsset, full-range SampleRegion (no envelope/loop) and instrument
metadata persist. Source recordings are never rewritten; private user recordings
and derivatives remain excluded from public release fixtures. Physical listening,
device/latency/xrun validation, contextual cropping, multitrack stems, resample
history/provenance libraries and Windows publication are not claimed by this slice.


### Independent bounded resample acceptance — 2026-10-24 UTC

The unchanged-code gate at
[`evidence/judge-resample-final-20261024/judge-receipt.json`](evidence/judge-resample-final-20261024/judge-receipt.json)
passed **649 core / 799 desktop tests**, plus a separate **160-test ui-capture
library run**. Coverage includes 19 resample integration tests plus a budget unit
and 128 real-process MCP tests; strict format/Clippy/builds and **12 exact compatibility goldens**. **38 fresh EGL frames**
were directly reviewed. The 36 workflow comparisons (546 requests / 544 calls)
include four earlier MCP path-policy refusals and 24 byte-identical publication
WAV pairs; 108 input texts and 26 GUI WAV hashes were checked. Both baseline grid
crops have zero changed pixels. The shared catalog has **42 tools / 52 operations**.

Acceptance includes the separate all-source Register preflight, exact storage/work
boundaries, artifact retention and the cold/pre-master/tail semantics above.
It does not promote Unix-only path checks to a race-free sandbox, synchronous MCP
to async jobs, a short print to contextual cropping/stems, or bounded reuse to a
complete recursive render/provenance library. The original sampler M5 stays
**PARTIAL**, and **physical host/device/listening acceptance remains open**.

### Timing-point tempo / pulse-phase hypotheses v1 — bounded cloud accepted

This is a bounded advisory **periodicity analysis of declared timing points**, not a complete B3.2 audio tempo/beat/downbeat analyzer. There is no dataset, learned model, random search, live tempo map, meter/loop-length inference, drum classification or automatic BPM assignment. Source audio and all default rendering paths are unchanged.

- Evidence is either an explicitly selected accepted-marker window, or the complete current v2 Peak / RefinedAttack points from an opaque verified-decoding cache and exact effective onset config. Accepted end boundaries are excluded; the accepted start is included only by explicit opt-in. A genuine cached detector point at the window start is evidence, not a fabricated boundary. Cached analysis internally re-picks the existing cache (bounded normalization/picking/refinement, no new FFT/decode/PCM copy); dismissed UI suggestions are not silently filtered. Both modes use **unit weights**: detector confidence/strength is not tempo probability.
- Marker analysis reads no source file and reports stored identity only. Cached callers must authorize and hash-check the source before analysis reuse and again at Use (off-thread in UI, synchronously on the MCP request thread), and bind the exact current cache/result/config epoch as well as document revision/base. Completion rechecks those guards; failed verification leaves metadata unchanged. The shared core/Editor metadata plan remains pure. Marker-only Use remains historical, without source hashing/decoding and independent of unrelated onset-cache changes. The pure core performs no file IO; MCP retains its established static pathname/root preflight. The shared result binds source ID/SHA/rate/frames, half-open window, marker or cache/config identity, chosen point basis and complete sorted point digest. It represents that historical snapshot, not continuous file verification.
- Cached effective-config limits differ by adapter: native UI uses `max_frames = 16,777,216`; MCP uses `source_rate × 15` (its existing 15-second policy). Exact config SHA and opaque analysis/Use IDs can therefore differ. Each path is verified against the shared algorithm with its actual config; numerical analysis and full Song results agree in the reviewed cases. No limit is silently widened or normalized to fabricate identical hashes. Native preparation also retains its existing source-duration/decoded-value/cache/work limits; MCP rejects a longer full source rather than cropping it. Both exposed caches use at most 4,096 proposals. Raw onset-v2 library configs can permit more, but tempo rejects `max_suggestions > 4096` before repick instead of silently narrowing them.
- Local limits: source rate 8,000..384,000 Hz, window at most 300 seconds, at most 1,024 selected unique points (no truncation; exact duplicates reject), at most 4,096 cached points scanned, and at least four points before proposing a BPM. Very sparse/no in-range evidence is an inspectable insufficient result, never a fallback guess. These restrictions do not change the general SampleAsset model. Pure base pretty-document/work preflights precede validation, and Use checks the actual prospective 8 MiB pretty project size.
- Version 1 BPM authority is an integer in tenths, 200..4000 (20..400 BPM). Each pair votes for beat spans 1..8 with weight 1/span, using exact half-up integer tick rounding. A [1/4,1/2,1/4] histogram filter yields positive local maxima; equal plateaus are retained, sorted by computed f64 vote then lower BPM, and explicitly limited to 64. The conservative pair/phase work bound is checked before expansion (maximum 16,000,000 units); cached repick has its separate existing work limits.
- For each shortlisted BPM, phase searches 128 evenly spaced residues plus the first 16 distinct observed residues in ascending source-frame order, scoring **all** selected points. The circular triangular kernel has radius 1/8 pulse. Integer kernel sums give exact phase ties to the lower phase. Candidate phase is the exact rational source-frame offset `phase_numerator / phase_denominator` relative to window start, modulo one pulse, **not a downbeat**. Approximate displays are not timing authority.
- Pulse frames use one exact rational half-up rounding, never cumulative rounded periods. Only nonnegative pulse indices whose rounded frames lie inside the half-open window are displayed, at most 2,048; overflow rejects, never truncates. A nearest negative/end-excluded pulse can contribute circular fit but is reported separately and never inflates in-window occupied-pulse coverage. Nearest-index half ties go to the higher index, including negative values. Mean residual covers all evidence; span is last-minus-first divided by the candidate period; same-pulse multiplicity is reported, not merged.
- Independent displayed components are phase fit, interval vote/relative vote, matched points, distinct occupied pulses, predicted pulse count, coverage, outside-window matches, same-pulse multiplicity, span and mean residual. Composite rank is 0.60 fit + 0.25 coverage + 0.15 relative interval vote, deterministic f64 ordering then lower BPM/phase. Up to eight returned candidates are separated by at least 1 BPM; total/shortlist counts and truncation are explicit. Half/double/third/quarter and 2:3 / 3:2 relatives within 0.2% are labeled across the shortlist, including relatives absent from the returned eight. This is a bounded search, not exhaustive musical interpretation.
- Weak/Moderate/Strong labels are **timing-evidence heuristics, not calibrated tempo confidence**. Weak means fit <0.5, fewer than three occupied pulses or span <2 pulses. Strong additionally requires at least eight points, fit >=0.8, coverage >=0.5, at least six occupied pulses and span >=4; otherwise Moderate. Missing beats, syncopation, noise and alternative beat units remain explicit ambiguity; even strong timing evidence can support a different musical beat unit.
- Analysis is nonmutating. Only a separately reviewed candidate and explicit same-source target MarkerSet may form an opaque Use-BPM plan. Guarded commit changes that target's `source_bpm` alone, retaining exact marker/name/source/lock values, Song clock/order/cells, existing frozen groove templates and baked patterns. Phase is advisory and is **not** applied: compiler origin remains source frame zero. Exact analysis tenths convert to the existing f64 BPM field; future compilation uses that stored binary value, not the advisory rational phase grid. Future compilation/groove capture may therefore change; old material does not regenerate. Editor Undo/Redo and no-op redo preservation use the existing single-transaction path. No persistent hypothesis recipe or hidden generator dependency is introduced.

Synthetic point/cache fixtures and independent rational/ambiguity/authority checks passed within this bounded gate. General audio-quality, downbeat, tempo-map and physical-host claims remain open; this slice does not complete all original B3.2 items.

#### Independent bounded tempo acceptance — 2026-10-24 UTC

[`evidence/judge-tempo-final-20261024/judge-receipt.json`](evidence/judge-tempo-final-20261024/judge-receipt.json)
records **685 core / 846 desktop all-target tests**, plus a separate **175-test
ui-capture library run**. Coverage includes 13 tempo integration tests and four
unit tests, 177 Editor/project/Steps tests and 139 real-process MCP tests. Strict
format/Clippy/builds and **13 byte-exact compatibility goldens** passed. **50 fresh
actual framebuffer images** were directly reviewed; 48 native/MCP workflow
comparisons used 1,798 requests / 1,796 calls, with 194 exact numeric inputs and
two byte-exact GUI exports. Both full baseline images have zero changed pixels.
The shared catalog has **46 tools / 52 typed batch operations**.

No remaining presentation finding was reported in this gate. Different native/MCP
cached config caps and opaque IDs are explicit above; the comparison does not
pretend those identities are identical. This is accepted timing-point periodicity
and reviewed BPM metadata, not calibrated tempo probability, downbeat/meter/role
recognition, automatic phase application or a genre-quality benchmark. **B3.2
remains PARTIAL; physical host/device/listening acceptance remains open.**

### Native slice-feature catalog and reviewed roles v1 — bounded cloud accepted

This is a compact native-region measurement/curation slice of B3.1/B3.4 and sampler M6, **not** a generally reliable drum recognizer, clean-stem extractor, similarity index, trained classifier, grammar generator or listening-quality acceptance. No dataset/ML dependency, source editing, automatic label assignment, playback effect, new engine, root/BPM/marker/pad/sequence edit is introduced.

**Authority and storage.** A pure plan names one existing source and 1..64 strictly ascending unique RegionIds. Fresh analysis uses the existing strict verified native decoder at an adapter-authorized explicit absolute path; no CWD resolution. An existing opaque onset cache may supply the same verified native PCM without decoding, but **new feature FFTs are still computed**. Public client-shaped PCM/features are not accepted by the analysis API. Completed compact measurements detach from later onset picker/config epochs because only PCM/bounds, not novelty suggestions, were used. Adapters authorize/hash-check before cache reuse and again before Store for both routes; pure core/Editor commit is full-base/revision guarded.

`Song.region_catalog` is optional/empty-omitted, keyed by existing RegionId, at most256 entries. Measured FeatureSnapshot and explicitly user-authored ReviewedRole have separate bindings to source ID/SHA/native rate/channels/extent and exact region bounds. Source/bounds edits retain **stale historical** entries; names, root, amp envelope and loop changes do not stale measurements of unprocessed native audio. File-path relocation alone does not stale content binding. Deleting an otherwise unmapped region removes its catalog metadata atomically; Undo restores it. Historical source IDs need not remain live after a region is explicitly rebound. A model `current_match` is only equality of recorded metadata, **not** verification of current external bytes, a signed attestation or proof that arbitrary loaded JSON metrics were computed. Raw snapshots validate finite values, nullable groups, bounds, exact version/layout and known compact rule output, not cryptographic scientific truth.

**Bounds.** Local source rates8k..384k, region duration<=30seconds, sum of visited region frames<=16Mi (overlapping selections charged repeatedly), and conservative work<=500M units are checked before decode/FFT. Work is native frames×channels plus Σ(windows×channels×N×(log2N+2)); the additional2 represents mean/window-fill scans. This is a conservative entrypoint policy, not a latency guarantee. No first-N sample analysis or silent truncation. Existing encoded/native decoder caps remain. Analysis must produce a storeable exact pretty8MiB project before exposing a descriptor preview; explicit label choices recheck the actual candidate. Only compact metrics/rules are persisted, never unbounded curves.

**Measurements.** All reads use exact half-open original source bounds, ignoring interpolation guards and samples outside the region. Native finite f32 values above unity are allowed and measured in f64 without audio normalization/clamping. Full-region RMS uses compensated energy accumulation; peak and per-channel DC retain native units. Stereo spectra average **channel powers**, never a waveform downmix that could cancel anti-phase content. Descriptors are not pitched/ADSR/loop/mix-output measurements.

- FFT size N is the next power of two of ceil(rate×32/1000), clamped to256..16384; hop H=N/4. Centered periodic-Hann windows are anchored at native region offsets0,H,...<length. Outside-region positions are zero padding, not neighboring audio. Each channel removes its unweighted valid-sample mean before windowing; raw per-channel DC is reported separately. One-sided squared magnitudes have factor2 except DC/Nyquist and divide by N. Aggregated spectral power uses valid Hann-squared weights; boundary windows do not receive equal normalized weight. Effective N/H and native envelope-bin size are stored.
- Analysed bins are20Hz through min(20kHz,Nyquist), inclusive at the outer edges. Low is[20,180), mid[180,2000), high[2000,upper]. DC is excluded; Nyquist retains its one-sided factor1. Band fractions and power centroid use aggregated analysed power. Flatness is geometric/arithmetic mean of aggregated bin power with a relative floor maxPower×1e-12. Zero analysed power yields unavailable/None power, fractions, centroid and flatness—not a confident invented zero.
- Spectral **shape change**, not raw amplitude-flux strength, is positive L1 change between consecutive nonzero unit-sum analysed spectra. Only numerically supported adjacent pairs contribute; mean/max and comparison count are reported. A window is supported only when analysed-band power is strictly greater than total one-sided post-demean power (including out-of-band/DC/Nyquist bins) × the persisted `shape_support_floor_relative=1e-24`. Unsupported windows break adjacency. Raw aggregate band power and raw nonzero-window counts remain unchanged; separate supported-shape-window counts and warnings expose the distinction. This scale-relative numerical floor prevents out-of-band FFT roundoff from being amplified into unit-shape evidence; it is not an absolute loudness cutoff. One window or no valid supported pair yields None. These units depend on the declared layout and are not silently comparable across incompatible algorithms/configurations.
- Envelope bins are nonoverlapping round-half-up(rate/1000) native frames, anchored at region start; the final partial bin divides by its actual frame/channel count. Peak anchors are absolute native source frames with first-maximum ties; duration fields are native frames. Attack spans the first bins crossing >=10% and >=90% of the global envelope peak, but is None if the first bin is already active (left-censored). Tail is bin-quantized time after the global envelope peak to the last >=−20dB activity, **not RT60 or proof of a single clean decay**. An active final bin gives None completed tail and a conservative lower bound from peak-bin START to last-active-bin START, not the exact last sample. Secondary rises are disclosed. Hysteretic rise count (>=50% on, <25% off) is an envelope descriptor, not a proven count of musical hits.

**Suggestions versus review.** Five fixed sets of four named handcrafted rules expose support as the fraction of satisfied conditions, never a posterior/confidence probability. The hypotheses are kick-like, snare-like, closed-hat-like, open-hat/ride-like and crash-like. Support>=0.75 participates in the advisory status; short/silent/no-AC regions are Unknown, multiple qualifying families or multiple envelope rises yield Mixed, and one qualifying family is merely SingleHypothesis—not an accepted clean label. Frequency overlap, bleed, tonal sources and truncated boundaries remain ambiguity. Rules and warnings are versioned and inspectable, not a claim of genre-wide recognition quality.

Metadata inspection is model-only and requires no media IO. Manual-role setters likewise make no source-byte verification claim: the pure core/native UI can edit a role while media is unavailable, but MCP `edit_batch` still applies its normal all-reference pathname/root preflight and may refuse that document. This adapter distinction does not add hashing/decoding or bypass path authority. Measured Analyze/Store fresh verification remains separate.

Reviewed labels are separate explicit human choices: Kick, Snare, ClosedHat, OpenHatRide, Crash, Mixed or Unknown, with independent Unspecified/Normal/**manual contextual Ghost** articulation and a <=256-byte UTF-8 note. Absolute low source level never implies a ghost note. Store with no choices stores descriptors and preserves every existing reviewed label and its historical evidence hash. Explicit choices replace only selected labels. A manual metadata-only setter does not verify audio or upgrade stale measured evidence; unchanged binding+label is a no-op preserving existing provenance. Clearing features preserves a label, clearing a label preserves features, and fully empty entries are pruned. Future protected grammar must require an explicit currently bound human role; it must not silently consume a stale override or heuristic as musical truth.

Synthetic numerical, independent spectral/envelope, persistence/Undo, shared UI/MCP and no-feature golden checks passed within this bounded gate. Broad recognition, drum/bleed quality, listening A–B and all other unfinished original items stay open.

#### Independent bounded slice-catalog acceptance — 2026-10-24 UTC

[`evidence/judge-slice-catalog-final-20261024/judge-receipt.json`](evidence/judge-slice-catalog-final-20261024/judge-receipt.json)
records **720 core / 894 desktop all-target tests**, plus a separate **188-test
ui-capture library run**. Coverage includes 15 core catalog tests, 185 focused
Editor/project/Steps tests and 151 real-process MCP tests. Strict fmt/Clippy/builds
and **14 byte-exact compatibility goldens** passed. Independent checks covered
37 authored fixtures / 39 complete NumPy metric groups, 46 guards, five verified/
cache comparisons and eight wire workflows (75 requests / 73 calls).

**56 main native frames** were directly reviewed, with **54 UI-to-MCP comparisons**
(766 requests / 764 calls), two byte-equal GUI exports and zero changed pixels in
both full baseline images. One auxiliary long-path retry also passed, outside the
56/54 totals. The catalog exposes **50 tools / 54 typed batch operations**.

Nonblocking presentation limits remain explicit: R64 was actually reached and
selected, but its virtual row may scroll out after selected-detail reflow; Unicode
stress names may use fallback glyph boxes, while stable IDs remain readable.
These do not imply a source/history/selection failure or physical OS-input proof.
The shape-support floor is the scale-relative numerical policy documented above,
not a loudness cutoff or changed raw aggregate. Current-match is model metadata,
not fresh verification or signed truth. Human roles remain separate from rules;
no calibrated recognition, automatic mappings/grammar, audio changes or broad
musical-quality acceptance is claimed. **B3.4 remains PARTIAL; physical M5 remains
open.**

## Reviewed-role seeded THIN v1 — bounded cloud accepted

This is the first bounded original B6.2 mutation operator: **remove secondary hits**, not rotate, substitute, fill, a genre preset or a complete composition grammar. Only ordinary stored cells change. No live stochastic engine, new DSP, source edit, dataset or ML dependency is introduced; all pre-existing defaults and no-mutation PCM/JSON must remain exact.

- **Explicit scope and authority:** one local pattern/track, nonempty half-open row range and strictly ascending selected note columns. Alias write targets reject; shared source edits affect every current repeated/alias use. Only ordinary `Note::Note` cells with no phrase trigger and no nondefault FX may be removed. Non-ON/phrase/FX cells are retained with reasons. Every actual unsuppressed root ON use is resolved by the existing whole-song model evaluator, including source choice after velocity zones, RR and keyzones. Zero gain still counts; the `NoAudibleUse` wire reason means **no evaluated unsuppressed ON**, not measured silence or audibility.
- **Human-reviewed roles, not guesses:** every resolved region use must have an explicitly reviewed role whose source/bounds binding matches current model metadata. Any unreviewed, stale, legacy/unknown-source, Mixed or Unknown use retains the shared cell. The default protected concrete roles are Kick and Snare; callers explicitly supply the unique protection list, and an empty list disables only those concrete-role protections. Normal/Ghost articulation does not silently bypass protection. Suggestions, support scores and source amplitude never authorize an edit. Matching metadata does not prove current external bytes or signed scientific provenance. Muted/non-starting contexts are counted but do not fabricate a source choice; this proof concerns captured current uses, not future unmute/reorder/remapping.
- **Pinned seeded decisions:** config version 1, required `u64` seed, integer removal percent 0..100, and `max_removed` 0..4096. Effective gate probability is `(f64(percent)/100.0) as f32`, exposed exactly. The actual pinned pattrns `ProbabilityGate` runs once per eligible stored cell in canonical row-then-column order, after all-use eligibility is established; true means remove. Protected/unsupported cells skip the gate. Interior comparisons use the pinned gate's strict `>`; endpoints consume no RNG draws. Zero percent bypasses gate calls and edits, but still validates the scope, complete model and budgets. Eligibility/base changes can shift later seeded decisions. Reproducibility means the same base/config, **not repeated-apply idempotence**. All decisions are made before testing `max_removed`; excess rejects the whole preview, never silently truncates to a first-N subset or retries another seed.
- **Whole-cell deletion and proof:** remove selected cell entries only, retaining empty row/track containers and all unrelated model/definition/catalog/generation metadata. Every surviving ON must be the same ordered subsequence by stable stored origin, with exact frame, selected instrument/source/zone/layer/root, pitch and f32 gain/pan bits. Voice IDs and RR provenance ordinals may renumber, but selected alternatives may not silently change. Newly exposed/canceled phrase starts or inherited-selection changes reject. Retained Volume/Pan/Glide/Offset/Direction/SampleStart/**Cut** commands must keep their control origin, stable target-ON origin, frame/value and order; removed-voice controls may disappear, but no retargeted retained control is accepted. Any failed proof rejects the whole candidate, without partial fallback.
- **What protection does not mean:** OFF/release/retrigger/replacement/choke lifetimes may differ after deleting a trigger, including later OFF ownership. This is onset/source/control protection, **not full audio, tail or cymbal-continuity invariance**. No pitch, timing or automatic compensation is applied. Existing empty-song rejection remains; 100% may fail if it removes every evaluated start. No fallback note is inserted.
- **Conservative prerequisites and bounds:** base and candidate must be fully model-evaluable with all used sources assigned, even at 0%. This existing evaluator path performs no filesystem/source-byte/decode IO; it does not establish source existence, hash freshness, PCM finiteness/clipping or successful rendering. Exact 8 MiB pretty-document preflights and local 65,536 ordered block/row/cell work bounds run before expansion. A separate selected-source scan cap counts up to 65,536 row containers/cells, including unselected columns and stored defaults; at most 4,096 nondefault decision records and 4,096 affected ordered blocks are allocated. Existing 2,048 starts/65,536 action limits apply. Before evaluator provenance cloning, checked `2048 × maximum declared source-path encoded bytes <=16 MiB` bounds replicated path payload, including unused paths; this is not a global allocator/memory guarantee or a broad evaluator-policy change.
- **Review and reversal:** opaque full-base preview (including exact serialized-base SHA, so unchecked signed-zero changes are stale) plus Editor revision guard; failed replacement preserves the last review, and changed drafts may only show it read-only. Commit is one protected undoable ordinary-document edit; exact no-ops preserve redo/cursor. Bounded decision, actual-use, context and before→None deletion pages accompany a versioned config/seed/gate-revision/hash record. The record is transient, not a persisted live recipe or general durable audit history. Saved projects contain ordinary editable cells; generic persistent mutation logs, multiple candidates, protected genre grammar and broader original operators remain open.

Independent direct-pattrns/randomness, all-use/protected-role/control ownership,
synthetic PCM, persistence/Undo and bounded UI/MCP checks passed within this scoped
gate. Physical listening and musical-coherence acceptance remain separate open gates.

### Independent bounded THIN final2 acceptance — 2026-10-24 UTC

[`evidence/judge-thin-final2-20261024/judge-receipt.json`](evidence/judge-thin-final2-20261024/judge-receipt.json)
records **754 core / 942 desktop all-target tests**, plus a separate **202-test
ui-capture library run**. Coverage includes 14 core THIN tests, 193
Editor/project/Steps tests and 163 real-process MCP tests; strict formatting,
Clippy/builds and **15 byte-exact no-mutation WAV/events goldens** pass. The MCP
surface is **53 tools / 54 typed batch operations**. Independent checks include
954 pinned-gate cases / 482,859 decisions, 55 THIN groups / 966 bridge calls /
58,176 gate decisions / 58 full retained-manifest comparisons, 17 resource guards,
and ten protocol workflows (97 requests / 95 calls).

All **52 fresh native frames** were directly reviewed; **50 native-to-MCP
comparisons** made 896 requests / 894 calls. These cover 106 exact inputs,
40 successful preview snapshots, ten refused attempts, four disabled native
commits, two byte-exact GUI exports and zero changed pixels in both full baseline
images. Corrected exact-use records are reachable and copy-exact: 59 layered and
45 keyzone rows were visited at both sizes. Capacity frames have 256 columns,
16 records and 48 starts; 4,096-record layout coverage is a separate test, not
misrepresented as a captured 4,096-hit song.

The first full UI gate was rejected for clipped maximum-use tooltip content and
is **superseded, not accepted**. Only `thin_ui.rs` changed in the authorized reopen;
final2 reran the complete aggregate and retains the rejected measurements plus
corrected evidence. A runner no-clobber retry is preserved as validation history,
not a product failure. Existing Unicode fallback and the 60k-byte current-
pattern name's background toolbar wrapping remain presentation limits. B6.1–6.4
and S M6 remain partial/uncompleted beyond the one disclosed operator; durable
mutation logs, general grammar and physical-host/listening acceptance stay open.



## Device-free cloud transport diagnostics — bounded acceptance

Receipt: `evidence/judge-transport-stress-final-20261024/judge-receipt.json`.
This additive verification batch changes only `examples/transport_stress.rs`,
`tests/transport_stress.rs` and `tests/support/transport_diagnostics.rs`.
All 96 previously accepted build-source files remain byte-identical. The shared
Engine/Producer/Shared implementation and Cargo dependencies were not tuned or
replaced. Two new integration tests are included in **756 core / 944 desktop**
all-target results; the separate ui-capture library run has **202 tests**. Existing
163 MCP tests and 53 tools / 54 operations are unchanged.

The independent oracle covers 16 groups / 17 cases / 557 fill calls / 134,516
requested frames. Five new CLI processes cover 40 bit-exact same-Engine PCM cases,
80 acknowledged commands and **at least 160** post-command ready observations;
the observation counter is a guaranteed minimum consumed by liveness waits, not
total callbacks. Seven negative CLI/output checks pass. Dense 64-voice and mixed
existing-feature fixtures exercise bounded callback shapes and controls. Cold seek
preserves compiled choices rather than restarting a new randomization policy.
Sixteen unchanged compatibility goldens and two new default-frame captures with
zero changed pixels preserve prior behavior; previous THIN workflow captures stay
historical. No host or new transport UI interaction workflow was exercised.

No alloc/realloc/dealloc was detected inside measured `Shared::fill` calls. That
is not a proof of no locks, syscalls, contention or real-device deadline misses.
Artificially paced concurrent cases check bounded command liveness and output
safety, not exact inter-thread scheduling or a unique waveform. An earlier owner
run reported **769 missing frames** and is retained in the accepted evidence:
this cannot support a zero-xrun or ten-minute physical performance claim. The
prefilled deterministic matrix was exact/underflow-free. Missing-frame counters
are not driver xruns; nonzero PCM is not a speaker-audibility measurement.
Synthetic producer/device failure is explicit test-harness shutdown, not an actual
hardware error callback. Timings and the runtime-path rustc string/debug flag are
informational; the external build receipt identifies the actual build artifact.

The CLI requires one absolute new directory and creates only authored synthetic
fixtures/reports. It rejects existing/relative destinations, leaves owned evidence
on failure and writes an incomplete failure report when possible. Run under an
external timeout with no default features, as documented in CLOUD-VALIDATION.md.
It opens no device and consumes no user project/audio. **Physical M5 remains
unchecked**. The later separately reviewed four-session host smoke is partial;
further device/window/listening checks need explicit permission. No host installation, settings change, transfer or runtime improvement is
implied by this acceptance; the original feature inventory remains partial.


## Exclusive region/pad Audition v1 — bounded cloud acceptance

Receipt: `evidence/judge-audition-final2-20261024/judge-receipt.json`.
The pure `audition` module creates an immutable, bounded synthetic Song for the
existing evaluator/Engine/Transport; it is never an Editor candidate and has no
commit API. Region drafts, including imported-but-unregistered sources, preserve
native bounds/root/ADSR/forward loop without Apply. Pad audition resolves a
committed kit/pad with its real root, tune, gain, pan and selected velocity-zone/
RR mapping. Unity input 1 selects bin127; every fresh snapshot starts at RR0.
It is not audition of every zone/alternative or the current arrangement context.

One ON has a fixed 96,000-frame musical span and existing 24,000-frame tail at
48 kHz (120,000 total). Existing OneShot/Gated, release and loop policies remain
in force; this is not a held-key gate or variable-duration editor. Raw selected
paths must be absolute; loaded projects retain existing project-directory
resolution. Metadata preparation performs no source-byte IO; the native worker's
existing Engine loading verifies/decodes the chosen source. Names/path lengths
and a 256 KiB synthetic-metadata ceiling bound preparation. Other retained pad
alternatives remain metadata, not extra decoded voices.

The desktop owns one exclusive output session. Busy Song transport is not taken
over or mixed; pending/active audition disables conflicting Song controls.
Explicit Stop, focus loss, request/context changes and closing revoke pending
intent and stop owned audition. A loss/regain pair in the same frame still cancels;
regain never restarts automatically. Hard Stop can click and already submitted
hardware buffers cannot be retracted. Mouse release is not a gated-note command.
Read Mode permits audition without editing. Full document bytes, signed-zero
scalars, cursor, dirty state, revision and Undo/Redo are unchanged; Save/Open bytes
and original source audio remain exact. No persistent Song schema is added.

MCP exposes metadata-only preview, not GUI IPC, a playback capability or fresh byte
verification. Its target source must already be registered, unlike native imported
drafts, and normal authorization checks every retained alternative pathname;
native Engine byte loading uses only the chosen source. Region descriptive names
can legitimately yield different exact preview IDs; those IDs are not transferable
review capabilities. No MIDI, held-key performance, variable velocity, full RR
cycling, mixed audition or automatic break-to-kit allocation is included.

The final source gate passes **780 core / 981 desktop** tests plus a separate
**215-test ui-capture library run**, 169 MCP tests, 54 tools / 54 operations and
16 unchanged compatibility goldens. Main evidence validates 58 captures:
56 workflows and two zero-pixel full-image baselines. It records 52 shared snapshots,
50 exact MCP replays / 303 requests and 346,496 bit-exact diagnostic prefix frames.
Two separate unregistered Load/Read-Mode frames add two shared snapshots and 8,192
frames. Direct visual review covers 13 selected critical frames across these
captures. Independent manual-reference coverage includes 23 complete 120,000-frame
PCM cases and 51 direct MCP requests. Device-free diagnostics are not a host stream.

The first focus-pair UI gate and candidate1 compile are explicitly rejected/
superseded for acceptance and retained in final2 history. Only the authorized
`desktop.rs` fix changed between candidates. Corrected candidate2 passed an
isolated locked/offline `desktop-audio` compile/link; canonical BEFORE/AFTER logs
match all 103 build sources and 119 candidate payloads. The executable was not
launched and its reported hash is not runtime attestation. Prior four short
CPAL/PipeWire output sessions are separately reviewed, not a physical test of this
new audition workflow. **Full M5 remains open:** speakers/listening, actual audition
output, native GUI input, latency/deadlines/hardware xruns and long-run stability
are unverified. No further host operation is authorized by this cloud acceptance.


## Reviewed marker-to-drum-bank v1 — bounded cloud acceptance

Receipt: `evidence/judge-marker-bank-final-20261024/judge-receipt.json`.
The pure `marker_bank` preview takes version1, an existing accepted MarkerSet,
1..16 strictly ascending unique interval indices and a nonblank name of at most
256 UTF-8 bytes. Sparse selections are allowed; each index is exactly the adjacent
`[markers[i], markers[i+1])` source interval. No sorting, merging, truncation,
minimum-duration inference, fade, extraction, resampling or source-BPM requirement
is introduced. A selected interval retains any tail up to its next boundary.

Checked fresh max+N region IDs and max+1 kit/instrument IDs never reuse holes or
overwrite old definitions. Full base validation happens before allocation, so a
new ID cannot activate an old dangling instrument reference. New local pad IDs
1..N occupy slots0..N-1, selectors36..51, native reference root60 (not detected
pitch), tune0/gain1/pan0, OneShot/Overlap. No choke, velocity zones, ADSR/loop,
feature descriptors or reviewed roles are inferred or copied. The source asset is
reused unchanged; no source file, pattern, occurrence or persistent recipe is added.

Existing source metadata/audio, regions, kits/instruments, marker positions/locks/
source BPM, Song BPM/LPB, aliases/mutes, patterns/order and cursor are preserved.
Explicit Create commits the private plan once in Edit Mode after full-base exact
serialized digest and Editor revision checks; stale/ABA/ID/cap/reference failures
leave history unchanged. Undo/Redo restores ordinary metadata, not audio files and
not a rerun of source verification. Later marker edits/deletion do not regenerate
this bank. Subsequent region/pad edits use ordinary existing validated operations.
Open new kit is explicit UI-only navigation, not part of shared commit; no note or
audition starts automatically.

Pure core/Editor preview/commit do no IO and can carry relative model paths.
Native/MCP adapters use existing explicit project/import resolution and freshly
authorize/hash the selected source at Create, with the existing 64 MiB encoded
source bound; they do not decode it or verify all other sources. This is not a
whole-project Save/render-readiness guarantee or a race-free filesystem sandbox.
Unrelated missing/changed media may fail later normal Save/playback checks.

Before expensive full validation/cloning, local resource guards cap ordered track
blocks and resolved definition/ordered row/cell visits at 65,536, phrase-definition
cells at 32,768 and prospective validation units at 1,048,576. The latter includes
the prospective instrument multiplier and unused/aliased content; it is a
conservative estimate, not a global evaluator rewrite or latency certificate.
Both base and candidate use the exact existing pretty-project 8 MiB preflight.
Existing structural region/kit limits and checked ID exhaustion also reject
atomically. MCP separately bounds compact candidate/complete response sizes and
retains one private review; no client-supplied candidate or hidden truncation.

Accepted aggregate: **811 core / 1,022 desktop tests**, separate **227 ui-capture
library tests**, 175 MCP integration tests, **56 tools / 54 operations**,
**17 unchanged compatibility goldens**,
and **46 native captures (44 workflow frames plus two zero-pixel baselines)**,
with 16 critical frames directly reviewed. Native/MCP review covers 34 exact shared
previews, 14 commits and 12 refusals in 439 requests; GUI/shared/MCP WAV and event
JSON bytes match. Independent checks cover 110 cases and 16 full 120,000-frame
pad PCM cases. Narrow three-pad inspection requires scrolling to the third row;
complete text/copy is available, but universal Unicode glyph coverage is not promised.
The exact candidate passed an isolated desktop-audio compile/link with all107
build-source and123 payload hashes unchanged before/after. No executable was
launched; device-free PCM/UI checks are not physical playback, GUI input or
listening. Earlier partial host smoke remains separate; **full M5 remains open**.
This closes only reviewed single-bank allocation, not automatic role classification,
generation/genre grammar, multi-bank paging, a sampler preset library or MIDI.


## Rectangular whole-cell editing v1 — bounded cloud acceptance

Receipt: `evidence/judge-cell-blocks-final-20261025/judge-receipt.json`.
The shared `cell_blocks` version1 contract is an explicit one-pattern/one-logical-
TrackId rectangle with half-open row/column bounds. Copy stores an immutable
row-major `Option<Cell>` snapshot: absent holes differ from stored defaults, and
note/OFF/empty, instrument, optional velocity/pan/delay, effects and phrase fields
retain their exact representation, including valid signed-zero values. Copy may
resolve an alias chain read-only; its provenance is historical, not a live link.

Paste Replace targets the same logical TrackId, optionally in another pattern,
and replaces every position including holes. Clear removes stored cells only in
its rectangle. Destinations must be local, fit existing dimensions, and pass the
shared model validation; alias writes, growth, cropping, wrapping, merging and ID
remapping are refused. Existing row/track containers are retained rather than
silently normalized away. Ordinary repeated/alias uses observe the edited content;
bounded inspections expose affected definitions and occurrence/mute contexts.
Raw instrument/phrase IDs resolve against the current Song. Neither source-time
selection memory nor frozen sound is carried, so a structurally valid paste is
not a render-readiness or unchanged-audio guarantee. Source audio is untouched.

Editor-owned opaque captures clone one bounded Song, not its history stack.
Private owner identity binds clipboard and preview authority without serialization
or a caller-supplied nonce. An installed clipboard survives ordinary edits, Save,
Undo and Redo; successful New/Open replaces the owner even for identical bytes,
while failed replacement preserves it. Independent Editors cannot exchange it;
a deliberate `Editor::clone` is a same-owner document fork. Async Copy completion
checks captured owner/revision/full-source digest before installation. Later paste
uses the retained owner and a fresh base; mutation previews always require their
own exact current base/revision/owner. Copy/review is allowed in Read Mode; commit
requires Edit Mode. Failures retain prior review/clipboard and leave Song, cursor,
revision, dirty state and undo/redo unchanged. A changed Paste/Clear is one Undo;
an exact no-op consumes no history and preserves redo.

The common `project::same_serialized_song` comparator first rejects structural
inequality cheaply, then compares both equal-looking pretty project encodings with
the existing bounded writer. Signed zero and lexical `PathBuf` spellings such as
`/a/./b` versus `/a/b` are not silently collapsed. Both no-op and prospective dirty
comparisons finish before commit mutates history. Oversize/serialization errors
refuse atomically; infallible Undo/Redo restores its snapshot and conservatively
marks dirty if exact saved equality cannot be established. This is not a new
unconditional size/IO check on every structurally different old Editor operation.
Both Editor and `Song::register_sample` preserve registered source lexical-path
immutability alongside existing metadata checks; there is no canonicalization,
relink feature or source IO. Legacy instrument-path spelling remains an editable
exact document change. MCP changed/revision reporting follows the actual Editor
revision delta, including signed-zero-only edits.

Limits are 4,096 positions including holes, 1 MiB serialized clipboard, 2 MiB exact
before/after diff, and at most 64 records per inspection page. Existing pretty 8 MiB
project size and conservative local work guards run before expensive clones/
validation: 65,536 for each counted block/row/cell budget, 32,768 phrase cells, and
1,048,576 validation units for counted content × instrument definitions. Projected
repeat/alias costs are checked too. Excess limits reject, never truncate or silently
change the scope. These are bounded metadata operations with no source/path/hash/
decode IO; normal later Save/render checks still apply. No persistent clipboard,
OS-backed cell clipboard, cross-document import, multi-track routing, field-mask merge or
playback-engine/transport change is included. Read-only exact-record text supports
ordinary Ctrl+C, distinct from an OS-backed cell clipboard or cell-paste capability.

The final gate passes **852 core / 1,074 desktop**, plus a separate **240-test
ui-capture library run**, 182 real MCP tests, 60 tools / 54 typed operations, and
18 byte-exact WAV/events reference pairs (17 retained compatibility fixtures and
one independently hand-authored expected post-edit reference). Independent probes
cover 170 core/history/
budget requests and 72 real wire requests. Fifty fresh captures (48 workflows and
two official default views) include 116 exact inputs and 20 direct critical-frame
reviews. Shared/MCP replay covers 1,115 requests, 44 copies, 34 previews, 12 commits
and 12 refusals. The new Cells entry intentionally changes the 900px default by 145,991
pixels (toolbar wrap/grid shift) and 1180px by 2,895 pixels; neither is zero-delta.
Harness-only failed scope/scene requests are retained, not reclassified as product
failures; the earlier complete 44 provisional frames are preserved outside the
accepted root with their full hash inventory/source/failure diagnostics inside it.
The final 50-frame run is the accepted run; no implementation fix was needed for
those harness corrections.

Exact before/after 111 build-source and 127 candidate-payload identity verifies the
isolated locked/offline Aqua `desktop-audio` compile/link. The binary was not run;
there is no new native OS input, audible output, latency/deadline/hardware-xrun,
long-run or listening claim. The earlier four short output-callback smoke sessions
remain separate partial evidence. **Full physical M5 remains open**, and these
cloud/compile results do not authorize further host actions or MIDI/extra scope.


## Track balance v1 — bounded accepted controls and live submission

This adds optional `Song.track_mix` metadata, omitted when empty, over the existing
Engine/Transport rather than a second renderer or mixer graph. Each stored entry
has gain 0..1, stereo balance −1..1, mute and solo. Strict version1 requests carry
at most 64 unique defined TrackIds; finite f64 bounds are checked before f32 storage.
Signed-zero model/history bits remain exact; effective neutral runtime coefficients
are canonical. Empty settings explicitly reset the complete map. Solo on an unused
track can silence nonsolo contributors, and mute wins over solo.

Original complete scheduling, phrase expansion, RR/choke counters and voice
advancement remain intact. Controlled region/pad/keyzone voices retain their
logical ON-track ownership. Tracks using the same actual legacy sampler retain
its original slot/smoothing history and must request identical effective L/R
targets; no partitioning or duplicate voices are invented. Nonneutral unsupported
layouts with more than 64 contributors reject; neutral unsupported or model-valid
but evaluation-unavailable settings may save with explicit status, not live-ready
or source/save/render guarantees. Native full-table editing has a separate limit
of 64 defined tracks: larger documents show all configured entries read-only,
with explicit neutral Reset still requiring Preview/Apply. No truncation occurs.

Neutral rendering bypasses additional multiplication and retains prior PCM.
Nonneutral contributions are scaled after existing fixed master gain without
regrouping its sum. Balance attenuates a stereo side, not equal-power pan/crossfeed.
Resample output already bakes its captured mix, so exact neutral reuse also needs
neutral effective replay mix; no automatic compensation is applied. Audition's
synthetic Song remains neutral.

The pure metadata preview captures one bounded Song, not Editor history. Exact
owner/revision/base checks and Edit Mode protect one complete-map Apply/Undo with
no navigation. Unchanged maps preserve history/redo/revision. New/Open revoke the
owner even for equal bytes; ordinary edits/Save/Undo/Redo retain document ownership.
Source/path/audio IO is not performed by this metadata transaction. Local bounded
work/8 MiB guards are not a claim of general evaluator hardening or missing-media
repair. MCP exposes shared settings/preview/commit/history/offline export in its
own session, not GUI runtime IPC.

A separate bounded latest-value mailbox requests a complete snapshot only for the
matching active Song playback owner and exact non-mix content. Loading, stopped,
ended, errored, audition or revoked sessions refuse; non-mix content changes need
playback restart. Document commit and live submission are not atomic: failure
never rolls back valid persisted settings. Changed targets ramp over 128 emitted
frames, unchanged targets do not restart the ramp, and seek/loop preserve target/
ramp progression. Already queued PCM is not flushed or retracted. Accepted revision,
first complete nonempty block enqueued and audible effect are distinct states.
Mix-only Undo/Redo may request the restored settings; Save or readiness alone never
retries. Explicit no-op Preview/Apply may retry live without consuming history.
Public Producer errors revoke mix authority while retaining host-owned shutdown;
no new callback allocation or changed old command path is claimed beyond measured
and reviewed behavior.

Acceptance retains the completed 914-test no-default core phase with exact source/
dependency proof that the final desktop cfg-only correction is excluded. This is
not a fresh core rerun. Fresh corrected desktop1,148 and ui-capture265 tests, strict
lint/fmt/build, 191 MCP tests, 62 tools/55 operations and 19 exact reference pairs
pass. Independent checks include 110 bridge and 72 wire requests plus five concurrent
processes/320 accepted submissions. Sixty fresh frames/58 workflows have 22 direct
critical views, 112 exact input observations, 50 successful preview branches/485
wire requests and 36 actual Applies. Fourteen nonempty device-free PCM records and
two GUI WAV/events pairs remain. The intentional default-image delta is 2,252 pixels
at both sizes. Single-line long text may clip horizontally while exact bytes,
validation and reachable full warnings remain verified. Retained public-error,
Close/cfg and infrastructure failures and helper-only corrections are disclosed,
not substituted for accepted final outputs.

Host evidence is split between candidate1's explicitly authorized two quiet output
passes and corrected candidate2's compile-only result. Candidate1 independently
supports 145/168 callbacks, 73,687 above-threshold copied frames per pass, zero
software missing frames and revisions1/2 separately accepted/enqueued; its authored
192,000-value f32 input was independently reconstructed. Candidate2 has exact
117-source/136-payload before/after identity and locked/offline desktop-audio plus
guarded-example compile/link, with no binary execution. This does not attest a
final-source runtime, physical coefficient/ramp PCM, speaker/listening, GUI,
latency/deadlines, hardware xruns or long-run stability. Full M5 remains open.
Meters, stems, buses/sends, effects, automation, MIDI and unrelated extras remain
unimplemented or outside this batch; no further host action is authorized here.


## Read-only sample-peak meters v1 — accepted bounded observation

The optional observer uses the existing Engine's actual f32 contribution order;
no second audio renderer or audio-feedback path is introduced. Master peaks are
maximum absolute samples of final stereo PCM. Available track peaks sum original
contributions after track balance and fixed master 0.5, then take per-channel
absolute maxima. Regrouped track sums need not reproduce master bits. These are
not RMS, integrated loudness, true peak/oversampling or acoustic measurements.
An entire track sharing an actual legacy sampler is unavailable, even if it also
has controlled voices; no partial subtotal is labeled complete. Meter-only
nonfinite track accumulation makes the whole window unavailable. Unsupported
neutral per-track layouts can expose master-only reports with an explicit reason,
not the first 64 tracks disguised as a full layout. Rows identify scheduled ON
owners, not every defined/configured track. Finite peaks above unity remain exact;
they are not clamped or diagnosed as device clipping. The native editable Mix
table still separately supports at most 64 defined tracks.

The shared accumulator forms 2,400-emitted-frame windows at 48 kHz. Reports preserve
observation origin, exact emitted and first/last Song-frame bounds, wrap count,
frame count and first/last mix revision. Worker publication requires the complete
nonempty audio block to be enqueued. A four-report latest-window queue exposes
evictions rather than claiming every report was retained. Late binding omits the
incomplete leading window; live Stop/natural end omits the trailing partial.
Automatic loops keep emitted-window continuity, while manual seek changes the
generation. Overflow disables telemetry rather than stopping audio. Opaque
owner/session readers and generation checks reject stale data; UI invalidates on
revision/session changes without hashing the entire Song each paint. A detached
historical token does not independently discover an Editor replacement; controller
revocation/rebinding remains explicit. Read APIs do not start output, submit mix,
seek, edit history or advance the callback position.

Native bars use display-only 250 ms hold and 24 dB/s decay to −60 dBFS. Measured zero is
−infinity dBFS; unavailable has its own explanation. The latest raw report remains
unchanged. Explicit Snapshot latest report freezes the raw text presentation while
collection and bars continue; Follow latest is explicit. Complete exact text can be copied normally,
including the bounded 64-row report; this is not a persistent measurement record
or a claim of simultaneous visibility. Read Mode is supported.

Offline `MeterAnalysisConfig` is strict version 1 with an explicit half-open
48 kHz Song-frame interval. Start is 2,400-aligned; end is aligned or the exact natural
musical-plus-tail terminal, with end ≤ 120,000. Engine advances 0..end with full
scheduling, aliases, mutes, phrase/choke, source and RR context; earlier windows are
filtered out, not skipped by a cold seek. Only the exact natural terminal permits
an explicitly partial window, with no zero padding. Full compiled used routes,
even after the reported prefix, must use registered regions; path-only legacy
routes require explicit import/register AND region assignment. Raw used relative
paths reject without guessing CWD; existing project loading resolves its explicit
base. Every used path is authorized before canonicalization and verified loading.
Unused source files are not opened or byte-verified, but all model metadata is
validated. `source_count`/`verified_sources` count used SampleIds, not decoder calls;
`source_sample_values` counts unique-SHA decoded values including guard frames.
Distinct used paths sharing SHA still receive authorization and verification.

Bounds reuse the existing encoded 64 MiB/file and 256 MiB aggregate loader policies,
64 MiB aggregate decoded values, 8 MiB document/local work checks, ≤64 track rows and
≤1 MiB completed report. The 2^28 voice-frame guard is conservative; the evaluator's
2,048-ON and 120,000-prefix limits can reject first, so no claim of a reachable
exact 2^28 boundary is made. Cancellation occurs between bounded stages/blocks,
not inside a decoder. Engine clipping/nonfinite errors, policy/hash failures and
cancellation return no completed report. No PCM is retained/returned/published,
no gain is corrected, and no schema, Song, cursor, owner, revision, dirty state,
Undo/Redo or saved bytes are changed. MCP analyze/inspect is separate synchronous
headless analysis, not GUI/live/device readback. Its static mix-revision fields
are zero by convention, not the GUI's accepted/enqueued counters. Result inspection
is source-IO-free and reports `source_identity_verified_now:false`; Analyze-time
verification is historical, not repeated by inspecting a retained report.

Acceptance: 980 fresh core / 1,222 fresh desktop tests, separate 300 UI-lib, 203 MCP,
64 tools / 55 operations, strict lint/fmt/build and 19 byte-exact reference pairs.
Independent 121 bridge requests / 50 core PCM files, 69 wire requests and five 64-round
concurrent processes pass. Thirty-eight captures comprise 36 workflows plus two
zero-difference defaults, with 16 direct critical views, 14 input observations and
64 actual nonempty poll deliveries versus 62 helper-oracle observations, covered
by 346 independent old-Engine reference calls/files. All 123 frozen build sources
match the isolated 139-payload host candidate before/after compile/link. No binary
was executed: previous authorized host smokes remain historical, not meter-device
proof. Full M5/hearing, deadlines/latency/hardware xruns, long-run and native GUI
acceptance stay open. Effects, stems, buses, automation, recording, MIDI and all
previously excluded dataset/ML work remain separate.
