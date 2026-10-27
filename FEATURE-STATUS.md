# Leviathan — original-plan feature status

> This ledger records the recovered October 25 source and its historical
> acceptance evidence. The Windows/main-repository integration changes are
> described in the current README. Historical test counts below are not a
> claim that the modified integration has passed those same checks. Referenced
> evidence directories are retained in the complete handoff archive.

**Inventory date: 2026-10-25 UTC. This is a scope ledger, not a claim that either
original plan is finished.** Earlier checkpoints retain their own historical counts.

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

**Preceding bounded acceptance:** optional persisted track gain/balance/mute/solo,
[receipt](evidence/judge-track-mix-final-20261025/judge-receipt.json).
**914 core tests are explicitly reused source-equivalent phase evidence**, not a
fresh rerun: the sole corrected desktop test-hook cfg is proved excluded from
that no-default build. **1,148 corrected desktop tests** and a separate **265-test
ui-capture run** pass freshly, with 191 MCP tests, 62 tools / 55 typed operations
and 19 byte-exact reference pairs (18 retained plus one authored effective-neutral
pair). Sixty fresh captures cover 58 workflows and two official defaults, with
22 direct critical-frame views and 112 exact input observations (104 numeric texts,
six path texts, two sliders). Fifty successful preview replays use 485 wire
requests; 14 nonempty diagnostic PCM files and two GUI WAV/events pairs remain.
The intentional Mix default-image delta is 2,252 pixels at both sizes, not zero.

At most 64 configured settings form one exact owner/base/revision-guarded full-map
transaction and one Undo. Empty maps preserve omitted-default JSON; stored signed
zero remains exact while runtime neutral coefficients are canonical. Native table
editing supports at most 64 defined tracks; above that, all configured entries
are read-only and explicit neutral Reset still requires Preview/Apply. This is
not a truncated table or the same limit as shared configured/contributing tracks.
Owners of an actual shared legacy sampler need equal effective L/R targets;
no sampler partitioning or changed RR/choke/phrase scheduling is introduced.
Persisting settings and live delivery are separate: accepted metadata remains if
submission refuses. Targets ramp over 128 emitted frames without retracting queued
PCM. Accepted/enqueued/audible are distinct. Explicit no-op Apply can retry without
history; Save/readiness alone do not. Mix-only Undo/Redo may submit restored settings,
New/Open revoke ownership, and non-mix content edits require playback restart.
MCP remains headless metadata/offline export, not GUI runtime IPC. Model-valid but
evaluation-unavailable settings can save with explicit status, not render readiness.

**Host scope is split:** [candidate1's separately authorized two-pass smoke](evidence/judge-track-mix-final-20261025/host-runtime-candidate1/independent-review.json)
records 145/168 callbacks, 73,687 above-threshold copied PCM frames per pass,
zero software missing frames and revisions 1/2 separately accepted/enqueued.
[Final candidate2](evidence/judge-track-mix-final-20261025/host-candidate2/independent-review.json)
has exact 117-source / 136-payload locked/offline compile/link proof only; neither
binary was run. Historical runtime is not relabeled final-source runtime. No
speaker/coefficient PCM, ramp completion at a device, GUI, latency/deadline,
hardware-xrun or long-run claim follows. Full M5 remains open. Meters, FX, buses,
stems, automation, MIDI and unrelated extras remain outside this bounded batch.

**Preceding bounded editing workflow — ACCEPTED (cloud):** same-document whole-cell
Copy / Paste Replace / Clear over explicit half-open tracker rectangles. Alias Copy
resolves an immutable snapshot; destinations are local. Paste retains the original
logical TrackId (another pattern is allowed); Clear uses its explicit local scope.
Holes differ from explicit defaults; whole fields and raw IDs are preserved/replaced, without growth, merge, remapping
or inherited-instrument repair. Current mappings remain authoritative, not frozen
sound. Clipboard owner identity survives edits/Save/Undo/Redo but successful
New/Open revokes it; failed replacement preserves it. Every mutation preview still
requires a fresh exact base/revision and protected one-Undo commit. True no-ops
preserve redo/revision/cursor. Common bounded exact history identity distinguishes
signed zero and lexical path spelling; comparison failure is atomic, infallible
Undo/Redo conservatively stays dirty, and registered source paths stay immutable.
Caps include 4,096 positions / 1 MiB clipboard / 2 MiB diff plus 8 MiB document and
local work guards. No source/path/audio IO, OS-backed cell clipboard/cross-document cell paste, multi-track
routing, persistent clipboard schema or audio-equivalence claim is included.
Read-only exact-record text supports ordinary Ctrl+C, not OS-backed cell paste.
General editing/performance plans therefore remain partial.

**Earlier rectangle host build — compile only:** [rectangle review](evidence/judge-cell-blocks-final-20261025/host-compile/independent-review.json)
verifies isolated locked/offline desktop-audio compilation and all 111 build-source/
127 candidate-payload hashes before/after. No executable launch, new GUI/audio,
speaker/listening, latency/deadline or hardware-xrun acceptance follows. Full M5
remains open; the earlier output smoke stays separate partial evidence.

**Latest bounded metadata workflow — ACCEPTED (cloud):** explicitly select 1..16
ascending unique accepted-marker interval indices, including sparse selections.
A pure opaque preview proposes fresh exact whole-interval regions, one ordinary kit
and one fresh instrument without overwriting existing mappings. Source audio,
markers/locks, clock, patterns/order and cursor stay unchanged. No source BPM is
required; native reference root60, selectors36..51 and OneShot/Overlap are explicit
defaults, not inferred pitch/roles. No layers, sampler options, automatic paging,
compiler edit, note insertion or audition is added. Later markers do not regenerate
this one-time allocation. Native/MCP Create freshly authorizes/hashes the selected
source before one exact-base/revision-guarded Undo; it is not decoding or an
all-source Save/render-readiness guarantee. Existing pretty8MiB/model/local-work
caps reject atomically. Automatic analysis, multi-bank allocation, genre grammar
and broader sampler/performance plans remain incomplete.

**Earlier marker-bank host build — compile only:** [marker-bank review](evidence/judge-marker-bank-final-20261024/host-compile/independent-review.json)
verifies the isolated locked/offline desktop-audio link and all107 source/123
candidate-payload hashes before/after. The executable was not run; no new GUI,
audio, speaker/listening, latency/deadline or hardware-xrun result is claimed.

**Latest bounded playback workflow — ACCEPTED (cloud):** exclusive read-only
region/pad audition on the existing Engine/Transport: fixed input 1 / bin 127 /
fresh RR0, one ON with two seconds plus the existing 500ms tail at 48 kHz.
No document, cursor, history or persistent-schema transaction; no Song/audition
mixing. Unapplied imported region drafts use native root/bounds/ADSR/loop; pads use
committed mappings and controls, including their selected velocity zone and first
RR alternative. Stop/focus loss/context invalidation cancels without auto-restart;
hard Stop may click and cannot retract already submitted hardware buffers.
The first same-frame focus-pair gate was rejected and corrected in final2.
Source paths use existing project-load/import resolution, never process-CWD
inference. Raw selected relative paths reject. MCP is metadata-only, not device
output or fresh source verification: its target must be registered and all retained
paths pass authorization, stricter than native chosen-source byte loading.
Held-key/variable-velocity performance, full RR cycling, MIDI and mixed arrangement
context remain missing. Reviewed single-bank allocation is now accepted above;
the wider automatic break-to-kit workflow remains partial.

**Earlier corrected audition host build — compile only:** [candidate2 review](evidence/judge-audition-final2-20261024/host-compile/independent-review.json)
confirms an isolated locked/offline desktop-audio link with exact before/after
103 build-source and 119 candidate-payload hashes. That executable was not run;
no new GUI, device audition or listening acceptance follows. Candidate1 is
superseded historical compile evidence, not the final implementation gate.

**Partial physical output smoke — separately accepted:** [Aqua host receipt](evidence/judge-aqua-host-smoke-20261024/receipt.json)
records four short authorized CPAL/PipeWire 48 kHz stereo float32 sessions, each 148
callbacks, nonzero above-threshold PCM and zero software missing frames. Seek/loop
position observations are not sample-exact PCM proof. Fresh post-build payload
hashes and independently reconstructed authored attenuated-audio hashes support
provenance, not historical pre-execution binary attestation. Full M5 remains open.

The preceding [THIN final2](evidence/judge-thin-final2-20261024/judge-receipt.json)
remains the latest seeded note-mutation addition. Its rejected initial UI gate is superseded,
not accepted; final2 contains the corrected exact-use view and full rerun.

**Earlier bounded note-mutation addition — ACCEPTED (cloud):** one explicit seeded whole-cell
THIN operator over ordinary local tracker cells. Actual pinned pattrns decisions
are deterministic for the same base/config; excess removal count rejects rather
than truncates. For a cell to be eligible for removal, every current repeated/alias use needs
current human-reviewed region attribution. Kick/Snare are protected by default; unknown, Mixed, stale or
unreviewed uses always preserve. A full model-evaluable assigned-source base is
required even at 0%. The shared proof retains exact remaining ON timing/source/
pitch/gain/pan and control ownership, **not release/choke/tail or complete audio
invariance**. Core/Editor do no IO; MCP retains normal all-reference pathname
preflight. Exact-base digest/revision guards and one Undo preserve history; no-op
keeps redo. Before→None records are transient, not persisted recipes/audit logs.
Notes intentionally change; source audio, mappings, markers and BPM do not.

The preceding [native slice catalog](evidence/judge-slice-catalog-final-20261024/judge-receipt.json)
remains a bounded foundation: native measurements, uncalibrated rule support and
separate human roles, with historical source/bounds bindings. Ghost is contextual/
manual. Current-match is model equality, not a fresh file check or signed truth.
Analyze/Store freshly verify; metadata inspection and pure core/native manual
roles need no media IO, while MCP manual batches retain pathname preflight.
**B3.1/B3.4 remain PARTIAL:** general similarity library, calibrated recognition,
bleed/genre quality and broader role-driven grammar are not completed by THIN.
Tempo remains timing-point heuristics with advisory phase; downbeat/meter/loop
quality is open. Resample publication stays Unix-only and separate from registration.

**Current scope:** bounded audition and reviewed single-bank allocation are accepted;
further authoring work is separately scoped, not assumed complete. Device-free
cloud diagnostics and four short
output sessions do not certify speakers, latency/deadlines, hardware xruns,
long-run stability, natural device end or native GUI. **Physical M5 remains open**;
additional host actions require explicit authorization. The remaining inventory
below is retained, not silently canceled or declared done. Existing Unicode font
fallback and 60k-byte-name background toolbar wrapping remain presentation limits.
The narrow three-pad bank inspector scrolls to its third row; exact full text/copy
is available, without a promise of universal Unicode glyph coverage.

**Explicit user exclusions:** no dataset acquisition, deep learning, or ML
integration. These are **EXCLUDED**, not optional implementation backlog items.
Continue with authored synthetic fixtures and only already authorized unchanged
private-input checks; never package those private inputs or their derivatives.
Future rule-based analysis must not silently introduce an ML/data dependency.

## Authorities and status meanings

This ledger reconciles both user-supplied originals, not just the shorter ten-item
sequencer checklist:

- **S:** `LEVIATHAN_Sampler_DrumMachine_Plan(1).md`, version 1.0, sections 0–12.
  Original SHA-256: `104d169541cd1b6018d8caf9722d915e53d0b22d4985434a1f1c0b25eeede631`.
- **B:** `Pasted text(20261024-014556).txt`, headed **BREAK COMPILER v1.0**, sections
  1–12. Original SHA-256: `e8b9015cc3f3b20bc9269c129a8286a7aa0aed04e6d4476e358ac7666bad574f`.
- Confirmed implementation and limitations in [SEQUENCER-ROADMAP.md](SEQUENCER-ROADMAP.md),
  [MCP.md](MCP.md), [CLOUD-VALIDATION.md](CLOUD-VALIDATION.md), the source modules and
  sealed independent receipts linked below. The original files are supplied-plan
  authorities; they need not be present in a source-only checkpoint.

**ACCEPTED (bounded cloud)** means the precisely stated subset has an independent
receipt; it does not imply physical-device acceptance or every related feature.
**PARTIAL** means some components exist but the original requested workflow is not
complete. **MISSING** means no accepted implementation, not merely an unavailable
button. **IN PROGRESS** is unaccepted work. **HOST BLOCKED / UNVERIFIED** requires
a separately authorized machine/device test. **OPTIONAL / LATER** preserves the
original plan's priority; it is not silently promoted to a mandatory current task.
**EXCLUDED** records a user constraint/non-goal, not unfinished authorized work.

The sequencer roadmap's M1–M10, sampler plan's M0–M7, and Break Compiler's A–E are
**different milestone schemes**. Completing bounded sequencer M10 does not complete
sampler M2/M7 or Break Compiler B–E. Proposed crate names in the originals are
architectural suggestions, not obligations to duplicate an existing subsystem.

## Accepted bounded capabilities and evidence

| Capability | Accepted boundary and implementation authority |
|---|---|
| One event authority | `song.rs`, `editor.rs`, `drum_steps.rs`: ordinary tracker cells drive the tracker and 16/32-step overlay. No separate drum pattern store. Explicit optional scalars, Empty/Note/Off, stable pattern/track/instrument IDs; shared Repeat versus independent Clone. Bounded same-document rectangular editing is accepted separately below; OS-backed/cross-document/multi-track cell clipboard is not implied. |
| Rectangular ordinary-cell editing | [Rectangle v1](evidence/judge-cell-blocks-final-20261025/judge-receipt.json): same-document owner-bound Copy/Paste Replace/Clear, alias read-only Copy/local same-TrackId destinations, exact holes/fields/current raw IDs, one guarded Undo and exact serialized history. 4,096-position / 1 MiB clipboard / 2 MiB diff / local-work caps; no OS-backed/cross-document/multi-track cell clipboard, inferred selection memory or frozen-audio claim. |
| Reviewed marker → drum bank | [Marker-bank v1](evidence/judge-marker-bank-final-20261024/judge-receipt.json): explicit 1..16 exact accepted intervals, fresh regions/kit/instrument, selected-source hash verification and one guarded Undo. No source BPM, existing sequence change, live marker recipe, automatic roles/generation or multi-bank paging. |
| Bounded track balance | [Track balance v1](evidence/judge-track-mix-final-20261025/judge-receipt.json): optional gain/balance/mute/solo, exact full-map one-Undo review, neutral PCM compatibility, shared-legacy equal-target restriction and separate owner-bound live submission with 128-frame ramps. Native table supports ≤64 defined tracks; configured/contributing limits differ. Meters are separately accepted below; no FX/stems/automation or full device certification. |
| Read-only sample-peak meters | [Meter v1](evidence/judge-meters-final-20261025/judge-receipt.json): original-order post-balance/post-master track peaks and actual final-PCM master peaks; explicit unavailable tracks, bounded full-window render-ahead queue/generation/drop semantics, presentation-only bars and copyable raw inspection. Separate registered-region-only aligned offline analysis processes full context through at most 120,000 frames without document/history/output-file changes. No RMS/loudness/true peak, heard/device/timing claim or GUI IPC. |
| Exclusive region/pad audition | [Audition final2](evidence/judge-audition-final2-20261024/judge-receipt.json): read-only draft/committed-pad snapshot, fixed two seconds plus 500ms tail, unity/bin127, first RR source, existing Engine/Transport with exclusive ownership and cancellation. Native imported draft support; metadata-only MCP uses registered targets. Cloud PCM/UI and corrected host compile pass; actual audition-device/listening and broader performance triggering remain unverified. |
| Device-free transport verification | [Cloud transport diagnostics](evidence/judge-transport-stress-final-20261024/judge-receipt.json): bounded exact PCM/count/control and measured fill-allocation checks over existing Engine/Producer/Shared, synthetic sources only; no production runtime change. Concurrent underflow disclosure and cloud timing limits remain explicit; not physical latency, audibility or xrun certification. |
| Shared scheduling and rendering | `song_render.rs`, `song_render/*`, `transport.rs`: shared frame-based evaluator/Engine, constant fractional-BPM timing, ordered patterns, bounded starts/actions, explicit cold seek and transport-loop policy. Offline/null rendering works without a device. Tempo automation and physical output proof are separate. |
| Typed note commands | [M6](evidence/judge-integrated-final-20261024/judge-receipt.json): retrigger, timed release, actual hard cut, source-frame offset, reverse, glide; common evaluator and ordinary cells. These are Leviathan units, not Renoise command fidelity. |
| Project transactions | `project.rs`, `session.rs`, `editor.rs`: strict additive schema-1 JSON, 8 MiB encoded project limit, relative paths, atomic save publication/confirmed overwrite, failed-open/save retention, protected Edit Mode, bounded Undo/Redo, revision/full-base stale guards and bounded exact signed-zero/lexical-path history identity. Comparison errors refuse atomically; Undo/Redo dirty fallback is conservative. No portable embedding or missing-media relink. |
| Verified assets and regions | [Slice foundation](evidence/judge-slice-foundation-final2-20261024/judge-receipt.json), `sample_asset.rs`: strict bounded native-rate mono/stereo WAV/FLAC, immutable identities/hash verification, shared PCM cache, non-destructive source-frame regions and root mapping. Fixed budgets are not user-configurable eviction. |
| Manual slicing and compiler | [Compiler](evidence/judge-compiler-final-20261024/judge-receipt.json): accepted manual boundaries, explicit source BPM distinct from target BPM, pure preview and single guarded append, source-rhythm/grid timing, bounded rolls through 1/128, overlap/release/cut tail choices, fresh region/instrument allocations. No automatic drum-role recognition; the separate reviewed single-bank allocator is now accepted above. |
| Onset suggestions and locks | [Onset v2](evidence/judge-onset-v2-final-20261024/judge-receipt.json): default heuristic detector plus optional experimental cached v2 evidence/refinement, manual corrections and persistent locks. Suggestions never silently replace accepted markers. This is not tempo/beat inference or broad recognition accuracy. |
| Instrument phrases | [M7](evidence/judge-m7-final-20261024/judge-receipt.json): bounded reusable instrument-associated note phrases, triggers and explicit lifetimes. These are sequencer phrases, not automatic multi-hit audio phrase extraction. |
| Drum kits | [Kit](evidence/judge-kit-final2-20261024/judge-receipt.json): 16 pads, configurable selector notes/stable IDs, source mapping, pad root/coarse tune/gain/pan, OneShot/Gated, overlap/release new-note policy, choke groups, atomic fresh kit/instrument creation. Selection is not performance audition. |
| Ordinary-cell drum overlay | [Steps](evidence/judge-drum-steps-final-20261024/judge-receipt.json): 16/32 steps, explicit subdivision/page, exact parent-cell velocity/delay/retrigger edits, visible off-grid/conflict inventory, safe insertion, alias protection and one Undo. No silent quantization or second scheduler. |
| Arrangement matrix | [Matrix](evidence/judge-matrix-final-20261024/judge-receipt.json): bounded same-geometry cross-track alias chains, explicit source navigation/Make unique, clone materialization, occurrence-local track mute. This is not a mixer or pad solo/mute. |
| Scoped baked transforms | [M9](evidence/judge-transforms-final-20261024/judge-receipt.json): independent note/glide/explicit-velocity/timing masks, bounded transpose/scale/nudge/fixed swing, exact scoped preview/change list and one guarded commit. No runtime groove engine or general seeded grammar. |
| Seeded pattrns generation | [M10](evidence/judge-generation-final2-20261024/judge-receipt.json): one versioned seeded Euclidean preset runs actual pinned pattrns and appends ordinary editable cells; historical recipe/digests never regenerate on Open. Generator-only serialization/traversal guards. Not jungle/breakcore grammar or arbitrary scripts. |
| Region ADSR and sustain loop | [Sampler v1](evidence/judge-sampler-v1-final-20261024/judge-receipt.json): opt-in linear fixed-48kHz ADSR and absolute-source forward loop, prefix once, loop during release, independent voice state. None preserves prior JSON/PCM. No crossfade/reverse loop, post-loop tail or keyzone instrument. |
| Velocity zones / deterministic RR | [RR](evidence/judge-velocity-rr-final-20261024/judge-receipt.json): complete inclusive 0..127 bins; at most 8 zones, 8 alternatives/zone, 32/pad; one source per hit; stable per-pad IDs and ordered alternatives; independent instrument/pad/zone cycles; exact pre-pad f32 selection and unquantized gain. No stacked/random/crossfade layers or per-layer processing. |
| Reusable explicit groove timing | [Groove v1](evidence/judge-groove-final-20261024/judge-receipt.json): reviewed accepted-marker window + explicit BPM/grid, immutable persisted template, historical provenance, scoped strength 0–100% capture/apply previews and one guarded ordinary-cell bake. No audio/beat inference, accent/role transfer, humanization or runtime template dependency. Conservative half-tie/u128/32-ULP/all-use frame bounds reject unrepresentable requests; changed interleaving may change RR/choke/phrase expansion, so preview is not full renderability proof. |
| Exclusive melodic keyzones | [Keyzone v1](evidence/judge-keyzones-final-20261024/judge-receipt.json): up to 64 shared maps / 128 stable-ID zones per map / 4096 total zones; inclusive disjoint note+velocity rectangles, explicit native roots, whole-map transactions and independent mapping copies sharing region references over immutable source audio; later shared-region edits affect all users. Empty/holey maps author safely but actual uncovered ONs reject; chosen-source glide does not remap. No melodic stacking/random/RR, SFZ import, full piano/quality workflow or performance audition. |
| Scoped render → new verified asset | [Resample v1](evidence/judge-resample-final-20261024/judge-receipt.json): one cold occurrence/all tracks, explicit musical-only or fixed 500 ms tail, pre-master-compensated float32 stereo, new no-clobber Unix publication, then separate all-source/storage preflight and guarded one-Undo sample/whole-region/instrument registration. No existing sequence edit, new persistent recipe, used legacy-path conversion or automatic artifact deletion. Native worker cancellation is not asynchronous MCP; neutral reuse requires full-region hold and neutral effective replay track mix, because printing bakes its captured mix. |
| Timing-point tempo / pulse phase | [Tempo v1](evidence/judge-tempo-final-20261024/judge-receipt.json): bounded marker/cache timing-point hypotheses, explicit source-frame windows, tenths-BPM authority, multiple candidates and exact rational advisory pulses, inspectable insufficient evidence and one guarded BPM-only metadata use. Marker mode is historical without source-byte hashing/decoding; the pure core is IO-free and normal MCP pathname/root preflight remains. Cached adapters freshly verify at Analyze/Use. Config caps and opaque IDs differ across UI/MCP as stated above. No automatic assignment, phase application, downbeat/meter/roles or calibrated musical confidence. |
| Native slice catalog / reviewed roles | [Catalog v1](evidence/judge-slice-catalog-final-20261024/judge-receipt.json): compact native RMS/peak/DC, spectral power/bands/centroid/flatness/supported shape change and censored envelope descriptors; explicit 1..64 regions from one source, at most 256 persisted entries; independent human roles/articulation and historical bindings. Rule support is not probability, Ghost is manual, model current-match is not current-file verification. No automatic playback/mapping/grammar or clean-classification claim. |
| Reviewed-role seeded THIN | [THIN final2](evidence/judge-thin-final2-20261024/judge-receipt.json): one seeded whole-cell delete with default Kick/Snare and unconditional uncertain-role protection across current uses; full retained-ON/control proof, explicit cap rejection, pure opaque review and one Undo. Transient exact deletion record, no live recipe. Not full audio/tail invariance, genre grammar or persistent audit history. |
| Native UI and shared MCP | Actual egui/EGL input and exact UI/MCP Song/manifest/save/export parity are reviewed in the receipts. Typed authoring/batches/inspection/preview tokens share validated helpers. MCP is a separate headless document session, not live GUI IPC; heavy calls remain synchronous/bounded. |
| Offline checks | Whole-song float32 WAV and manifest, finite/clipping/peak/energy checks, deterministic synthetic PCM/frame/serde/history tests and compatibility goldens. Fixed export-tail/cold-reset semantics are explicit. No automatic normalization, physical listening or unimplemented-mode test claim. |

The historical RR gate also verifies both prior sampler wording corrections. Historical
receipts retain the earlier notes unchanged. Structural mapping/voice limits are
not a guarantee that every maximal model fits the independent 8 MiB save budget.

## Full original feature inventory: unfinished portions

Every **PARTIAL** row below names both its accepted component and the still-open
parts. Absence from a narrow accepted milestone is not treated as a hidden success.

| Area / original references | Status | Existing subset and remaining work |
|---|---|---|
| Import formats — S3.1, B3.1 | **PARTIAL** | WAV/FLAC accepted; **AIFF missing**. Cargo enables Symphonia wav/pcm/flac only. MP3 and OGG are separate optional items below, not inferred from the decoder library. Mono/stereo accepted; other layouts reject, no general multichannel downmix. |
| Waveform / analysis assets — S3.1, B3.1 | **PARTIAL** | Bounded waveform/onset views and compact native-region acoustic catalog exist. Zoomable peak pyramids, general searchable spectral/acoustic-feature library, expanded browser/index/similarity search are missing. True drag-and-drop loading is not established by path-entry/import controls. |
| Asset lifecycle — S3.1, S7 | **PARTIAL** | Relative paths, hashes, strict decode/identity checks and immutable buffers accepted. User-configurable memory budgets/eviction policy, portable source copy/embed, missing-media placeholders and explicit same-identity relink UI are missing. No silent replacement by a different sound. |
| Region editing — S3.2, S3.3, B4 | **PARTIAL** | Integer trim/regions/root, source offset/reverse, ADSR and forward loops accepted. Independent region gain, fine cents, explicit non-destructive normalization, trim fades, attack pre-roll, zoom/snap/zero-crossing controls are missing. Zero-crossing snapping must not move a protected attack silently. |
| Loop and release variants — S3.2, B4.3 | **PARTIAL** | Forward loop and documented release/OneShot/Gated behavior exist. Reverse/ping-pong loops, short loop crossfades, post-loop-tail continuation, independent release samples and configurable broader one-shot cut/release/continue policies remain missing/later. Loop voices currently reject active offset/Reverse and can click. |
| Resampling / pitch — S3.3, B7.2–7.4 | **PARTIAL** | Native-rate repitch, coarse pitch and glide accepted. Per-note/region fine cents, pitch envelopes, controlled 4-point/HQ/anti-aliased mode and steep-shift quality validation are missing. Linear controlled resampling is not phonic/HQ parity. Offline SRC/preconversion caching is not a new per-note callback resampler. |
| Melodic multisample mapping — S3.4, S10 M2 | **PARTIAL** | Exclusive note+velocity rectangles, explicit roots, shared mutable maps, bounded keyzone UI and transactional copies are accepted. Play-all/cycle/random overlap policies, reusable sampler presets, SFZ import and the full multisample-piano/editor/quality workflow remain missing. Stored coverage gaps are deliberate drafts, not render-ready silence/fallback. |
| Voice limits / modulation — S3.4, B7.2 | **PARTIAL** | Bounded polyphony, pan/smoothed gain and independent amp envelopes exist. User-configurable per-sampler/per-kit limits, deterministic click-safe voice stealing, velocity response curves, low/high-pass filters, shared modulation templates, pitch envelopes, key switches, mod matrix/LFOs are missing. |
| Pad model and source architecture — S4.1–4.2 | **PARTIAL** | 16 pads, custom note mapping, coarse tune/gain/pan, region references, one-source velocity zones/RR and choke/release ownership accepted. Stacked simultaneous layers/blending, seeded random selection, velocity crossfades, per-layer tune/start-offset/gain/filter/envelope overrides, 8/32/64-pad banks, pad color/output bus and general kit presets are missing. Region settings may be shared, not falsely advertised as independent per-layer controls. |
| Pad processing / interaction — S4.2–4.3 | **PARTIAL** | OneShot/Gated, overlap/release NNA, ordinary commands and region trimming work. Bounded exclusive committed-pad audition is cloud accepted; actual audition-device/listening remains unverified. Fine pad tune, velocity curves/filter, NNA hard-cut policy, pad solo/mute, held-key/variable-velocity or mixed performance triggering, drag/load interaction and optional GM presets/helpers are missing. Explicit hard-cut events do not implement every NNA mode. |
| Step editing — S4.3, S5, B6/B8 | **PARTIAL** | Same-cell 16/32 overlay, velocity/delay/retrigger and tracker offset/reverse/cut/glide accepted. 64-step page, runtime per-step probability, every-N conditional repeats, fine tune/filter locks/automation, general gate/duration editor and shaped retrigger velocity/pitch/envelope controls remain missing. Euclidean baked probability is not runtime conditional sequencing. |
| Tempo/beat analysis — B3.2 | **PARTIAL** | Bounded deterministic timing-point tempo/pulse-phase hypotheses, beat-unit ambiguity and explicit reviewed BPM-only marker metadata use are accepted. Scores are not calibrated confidence. Downbeats, meter, loop-length, robust genre/bleed quality, automatic clock/marker changes and phase application remain missing. Explicit source BPM and compiler file-frame-zero origin stay authoritative; no onset point becomes an automatically accepted beat. |
| Onset methods / quality — B3.3 | **PARTIAL / EXPERIMENTAL** | Heuristic multi-evidence suggestions, spacing/refinement and saved manual locks exist. A generally reliable adaptive confidence system across real genres, complete specified detector/feature coverage and broad recognition/performance acceptance remain open. V2 tonal false positives, weak-hit misses and harmful refinement are disclosed; no default automatic acceptance. |
| Percussion roles — B3.4 | **PARTIAL** | Native spectral/envelope descriptors, five handcrafted rule families, Mixed/Unknown advisory states and explicit human Kick/Snare/ClosedHat/OpenHatRide/Crash/Mixed/Unknown roles plus contextual Ghost articulation are accepted. Unreviewed is distinct from reviewed Unknown. Rule fractions are not calibrated probabilities; envelope tail is not clean decay/RT60, and rises are not proven hit counts. Reliable genre/bleed recognition, calibrated confidence, automatic role acceptance and broader role-driven grammar remain missing. No ML/data acquisition is authorized. |
| Audio slice hierarchy — B4.1, S8 | **PARTIAL** | Arbitrary non-destructive regions, manual marker compilation and reviewed 1..16-interval single-bank allocation exist. Distinct hit/audio-phrase/loop hierarchy, automatic phrase extraction, role-driven allocation and multi-bank paging remain missing. The earlier private 13-pad conversion was test-side only; the accepted allocator is the separate shared workflow above. Instrument note phrases do not close audio phrase slicing. |
| Attack/tail/phase protection — B4.2–4.4 | **PARTIAL** | Natural source playback, overlap/release/cut, choke and independent region loop/ADSR exist. Attack protection/pre-roll/snap, crossfades, adjacent-source phase continuity, stretch-tail and room/noise-bed continuation are missing. Region sustain looping is not a general compiler tail-mode/phase-preservation system. |
| Reusable groove transfer — B5, S4.3 | **PARTIAL** | Explicit accepted-marker timing capture, frozen BPM/rate/frame authority, historical source provenance, persisted templates, scoped strength 0–100% blending and guarded baked apply are accepted. Automatic audio-derived beat/onset relationships, accent/role/ghost transfer, confidence, general straight/source/hybrid reconstruction, repetition variation and controlled humanization across sources remain missing. Timing-only capture is not completion of the full original groove workflow. |
| Musical grammar / presets — B6.1 | **MISSING** | Editable Jungle, Atmospheric DnB, Breakcore and Glitch composition presets with meaningful anchors, phrase reuse, cymbal continuity, structured fills and sparse/busy contrast. More notes or one Euclidean preset is not genre coherence. |
| Seeded mutation operators — B6.2 | **PARTIAL** | Ordinary manual reverse/pitch/offset/retrigger and bounded baked transpose/nudge exist. Seeded whole-cell THIN with current reviewed-role protection, retained-ON/control proof and a transient exact deletion record is accepted. General seeded rotate, compatible substitute, phrase repeat, stutter variation, double, fill, cross-break swap, intentional rest, resample and durable mutation records remain missing. Existing commands must not be relabeled as a complete grammar operator suite. |
| Controlled generation / structure — B6.3–6.4 | **PARTIAL** | One seeded Euclidean bake and explicit marker reconstruction preserve input content. THIN protects current reviewed Kick/Snare uses by default, not general genre/position anchors. Density limits, hit/beat/bar/4/8-bar variation, source IDs/style/density/variation/groove-strength/reference-pattern/max-edit-complexity controls, cross-source blends and multiple reviewed candidates are missing. |
| Tempo map / event identity — S5/S7, B7.1 | **PARTIAL** | Constant BPM/LPB, exact delayed integer-frame scheduling, pattern/track/row/column identity and stable model IDs exist. Tempo automation/map and optional stable per-event IDs are missing; content is not a persisted absolute-frame event list. |
| Render products / analysis — S6.2/S8, B7/B9 | **PARTIAL** | Whole-song float32 WAV/manifest, finite/clipping and peak/energy checks, plus Unix-only cold-occurrence print→verified asset, bounded repeated reuse and aligned read-only sample-peak analysis are accepted. Contextual arrangement crops, arbitrary pattern/instrument/pad/track stems, conventional PCM with dither, whole-render RMS reporting (native region RMS now exists), configurable silence/max-tail termination, comparisons/A–B, persistent resample provenance and generalized recursive workflow remain missing. Explicit zero/500 ms tails are not silence detection; used legacy paths must first be explicitly imported/assigned as regions. |
| Stretch modes — S3.3, B7.4 | **PARTIAL** | Repitch and slice reconstruction accepted; preserve-pitch and hybrid moderate-stretch/chop modes missing. Advanced independent stretch remains later/optional, not a prerequisite silently added to simple 70→170 reconstruction. |
| Mixer / effects / automation — S4.2/S6, B2/B8 | **PARTIAL** | Bounded track gain/stereo balance/mute/solo, neutral offline rendering, separate guarded live updates and read-only sample-peak meters are accepted. RMS/loudness/true-peak metering, low/high-pass, drive/saturation/EQ/transient shaping/bitcrush, per-pad/track buses/submixes, shared reverb/delay sends, modulation/parameter locks and automation remain missing. This is not a full mixer/routing graph or pad mute. |
| Native workflow — S4/S10, B8 | **PARTIAL** | Keyboard tracker, multiple note columns, typed commands, LPB, source/region mapping, compiler, phrases, aliases, step overlay, keyzone map editing, preview and Undo exist. Bounded exclusive region/pad audition UI is cloud accepted; general performance triggering, broad sample browser, full mixer beyond bounded track-balance controls, history/A–B panel, OS-backed/cross-document/multi-track cell clipboard beyond the accepted bounded rectangles, full selection-based grammar and complete keyboard reachability for every future workflow remain open. |
| Presets / revision history — S3–4, B8/B11E | **PARTIAL** | Fixed built-in generator/onset configurations, bounded Undo, transform CellChange previews and historical generation digests exist. General sampler/kit/groove preset save/load library, generic cross-revision pattern diff, persistent named transformation/audit records and A–B history are missing. |
| MCP completeness — S8, B9 | **PARTIAL** | Typed IDs, authoring/atomic batches, compact paginated inspections, guarded preview tokens, revision/Undo and destructive-operation checks exist. Broader groove-analysis/grammar, library-search/render-compare tools, async job handles/cancel, persistent input/output/seed/mutation logs and live GUI-instance/transport IPC are missing. Separate-session synchronous APIs do not satisfy those workflows by renaming tools. |
| Test breadth / musical acceptance — S11, B10 | **PARTIAL** | Strong synthetic known-frame/PCM/hash/serde/history/golden/EGL parity tests exist. General audio tempo-quality/automation, stealing/unbuilt modes, broad genre/bleed/ghost coverage, reference hand-programmed musical coherence, cymbal/attack quality and listening A–B remain open; broad coverage does not authorize acquisition of datasets. Numerical scores are not listening acceptance. |

## Optional or explicitly later items — individually unimplemented

These remain visible rather than disappearing inside a generic “advanced features”
label. They require their own design, dependency/license review and authorization
where applicable; listing them does not authorize device access or new datasets.

| Item | Original priority / status |
|---|---|
| MP3 import | Conditional on already enabled decoder support; **not enabled**. |
| OGG/Vorbis import | Conditional on already enabled decoder support; **not enabled**. |
| Sample recording | Production extra; **missing**, host/device authorization required. |
| External MIDI controller mapping | Production extra; **missing**. |
| GM drum-note preset/helper | Optional preset; **missing**; current custom mappings do not assume an octave convention. |
| SFZ multisample import / completeness | Later; **missing**. |
| XRNS research / prototype import | Optional later compatibility; **missing**. |
| XRNI research / prototype import | Optional later compatibility; **missing**. |
| Synth kick layer | Optional post-sampled-kit extra; **missing**. |
| Synth snare layer | Optional post-sampled-kit extra; **missing**. |
| Synth hats layer | Optional post-sampled-kit extra; **missing**. |
| Independent release samples / velocity-triggered release samples | Later; **missing**. |
| Key switching | Later; **missing**. |
| Modulation matrix / LFOs | Later; **missing**. |
| Advanced independent time stretch | Later, non-blocking for first compiler; **missing**. |
| Granular synthesis | Deferred; **missing**. |
| Complex recording / hardware routing | Deferred; **missing**, host authorization required. |
| Windows ASIO path | Only if required after baseline WASAPI; **not established**. |
| Full Renoise import/export / command fidelity | Explicit deferred/non-goal for native v1; **not implemented or claimed**. |
| Dataset acquisition / deep learning / ML integration | **EXCLUDED by user instruction**, not optional backlog. |
| Large generative audio model / training inside realtime engine | Explicit non-goal and outside current user constraints; **not planned**. |
| Separate drum-machine sequencer/DAW engine | Explicitly prohibited architecture, **not a missing feature to add**. |

## Physical-host and performance gate — still open

The latest track-balance gate adds the separate bounded candidate1 two-pass
callback/enqueue smoke and final candidate2 compile-only evidence described above.
It does not close the physical limitations below or relabel the earlier diagnostic
checkpoint as current unchanged transport. No further host action is authorized.

**Earlier device-free transport verification — ACCEPTED, bounded cloud only:** the
`transport_stress` example and two integration tests exercise existing shared
Engine/Producer/Shared behavior without CPAL or a device, using only a new absolute
output directory and authored synthetic fixtures. Five fresh CLI processes provide
40 bit-exact same-Engine PCM cases and 80 acknowledged controls / at least 160
post-command ready observations. Sixteen compatibility goldens and two unchanged
default images pass. No allocations/reallocations/deallocations were detected in
measured fill calls; this is not a no-lock/syscall/deadline proof. Production
Engine/transport and dependencies were unchanged at that historical checkpoint. An earlier artificially paced
concurrent run's **769 missing frames** remain disclosed; deterministic prefilled
cases are exact and underflow-free, not a general zero-xrun claim. Cloud/debug
metrics are informational, runtime-path rustc is not the build compiler receipt,
and synthetic failure tests explicitly invoke harness shutdown. There is no new
physical deadline/latency/speaker/device-negotiation certification or ten-minute
host run. At that sealed cloud checkpoint host permission was pending. A later limited
host smoke is reviewed separately below; cloud evidence implies no further host
access, installation or transfer. Receipt: [transport diagnostics](evidence/judge-transport-stress-final-20261024/judge-receipt.json).

Current `desktop` deliberately omits CPAL/JACK/ALSA; `desktop-audio`/`realtime`
select the physical backend. Default-device transport code exists with a 48 kHz
stereo float configuration and a conservative 512-frame target. The short host
smoke confirms one 48 kHz stereo float32 route, not broader device negotiation or
a low-latency guarantee.

- **PARTIAL HOST OUTPUT PASS:** [four short isolated sessions](evidence/judge-aqua-host-smoke-20261024/receipt.json) establish
  real CPAL/PipeWire output callbacks with 48 kHz stereo float32, 148 callbacks each,
  above-threshold PCM and zero software missing frames. Logged seek/loop position
  transitions are not sample-exact command realization. The property 512/48000 is
  not measured latency. Current payload hashes match the accepted checkpoint and
  four authored ×0.1 WAV hashes were independently derived; no historical runtime
  binary attestation or independently observed system-state history is claimed.
- **HOST UNVERIFIED:** human speaker/headphone audibility, actual native OS
  window/input, offline-versus-live sample tolerances, natural device end and
  listening A–B.
- **MISSING / UNVERIFIED:** device enumeration/selection/hot-switch, configurable
  sample rate/channels/format/buffer negotiation beyond the observed fixed route; broader Linux desktop/PipeWire and
  Windows WASAPI/optional ASIO coverage. The backend underrun counter and desktop
  display exist; device-validated measurement/profiling remains unverified.
- **TARGETS, not results:** reliable 128/256-frame operation; 64 concurrent stereo
  voices at 48 kHz / 128 frames; callback peak CPU/allocation/xrun profiling;
  ten-minute no-avoidable-underflow run on an explicitly validated device setup.
- Stop/seek/mute/project replacement have shared-engine tests; physical device
  hot-switch/stuck-voice acceptance does not follow from them.
- Real audio callback must continue avoiding filesystem/decode/probing, blocking
  locks, logging, discovery and new allocations. Offline/source inspection is not
  a substitute for an instrumented realtime performance gate.
- No new PC calls, settings/services/system-package changes, recording, deployment
  or external audio acquisition are authorized by this ledger. Existing private
  recordings/derivatives remain excluded from source deliverables.

### Remaining path-publication and presentation notes

The latest catalog gate proves R64 is reachable/selected, but its virtual list row
may scroll out after detail reflow. Unsupported Unicode can render as fallback
boxes; stable IDs remain readable. These are nonblocking presentation limits, not
proof of all-platform typography or OS-window/input acceptance.

Resample v1 fails closed outside Unix. Its retained identity and path rechecks are
not an OS capability sandbox against concurrent retargeting. An additional parent
check immediately before temporary creation remains defense-in-depth follow-up;
stronger descriptor-rooted/openat authority is separately scoped. These limits
must not be hidden by successful ordinary no-clobber/cleanup tests.

The earlier keyzone narrow Inspect capture's lingering waveform-help tooltip was
nonblocking (no data/interaction defect) in that historical receipt. The latest
tempo gate reports no remaining presentation finding in its reviewed workflows;
that does not rewrite older evidence or claim a separate keyzone-tooltip fix.

## Original milestone crosswalk

| Original milestone | Current disposition, not the sequencer checklist's similarly named milestone |
|---|---|
| S M0 repository/audio preflight | **PARTIAL / HOST OPEN**: source reuse/dependency map exists; earlier four-session and later bounded two-pass output-callback smokes passed; final track-mix candidate2 and the current meter candidate are compile-only, and broader device/audibility acceptance remains open. |
| S M1 first sound live + WAV | **PARTIAL / HOST OPEN**: immutable import, note dispatch and offline render accepted; four earlier short output sessions and two bounded mix callback/enqueue passes passed; no speaker/ramp-PCM proof, and full physical/listening gate remains open. |
| S M2 melodic sampler | **PARTIAL**: pitch/polyphony/ADSR/forward loop/recall and exclusive note+velocity keyzones/roots/shared maps accepted; full multisample-piano/editor/quality/import/overlap workflow remains missing. |
| S M3 drum kit/editor | **PARTIAL**: kits/choke/mapping/one-source velocity/RR and bounded exclusive audition cloud accepted; actual audition-device/listening unverified; drag/load, broader performance, fine/per-layer controls and broader layers missing. |
| S M4 drum overlay | **BOUNDED CLOUD ACCEPTED**: same-cell 16/32 view, velocity/delay, baked swing, retrigger, history and clone/repeat; broader runtime/64-step/performance items remain separate. |
| S M5 break-oriented sampler | **PARTIAL**: manual slicing, native-rate reconstruction, basic tails and Unix-only isolated-occurrence render-to-new-asset accepted; contextual crop/stems, generalized recursive/provenance and remaining edit/tail workflow missing. |
| S M6 compiler hookup | **PARTIAL**: onset suggestions, editable compilation, explicit marker-timing templates and reviewed native region/role metadata and one protected THIN operator accepted; broader role-driven grammar, automatic audio phrase/groove analysis, A–B and generalized recursive workflow missing. |
| S M7 production extras | **MOSTLY MISSING / OPTIONAL**: bounded track balance and read-only sample-peak meters are accepted; RMS/loudness/true peak, FX/buses/automation, recording/MIDI/SFZ/synthesis and full cross-platform/performance gates not completed. |
| B A first playable prototype | **PARTIAL / HOST OPEN**: explicit-BPM marker→editable target pattern→WAV exists; complete original import/analysis/live workflow not closed. |
| B B musical reconstruction | **MISSING AS A WORKFLOW**: role/phrase/anchor grammar and coherent multiple seeded variations not supplied by onset + Euclidean generation + timing-only groove transfer + one THIN operator. |
| B C breakcore tools | **PARTIAL**: reverse/retrigger/pitch/choke/basic tails exist; envelope-shaped retriggers, cross-source structure, resampling/density arrangement acceptance missing. |
| B D AI automation | **PARTIAL**: typed headless batch/preview/commit/render exists; general coherent candidates, logs/diffs/comparison and async lifecycle missing. |
| B E compatibility/polish | **PARTIAL / OPTIONAL**: some tested native polish exists; presets/browser/stretch/groove generators, optional XRNS/XRNI and cross-platform performance remain. |

## Dependency-ordered continuation batches

These are dependency groupings, not promises that each broad group fits one patch.
Each concrete subset needs an exact contract, bounded implementation, UI/MCP
parity, old-JSON/PCM and history regressions, independent judgement and a clearly
scoped checkpoint. Update this ledger after acceptance, never in anticipation.

1. **Reusable explicit groove timing transfer — bounded foundation accepted**:
   typed persisted template, reviewed capture, target scope/strength and guarded
   baked apply. Audio-derived inference, accents/roles/humanization and the rest of
   original B5 remain separate continuation work, not implied completion.
2. **Melodic keyzones — bounded exclusive foundation accepted**: note/velocity/root
   mapping, independent selected-source voices, shared map UI and persistence.
   Broader piano/editor/quality/import, overlap modes and presets remain separate.
3. **Scoped render→immutable asset — bounded Unix foundation accepted**: cold
   occurrence/tail scope, no-clobber publication and separate guarded registration.
   Contextual crops, stems/A–B, cross-platform safe publication, persistent provenance
   and generalized recursive/compare workflows remain separate continuation work.
4. **Tempo/beat hypotheses and reviewed audio structure — bounded timing-point
   foundation accepted**: continue explicit beat/downbeat/meter/loop reasoning,
   role-driven hit/phrase/loop structure; native descriptors and explicit human
   role metadata are now a bounded accepted foundation, not calibrated recognition
   or grammar. Preserve corrections and mixed hits; timing/rule scores are not
   calibrated confidence or genre-quality proof. Dataset acquisition,
   deep learning and ML integration remain explicitly excluded.
5. **Seeded structured operators and genre grammar — bounded THIN foundation accepted**:
   one current-role-protected deletion operator with a transient exact record.
   Broader position/genre anchors, compatible substitution, multi-scale variation,
   multiple candidates and durable mutation records remain; musical intent and
   listening require separate evidence from deterministic gate decisions.
6. **Sampler editing / loop / resampling / voice-policy completion** — fine pitch,
   gain/fades/snap, loop variants/tails, HQ quality and bounded click-safe stealing;
   do not bundle all DSP risk into one unreviewed change.
7. **Assets and workflow completion** — AIFF, portable/relink, configurable caches,
   waveform/browser/presets and broader role-driven/multi-bank workflow; reviewed
   single-bank marker→kit allocation is accepted, while optional codecs remain
   separate decisions.
8. **Mixer / FX / automation** — bounded track balance and sample-peak meters are
   accepted; RMS/loudness/true-peak analysis, buses/sends, filters and processing, locks and tempo automation remain. Extend
   the existing core rather than adding another engine.
9. **Optional/later extras** — individually scoped from the table above, not hidden
   under a single “complete sampler” checkbox.

**Parallel explicit gates:** host/device/performance/listening; additional authored
synthetic fixtures and already authorized private inputs; and general public-
evaluator traversal-budget hardening. The accepted
M10 and groove entrypoint caps are local preview guards and must not be advertised
as a global evaluator fix.

## Updating this ledger

For each changed row, record the exact accepted subset and receipt; retain what
is still missing. Keep deprecated/historical evidence intact. Successful compile,
synthetic PCM, UI pixels and host listening are different evidence classes.
The original first-release definitions (drag samples, complete melodic-piano workflow,
low-latency live play, deterministic WAV/stems, coherent editable break generation)
remain **not fully met** until their missing and physical gates are explicitly closed.
