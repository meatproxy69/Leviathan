# Leviathan local MCP — API 1

Leviathan now includes **64 labeled MCP tools and 55 typed edit operations** for an
isolated, headless song-editing session. It uses the same `Song` validation,
`Editor` undo/dirty transactions, project writer and song evaluator/renderer as the
desktop application. It is **not** a controller for an already-open GUI.

## Run explicitly, without installing a service

```sh
cargo build --locked --bin leviathan_mcp
./target/debug/leviathan_mcp --workspace-root /absolute/path/to/music-workspace
```

The root must already exist and have a UTF-8 path. The server never changes the
working directory, starts a network listener, installs a service, registers itself
with an MCP host, requests credentials, or opens an audio device. The host launches
it as a child process. Closing its stdin ends the session; unsaved memory-only work
is discarded. Use the release binary for ordinary use if desired.

Example **illustrative** host configuration (replace both paths; exact host format
varies). This file has not been installed or registered anywhere:

```json
{
  "mcpServers": {
    "leviathan": {
      "command": "/absolute/path/to/Leviathan/target/debug/leviathan_mcp",
      "args": ["--workspace-root", "/absolute/path/to/music-workspace"]
    }
  }
}
```

### Supported protocol profile

This implementation deliberately supports **MCP `2025-11-25`**, using its
`initialize` / `notifications/initialized` lifecycle and newline-delimited stdio.
Current `2026-07-28` MCP uses a different, per-request negotiation model; that modern
profile is **not implemented**. Use a host supporting the legacy profile or fallback.
An initialization request for another version receives the implemented version;
the client must disconnect if it cannot support it.

Reviewed official sources:
[2025 lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle),
[2025 stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports),
[2025 tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools),
and [current versioning/compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).
No SDK dependency was added: the bounded stdio profile uses existing `serde` and
`serde_json`. This is tested interoperability for the stated profile, not a claim
of universal MCP certification or support for every protocol revision.

Send one JSON object per line. stdout contains JSON-RPC responses only; diagnostics
and usage go to stderr. For example:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"My agent","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"leviathan_capabilities","arguments":{}}}
```

Only the tools capability is advertised. `ping` is supported. Notifications receive
no response. Tools are a static single page; resources, prompts, tasks, subscriptions,
HTTP and asynchronous cancellation are not advertised. Synchronous export, import, source inspection and transient analysis occupy the
process until completion; their input/output budgets are finite, but they do not
promise a wall-clock deadline. The host can terminate the child if needed; there
is no in-process cancel/job API. Analysis never mutates the document or creates files.

## Discoverability and result contract

Each tool has a stable name, human-readable title, description, strict typed JSON
Schema 2020-12 input, output envelope schema and conservative annotations. Run
`tools/list` rather than guessing fields. `leviathan_capabilities` identifies the
application version, API version, protocol pin, root, limits and explicit unavailable
features. Tool-result structured content is mirrored as serialized JSON text:

```json
{"ok":true,"revision":3,"data":{"changed":true}}
```

A business/validation failure is a normal tool result with `isError:true` and:

```json
{"ok":false,"revision":3,"data":{},"error":{"code":"revision_conflict","message":"Read current state and retry against its revision; no changes applied","details":{"expected_revision":2,"actual_revision":3}}}
```

Malformed JSON/JSON-RPC, unknown methods, unknown tool names, and malformed
`tools/call` envelopes use JSON-RPC errors instead. Invalid argument shapes or
musical/path constraints produce inspectable tool errors. Batch failures identify
`operation_index` and confirm `atomic:true`. Important codes include
`invalid_arguments`, `revision_conflict`, `unsaved_changes`, `path_denied`,
`validation_error`, `not_found`, `document_too_large`, `output_exists`,
`overwrite_confirmation_required`, `invalid_overwrite_token`, and `io_error`.
Project-open policy failures can be wrapped as `validation_error` by the shared
project loader; the message identifies the denied asset before any asset read.

## Tool coverage

| Tool | What it reaches |
| --- | --- |
| `leviathan_capabilities` | Versions, supported operations/effects, boundaries and limits |
| `leviathan_project_read` | Summary, full persistent song, tracks, persisted track-mix settings, instruments, patterns, phrases, drum kits/pads, melodic multisample maps/key zones, source metadata, regions, accepted marker sets, historical generation/current-content match, frozen groove templates, order or history state |
| `leviathan_project_new` | Same blank document as desktop New; explicit discard guard |
| `leviathan_project_open` | Root-confined `.leviathan.json`, shared loader, pre-read asset policy |
| `leviathan_project_save` | Shared atomic save; new path or explicitly confirmed replacement |
| `leviathan_project_prepare_overwrite` | Single-use confirmation token bound to destination bytes and current revision |
| `leviathan_edit_batch` | 55 typed song/track-mix/phrase/kit/pad/step/alias/mute/region/catalog/marker/lock/mapping operations below; source identities come only from verified import |
| `leviathan_history_undo` / `leviathan_history_redo` | Shared 64-entry editor history; one batch is one undo step |
| `leviathan_song_evaluate` | Shared event manifest and declared source provenance; compilation does not verify current source bytes |
| `leviathan_song_export` | Shared renderer; new stereo float32 48 kHz WAV and `.events.json` pair |
| `leviathan_sample_generate` | Existing deterministic one-second demo sample generator; new WAV only |
| `leviathan_workspace_list` | Immediate regular `.wav` / `.flac` / `.leviathan.json` files, no generic filesystem browsing |
| `leviathan_sample_import` | Root-confined strict WAV/FLAC decode, immutable source ID/hash/native metadata, one undo transaction |
| `leviathan_audition_prepare` | Shared bounded region-draft or committed-pad snapshot summary; metadata only, no playback, source-byte verification or output files |
| `leviathan_sample_inspect` | Verify registered source bytes and decoded metadata; independent region, kit/pad and multisample/keyzone mapping pages, not PCM |
| `leviathan_multisample_inspect` | Bounded exact melodic map/zone metadata and independently paginated assigned instruments; no source IO |
| `leviathan_resample_prepare` | Pure shared cold-occurrence/all-track scope, bounds/fresh IDs and review token |
| `leviathan_resample_render` | Explicit durable new pre-master WAV publication through the same Engine, no document edit |
| `leviathan_resample_inspect` | Bounded source identity pages and prepared/printed receipt, no fresh IO verification |
| `leviathan_resample_register` | Reverify published artifact then one-undo sample/whole-region/Pitched-instrument registration |
| `leviathan_tempo_analyze` | Shared bounded timing-point BPM/pulse-phase hypotheses, no automatic write |
| `leviathan_tempo_inspect` | Exact accepted/cache point or candidate-pulse pages with provenance and ambiguity |
| `leviathan_tempo_use_bpm_preview` | Review one explicit candidate for a same-source marker set, BPM only |
| `leviathan_tempo_use_bpm_commit` | Guarded one-undo source-BPM metadata change, never phase application |
| `leviathan_slice_features_analyze` | Verified native source/bounds descriptors and transparent heuristic role suggestions, never automatic labels |
| `leviathan_slice_features_inspect` | Bounded pending measurements or stored historical catalog pages; metadata-only, with explicit binding status |
| `leviathan_slice_catalog_preview` | Review descriptor storage and only explicitly selected role/articulation changes |
| `leviathan_slice_catalog_commit` | Fresh-source-verified, exact-base/revision guarded catalog transaction, one undo |
| `leviathan_thin_preview` | Shared seeded pattrns whole-cell deletion proposal with reviewed-role protection across all resolved uses |
| `leviathan_thin_inspect` | Independently paginated exact decisions, uses, contexts and reversible before→None changes |
| `leviathan_thin_commit` | Full-base/revision-guarded ordinary-cell deletion transaction, one undo; no live randomness |
| `leviathan_sample_analyze` | Shared bounded transient suggestions and waveform overview in original source frames; no mutation |
| `leviathan_marker_cleanup_preview` | Shared read-only cleanup proposal with each removed boundary/reason; never auto-accepts |
| `leviathan_marker_bank_preview` | Pure accepted-marker interval selection and complete fresh region/kit/instrument/pad plan, no BPM or source-file IO |
| `leviathan_marker_bank_commit` | Selected-source lexical path plus fresh SHA verification, exact-base/revision/token guards and one-Undo metadata allocation |
| `leviathan_break_compile_preview` | Verified-source, nonmutating shared compiler diff/config/generated IDs and review token |
| `leviathan_break_compile_commit` | Revision/base/hash-guarded allocation, target tempo and ordinary pattern/order changes in one undo |
| `leviathan_onset_v2_prepare` | Explicit experimental engine/preset, verified native-PCM cache, effective config/bindings/counts; no document mutation |
| `leviathan_onset_v2_repick` | Shared cached picker/refiner without FFT/decode; explicit phase/preset/sensitivity controls |
| `leviathan_onset_v2_evidence` | Bounded pages of raw source-frame multiband evidence, not normalized confidence |
| `leviathan_onset_v2_results` | Separate bounded pages of actual coarse candidates or refined suggestions, with independent totals |
| `leviathan_drum_steps_inspect` | Exact ordinary-cell pad lanes, page/grid origin, source provenance/conflicts and revision/base/view-bound edit token |
| `leviathan_matrix_inspect` | Bounded occurrence×track pages, direct aliases/shared source chains, stored versus resolved cell counts and per-occurrence mutes |
| `leviathan_transform_preview` | Shared deterministic scoped transform, effective options/context summary and private review token |
| `leviathan_transform_inspect` | Bounded exact before/after parent-cell diff pages from the reviewed shared plan |
| `leviathan_transform_commit` | Exact-base/revision/path-guarded ordinary-cell commit, one undo; no caller candidate |
| `leviathan_generation_preview` | Actual pinned pattrns Euclidean preset with explicit version/seed/finite config, checked fresh IDs and historical provenance |
| `leviathan_generation_inspect` | Bounded exact shared generated-cell/timing/provenance pages, no caller patch |
| `leviathan_generation_commit` | Reviewed fresh track/pattern/order allocation, one undo, ordinary editable Song content |
| `leviathan_cell_block_copy` | Freeze an exact bounded tracker rectangle, including holes/default cells, in one private same-document clipboard |
| `leviathan_cell_block_preview` | Shared whole-cell Paste replace or Clear plan; explicit scope, owner/revision/exact-base guards, no source IO |
| `leviathan_cell_block_inspect` | Independent bounded clipboard-position, exact-change, affected-alias or occurrence-context pages |
| `leviathan_cell_block_commit` | Edit-Mode-protected shared rectangle transaction; one Undo if changed, true no-op preserves redo |
| `leviathan_track_mix_preview` | Pure complete-set gain/balance/mute/solo review with explicit compiled-routing status and shared-legacy constraints; no live GUI command |
| `leviathan_sample_peaks_analyze` | Bounded device-free full-context Song sample peaks, exact aligned windows and verified used region sources; no files, PCM payload or live GUI readback |
| `leviathan_sample_peaks_inspect` | Exact bounded pages from one retained complete offline report; metadata only, no fresh source verification |
| `leviathan_track_mix_commit` | Guarded one-Undo persisted settings in this headless Song only; future start/export use, no mailbox/device access |

`project_read` returns the current revision and stable IDs. IDs are nonzero `u64`
values, not list positions; names and order do not change them. JSON clients should
preserve integer precision, or choose IDs within their language's exact-integer
range. Row, column and operation indices are zero-based. Native in-memory sample
references in read results are absolute; supply workspace-relative sample paths
when editing. The capabilities result provides the explicit root.

### Groove review tools

| Tool | Capability |
|---|---|
| `leviathan_groove_capture_preview` | Pure accepted-marker recipe preview at a required fresh GrooveId |
| `leviathan_groove_capture_commit` | Guarded one-undo registration of the reviewed immutable template |
| `leviathan_groove_template_inspect` | Paginated stored raw anchors and historical provenance; no source access |
| `leviathan_groove_apply_preview` | Pure scoped timing-only transfer with explicit strength0..100 |
| `leviathan_groove_preview_inspect` | Exact pending capture anchors or apply CellChange records |
| `leviathan_groove_apply_commit` | Guarded one-undo ordinary-cell edit; no-op preserves redo/revision |

### Typed edit operations

| `op` | Semantics |
| --- | --- |
| `set_song` | Optional title/BPM/LPB; omitted or null fields preserved |
| `set_track_mix` | Complete versioned gain/balance/mute/solo set, same shared validation as preview; empty resets neutral defaults, not a patch or live GUI update |
| `upsert_track` | Stable ID, name and note-column count; new tracks append to display order |
| `remove_track` | Reject references unless `clear_cells:true` explicitly clears cells; a persisted mix entry always requires explicit full-set removal first |
| `set_track_order` | Every track ID exactly once |
| `upsert_instrument` | Stable ID, name and **required** legacy sample: explicit path selects Pitched and switches away from region/kit/multisample; null clears legacy path but preserves region/kit/multisample kind |
| `remove_instrument` | Reject if cells still reference it |
| `upsert_pattern` | Name/row count; retain cells, reject shrinking over occupied rows |
| `remove_pattern` | Reject if order still references it |
| `clone_pattern` | Independent deep copy to a new stable ID, materializing aliases; does not append order |
| `set_order` | Complete replacement by **fresh unmuted occurrences**; repeated IDs share content; use `move_occurrence` to carry existing mute metadata |
| `append_repeat` | Append a **fresh unmuted** occurrence of existing shared pattern content |
| `set_track_alias` | Required nullable source block; link empty target or explicitly unlink without copying; source content is unchanged |
| `make_track_unique` | Materialize resolved scalar content at the target and detach; local block is a no-op |
| `set_occurrence_track_mute` | Explicit boolean for one order index/target TrackId; no global pattern mute |
| `move_occurrence` | Swap complete occurrence records at two indices, carrying mutes; not insertion/rotation |
| `set_cell` | Replace note/instrument/velocity/pan/delay/effects at explicit pattern/track/row/column |
| `clear_cell` | Remove all fields and effects from one cell |
| `set_effects` | Replace effects only, preserving other cell fields |
| `clear_pattern_track` | Explicitly clear one track's cells in one pattern |
| `upsert_phrase` | Create an empty instrument-bound phrase or update its metadata without dropping cells |
| `remove_groove` | Remove a historical timing template; baked ordinary cells and source/marker data are unchanged |
| `remove_phrase` | Remove only an unreferenced definition; no implicit cascade |
| `set_phrase_cell` | Replace one phrase row/column's scalar musical fields and supported effects |
| `clear_phrase_cell` | Remove all fields/effects from one phrase cell |
| `set_phrase_trigger` | Replace a pattern cell's phrase reference only; required nullable trigger, other fields preserved |
| `drum_step_toggle` | Shared exact page-local pad toggle; conflicts/unsafe allocation reject; source metadata remains on note removal |
| `drum_step_set_velocity` | Replace/clear only an inspected exact parent event's velocity; required nullable value |
| `drum_step_set_delay` | Replace/clear exact source-row delay without quantization; required nullable f64 value |
| `drum_step_set_retrigger` | Replace/clear parent-cell retrigger; children are not independent notes; required nullable value |
| `create_multisample_instrument` | Shared checked fresh-ID allocation of a new empty melodic map and new assigned instrument, preserving occupied song content |
| `upsert_multisample` | Create or rename a mutable shared map while preserving zones |
| `set_keyzones` | Explicit complete zone-array replacement; duplicate IDs/overlapping rectangles/dangling references reject; empty array clears |
| `remove_multisample` | Remove only an unassigned map; regions/files preserved |
| `assign_multisample` | Required nullable map ID; Some deliberately switches source, null detaches only current Multisample and is a no-op on other kinds |
| `create_drum_kit_instrument` | Shared checked fresh-ID allocation of an empty kit plus a NEW assigned instrument, preserving occupied song content; one batch/undo |
| `upsert_drum_kit` | Create an empty kit or rename while preserving every pad; stable kit ID and name |
| `remove_drum_kit` | Remove only an unassigned kit; no region/source deletion |
| `upsert_drum_pad` | Replace pad metadata by stable ID; null region preserves existing zones, nonnull region on a zoned pad rejects. Explicit source-mode replacement uses the next operation |
| `set_pad_sources` | Replace single-region / velocity-zone deterministic round-robin / unassigned mode atomically; both region ID and complete zone vector are required |
| `remove_drum_pad` | Remove one pad only if all current note mappings remain valid, including unordered patterns |
| `assign_kit` | Required kit ID or explicit null; selecting a kit clears legacy/region source; null returns to Pitched without clearing an already pitched source |
| `remove_sample` | Remove an unreferenced imported source record; never delete a file |
| `upsert_region` | Create/edit name, source ID, inclusive start/exclusive end source frames and root MIDI note; preserve stored playback options and reject invalidating a loop |
| `set_region_playback` | Replace both required nullable region amp envelope and forward loop through the shared model helper; original source audio is unchanged |
| `remove_region` | Remove only an unmapped region; no implicit cascade |
| `assign_region` | Required region ID or explicit null; selecting a region clears legacy/kit source and selects Pitched; null only unmaps region |
| `upsert_marker_set` | Replace accepted boundaries/metadata while preserving saved locks; explicit optional source BPM, separate from suggestions |
| `remove_marker_set` | Remove an unlocked set, not source bytes or already generated ordinary patterns |
| `set_marker_lock` | Explicitly lock/unlock an existing internal boundary; endpoints always fixed |
| `add_marker` | Insert one exact free internal source frame; no automatic snap/lock |
| `move_marker` | Move one unlocked internal boundary; duplicate/endpoint/locked edits rejected |
| `remove_marker` | Remove one unlocked internal boundary, never a fixed endpoint |

The cell representation matches the shared model: `{"Note":60}` is MIDI pitch 60,
`"Off"` releases pitched/Gated voices (OneShot ignores ordinary OFF while its column
pointer clears), and `"Empty"` does not specify a note. Missing/null optional
cell values are unspecified; explicit velocity `0` is silence, not absence.

Supported typed effects are **retrigger, timed release, glide, hard cut, sample
offset and direction**. The advertised schemas describe units, nullable fields and
basic ranges; shared validation applies combinations and contextual constraints:

- Retrigger has 1–16 additional starts, spaced in fractions of a row after the delayed
  original note. All starts must remain in the row.
- Release is an absolute row fraction that initiates the current release policy:
  legacy de-click or opted-in region `release_ms`. A zero-ms envelope release can
  stop abruptly; the separate Cut command remains an unconditional hard stop.
- Glide has a MIDI target and 0.01–120 semitones/second rate, not a duration.
- Cut is an abrupt active-voice stop at an absolute row fraction, at/after delayed
  starts/retriggers. It may click and cannot combine with release or an OFF cell.
- Offset is a zero-based **source** frame, bounded by `u32::MAX` in the document and
  by the actual decoded sample during rendering. Mapped-region offsets are absolute
  source frames and must stay within that region's inclusive start/exclusive end.
  Any offset is incompatible with a used forward-loop voice.
- Direction is `"Forward"` or `"Reverse"`. A reverse new note starts at the last real
  source frame unless offset is supplied. Effect-only commands need an active voice;
  these controls do not latch onto unrelated future notes. Reverse is incompatible
  with a used forward-loop voice; explicit Forward remains supported.

Legacy-path advanced sample controls require nonempty mono/stereo sources at 1–384,000 Hz,
with at most 64 MiB total decoded controlled PCM (and a 64 MiB controlled input-file
limit). Embedded sample loops are explicitly rejected. Any offset or direction
command anywhere in a voice selects the linear-interpolation adapter for that
voice's entire lifetime, including samples before a later command. This does not
claim phonic's antialiasing parity. Unsupported effect codes are rejected, not
silently accepted as working effects.

### Instrument-bound phrases (M7)

Definitions live in `Song.phrases`, keyed by globally stable `PhraseId`, and each
binds to one existing instrument. `project_read` with `section:"phrases"` returns
these actual shared definitions; summary state includes `phrase_count` and
`phrase_trigger_cell_count` (stored pattern cells, not expanded playback triggers).
The full `document` and `patterns` sections also expose trigger references.

`upsert_phrase` requires `id`, `name`, `instrument_id`, `rows`, `lines_per_beat`,
`note_columns` and `root_note`. Metadata updates preserve all existing sparse cells.
`set_phrase_cell` and `clear_phrase_cell` use `phrase_id`, `row` and `column`.
They delegate the shared validated `Song` methods, just as the Editor wrappers do;
there is no MCP-only phrase renderer or separate musical interpretation.

A pattern cell can include `"phrase":{"phrase":41,"cycles":2}` in `set_cell`, or
an existing cell's reference can be changed without altering its other fields:

```json
{
  "op":"set_phrase_trigger",
  "pattern_id":1,"track_id":1,"row":0,"column":0,
  "trigger":{"phrase":41,"cycles":2}
}
```

`trigger` is required for this operation; explicit null clears only the reference.
The cell's pitched note remains, so use `clear_cell` if the entire note should go.
The trigger must use a `Note` value. For pitched instruments, its pitch transposes phrase notes and glide
targets relative to `root_note`; out-of-range MIDI results are rejected. Kit phrases
require a neutral trigger equal to the phrase root; inner notes remain pad selectors. Explicit
instrument fields on the trigger or inner phrase cells must match the bound
instrument. Omitting the inner instrument uses that bound instrument.

The phrase has its own LPB and 1–64 **total** finite cycles, with `1` a one-shot.
Trigger cells permit delay, velocity, pan and parent release/cut; put other effects
inside phrase cells. Trigger gain multiplies internal velocity; trigger pan adds to
internal pan and clamps to -1..1. These are immutable trigger snapshots, not global
instrument controls. Phrase cells support the same six scalar step effects but
cannot trigger nested phrases. There is no recursive expansion, aliasing or
unbounded phrase loop. Limits are 128 definitions, 1–1,024 rows, 1–16 note columns,
LPB 1–256, root MIDI note 0–127, and 32,768 total definition cells. Evaluated limits
also cap phrase trigger occurrences at 256, expanded actions at 65,536 and note
starts at 2,048; errors fail closed. Ordered phrase triggers use manifest schema 4
with phrase/instance/internal-column/cycle/row provenance; songs without ordered
triggers retain the prior evaluator path and manifest versions, even if they contain
unused phrase definitions. Manifest instance IDs identify one expansion in that
snapshot; unlike document PhraseIds, they are not persistent editing handles.
Pattern clones copy their trigger cells but continue referencing the same phrase
definitions, so editing a definition affects every reference to it.

Edits that shrink over occupied cells, rebind to an invalid instrument, break
existing trigger transpositions, or delete referenced definitions are rejected
atomically. Clear references before removal. Existing instrument upserts leave phrase definitions intact. Explicit non-null
legacy sample assignment switches away from a region; null preserves a region. Old phrase-free
projects and operations remain valid; empty phrase maps and absent trigger fields
are omitted from serialization, rather than being injected into old documents.

### Verified sources, editable region boundaries and transient suggestions

`leviathan_sample_import` takes `expected_revision`, `sample_id` and a root-relative
`path`. It uses the shared strict WAV/FLAC importer, computes SHA-256 over the
original encoded bytes, and registers real native `sample_rate`, `channels` and
`frames` without resampling, normalization or file modification. The decoder's
extra interpolation guard frame is **not** part of the reported frame count. A
source ID is immutable: repeating the same import is a no-op; changed identity or
metadata requires a new ID. Clients cannot supply arbitrary hashes or metadata.
Import is one undo step, does not assign an instrument, and never copies/deletes
source files. Files must remain available under the authorized root.

`project_read` sections `samples` and `regions` expose the shared persistent maps;
summary includes `sample_count` and `region_count`. Native paths in inspection are
absolute, as with older sample paths. Use workspace-relative paths for import.
`sample_inspect` verifies the current encoded hash **and** decoded metadata and
returns independent pages of referencing regions and pad-source mappings, with
explicit totals/next offsets (see below). It does not return PCM. Open/save verify all registered
source identities, even unused ones; engine export verifies each used source before
writing output. `song_evaluate` only compiles events and expected source provenance:
its `source_identity_verified:false` is explicit. It is not a hash/decode check.

Region boundaries are editable non-destructive views, in **original source frames**:
`0 <= start_frame < end_frame <= source.frames`; start is inclusive, end exclusive.
`root_note` plays the source at its native rate independently of song BPM. Notes
other than root intentionally pitch/rate-shift; this chunk does not stretch audio.
Regions default to finite one-shots; explicit optional forward loops are described
below (embedded file-loop metadata is not imported). Editing a region changes
all instruments and kit pads mapped to its ID. Use another region ID for an independent view.
Source data remains intact. Model limits: 128 sources and 4,096 regions; mono/stereo,
1–384,000 Hz, at most 64 MiB encoded and 64 MiB decoded per import. Engine decoded
budget is shared across used controlled sources; shared open/save also caps total
registered encoded bytes at 256 MiB. Unsupported/corrupt/truncated input fails closed.

Map with `assign_region`, or unmap with **required** `region_id:null`. Choosing a
region explicitly replaces a legacy sample path; choosing a non-null legacy sample
in `upsert_instrument` explicitly clears the region. Important for old clients:
`upsert_instrument` with `sample:null` clears the legacy path **but preserves an
existing region** (including rename-only usage). To make an instrument completely
unassigned, clear its region explicitly as well. Raw documents with both source
forms are invalid. Referenced region/source deletion is rejected; clear mappings,
remove regions, then remove source records in one ordered batch if desired.

`sample_analyze` takes `expected_revision`, `sample_id`, and optional strict `config`:
`sensitivity` 0–1 (default 0.5), `min_spacing_ms` 1–1,000 (default 25), `minimum_rms`
0–1 (default 0.00001), `max_suggestions` 1–4,096 (default 4,096), and `waveform_bins`
0–2,048 (default 1,024; zero disables overview). Omitted/null option values use
defaults. MCP analysis is synchronous and stricter than the backend: sources must
be 8,000–384,000 Hz, at most **60 seconds** and 64 MiB decoded. It fails rather than
truncating input or excessive suggestions; the client cannot raise these caps.

Analysis verifies source bytes/metadata first, excludes the guard frame, and calls
the same immutable-PCM adaptive energy/first-difference detector as the backend.
Results include source SHA-256, algorithm/config provenance, sorted source-frame
suggestions, heuristic confidence and per-channel waveform extrema. The internal
fingerprint is a deterministic cache/debug hint, **not** the cryptographic asset ID.
This is not FFT/spectral flux, beat/tempo detection or a calibrated probability of
correct onset identification; detector resolution is roughly 2–6 ms. Identical
inputs/configuration are deterministic on the same floating-point platform.

Suggestions are ephemeral proposals, not accepted manual markers. Analyze success,
failure and limit errors never change revision/history, accepted marker sets,
region boundaries, BPM or source audio. Accept/edit desired boundaries through
`upsert_marker_set` for compilation, or `upsert_region` for direct mapping. Example:

```json
{"expected_revision":0,"sample_id":7,"path":"break.flac"}
```

After `sample_import`, use its returned revision for `sample_analyze`, inspect the
suggestions, then accept selected boundaries in a single `edit_batch`:

```json
{
  "expected_revision":1,
  "operations":[
    {"op":"upsert_region","id":21,"name":"First hit","sample_id":7,
     "start_frame":1200,"end_frame":5400,"root_note":60},
    {"op":"assign_region","instrument_id":1,"region_id":21}
  ]
}
```

Those example bounds must fit the actual imported source; analysis never invents
accepted regions automatically. Existing typed note edits can render a mapped
instrument through offline export. The shared compiler below generates ordinary
editable patterns from explicitly accepted full-source boundaries.

### Optional experimental onset v2 — v1 remains default

The existing `leviathan_sample_analyze` arguments and behavior are unchanged and
remain the default v1 path. Experimental v2 is a separate explicit opt-in:

```json
{
  "expected_revision":2,"sample_id":7,"engine":"experimental_v2",
  "preset":"Balanced","picker":{"threshold":0.52,"enable_phase":false}
}
```

Pass that to `leviathan_onset_v2_prepare`. Supported presets are `Conservative`,
`Balanced` and `GhostHunter`. The shared application policy
`experimental-phase-off-v1` starts with phase support **off**; it does not relabel
or rewrite the separately judged core phase-on presets. The optional explicit
`picker.enable_phase:true` enables supporting phase evidence, never a standalone
trigger. The actual flag is returned in `phase_enabled` and `effective_config`.
The algorithm/version, complete effective config, source SHA/native metadata,
extraction key and full config SHA are always inspectable.

This is experimental multiband spectral/energy/fast-attack evidence, **not** a
calibrated probability, automatic tempo inference, drum classifier or ML model.
Vibrato, chirps and beating tones can create false positives; weak dense hits can
still be missed. `GhostHunter` is a sensitivity preset, not an instrument label or
a guarantee of better results. Phase-off changes false-positive/recall tradeoffs,
not a claim of universal improvement. No dataset is downloaded or trained here.

The shared `onset_service` performs strict root-authorized native WAV/FLAC decoding,
verifies encoded SHA-256 and metadata, and excludes the interpolation guard frame.
MCP holds **one** immutable raw-evidence/native-PCM cache and one bounded shared
analysis result. A new verified preparation retires its old cache **before**
extraction so caches do not accumulate; a later preparation failure leaves no
cache and reports `previous_cache_retired:true`. Argument/revision/path/duration
preflight failures do not start preparation. Nothing changes document revision,
undo/redo, accepted markers, locks, regions, BPM or source files.

The response contains `cache_token`, `effective_config`, `source_analysis_summary`,
resource counts and proposed-only marker capacity. It intentionally does **not**
inline thousands of candidate/suggestion records. Use:

- `onset_v2_results` with `kind:"candidates"` for coarse detector peaks, or
  `kind:"suggestions"` for refined boundary estimates. `start` is a zero-based
  record index; default/max `limit` is 64. Each kind has its own `total` and
  `next_start` because refinement collisions can reduce the suggestion count.
  `start == total` returns an empty final page; out-of-range pages fail.
- `onset_v2_evidence` for raw cached multiband curves. `start` is a curve-bin index,
  default `limit` 128, maximum 256. Each point's `source_frame` is an original
  native frame, with hop information returned. These are raw unthresholded curves,
  **not** normalized confidence, accepted labels or a spectrogram. The full
  normalized envelope is not retained by this MCP profile.

Every page includes source/algorithm/config binding and a next offset; incomplete
pages never masquerade as complete results. Candidate `peak_frame` and refined
`attack_frame`, search bounds and shift remain separate. Confidence is heuristic.
The result summary counts unique **proposed** refined attacks plus fixed endpoints;
a source-start attack is counted only once. `fits_one_marker_set` does not imply
that unioning proposals with arbitrary existing manual markers will fit. More than
257 accepted boundaries is an explicit model/tool error, never silent truncation.
Select/review/clean proposals or edit individual boundaries before accepting.

`onset_v2_repick` takes the current `cache_token`, `expected_revision` and `picker`.
It verifies the current source bytes again, then invokes the same picker/refiner
on the server-owned cache with **no FFT and no source decoding**. This is not a
zero-work operation: hashing, normalization, picking and refinement remain bounded
work. Omitted/null picker fields preserve values. An optional `preset` resets to
shared phase-off preset defaults before applying overrides, matching the UI.
Supported picker controls are phase support, `threshold` (0.1–0.95; lower is more
sensitive), `mad_multiplier` (0.5–12), `median_radius_ms` (20–250), `min_gap_ms`
(1–1,000), `dense_gap_ms` (1..min gap), `dense_evidence` (0.5–1), and refinement
`enabled`, `backtrack_ms` (0–100), `lookahead_ms` (0–50), `attack_fraction`
(0.001–0.5). Spectral bands/stereo/RMS floor/resource settings are not caller
picker overrides. No client-supplied `Detection` or arbitrary PCM is accepted.

A successful repick returns a **new token bound to the full effective config SHA**
and invalidates the previous token, while reusing the same immutable extraction
cache. A failed repick preserves the current token/result. Source/config/algorithm
bindings cannot be overridden by clients. Every repick/result/evidence request
reauthorizes the path and actually rehashes current source bytes before exposing
cached information; `current_source_sha_verified:true` is not a claim based only
on an old snapshot. Tampering/symlink replacement fails closed without silently
reloading a different source. Any successful document revision—including edit,
undo/redo, New/Open or Save—invalidates all experimental tokens, even after ABA
content changes. These tokens are local review/cache handles, not authentication.

This MCP profile is deliberately **synchronous and bounded**, not an async job API:
8,000–384,000 Hz, at most **15 seconds** of source, 64 MiB encoded/decoded source
limits, 4,096 proposals, 500 million estimated work units, 128 MiB estimated working
memory, 96 MiB retained native PCM/raw curves, and 8 MiB serialized retained
analysis. The duration is stricter than the desktop worker's general backend limit;
parity is for identical effective configurations, not different resource settings.
Summaries and each result/evidence page are at most 256 KiB structured payload
before MCP text mirroring/JSON-RPC overhead. These are explicit accounting limits,
not measured wall-clock deadlines or OS memory quotas. Input/output/work excess
fails rather than truncating. The host can terminate the child; no automatic marker
mutation or source-file output needs to be rolled back.

Use `add_marker`/`move_marker` to choose an exact peak or refined attack frame,
`set_marker_lock` to preserve an accepted boundary, or explicitly replace a reviewed
full marker vector. V2 never silently accepts proposals or overwrites prior manual
work. GUI-only visualization toggles/rejection drafts are not synchronized with
this isolated headless session; their persistent editing outcomes use the same
validated marker APIs.

### Accepted markers and reviewable break compilation (M8b)

Accepted boundaries live in `Song.marker_sets` under stable `MarkerSetId` values.
`project_read` with `section:"marker_sets"` returns them; summary includes
`marker_set_count`. `upsert_marker_set` requires `id`, `name`, `sample_id`,
`source_bpm` and `markers`. The entire vector is replaced explicitly, but existing saved locks are preserved.
There are at most 128 sets, each with 2–257 strictly increasing original-source-frame
boundaries, **including exactly frame 0 and the real source end**. This creates
1–256 slices. Source origin is always file frame 0, not an inferred first beat.
The required nullable `source_bpm` is either an explicit 20–400 BPM value or null
when unknown. Null can be saved, but compilation refuses to guess a tempo.

`locked_frames` persists sorted unique internal boundaries (empty lists are omitted
from old JSON). Endpoints are always fixed implicitly. Existing full upserts retain
the exact lock list; dropping a locked boundary, rebinding its source or deleting a
locked set is rejected. Only explicit `set_marker_lock` changes lock status.
`add_marker`, `move_marker` and `remove_marker` use exact native source frames and
shared validated Song/Editor helpers. An explicit unlock → move → relock sequence
can be one atomic batch/undo entry. Duplicate, out-of-range, endpoint or locked
operations never silently snap, merge or erase accepted boundaries.

`marker_cleanup_preview` takes a registered `sample_id`, up to 4,096 explicit
candidate frames, `minimum_slice_frames` and `expected_revision`. It verifies the
source, calls the shared cleanup helper and returns a proposed sorted full-source
set plus each discarded duplicate/near-start/near-end/too-close boundary and its
reason. It does not snap attacks, change a stored marker set, or consume a compiler
preview. Review and explicitly apply `upsert_marker_set` if desired. The helper
fails rather than silently truncating more than 257 resulting boundaries; cleanup
and compiler structured preview payloads are capped at 256 KiB (MCP's mirrored
text and JSON-RPC envelope add bounded wire overhead). These are synchronous bounded
tools, not asynchronous jobs.

`break_compile_preview` requires a current `expected_revision` and typed `config`:

```json
{
  "expected_revision":2,
  "config":{
    "marker_set":31,"target_bpm":170,"timing":"SourceRhythm","cycles":1,
    "roll":null,"tail":"CutPrevious","minimum_slice_frames":32,
    "root_note":60,"pad_tails":false,"name":"Break at 170"
  }
}
```

- `SourceRhythm` maps accepted source-frame positions through the **declared**
  source BPM to beat positions, then onto the target song sample-frame clock.
  `{"Grid":{"division":"Sixteenth"}}` instead puts consecutive slices on that
  explicit whole-note grid. Both modes require the declared source BPM; no mode
  claims to infer timing, instrument class or musical intent from a recording.
- `target_bpm` is an explicit **global song tempo change**, including pre-existing
  material. Existing LPB is preserved. The diff reports previous/target BPM and
  warnings; inspect them before committing. Source BPM never becomes a sampler
  playback-rate multiplier: generated notes at `root_note` play native pitch/rate.
- `cycles` is 1–16 total sequence repetitions. Optional `roll` is, for example,
  `{"division":"OneTwentyEighth","hits":8}`. Supported divisions are
  `Sixteenth`, `ThirtySecond`, `SixtyFourth`, `OneTwentyEighth` (1/16, 1/32, 1/64,
  1/128 of a whole note). `hits` is 2–16 **total** starts, including the original;
  every roll must finish before the next source/grid hit. Starts are calculated
  from absolute beat positions, not accumulated rounded frame spacing.
- `Overlap` retains native one-shot tails on allocated columns; `ReleasePrevious`
  releases the previous generated voice using the existing fade; `CutPrevious`
  stops it abruptly and may click. `pad_tails:true` conservatively extends the
  generated pattern to hold native tails (for release, potentially earlier tails
  too); the diff reports sequence, padding and total row counts. It is not stretch,
  crossfade, loop reconstruction or guaranteed equal-duration reverse playback.
- `minimum_slice_frames` is positive, in original-source frames. Shorter accepted
  slices fail with an actionable error; the compiler never silently cleans or
  moves markers. `root_note` is MIDI 0–127. `name` must be nonblank and at most
  256 UTF-8 bytes. There is no random generator or unused seed parameter.

The preview performs root-confined strict source verification and invokes the same
pure `break_compiler::preview` used by the Editor. It allocates a new region and
instrument per accepted slice, one new track/pattern, and an appended order entry
in its **private candidate**, never in the current document. The diff exposes exact
allocated IDs, source identity/BPM, target tempo, occurrence, slice/start/column/row
counts and warnings. It does not return an unbounded full-document patch. The
current document/revision/history remain unchanged; only one pending preview is
retained. A newer successful preview replaces the old token; a failed preview
preserves the last valid token.

After reviewing the diff, call `break_compile_commit` with the same revision and
returned `preview_token`. Commit reauthorizes the source path, verifies its encoded
SHA-256 again, and uses shared Editor revision plus exact base-document checks.
All new regions/instruments/track/pattern/order and the target tempo commit as **one
undo transaction**. Generated notes, delay/effect cells and patterns are ordinary
editable song content, not an opaque playback recipe. Later marker changes do not
silently rewrite earlier generated output. Source bytes are never changed.

Any successful document revision change—including edit, undo/redo, New/Open or
Save/Save As—invalidates pending previews, even if content later looks identical.
Wrong/stale tokens, source changes, allocation limits or validation failures do
not partially commit. A failed commit leaves its current pending preview intact;
a successful commit consumes it. Restoring the exact authorized source bytes can
allow retry after a failed hash check. Tokens are review handles, not authentication
or a substitute for user approval of the described changes.

Compiler bounds include the shared conservative 2,048 total arrangement starts
(including existing phrase/retrigger expansion), 256 generated note columns,
65,536 pattern rows, 4,096 regions and 600-second arrangement duration. ID exhaustion,
cell collisions, rolls reaching the next hit and unsafe allocation fail closed.
MCP additionally caps the candidate at 8 MiB and each structured preview payload
at 256 KiB before MCP text mirroring and the JSON-RPC envelope.
No claim of measured real-time latency or listening quality follows from these tests.

### Atomicity, revisions and history

Every mutation requires `expected_revision`. Successful document replacements,
changed batches/imports/compiler commits, undo/redo, and saves advance the revision. Read, evaluation,
export, sample generation/inspection/analysis, experimental preparation/repick/pages,
cleanup/compiler previews, overwrite preparation and no-op edits do not. All calls
are serialized in this one process; revision checks protect clients that planned
against stale state.

Change reporting follows the actual shared Editor revision delta, not approximate
floating-point equality. A `-0.0` to `+0.0` scalar-only edit is an exact content
change: it advances the MCP revision, invalidates pending reviews and participates
in dirty/Undo/Redo tracking. An exact no-op preserves revision and redo.

A batch executes on a candidate copy. Each ordered operation must produce a valid
`Song`; clear references before deleting their targets. A failure commits **none**
of the operations. The final candidate goes through shared
`Editor::apply_document_edit`, preserving dirty/no-op/undo/redo behavior and creating
one undo entry. The 8 MiB candidate size limit is checked after **each** operation,
so many clones cannot first amplify a small batch into an unbounded candidate.

This typed adapter is the initial application-operation boundary, not a second
sequencer or renderer. Existing GUI edits retain their shared Editor methods.
Future GUI IPC should reuse these typed operations and revision contract rather than
introducing another implementation; that IPC/refactoring is not present today.

New/Open clears history and refuses dirty replacement unless the current-revision
call explicitly includes `discard_unsaved:true`. Saves mark the same shared history
baseline as the desktop. Undo affects document edits (including compiler commits), **not** files already
saved or exported. Edits remain in this headless process until saved.

### Small end-to-end example

After initialization, a new server starts at revision `0` with a blank Untitled song
and stable track/instrument IDs 1–4 plus pattern 1. Calls below show **arguments**:

1. `leviathan_sample_generate`: `{"expected_revision":0,"path":"demo.wav"}`
2. `leviathan_edit_batch`:

```json
{
  "expected_revision": 0,
  "operations": [
    {"op":"set_song","title":"AI sketch","bpm":120,"lines_per_beat":4},
    {"op":"upsert_instrument","id":1,"name":"Demo","sample":"demo.wav"},
    {"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,
     "cell":{"note":{"Note":60},"instrument":1,"velocity":0.25,
             "effects":{"retrigger":{"repeats":1,"spacing":0.25},"release":0.75}}}
  ]
}
```

3. Read the returned revision (normally `1`), then
   `leviathan_song_export`: `{"expected_revision":1,"path":"sketch.wav"}`.
4. `leviathan_project_save`:
   `{"expected_revision":1,"path":"sketch.leviathan.json"}` (normally returns revision `2`).
5. Read state again before additional edits/undo or replacing an existing file.

To overwrite, first call `leviathan_project_prepare_overwrite` with the existing
path and current revision. Review/approve that target, then pass its `overwrite_token`
to `leviathan_project_save`. The token is single-use and invalidated by revision
changes; the shared writer compares the destination's exact bytes again before
replacement. A token is a confirmation mechanism, not authentication or permission
to overwrite without the user's authorization.

## Filesystem and resource boundaries

- The startup root is the only authorized workspace. Tool paths must be relative,
  have existing parent directories, and cannot contain parent traversal. Every path
  component is checked; symlinks and nonregular final files are rejected.
- A project's embedded sample references are checked by the shared loader **before**
  legacy sample metadata/open calls and registered source hash/decode access. Relative embedded paths resolve against that project's
  directory and still must remain inside the root. All assigned assets are checked
  again before save/evaluation/export; inspect/analysis authorize their source before
  hash/decode. Imported-source changes are detected by the shared byte identity checks.
- Save is restricted to `.leviathan.json`; export/generation to `.wav`. Export also
  checks the companion `.events.json`. New-file writers use no-clobber creation;
  exports and generated samples never overwrite existing targets.
- No directory creation/deletion, generic file-content read/write, shell, network,
  credential, system-setting or host-registration tool exists.
- Input is bounded to 1 MiB per line, drained safely if oversized. Batches have 1–256
  operations; documents have an 8 MiB serialized limit and shared history has 64
  entries. Workspace listing scans at most 2,048 immediate entries and reports
  truncation. Rendering keeps the core duration/trigger/sample limits.
- This is a **pathname confinement policy, not an OS security sandbox**. Static
  traversal/symlink escapes are rejected and tested. A hostile local process that
  concurrently replaces/renames directories between path validation and shared
  filesystem calls can still exploit TOCTOU. Use a trusted/private root; capability-
  based filesystem handles/OS isolation would be needed for that stronger threat
  model. No malicious-concurrent-filesystem-race resistance is claimed.

## Explicit gaps — not “everything in the GUI”

Not available: controlling an already-open GUI, GUI cursor/selection/Edit Mode,
live play/stop/seek/loop/device selection, legacy pattrns experiment panel/probes,
plugins, phrase aliases/nested phrase triggers, arbitrary effect-code compatibility,
format conversion, time stretch, HTTP hosting, protocol 2026-07-28, registration/deployment, and async task control.
These are unavailable capabilities, never reported as successful actions. Physical
latency and speaker audibility are not measured by this server or its tests.

The deterministic sampling workflow now includes strict WAV/FLAC import,
transient/cleanup suggestions, editable persistent accepted marker sets and region
boundaries, slice mapping, and reviewed shared compilation of ordinary patterns at
an independent target BPM with exact 1/16–1/128 rolls and explicit tail policies.
Bounded timing-point BPM/pulse-phase hypotheses and explicit reviewed Use BPM are
available, along with native slice descriptors and transparent heuristic role
suggestions. Full raw-audio tempo/downbeat/meter analysis, automatic BPM assignment,
authoritative automatic drum classification, automatic boundary acceptance, broader
seeded/random break rearrangement beyond reviewed whole-cell THIN, time stretching
and GUI IPC remain absent. Source playback at its
root note remains independent of BPM. No hypothesis is a known private recording's
tempo or automatic musical break understanding.

## Verification

`cargo test --locked --offline --test mcp` exercises real subprocess stdio:
initialization/notification/ping/tools discovery, protocol versus tool errors,
malformed IDs/JSON, oversized input recovery, typed catalog, revision conflicts,
atomic invalid batches, single-step undo/redo, dirty replacement, explicit overwrite
and changed-byte refusal, traversal/symlink/embedded-asset escapes, end-to-end
sample/edit/effects/evaluate/save/export/reopen, and per-operation clone bounds.
M7 regressions additionally cover phrase CRUD/trigger schema, shared Editor/MCP
state and evaluator parity, independent LPB/transposition/finite cycles, phrase
save/export/reopen, one-batch undo/redo, unsafe shrink/reference/nesting/cycle/count
limits, and old-client instrument-edit preservation. Source/region regressions cover
real WAV/FLAC import/inspection/analysis, native metadata and guard exclusion,
read-only deterministic suggestions, config/duration limits, exact Editor/MCP state
and evaluator parity, region bounds/references/source-switch semantics, single-batch
undo, save/reopen/byte-identical export, changed-source rejection, traversal/symlink
preflight and embedded registered-source escape rejection. M8b regressions cover
persistent marker/cleanup bounds, shared Editor/MCP exact candidate/diff/manifest
parity, real 70→170 native-pitch PCM, grid cycles and all four roll units/tail policies,
unchanged source/save/reopen exports, single-undo compiler commit, token supersession,
edit→undo ABA, New/Open/Save As invalidation, hash-failure retry, source reauthorization,
unknown source BPM and compiler density/ID-exhaustion rejection. Experimental/lock
regressions cover exact shared service versus real MCP result/page concatenation,
phase/preset overrides, cached repick versus fresh extraction, separate peak/attack
and collision counts, v1 unchanged, marker-lock Editor parity and save/reopen,
atomic unlock/move/relock, pagination/end/range/capacity semantics, stale tokens,
source tamper/symlink reauthorization and explicit preparation-failure retirement.

The independent judge additionally validates advertised input/output schemas using
JSON Schema 2020-12 and runs wire-level tool flows. These tests establish the stated
headless profile; they do not establish GUI synchronization, physical playback or
all-version MCP interoperability.

## Single-layer 16-pad drum kits

`project_read` section `drum_kits` returns the same persistent kit/pad map used by
the desktop and renderer; `summary` adds `drum_kit_count` and `drum_pad_count`.
`instruments` exposes the routing, for example
`"kind":{"type":"DrumKit","data":{"kit":41}}`. Pitched instruments omit `kind`,
and documents without kits omit `drum_kits`: the old document/evaluator path is
unchanged by unused kit definitions. `sample_inspect` includes the bounded list
`kit_pad_mappings` linking verified source regions to kit IDs, pad IDs, slots and
selector notes. No second pattern store or MCP-only drum sequencer is introduced.

To start a kit without replacing a used instrument, apply
`{"op":"create_drum_kit_instrument","name":"New kit"}` in a revision-checked
batch. The shared Song/Editor action allocates fresh checked IDs for both the kit
and a new instrument, with no overwrite or change to existing musical content or
cursor. Read `drum_kits` and `instruments` afterward to discover their IDs before
adding pads/notes. Empty names, kit capacity or either ID namespace's exhaustion
reject the complete batch. This is one undo transaction, not two partial actions.

A kit has at most **16 pads, with one selected source per hit**; the Song has at most 128 kits. Kit names
are at most 256 UTF-8 bytes and pad names at most 128. Pad IDs are stable nonzero
`u64` values, independent of the unique visual `slot` 0..15 and unique MIDI
`note` 0..127 within the kit. `upsert_drum_kit` requires `id,name` and preserves all
pads when renaming. `upsert_drum_pad` requires every field shown below except
`tune_semitones`, which defaults to zero on this **metadata replacement** operation.
On an existing zoned pad, `region_id:null` preserves its entire zone/alternative
vector; a nonnull region rejects with a `set_pad_sources` instruction rather than
silently switching modes. Use that explicit operation to replace or clear sources.
In particular `region_id` and `choke_group` are **required nullable fields**:
omission is rejected, not mistaken for unmapping or disabling choke.

```json
{
  "expected_revision": 2,
  "operations": [
    {"op":"upsert_drum_kit","id":41,"name":"Break kit"},
    {"op":"upsert_drum_pad","kit_id":41,"id":301,"slot":0,"name":"Open hat","note":46,"region_id":21,"root_note":60,"tune_semitones":0,"gain":1.0,"pan":0.0,"mode":"OneShot","new_note":"Overlap","choke_group":1},
    {"op":"upsert_drum_pad","kit_id":41,"id":302,"slot":1,"name":"Closed hat","note":42,"region_id":22,"root_note":60,"tune_semitones":0,"gain":0.8,"pan":0.0,"mode":"OneShot","new_note":"ReleasePrevious","choke_group":1},
    {"op":"assign_kit","instrument_id":1,"kit_id":41},
    {"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":46},"instrument":1}},
    {"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":{"Note":42},"instrument":1}}
  ]
}
```

This assumes regions 21/22 already exist and the addressed track/pattern has room.
The whole batch is one revision and one undo. **A pad's `note` selects the pad; it
does not chromatically transpose its source.** `root_note` is the pad's native
pitch reference. Explicit `tune_semitones` −48..48 adds to that reference (the sum
must remain MIDI 0..127), changing source playback rate/pitch without modifying
the original bytes. At tune 0 playback remains source-native regardless of selector
or song tempo. Explicit glide targets remain absolute MIDI notes, not selectors;
tuning is not added again. Gain 0..1 multiplies ordinary note velocity and pan −1..1
combines with note pan using the shared renderer. Imported regions remain
inclusive-start/exclusive-end views of verified, immutable sources.

- **OneShot** ignores ordinary tracker OFF and phrase-end release. **Gated** honors
  those releases. Both ignore automatic tracker-column replacement OFF, allowing
  an earlier pad to continue when another selector arrives.
- **Overlap** allows older held voices of the same pad to continue.
  **ReleasePrevious** releases older held same-pad voices before starting the new
  hit. Already-releasing tails continue rather than being cut/released again.
- A nullable `choke_group` chooses none or 1..16. A hit releases older held members
  of that group **within the same InstrumentId**, across tracks and phrase
  instances, before its new ON. Two instruments referencing the same kit do not
  choke one another. Even a zero-gain hit performs its explicit choke operation.
  These are release tails, not instantaneous hard cuts; typed hard-cut and timed
  release effects remain explicit overrides.
- Empty kits and unassigned pads (no single region **and** no zones) can be authored, but a note
  selecting an absent/unmapped pad rejects. Duplicate slots/selectors, invalid
  IDs/bounds, dangling region/kit/pad mappings, unsafe removals and invalid notes
  in **unordered patterns** also reject atomically. Changing a used selector may
  require clearing its old cells first and inserting replacements later in the
  same ordered batch. Each intermediate state must remain valid.
- Kit phrases currently require **neutral transposition**: the parent trigger note
  equals the phrase root. Inner notes select pads; transposing a kit phrase is
  rejected rather than silently selecting different drums.
- Instrument metadata edits with `upsert_instrument` and `sample:null` preserve
  kit routing and pads. An explicit nonnull legacy path or nonnull region mapping
  switches to Pitched; `assign_kit` with a kit ID clears both pitched sources.
  `assign_kit` with explicit null changes only kind to Pitched (an already pitched
  source is retained). Removing a kit requires clearing its instrument references.

Use the existing `song_evaluate`/`song_export` tools for ordinary events and offline
PCM. Used kit voices select manifest schema 6 with per-voice kit/pad/selector and
verified-region provenance; unused kits do not change the ordinary manifest path. Evaluation declares expected source identity but does not hash/decode current
bytes; export verifies sources before reserving output files. Existing root
confinement, no-clobber output, revision and document/batch budgets are unchanged.
`leviathan_audition_prepare` now exposes the shared bounded region/pad selection
and playback metadata for AI review; it does not produce audio (see below).
There is **no live pad audition, MIDI input, GUI synchronization, simultaneous
layer stacking, random alternative selection or per-pad effect chain** in this headless server. Static
path policy is still not an OS sandbox against hostile concurrent renames.

## Guarded drum-step overlay, ordinary tracker cells

`leviathan_drum_steps_inspect` projects one explicit **pattern + track + InstrumentId**
into pad lanes. Different instruments sharing a kit remain distinct; no second
sequencer, hidden pattern store, audio analysis or inferred hit deletion is used.

```json
{"expected_revision":12,"pattern_id":1,"track_id":1,"instrument_id":5,"division":"Sixteenth","steps":16,"page":0}
```

`steps` is exactly 16 or 32. `division` is `Quarter`, `Eighth`, `Sixteenth`,
`ThirtySecond`, `SixtyFourth` or `OneTwentyEighth` (1,2,4,8,16,32 steps per beat).
Pages are zero-based; the grid origin is always pattern row 0. Anchors use the
Song's LPB, not rounded displayed milliseconds. Partial final pages mark
out-of-pattern slots `in_bounds:false`; invalid pages and arithmetic extremes
reject rather than allocate or wrap. Repeated occurrences of the same PatternId
share these cells; use an independent clone first when that is not intended.

The result contains `projection_token`, exact `projection.view`, total steps/pages,
lanes sorted by visual pad slot, and every slot's exact row/delay, events and
conflicts. Each event includes pad, InstrumentId, source row/column, original Cell
and `on_grid`. `issues` preserve source references for phrase/selection/other-cell
cases. Alias projections expose `source` and `read_only:true`; all alias-target step edits
reject until explicit Make unique or source navigation. Source snapshots retain `None` versus explicit zero and original f64 delay
values; they are not display-rounded replacements. Retriggers remain a single
editable parent, not fabricated separate source notes. This read-only tool does
not decode/hash source files or claim verified current PCM.

A successful inspection replaces the one retained page token. It is bound to the
exact Song, view, MCP revision and Editor generation, not only the page number.
Any changed edit, undo/redo (including edit→undo ABA), New/Open or Save invalidates
it. Failed inspection leaves the previous token intact. No-op edits preserve it.
Only server-retained source events are accepted: clients cannot submit arbitrary
Cell snapshots as editing authority. Example field edit:

```json
{"expected_revision":12,"operations":[{"op":"drum_step_set_velocity","projection_token":"TOKEN_FROM_INSPECT","pad_id":301,"row":0,"column":0,"value":0.7}]}
```

- `drum_step_toggle` takes token, `pad_id`, and **page-local** `step` 0..15/31.
  It uses the shared safe-placement policy within existing columns, never expands
  a track, overwrites another cell, guesses incoming selection or snaps an onset.
  Removing a representable hit clears its note and retrigger only, retaining other
  source attributes. Off-grid, multi-hit, additional-command, phrase and unknown
  selection conflicts block ambiguous toggles; inspect the exact sources instead.
- The three field operations take token, `pad_id`, source `row,column` and a
  **required nullable** `value`. Missing value is an error, not an implicit clear.
  Velocity is 0..1 or null; zero is silence. Delay is an absolute source-row fraction
  `[0,1)` or null, **not a fraction of the displayed step**. Retrigger is null or
  `{ "repeats":1..16, "spacing":positive_source_row_fraction }`, with normal
  delayed-final-hit bounds. These change only the selected parent's field; off-grid
  and multiple-hit parents remain explicitly addressable. No children are inferred.
- An ordered batch validates the original token/base against the live document,
  then reprojects its explicit candidate before each shared edit. Multiple fields
  on the same reviewed source can be changed in one undo transaction. An event
  must have existed in the inspected page and remain owned at its source address.
  Newly inserted hits, or hits moved outside that page, require fresh inspection
  before another field edit. Any later failure rolls back the whole batch.
- Other source cells/fields remain intact. Existing tracker attribute-memory and
  source selection rules still apply: changing a velocity can intentionally affect
  later cells that inherit it. This is ordinary Song/evaluator semantics, not an
  independent pad-player implementation.

Projection is synchronous and bounded: at most 65,536 stored plus resolved-alias pattern cells scanned,
2,048 source event/issue records per page, 16 lanes×32 slots, and 256 KiB serialized
structured payload before MCP mirroring. Oversized/unsafe representations fail
explicitly without truncating source records or changing revision/history. No live
pad audition, GUI instance control or audio-device claim is added.

## Track-block aliases and per-occurrence mutes

These are scalar tracker-content aliases, **not automation aliases**, whole-song
playback state sharing or GUI-instance control. `Pattern.track_aliases` maps a
target TrackId to `{"pattern":source_pattern_id,"track":source_track_id}`.
Cross-track links are allowed only when source/target row and note-column counts
match exactly. Chains resolve through the shared Song resolver, with cycle checks,
at most 32 edges, 4,096 alias links and 65,536 resolved cells summed across **all**
alias targets, including unused patterns. Resolved alias row containers, ordered alias blocks, and the cells/row containers
expanded by ordered alias uses are independently capped at 65,536 each to prevent
empty-row or repeated-order amplification. Source/target shape edits and reference
removals must leave these constraints valid. Empty terminal blocks are legitimate.

Aliases share ordinary cells, typed scalar effects and phrase triggers. They do
not share runtime voices, command memory or occurrence mutes: evaluation still
uses the destination track and occurrence. Raw `project_read` sections `patterns`
and `order` expose stored alias maps and mute sets; `summary` adds
`track_alias_count` and `occurrence_track_mute_count`. Empty maps/sets remain
omitted, preserving old plain-document serialization.

### Inspect the matrix without flattening content

```json
{"expected_revision":8,"occurrence_start":0,"occurrence_limit":16,"track_start":0,"track_limit":16}
```

Pass this to `leviathan_matrix_inspect`. Occurrence and display-track starts are
zero-based **current indices**, not stable occurrence IDs. Both limits are 1..32
(default 16). Each block includes target occurrence/PatternId/TrackId, muted flag,
direct alias, complete target-first/terminal-last `source_chain`, resolved source,
row/column dimensions, stored-local versus resolved cell counts and local-editable
status. Read exact stored source cells from `project_read patterns` using terminal
IDs; there is no second alias-resolution implementation in MCP.

The two axes have independent totals and next offsets. A start equal to the total
returns an empty end page; greater/out-of-range starts reject. At most 32×32 blocks
and 256 KiB structured payload are returned before MCP text mirroring. A larger
source-chain payload rejects with `matrix_inspection_limit`; request smaller
pages rather than receiving a silently truncated matrix. Inspection does not
mutate the document, read audio sources or promise that current source bytes match
the expected metadata. Existing source policy and export verification still apply.

### Explicit linking, detaching and source editing

```json
{"expected_revision":8,"operations":[
  {"op":"set_track_alias","pattern_id":2,"track_id":3,"source":{"pattern":1,"track":1}},
  {"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":3,"muted":true}
]}
```

`source` is **required nullable**. An object links an empty local block; occupied
local content is never implicitly discarded. Clear it explicitly in a prior
operation if that destructive change is intended. Explicit `source:null` removes
an existing link **without copying**, leaving that alias target empty; an
already-local block is unchanged. `make_track_unique` copies the exact resolved
scalar block locally and removes the link, retaining source content and other
aliases. The latter can be followed by ordinary cell edits in the same one-undo
batch. A later failure rolls back both the copied content/graph and mute changes.

Normal cell, effects, phrase-trigger, clear-track and drum-step writes reject an
alias target; they cannot create hidden local cells or silently edit the source.
Drum-step inspection shows the resolved source and `read_only` state. Either make
the target unique or explicitly address the resolved source PatternId/TrackId.
Source edits intentionally propagate to every still-linked target. Whole-pattern
`clone_pattern` materializes aliases into independent local content. Removing an
unused-but-referenced source, creating a cycle, changing linked geometry or
exceeding expansion limits fails atomically. Removing a track requires clearing
its dependent alias and mute references; `clear_cells:true` does not erase other
tracks' dependent content implicitly.

### Mute is an arrangement property, not a pattern rewrite

`Occurrence.muted_tracks` is a sorted set of target TrackIds, at most 65,536 mute
entries across the Song. `set_occurrence_track_mute` takes a current zero-based
`occurrence_index`, stable `track_id` and required boolean `muted`. Only that
occurrence/track is affected, even when its PatternId is repeated elsewhere.

The shared evaluator suppresses muted block cells and their parameter-memory
updates, cancels pending phrase actions for that track, and de-click releases
held destination-track voices at the muted boundary (including kit one-shots).
Already-releasing tails may continue fading. This is **not instantaneous hard
silence**, and it is not a global mute of all instruments sharing a sample or kit.
Ordered alias/mute use selects manifest schema 7, including resolved alias chains
and occurrence mute frame intervals; unused alias definitions alone preserve the
previous manifest path. Content, aliases and other tracks remain intact. There is no live audio/device
control in the MCP server.

`move_occurrence {from_index,to_index}` uses the shared order-arrow **swap**
semantics, carrying complete occurrence records including mutes. Equal indices
are a no-op; invalid indices reject. Existing `set_order {pattern_ids}` deliberately
replaces the order with **fresh unmuted occurrences**, while `append_repeat` adds
one new unmuted occurrence. Use the swap operation when existing mute metadata
must travel with an occurrence. Revision checks prevent stale index plans; all
mutations remain ordinary validated Song/Editor transactions and one batch is one
undo. Save/reopen and offline rendering use this same persistent/evaluator path.

An explicitly all-muted note arrangement exports valid zero PCM at its normal
arrangement duration, without decoding/hashing unused sources; an ordinary empty
song still reports no note triggers. MCP retains its existing **all-reference
pathname** preflight even for muted assets (root, regular-file existence and no
symlinks). Thus a missing/outside/symlink source can still fail the MCP policy,
whereas unchanged authorized paths whose unused bytes changed need not be decoded
for silence. Save/Open verify all registered identities; normal export verifies
sources actually used by the shared Engine. No unused-source verification is
implied by a successful silent export.

## Scoped offline transforms — review, then commit

These are deterministic edits to **ordinary stored pattern cells**, not a runtime
or global groove engine. They add no DSP, source-audio rewriting, generation recipe,
random humanization or new Song fields. There is no seed parameter: unknown `seed`
(or caller-supplied candidate/patch) fields are rejected, never silently ignored.
The same pure `transforms` preview and protected Editor commit are used by UI and
MCP. Existing 44 batch operations remain unchanged; transforms use an explicit
review/commit lifecycle instead of an unreviewed batch operation.

### Explicit selection and independent field mask

Call `leviathan_transform_preview` with a current revision and all options:

```json
{
  "expected_revision":8,
  "config":{
    "scope":{"pattern":1,"track":1,"row_start":0,"row_end":16,"columns":[0,1]},
    "fields":{"notes":true,"glide_targets":false,"velocity":true,"timing":false},
    "transpose_semitones":12,
    "velocity_scale":0.8,
    "nudge_frames":0,
    "swing":null
  }
}
```

Rows use a half-open range: `row_start` inclusive, `row_end` exclusive. Columns are
sorted, unique, zero-based **actual source note columns**. There is no inferred
whole-song or visible-GUI selection. Alias targets are read-only; explicitly
address the resolved source or make the target unique first. Editing a stored
pattern affects every repeated occurrence and alias use, not only the visible
occurrence. Inspect `affected_occurrences` and `affected_aliases` in the summary.

All four field-mask booleans are required and independent:

- `notes`: transpose pitched MIDI notes by −48..48 semitones. Kit selector notes
  and unresolved/ambiguous instrument contexts reject rather than retuning pads.
- `glide_targets`: independently transpose explicit absolute glide targets. No
  extra tune offset is added and MIDI out-of-range results reject, never clamp.
- `velocity`: multiply only explicit stored velocities by a finite 0..4 factor.
  `None` remains unspecified, not an inferred remembered gain. Scaling an explicit
  value updates ordinary column velocity memory: later notes with `None`, including
  notes outside the selected rows, may inherit the new gain even though their
  stored cells are unchanged. Resulting values outside 0..1 reject rather than clamp. MCP checks the incoming JSON number before
  converting to the shared f32 scalar; `effective_config` reports that exact scalar.
- `timing`: apply the signed `nudge_frames` (−48,000..48,000) and optional offline
  swing to whole parent cells with their commands. Units are **48 kHz output-clock
  frames**, not native source frames, milliseconds or ticks.

`swing` is required nullable. For example
`{"division":"Sixteenth","amount_percent":25}` displaces odd pattern-local
whole-note subdivisions by a percentage of subdivision duration, rounded once to
integer output frames. Allowed divisions are Sixteenth, ThirtySecond, SixtyFourth
and OneTwentyEighth; amount is 0..50 percent. Pattern row zero is the anchor. This
writes ordinary row/delay values; it does not change song BPM or install a later
playback modifier. Exact-frame proof applies to the captured BPM, LPB and current
uses. Later BPM/LPB edits rescale those baked row/delay values: there is no persistent
frame-offset object that holds a nudge constant through tempo changes.
Inactive-mask parameters still require valid ranges but are ignored. Empty masks and identity options preserve the exact document, including
absent versus explicit zero delays.

Timing edits must remain in the selected row range, avoid collisions and overtaking,
and be exactly representable across all ordered and aliased uses. They do not
quantize, overwrite destination cells, expand the pattern, move unrelated cells or
silently modify conflicting commands. Unsupported contexts and bounds fail before
commit; the shared error supplies the relevant conflict information. Nonzero timing changes of phrase-trigger cells are unsupported and reject; pitched
phrase-trigger transpose remains an ordinary validated edit. The complete
Song is validated before an opaque candidate is retained. Selection isolation means
unchanged stored cells outside the scope, not guaranteed audible isolation:
ordinary tracker velocity/instrument memory and shared source uses still apply.
The timing proof concerns raw scheduled frames; moving events across occurrence
mute boundaries can change actual audibility and event counts. It is not a
guarantee of preserving the audible event count after mute suppression.

### Exact bounded diff and atomic application

Preview returns `effective_config`, shared algorithm/summary, selected/changed/
moved and per-field counts, skipped unspecified velocities, affected use context,
`changed_cell_count` and `preview_token`. It does **not** mutate the document,
revision, undo/redo, selection or source files. Inspect every desired page:

```json
{"expected_revision":8,"preview_token":"transform-1-revision-8","start":0,"limit":64}
```

Send this to `leviathan_transform_inspect`. The result includes actual shared
`CellChange` records: `from_row`, `to_row`, `column`, exact `before` and `after` cells,
with original optional/zero/f64 values. `total` and `next_start` make pagination
explicit. `start == total` returns an empty final page; a larger start or a limit
outside 1..64 rejects. The source PatternId/TrackId is in `effective_config.scope`.
No second MCP selection traversal or transformation implementation exists.

After review, call `leviathan_transform_commit` with the same current revision and
`preview_token`. The server checks its revision, Editor generation, entire exact
base Song, and current root-path policy, then delegates the private plan to Editor
as **one undo entry**. The client cannot inject a candidate. A no-op consumes the
review token but preserves document/revision/history/redo; changed commits advance
revision and invalidate all pending reviews. Successful newer transform previews
supersede older transform tokens; failed previews/inspections/commits preserve the
current plan. Any changed edit, Undo/Redo, Save, New or Open invalidates it, including
edit→Undo and reopening identical content. Existing compiler and transform review
tokens are distinct; neither authorizes the other's commit endpoint.

Budgets: at most 4,096 selected cells and affected uses, 65,536 event×use timing
checks and ownership/order scan cells, 4,194,304 encoding frame comparisons,
8 MiB serialized candidate, and 256 KiB structured summary/diff page before
MCP text mirroring. Over-budget requests fail, never truncate. The process holds
one private transform plan; work is bounded synchronous, not an async cancellable
job or a wall-clock deadline guarantee.

Preview, inspection and commit reauthorize all referenced paths under the startup
root (regular existing files, no traversal/symlinks). Pure scalar transforms **do
not read/hash/decode audio**, so `source_identity_verified:false` is explicit.
Changed bytes at an otherwise authorized path are not a scalar-plan base change;
normal verified import/inspect, Save/Open and used-source export retain their own
identity guarantees. This is the same static trusted-root path policy, not a
security sandbox against concurrent hostile filesystem renames. No live GUI IPC,
transport, runtime groove or physical output is implied.

## Seeded finite pattrns generation — typed preset, not arbitrary code

`leviathan_generation_preview` runs the **actual pinned pattrns 0.9.3** through the
same bounded pure `generation` API used by the UI. The initial preset is Euclidean;
this is not a generic pattrns/Lua/script evaluator, a text-to-code tool or a second
playback engine. Every request supplies its version, seed and finite configuration:

```json
{
  "expected_revision":8,
  "config":{
    "version":1,
    "name":"Seeded Euclidean",
    "bars":2,
    "instrument":1,
    "seed":42,
    "preset":{"Euclidean":{"steps":16,"pulses":7,"rotation":3,"probability":0.75}},
    "sequence":[{"note":60,"velocity":0.5},{"note":64,"velocity":0.75},{"note":67,"velocity":1.0}]
  }
}
```

Required typed fields and limits:

- `version:1`: explicitly negotiated **preset format**, separate from MCP API or
  protocol versions. Unknown versions reject rather than silently upgrade.
- `name`: nonblank, at most 256 UTF-8 bytes; applied to the new pattern and track.
- `bars`: 1..16 finite four-beat bars. Current Song BPM/LPB are preserved.
- `instrument`: an existing source-assigned InstrumentId. A Pitched instrument
  needs a legacy sample or region; each Multisample note/velocity choice must match
  a key zone. Every kit selector needs a pad with assigned single-region or
  velocity-zone sources. All choices are checked even if probability gates them out.
- `seed`: required unsigned 64-bit integer, including zero. Preserve JSON integer
  precision. There is no default random seed, ambient random state or hidden retry.
- `preset.Euclidean`: 1..128 `steps` per bar, 1..steps `pulses`, `rotation` less than
  steps, and finite 0..1 `probability`. Probability is a seeded pattrns gate, not an
  ML confidence value. All-closed output rejects with no fallback note or retry.
- `sequence`: 1..16 explicit `{note,velocity}` entries; it cycles on **emitted hits**,
  not silent ticks, continuously across bars. MIDI 12..127 is the pinned pattrns
  +12 conversion range. Notes are audible MIDI values for Pitched/Multisample
  instruments and selectors—not playback pitch—for drum kits. Velocity is finite 0..1, including
  explicit zero. MCP validates probability/velocity as f64 before converting to
  the shared f32 scalar; `effective_config` reports the actual values used.

No `code`, `lua`, `script`, custom-program or client-candidate field is accepted.
The algorithm and gate identity are exposed in capabilities/provenance, including
pattrns revision `8505bfe09111d4123a202af5222466f55021db9f` and the locked
`pattrns-0.9.3/rand-0.9.5/Xoshiro256PlusPlus/seed_from_u64` gate. A fixed seed/config
and captured model/time context reproduce the same output; a seed is not a promise
that output differs for every seed (probability one deliberately emits all pulses).

### Preview exact output, then allocate once

The preview returns its token/ID, effective config, shared summary and provenance.
Summary includes proposed fresh PatternId/TrackId, appended occurrence index,
rows/columns/hits, raw tick count, start/end output frames and original timing.
Preview reserves no IDs and makes no document, history or file changes. Existing
content—including aliases, occurrence mutes, markers, sources and tempo—is not
rewritten. Extending the arrangement can nevertheless expose previously truncated
phrase-child events or sustain existing voices longer; unchanged stored content
is not an audible-isolation guarantee. The shared warnings and combined evaluator
budgets cover that extended arrangement. The shared allocator uses checked max+1
IDs, rejecting exhaustion rather than filling holes or overwriting an occupied ID.

Inspect using `leviathan_generation_inspect`:

```json
{"expected_revision":8,"preview_token":"generation-1-revision-8","start":0,"limit":64}
```

Pages contain the actual shared `GeneratedCell` records, never a reimplemented MCP
sequencer:

- `step`: global zero-based pattrns tick across the finite bars.
- `virtual_time`: `step*1024` ticks in the synthetic 60 BPM / 1024 Hz scheduling
  clock. **Not** elapsed Song seconds, milliseconds, source frames or output frames.
- `frame`: absolute appended Song **48 kHz output frame** under the captured clock.
- `row`, `column`, `cell`: exact ordinary destination parent cell; generated notes
  carry explicit instrument, velocity and centered pan.

The shared baker chooses the lowest free column within each row and reuses columns
in later rows. Normal tracker replacement, kit mode/choke and SongEnd rules apply;
pattrns event duration is **not imported as note length**. A generated kit hit may
choke held voices on other tracks sharing that InstrumentId. These are explicit
shared preview warnings, not an independent hidden playback engine.

Pages have independent `start`, `total`, `next_start`; 1..64 records and at most
256 KiB structured payload. `start == total` returns an empty end page, larger
starts reject. Nothing is silently truncated. The summary/provenance remains
attached so each page is bound to the same seed/config/version and private plan.

After review, call `leviathan_generation_commit` with `expected_revision` and
`preview_token`. The exact shared Editor method checks both revision generations
and the full base Song, then creates a **new track, new pattern and one appended
unmuted occurrence as one undo transaction**. No arbitrary candidate is accepted.
A newer successful generation preview supersedes the old generation token; failed
preview/inspect/commit leaves the current plan and document intact. Any changed
revision—including edit→Undo, Save, New and Open identical content—invalidates it.
Transform/compiler/generation tokens are not interchangeable. Merely reviewing a
plan preserves existing redo history; committing changed content follows normal
Editor history semantics.

The raw horizon is exactly bars×steps, at most 2,048 ticks/hits; existing Song
budgets still apply to the combined evaluator (including 2,048 total starts,
65,536 expanded actions and the ten-minute duration limit). Shared generation work
preflight additionally caps the prospective `(order_count+1)*(display_track_count+1)`
at **65,536 ordered track blocks** using checked arithmetic before full model
validation. Ordered resolved row-container visits and cell visits have separate
65,536 limits, including empty row maps, explicit default cells and muted content;
aliases resolve through the existing bounded shared resolver. The exact generated
candidate is rechecked rather than reserving 2,048 rows/cells blindly. These are
work budgets, not just note-on counts; reduce repeats or stored grid size on error.
Unused pattern×track combinations are not substituted for ordered-work counts.
The MCP candidate is
limited to 8 MiB compact serialized, summary/diff pages to 256 KiB before text
mirroring. The shared generator also checks the **pretty wrapped project** size
before retaining a plan, with the actual project serializer and a bounded writer:
8 MiB on both base and candidate. This uses current paths conservatively (Save can
shorten absolute paths), not an independent MCP estimate. Thus a compact candidate
under 8 MiB that would exceed the persistence budget still rejects before commit;
failed preview preserves the previous smaller plan and revision/history.
Work is bounded synchronous, not a wall-clock SLA or cancellable async job.

### Historical provenance, ordinary editable content

`Pattern.generation` stores historical algorithm/gate/pattrns revision, original
config/seed/version, captured BPM/LPB/prefix, raw tick count and raw/content SHA-256
digests. `raw_sha256` identifies actual emitted raw pattrns ticks (including closed
gates); it is not an audio-source SHA. `content_sha256` identifies stored generated
rows, alias map and track/cell content, excluding names and the metadata itself.
No generator state runs during playback or project reopening.

Read `project_read {"section":"generation"}` for every generated pattern's
provenance and **shared** `content_matches` result. Renaming preserves a match;
ordinary cell/row/alias edits can make it false. That mismatch is valid, not a
reason to regenerate or overwrite user edits. Clone/history/persistence preserve
historical metadata. It is not a live instrument reference: after explicitly
clearing actual cell references, removing the old instrument does not invalidate
history. Imported metadata is descriptive, not a cryptographic attestation of
who ran a generator. Old documents omit the optional field unchanged.

Generated cells are editable by the existing tracker, steps, typed batch and
transform tools. Frame proof concerns the captured tempo/LPB and appended position;
later tempo/LPB or order-position edits use normal row/delay scheduling and may
change frame positions. No persistent frame-offset or hidden regeneration object
holds the original timing. Save/reopen and offline rendering use the same ordinary
Song/Engine path.

Every MCP generation preview/inspect/commit reauthorizes current referenced paths
under the explicit startup root; missing/outside/symlink/nonregular paths reject.
The pure backend itself performs no filesystem checks. No generation tool hashes
or decodes audio or claims playback verification: `source_identity_verified:false`
is explicit. Normal source inspection, Save/Open and used-source export retain
their existing identity checks. No source-file rewriting, network/script execution,
GUI IPC, MIDI, live transport, arbitrary generation language, automatic break
understanding or unseeded random feature is implied.


## Optional region amp envelopes and forward loops

`set_region_playback` adds opt-in playback metadata to an existing region, through
**the same validated Song helper and one-undo Editor transaction as the GUI**:

```json
{
  "expected_revision": 2,
  "operations": [{
    "op": "set_region_playback",
    "region_id": 21,
    "amp_envelope": {
      "attack_ms": 10,
      "decay_ms": 20,
      "sustain_percent": 25,
      "release_ms": 30
    },
    "forward_loop": {"start_frame": 100, "end_frame": 1000}
  }]
}
```

Both `amp_envelope` and `forward_loop` are **required nullable fields**. An explicit
null clears that option; omission is an error, not accidental unassignment. All
inner fields are required integers; fractional milliseconds, unknown curve modes,
extra fields and out-of-range values reject. There is no raw-DSP or script input.

- Attack and decay: 0..10,000 integer **output-clock milliseconds**.
- Sustain: 0..100 integer percent; this multiplies normal voice gain/pan.
- Release: 0..500 output-clock milliseconds. At the fixed 48 kHz engine clock,
  one millisecond is exactly 48 output frames; envelope time does not follow
  source sample rate, pitch, song BPM or LPB. Linear attack starts at zero when
  nonzero, decay reaches the sustain level; zero stages skip immediately.
- Ordinary OFF/timed release/SongEnd use the opted-in release, starting at the
  current envelope level. Existing kit OneShot/Gated rules still determine whether
  ordinary OFF/PhraseEnd releases a voice. Choke, PadReplaced and TrackMute instead
  use a fixed **50 ms linear release**. An already-releasing voice never restarts
  its release. Hard cut remains immediate. `release_ms:0` stops abruptly and may
  click; 500 ms fits the existing export tail budget.
- Loop bounds are absolute **original source frames**, `[start_frame,end_frame)`:
  at least two frames and wholly inside the region. The pre-loop prefix plays once,
  then forward playback wraps during sustain **and release**. Interpolation across
  the loop end reads the loop start, not a guard or post-loop tail. No post-loop
  section, crossfade, ping-pong/reverse loop or endpoint de-click is implied;
  discontinuous endpoints can click. Pitch/native-rate conversion still applies.

The two options are independent. Envelope without loop still ends naturally if the
source/region ends; an envelope does not extend missing audio. Loop without envelope
retains the existing de-click release. **Both null preserve the legacy path**;
explicit `{attack_ms:0,decay_ms:0,sustain_percent:100,release_ms:50}` is not the same
as absent metadata. Old region JSON omits the optional fields and unchanged legacy
projects retain their prior evaluator/PCM behavior. Used opted-in regions select
manifest schema 8 and expose exact options in region/pad provenance; unused options
alone do not claim that the renderer exercised them.

`project_read` sections `regions`/`document` and `sample_inspect` expose the same
persistent options. Metadata is shared by every pitched instrument or kit pad
referencing that region; use another region ID for an independent view. Existing
`upsert_region` keeps stored options when changing name/source/bounds/root, so old
clients cannot silently clear them. A shrink/rebind that makes a loop invalid
rejects atomically. To intentionally shorten beyond a loop, explicitly clear/change
the loop first and then edit bounds in the same ordered batch. Any later failure
rolls back the entire batch, including options, bounds, revision and history.

Used forward-loop voices reject **any offset or Reverse command**, including
later effect-only commands, in the shared actual-voice evaluator after kit/phrase/
mute resolution. Envelope-only regions retain the existing supported effects.
This is an evaluation/export constraint, not a duplicated per-cell check in the
metadata operation: a structurally valid stored document may need its commands
adjusted before it can render. Unsupported combinations fail before audio decoding
or output reservation, although normal path authorization can occur first. Muted
or never-triggered controls are not treated as active voices.

Playback metadata edits use the existing revision, per-operation document-size,
root/regular-file/no-symlink and atomic-history checks; they do not hash or decode
audio and do not modify the source file. Evaluation remains
`source_identity_verified:false`. Inspect, Save/Open and used-source export keep
their existing byte/metadata verification. Changed bytes, unsafe paths and invalid
controls do not produce partial exports. No new network service, registration,
real-time audition, GUI synchronization or physical-device claim is introduced.


## Drum velocity zones and deterministic round-robin sources

`set_pad_sources` is an atomic source-mode replacement on an existing kit/pad.
It delegates to the shared Song/Editor helper; there is no MCP-only chooser,
allocator, sequencer or stored counter. Both `region_id` and `velocity_zones` are
required, even when empty:

```json
{
  "expected_revision": 3,
  "operations": [{
    "op": "set_pad_sources", "kit_id": 41, "pad_id": 301,
    "region_id": null,
    "velocity_zones": [
      {"min_velocity":0,"max_velocity":63,"alternatives":[
        {"id":11,"region":21},{"id":12,"region":22}
      ]},
      {"min_velocity":64,"max_velocity":127,"alternatives":[
        {"id":21,"region":23},{"id":22,"region":24}
      ]}
    ]
  }]
}
```

Modes are exclusive: a nonnull region plus `[]` selects one source; null plus a
complete zone vector selects velocity/round-robin mode; **null plus `[]` explicitly
unmaps** the pad. A region together with nonempty zones rejects. An unassigned pad
is authorable until ordinary cells actually require its mapping. Legacy
`upsert_drum_pad` does **not** silently destroy a zone bank: null region preserves
all zones, while nonnull region on a zoned pad rejects with an actionable instruction
to use `set_pad_sources`. Other pad fields retain their documented replacement
semantics. Old single-region documents omit the new empty vector unchanged.

Zones are sorted, nonoverlapping, inclusive integer velocity-bin ranges that form
one gapless partition from **0 through 127**. There are 1..8 zones, 1..8 ordered
alternatives per zone and at most **32 alternatives per pad**. Each alternative has
a stable nonzero `PadLayerId`, unique across all zones of that pad (not globally),
and an existing `RegionId`. Structural caps and the independent 8 MiB serialized
project/document limits both apply; every simultaneous structural maximum is **not**
a promise that the pretty saved project fits. Every alternative validates, even if no current note
would select it. Vector order is the cycling order; stable IDs do not sort it.
Removing a referenced region/source fails rather than silently remapping an
unplayed alternative. Bounds, duplicate IDs, unknown fields, missing required
fields and late batch failures preserve the whole original document/history.

For each actual unsuppressed ON, the shared evaluator takes its incoming f32 volume
(after column memory/phrase gain, **before pad gain**), computes
`round(127 * volume)` in f64, and finds the inclusive zone. This bin selects a source;
the actual continuous amplitude is **not** quantized. Exactly one source is chosen,
with an independent counter for each **InstrumentId / PadId / zone**. Alternatives
rotate in vector order. Every unsuppressed ON—including retriggers and zero-volume
hits—advances its zone counter; muted or never-created events do not. Sharing a kit
across instruments does not share counters. Full compilation starts at ordinal zero;
cold seeking uses already compiled choices, and transport looping repeats those
choices. There is no random seed, hidden RNG, wall-clock variation or MIDI-input
state in this feature.

The selected region keeps the pad's native reference/tuning and the region's own
optional envelope/loop. Existing OneShot/Gated, new-note/choke, phrase, alias/mute,
retrigger and release semantics still apply. Region-specific unsupported controls
(such as Reverse on a selected forward loop) reject through the actual-voice
shared evaluator, not a guessed MCP scan. Used zoned voices select manifest schema
9 with `layer_selection`: stable layer ID, input velocity, rounded bin, zone bounds,
**zero-based** round-robin index and ordinal. Evaluation remains an event/provenance
operation (`source_identity_verified:false`); export verifies used audio normally.
Unused zone definitions alone do not change legacy scheduled/PCM output.

### Bounded source-mapping inspection

`project_read {"section":"drum_kits"}` exposes the complete persistent zone vectors
and stable IDs. `sample_inspect` additionally verifies the source and returns its
region/mapping references with explicit **independent pagination**:

- `regions_start` / `regions_limit`: offset and page size for matching RegionIds.
- `mappings_start` / `mappings_limit`: offset and page size for matching pad sources.
- Defaults are zero / **64**, with limits **1..128** on each axis.
- `regions_page` and `mappings_page` each report `start`, `total`, `next_start`.
  Follow each nonnull `next_start`; a response is not implicitly the full inventory.
- Single-region mapping records keep their prior fields. Zoned mappings add stable
  `layer_id`, zero-based `zone_index`/`alternative_index`, and exact velocity bounds.
  Order is KitId, PadId, zone vector, alternative vector. A mapping may reference a
  region absent from the independently selected region page; membership is checked
  against the complete source metadata, never the visible region subset.

Only the requested mapping records are materialized, rather than constructing all
65,536 possible references first (128 kits × 16 pads × 32 alternatives). Region and
mapping start equal to their respective totals return empty end pages; greater
starts and invalid limits reject. Every page is revision-checked and re-verifies
current source identity. The structured result is limited to 256 KiB before text
mirroring; excessive region names require smaller pages or explicit metadata edits,
not silent truncation. This additive pagination is visible to old callers through
totals/next offsets. No page read modifies document, counters, history or source.


## Accepted-marker groove capture and timing transfer

This is a **bounded offline timing transfer**, not an automatic feel detector,
accent classifier, audio stretch, random humanizer or runtime/global groove engine.
It uses the shared `accepted-marker-groove-v1` algorithm (version1), the same opaque
plans and Editor commits as the GUI, and produces ordinary editable row/delay cells.
Existing source material is not decoded or modified by capture/apply tools.

### Capture a historical recipe, then explicitly commit

First import/register a source and author or accept an explicit marker set. Its
`source_bpm` must be known and explicitly stored; null is rejected, never guessed.
Review capture with a **fresh nonzero `groove_id`**:

```json
{
  "expected_revision": 2,
  "groove_id": 51,
  "config": {
    "version": 1,
    "name": "Accepted late feel",
    "marker_set": 31,
    "start_marker": 0,
    "end_marker": 4,
    "include_start_anchor": true,
    "division": "Sixteenth",
    "cycle_steps": 4
  }
}
```

Pass this to `leviathan_groove_capture_preview`. All config fields are required.
Marker numbers are **indices**, not frame positions: start must precede end and
both must be existing accepted boundaries. The origin is `markers[start_marker]`.
The end boundary is **excluded**; the start boundary is an anchor only when
`include_start_anchor:true`. A capture with no anchors rejects. The declared
whole-note subdivision is Sixteenth/ThirtySecond/SixtyFourth/OneTwentyEighth;
cycle length is explicit1..1024 steps, not inferred from file duration or attacks.
Accepted markers map to nearest rational grid slots with half-up ties. Duplicate
nearest slots or anchors outside the declared cycle reject, never merge or wrap.

The shared preview stores exact raw source-frame deltas from the window origin,
the source's native rate and explicit BPM, grid/cycle, and historical source/marker
IDs, SHA-256 and window metadata. It does **not** round away raw anchor timing.
`groove_preview_inspect` returns bounded capture records with both authoritative
integer `source_frame_delta` and approximate display-only `offset_beats`. Do not
reconstruct a recipe from the display approximation. After review,
`groove_capture_commit` takes current revision and `preview_token`, registering
one immutable template in one undo transaction. Even an identical capture into an
already-present ID rejects; use a genuinely fresh ID. No raw-template insertion or
replacement operation is exposed by MCP.

`project_read {"section":"grooves"}` exposes the stored map. For bounded detailed
inspection use `groove_template_inspect` with current revision, `groove_id`, optional
`start` and `limit`. The `template` metadata excludes the anchor vector; the
`anchors` page contains shared exact records. Default/max64 records and256KiB,
explicit `total`/`next_start`; `start == total` returns an empty page and larger
starts reject. This read performs **no source-file access**. Historical IDs/hash
need not remain live: after capture, marker/source edits or deletion do not change,
regenerate or invalidate the recipe. Imported metadata describes history, not an
attestation of who captured it or proof of current audio bytes. `remove_groove`
removes a template without reversing already baked note edits.

### Apply only to an explicit selection

```json
{
  "expected_revision": 3,
  "config": {
    "groove": 51,
    "scope": {
      "pattern": 1, "track": 1,
      "row_start": 0, "row_end": 8, "columns": [0]
    },
    "strength_percent": 100
  }
}
```

`groove_apply_preview` uses **scope.row_start as its grid origin**, a half-open row
range and sorted unique zero-based columns. It classifies each ordinary pitched or
kit-selector Note by its nearest rational grid tick, repeats the declared cycle
(including partial final cycles), and looks up the captured sparse slot. Missing
slots remain unchanged; no notes are invented. A matched tick at the excluded
selection end rejects rather than wrapping to its beginning.

Strength is an integer0..100 **blend of the existing note phase toward the captured
phase**, not an additive nudge. Thus100% can straighten an already-late note, and
reapplying100% is not an accumulating delay. At0 the existing stored representation
is bit-preserved, including absent delay versus explicit zero, and classification/
encoding is bypassed; normal template/scope/resource validation still applies.
At100 the target is the exact rounded rational timing at the captured Song clock,
subject to a representable common integer output-frame displacement across every
current repeat/alias use. The encoded row/delay must also retain the intended nearest
grid slot: a rounded frame that would cross a half-tie slot boundary rejects rather
than silently reclassifying the note. Zero-output-frame changes retain the original cell.

Notes/kit selectors, instruments, gain/pan and other musical fields are not
transposed or rescaled. Whole parent cells move through the shared timing encoder,
with supported row-local actions translated by the same integer displacement.
Changed phrase triggers, alias-target writes, collisions (including stored default
destinations), overtaking, duplicate selected grid slots, out-of-scope movement,
excluded-end anchors or changed occurrence-mute eligibility of any moved action
reject atomically. No clipping, snapping, overwrite or changed mute suppression
is accepted. Edit a resolved alias source explicitly or Make unique first. A source
edit affects every listed repeat/alias, not just the currently visible occurrence.

This stored-cell/timing proof is **not full renderability or audible-event-count
preservation**. Recompiled cross-track/column interleaving may change round-robin
choices, choke, phrase replacement and overlap. Moving an ordinary replacement note
can expose or cancel pending phrase children. Review the shared warnings; normal
evaluation/export still checks source assignment/decoding and expanded child/action
budgets. Pure authoring does not require prematurely rendering every candidate.

Inspect apply-kind records with `groove_preview_inspect`: exact shared
`CellChange` before/after cells, source/destination rows and column, up to64 records
per page with explicit total/next offset and256KiB maximum. The shared summary
reports selected/matched/unmatched notes, changed/moved/zero-frame counts, affected
occurrences/aliases, warnings and template identity. Commit with
`groove_apply_commit`; it is one shared Editor transaction, not a second scheduler.
The resulting Song is ordinary editable content: later BPM/LPB/order edits rescale
baked row/delay timing. There is no persistent frame-offset or automatic regeneration
object holding the original captured clock.

### Review authority, limits and source disclosure

There is **one active groove plan**, either capture or apply. A successful preview
of either kind supersedes the previous groove token; failed previews preserve the
previous valid plan and document. The matching commit endpoint is required;
wrong-kind/cross-tool tokens cannot authorize another action. Inspection/commit
check the token, both MCP/Editor revisions and exact full base Song. Edits, Undo,
Redo, Save, New or Open invalidate reviews, even when content returns to the same
value. A changed commit consumes its plan and advances normally. A no-op apply
consumes only its token: no revision/history/dirty/redo changes. Clients never submit
candidate Song objects or patched diff records.

Shared limits include64 templates,256 raw anchors per template and1024 cycle steps;
4096 selected notes/affected ordered uses;65,536 bounded traversal/action/context
visits plus the shared timing encoder's frame-comparison budget. Capture/apply use
shared exact pretty-project size preflight; the MCP candidate also stays under8MiB
compact and each summary/page under256KiB before text mirroring. Structural caps
and independent encoded-size limits both apply, not a promise that every maximum
can be combined. Work is synchronous and bounded, not a latency SLA or cancellable
background analysis job. No detector rerun, random seed, Lua/code or source PCM input
is accepted by these tools.

MCP preview, pending inspection and commit retain the current root/regular-file/
no-symlink pathname checks on actual registered/assigned sources. Those checks are
not audio-byte verification: **`source_identity_verified:false`** is explicit, and
captured SHA is historical declared identity. Pure backend operations do no IO;
MCP pathname authorization can read metadata, but no capture/apply tool hashes or
decodes audio. Normal Save/Open, source inspection and used-source export keep
their existing identity checks. Historical template inspection remains possible
without its original source record/file. No GUI IPC, audio device, deployment or
complete groove/arrangement-understanding claim is introduced.

## Melodic multisample key zones

`Song.multisamples` contains mutable shared maps keyed by stable `MultisampleId`.
Each map contains stable `KeyZoneId`s (unique **within that map**, not globally).
Assigning a map uses the new `Multisample` instrument kind; ordinary tracker and
phrase notes remain the only sequencer. This is not a kit selector: the MIDI note
is the **audible chromatic pitch**, relative to the selected zone's explicit
`root_note`. That root overrides the region's root without modifying the region.
The region still supplies its original source-frame bounds, source identity and
optional envelope/forward loop. No audio file is rewritten.

Five typed batch operations expose the shared model/Editor semantics:

- `create_multisample_instrument {name}` allocates a fresh empty map and a fresh
  assigned instrument together, using checked shared IDs. It never repurposes an
  occupied instrument. Read instruments/maps to obtain the allocated stable IDs.
- `upsert_multisample {id,name}` creates an empty map or renames it **without
  discarding zones**. Names are nonblank and at most256 UTF-8 bytes.
- `set_keyzones {map_id,zones}` replaces the **entire** zone map. `zones:[]`
  explicitly clears it; omission is an error. Each array entry requires every
  field shown below. Duplicate IDs reject before conversion to the shared map,
  never silently overwrite an earlier entry. Array order has no selection priority;
  stored/inspected zones are ordered by stable ID. Whole replacement lets adjacent
  boundaries move atomically without an intermediate overlap.
- `assign_multisample {instrument_id,map_id}` requires a map ID or explicit null.
  A non-null ID deliberately switches away from any previous source kind and
  clears legacy/region assignments. Null detaches **only a currently Multisample
  instrument**; it is a true no-op for Pitched/DrumKit instruments. Existing
  `upsert_instrument` with `sample:null` preserves a Multisample kind; an explicit
  non-null legacy sample deliberately switches back to Pitched.
- `remove_multisample {id}` rejects assigned maps. No cascade deletes regions,
  files, notes or other definitions.

For example, one full-range zone:

```json
{
  "op": "set_keyzones", "map_id": 51,
  "zones": [{
    "id": 11, "name": "Main chromatic source",
    "note_min": 0, "note_max": 127,
    "velocity_min": 0, "velocity_max": 127,
    "region": 21, "root_note": 60
  }]
}
```

Both note and velocity bounds are **inclusive integer0..127**. Two rectangles may
share note ranges if their velocity ranges are disjoint, or vice versa. Overlap in
**both** dimensions rejects; there is no priority fallback, crossfade or layering.
Zone names are nonblank and at most128 UTF-8 bytes. Every region reference is
validated and protected from deletion, including zones not currently played.
Limits are64 maps,128 zones per map and4096 zones across the song, independently of
existing document/message/output byte budgets. Empty maps and uncovered ranges
are valid **drafts**, not guaranteed-renderable instruments.

The shared evaluator selects exactly one zone for each actual unsuppressed ON,
using its audible note after phrase transposition and its incoming pre-source f32
velocity. The selection bin is `round(127 * f64(incoming_f32_velocity))`, nearest
half-up; the audio gain itself is **not quantized**. Existing melodic-region gain/pan
smoothing is retained (not the kit immediate-start path); zero-gain starts remain
silent. Uncovered actual ONs fail with
an actionable gap error before source decode/output, never silently fall back.
Muted/cancelled events do not demand a source. Later gain changes or glide modify
the already-selected voice without remapping it. There are no counters, random
selection, round-robin key zones, stacked layers or automatic root-pitch detection.
Used maps add exact map/zone/note/bin/range/root/region provenance in manifest
schema10; unused definitions and existing instrument routes retain their prior
manifest path. Evaluation compiles declared provenance, while actual source bytes
are still verified by the existing render/import/inspection gates.

**Maps are shared and mutable.** Replacing a zone or root changes the future render
of existing notes for **every assigned instrument**; this is not a frozen per-note
source snapshot. For independent mappings, explicitly create a separate map and
copy its typed zone definitions. Existing review tokens become stale after a changed
map edit, and the whole successful batch remains one undo step. Shape/reference
validation does not pre-render all possible notes: use evaluate/export to verify
actual coverage and voice-control compatibility. Loop restrictions remain the same
as other region voices; there is no new live audition or GUI-instance control.

`project_read {"section":"multisamples"}` exposes stored definitions. For bounded
inspection, `leviathan_multisample_inspect` requires current `expected_revision`
and `map_id`, with independent `zones_start/zones_limit` and
`instruments_start/instruments_limit`. Both limits default/max64. Results include
exact zone definitions and assigned instrument IDs/names, independent totals and
next offsets; an offset equal to its total is an empty page, larger offsets reject.
Each structured page is capped at256KiB before mirrored-text overhead. The inspector
performs **no source IO** and reports `source_identity_verified:false`.

The existing verified `leviathan_sample_inspect` also accepts independent
`keyzones_start/keyzones_limit` (default64, maximum128). Its
`multisample_mappings`/`keyzones_page` enumerate each defined `(map_id,zone_id)`
reference once, at most4096 total, **not** multiplied by assigned instruments.
Membership is independent of which regions appear on the separate region page.
Follow each page's next offset; current revision and fresh source verification bind
these requests. Existing region and kit-mapping pagination/semantics are unchanged.

## Isolated occurrence resampling into a new asset (v1)

The four `leviathan_resample_*` tools use the same shared planner, Engine and
registration transaction as the UI. This is an explicit **cold print of one order
occurrence, all tracks**, not a crop of the running arrangement. Aliases and that
occurrence's mutes are retained; incoming voices/column memory are absent, phrase
instances and drum round-robin counters restart. Missing incoming instrument
selection rejects instead of being guessed. Actually used legacy `sample` paths
are not supported here: first import/register the source and assign a region.
Unused legacy metadata is permitted, subject to the normal root/path policy.

MCP intentionally rejects earlier at Prepare when any referenced source path is
missing/outside its root/symlink policy or the output already exists; pure shared/UI
Prepare can defer these failures until Print or Register, so equivalent rejected
workflows do not necessarily fail at the same stage. The UI uses a worker for these
shared operations (the summary warning's “off-thread” wording), whereas MCP executes
them synchronously with explicit bounds on its request thread, not as an async job
or a latency guarantee.

1. **Prepare** takes current `expected_revision` and complete `config`:
   `version:1`, zero-based `occurrence`, nonblank `name` (at most256 UTF-8 bytes),
   `root_note` (0..127), `tail` (`MusicalOnly` or `ReleaseTail`) and a new
   workspace-relative `.wav` `output`. There is no arbitrary source/candidate/code
   input. The pure shared plan checks model/selection, fresh checked IDs, work and
   exact pretty-project size; MCP also authorizes paths. It does not decode/hash
   source bytes, render audio or create files. The returned `workflow_token`
   identifies the one retained workflow. A successful new Prepare explicitly
   supersedes its predecessor; failed preparation preserves the previous plan.
2. **Render** takes that token and revision. It is an explicitly side-effectful,
   bounded **synchronous** call, not a background job, latency promise or live
   recording. Actual selected registered sources are reauthorized and their
   encoded SHA/metadata verified by the existing Engine. An owned sibling
   temporary is rendered, strictly imported/verified, then published by atomic
   no-clobber hard link after parent/path reauthorization. The output is48kHz
   stereo float32 WAV only, with **no event sidecar**. Unix publication is supported
   in v1; other platforms fail closed before decode/temp creation. Check
   `capabilities.resample.publication_supported`; this is not a Windows validation
   claim. A successful print keeps the same token in `printed` phase, without a
   document/history edit. A second Render refuses rather than overwriting.
3. **Inspect** returns exact effective config, summary/IDs, output identity and
   render receipt (when printed), with source records paginated by `start/limit`
   (default/max64). Each page has independent `total/next_start`; total itself is
   a valid empty end offset. At most256KiB of structured JSON is returned before
   mirrored-text overhead; an oversized page rejects instead of truncating.
   This is **metadata-only**: prepared source identities are declared metadata;
   printed receipt identities were verified during rendering, not reverified by
   inspection. There is no retained event-snapshot endpoint or fresh existence/hash
   claim. A printed receipt remains inspectable at the latest revision after
   document edits/New/Open/Undo, with `registration_current:false`; its stale
   candidate cannot be registered. A new successful Prepare supersedes it.
4. **Register** requires the still-current printed token, both MCP/Editor revisions
   and the exact full base Song. Shared verification reauthorizes the published
   output/parent and performs prospective **whole-candidate storage preflight**:
   every legacy/registered path, all registered identities (including output),
   existing64MiB per-source/256MiB registered aggregate and8MiB pretty-project
   bounds. No files are repaired or silently relinked. One Editor transaction adds
   fresh immutable SampleId, a whole Region covering every printed frame, and a
   fresh Pitched Instrument. Existing notes/order are untouched. There is no raw
   client metadata injection. Failed registration preserves the receipt and WAV;
   success consumes the workflow and adds exactly one undo step.

**Gain and duration are explicit.** The asset is Engine PCM multiplied by exactly2
under the existing fixed master gain0.5, **not normalization**; float values can
exceed1. The receipt reports post-master peak/energy and asset peak. Existing
Engine clipping/nonfinite checks remain active. Printing already bakes the current
effective track mix. Neutral effective track mix at replay, native-root,
unity velocity/gain, center pan and no extra playback options/effects can reproduce the original Engine
output only when held through the **whole printed region**, without an earlier
OFF, replacement or SongEnd. `root_note` sets the new native replay reference, not
print transposition. `MusicalOnly` ends at the isolated occurrence boundary and
may click. `ReleaseTail` bakes the existing24000 output frames/500ms into the asset
and whole region; replaying in the old shorter occurrence can release this baked
part again. Other pitch/gain/envelope/loop changes are ordinary editable playback,
not an exact-reuse guarantee. Reapplying the original nonneutral track settings to
the printed asset applies that balance/gain again rather than undoing the baked mix.

Print budgets are at most8,388,096 output frames and268,435,456 voice-frames,
in addition to existing source/model/evaluator/project limits. The same bounded
shared candidate is retained; no unlimited render queue or event dump exists.
There is no in-process cancellation tool. Before publication, failure cleans only
the owned temporary (cleanup failures are reported). After publication, cleanup
or sync trouble is a receipt warning: a successful durable artifact is never
hidden behind a pretend rollback.

**File lifetime is separate from Undo.** The WAV survives failed or stale
registration, supersession, New/Open, Undo and process exit. Undo removes only
registered document references; Redo restores references without rewriting or
reverifying bytes. Closing the process loses private receipts, not published
files. No filesystem delete tool is exposed. Strict root/symlink/no-clobber policy
and held parent/temp identity checks reduce accidental races, but do not turn a
trusted workspace into an OS-level sandbox against malicious concurrent changes.
No host audio, GUI-instance IPC, service registration or automatic note replacement
is involved.

## Reviewable tempo and pulse-phase hypotheses (v1)

`leviathan_tempo_analyze` runs the shared deterministic
`timing-point-periodicity-v1` analysis, with explicit config
`{"version":1,"min_bpm_tenths":200,"max_bpm_tenths":4000}`. Bounds are **integer
tenths of BPM**:200 means20.0 BPM,4000 means400.0 BPM, and the lower bound must not
exceed the upper. There is no hidden prior from Song BPM, no random seed, dataset,
ML, arbitrary code or caller-supplied evidence array.

Choose exactly one typed evidence source:

```json
{"kind":"markers","marker_set_id":31,"start_index":0,"end_index":8,"include_start":true}
```

This uses an explicit accepted-marker index window. Its end marker establishes the
**exclusive** window end, not an extra attack; counting the start as timing evidence
requires `include_start:true`. It does not require an already-known source BPM.
Source/marker SHA and timing are stored metadata, **not freshly verified audio**.
Changing source bytes alone does not transform manual marker evidence into decoded
analysis. MCP still performs its ordinary all-reference pathname policy checks.

```json
{"kind":"cached_onsets","cache_token":"from-onset-v2","start_frame":0,"end_frame":192000,"basis":"refined_attack"}
```

Cached mode uses only the current server-owned experimental v2 cache. `basis` is
explicitly `peak` or `refined_attack`: candidate peaks and refined attacks are
separate timing interpretations. The shared service re-picks the immutable cache
with the exact effective onset config; there is **no new FFT or decode**, but
normalization/picking/refinement still take bounded work. No client Detection,
SourceAnalysis, confidence, point list or source metadata is accepted as authority.
MCP reauthorizes and freshly hashes the source through its existing onset policy
before every cached-mode Analyze/Inspect/Use/Commit; the embedded binding remains
an immutable historical snapshot, not a promise about later filesystem state.
The adapter additionally authorizes all referenced paths before these operations.

The returned `analysis_token`, full effective config and binding identify the
source SHA/rate/extent, half-open window, evidence basis/marker or onset-config SHA,
point SHA and algorithm. Candidates include exact `bpm_tenths` authority, display
BPM, exact rational phase numerator/denominator and approximate phase frames,
component scores, point/pulse support/error counts, heuristic strength and ratio
aliases. Phase is relative to **window start modulo a pulse**, not a downbeat or
source-origin correction. Scores and Weak/Moderate/Strong are versioned timing-fit
heuristics, **not probabilities or calibrated confidence**. A strong fit can support
several beat units; missing hits, syncopation, noise and half/double/triple-time
interpretations remain ambiguous. No known BPM, meter, instrument role or loop
length is inferred as fact.

The search uses point-pair beat spans1..8, bounded smoothed-vote maxima/shortlisting
and phase fitting; it is not an exhaustive raw-audio beat tracker. Returned
`local_maxima_count`, `shortlist_truncated`, exact shortlist,
`separated_candidate_count` and aliases disclose the bounded ranking. At most64
shortlisted hypotheses and8 separated candidates are retained; an alias may be
listed without being in the final8. **Evidence points are never silently truncated
or merged.** Fewer than four distinct points, or no in-range periodicity votes,
returns an explicit insufficient-evidence result and no fallback BPM. Insufficient
analysis cannot be applied as a guessed default.

`leviathan_tempo_inspect` requires current revision/token and either:
- `kind:"points"` for exact sorted source-frame observations, without
  `candidate_index`; or
- `kind:"pulses"` with explicit `candidate_index` for that shared candidate's
  predicted source-frame pulse grid.

Pages take `start/limit` (default/max64), include exact totals and `next_start`, and
permit an empty page at total. Greater offsets, invalid candidate indices and
oversized limits reject. Each structured response is capped at256KiB before text
mirroring; no page is silently shortened. Predicted pulses are **not accepted
markers**, and inspection changes no source, markers, history or Song tempo.

### Explicit Use BPM, not automatic application

1. Analyze and inspect alternative candidates.
2. Call `leviathan_tempo_use_bpm_preview` with `analysis_token`, explicit
   `candidate_index` and `target_marker_set_id`. The target must already exist and
   refer to the **same SampleId** as the evidence; identical bytes registered under
   another SampleId do not silently grant target authority. Review the exact shared
   `before_bpm/after_bpm` summary and returned `preview_token`.
3. Call `leviathan_tempo_use_bpm_commit` with current revision and that preview token.
   It delegates to the same protected Editor operation as the UI. A changed result
   is one undo entry; same-value application consumes the Use preview without
   changing revision, dirty state, cursor or undo/redo. No raw BPM or candidate Song
   is injected through this workflow. The Commit result is preflighted before
   mutation and returns bounded dirty/undo/redo indicators, explicitly omitting
   title/path/document counts; use `project_read` for full metadata instead of
   echoing a potentially huge title after a successful edit.

Only the target marker set's `source_bpm` changes. Name, sample reference, marker
frames, locks, Song BPM/LPB, audio bytes, existing frozen grooves and ordinary baked
cells remain unchanged. Analysis scores, phase and evidence binding stay transient in the
review response; Save persists the selected BPM, not a new analysis/provenance
object. Future break compilation/groove capture may use the chosen
BPM, converted from exact integer tenths into the existing stored f64 source-BPM
clock; this is not an exact rational pulse-grid playback object. **Phase is advisory and never applied**: the break compiler's source origin
remains frame0. This does not stretch, quantize, regenerate or relabel existing audio.

One analysis and one nested Use preview are retained, without another PCM/cache
copy. Successful new analysis supersedes both; successful new Use preview
supersedes only the older Use review. Failed requests preserve prior review unless
its separately owned onset cache has already been retired. Both MCP/Editor
revisions and the entire base Song are checked: edits, edit→Undo ABA, document
replacement and Save invalidate pending review even if selected marker values look
unchanged. A changed Commit invalidates analysis; a no-op consumes only Use and
leaves the unchanged analysis available.

Cached results additionally bind the **exact current onset cache/config token**,
not just Song revision. Successful repick creates a new epoch; starting extraction
retires the old cache even if extraction fails, so old cached-mode reviews refuse.
Failed repick that retains the old cache may retain the review. Marker-derived
analysis is independent of an unrelated optional onset cache. Source hash/path
failure rejects cached reuse without document writes; exact authorized restoration
may retry the retained same-epoch review. These are intentionally stricter MCP
filesystem checks than the pure historical metadata-only shared Use operation.

Shared limits are source rates8,000..384,000Hz, window at most300 seconds,
1,024 selected points,4,096 cached repick scan points,64 shortlisted hypotheses,
8 returned candidates,2,048 predicted pulses per candidate and16,000,000 timing
work units, plus existing local whole-base traversal/pretty-project limits. The
current MCP v2 cache preparation remains separately capped at15 seconds; a longer
tempo window does not bypass that cache limit.

The current UI v2 effective config retains `max_frames:16_777_216`, whereas MCP
sets `max_frames = source_rate * 15` (720,000 at48kHz), independently of the other
source/decode/work limits. Because the **complete effective config** is hashed,
config SHA and derived tempo/Use-plan IDs can differ: each side is checked against
the shared core with its actual config, while evidence points, numerical candidate
results, Use summary and resulting Song parity are checked separately, without
silently normalizing provenance. Matching musical results or a content ID never
transfers a private review token between the isolated GUI and MCP sessions.

MCP’s bounded synchronous requests are not background jobs or a latency guarantee.
No default onset-engine replacement,
automatic marker/BPM assignment, calibrated beat/downbeat detection or physical
playback claim is made.

## Native slice-feature catalog and explicitly reviewed roles

`leviathan_slice_features_analyze` takes a required versioned request with one
registered `sample` and **1..64 strictly ascending unique region IDs**. Choose
`source:{"kind":"verified_source"}` for strict native decode, or
`{"kind":"cached_onsets","cache_token":"…"}` for the current authorized opaque
v2 PCM cache. Both freshly authorize/verify the source and compute **new feature
FFTs** over the complete selected bounds. Cached analysis is not a no-FFT repick.
The result retains compact measurements, not another PCM buffer; subsequent
onset repicks or cache retirement do not invalidate that completed result.

Measurements describe **native source regions before pitch, gain, envelopes,
loops or mixing**. The exact source SHA/native metadata/bounds and effective FFT,
hop, window and bands accompany RMS, peak/DC/crest, spectral power/fractions,
centroid/flatness, spectral shape change and attack/tail evidence. Frame values
are native source frames, not output frames or milliseconds. Shape change is
positive L1 difference between supported adjacent unit-sum spectra, not amplitude
flux. `shape_support_floor_relative` is1e-24: shape normalization requires analysed
band power above total one-sided post-demean power times this floor. Unsupported
windows break adjacency; this is a relative spectral-support test, **not a loudness
cutoff**. `supported_shape_windows` is separate from raw
`nonzero_spectral_windows`; raw aggregate descriptors remain distinct.
The −20 dB tail is activity after the global envelope peak, not clean decay/RT60;
censor flags and lower bounds matter. Envelope rises are not proven hit counts.

Suggestions are named handcrafted rules with `rule_support`, **not probabilities
or calibrated classifications**. Mixed/Unknown, short, contaminated and censored
results are ordinary. Suggestions never become reviewed roles automatically.
`ghost` is an explicit contextual human articulation, never inferred from level.

1. Analyze, then page `leviathan_slice_features_inspect` with `kind:"pending"`
   and `analysis_token` to review exact records.
2. `leviathan_slice_catalog_preview` requires `roles`, a list of explicit
   `{region,label:{role,articulation,note}}` choices from this analysis. All measured
   descriptors will be stored, but only these choices replace reviewed labels.
   **`roles:[]` preserves all previous labels and their original bindings**, even
   if those labels are stale. Notes allow blank text, at most256 UTF-8 bytes.
3. `leviathan_slice_catalog_commit` accepts only the current private preview token.
   Preview and commit freshly reauthorize/hash the selected source, enforce both
   revisions and the exact full base, and delegate one validated Editor transaction.
   No source bytes, mappings, markers, BPM or playback settings change. A no-op
   consumes only the Store review and preserves revision/history/redo. Failed
   Analyze/review/commit leaves the prior valid review and document unchanged.

Both inspector modes are **metadata-only, without source-file IO or a fresh-hash
claim**. `kind:"stored",sample_id:…` pages all current regions of that source,
including uncatalogued entries. Pending inspection still requires its full-base
and revision token. Missing/tampered media does not hide historical measurements;
Store refuses until source authority/identity is restored. Pages default/max16
records, bounded to256KiB structured data, with exact totals/next offsets and empty
end pages. Oversized pages return an actionable smaller-limit error, never truncate.

Persisted `Song.region_catalog` defaults empty and is omitted in old documents.
Source or bounds edits **retain** old descriptors/labels, with shared inspection
reporting their binding as stale. Name, root, ADSR and loop changes do not stale
pre-playback measurements; portable project path rebasing is not in the binding.
Region removal clears its catalog entry only after normal reference validation.
Historical IDs need not remain live after rebinding. Parsed snapshots validate
shape and finite values, **not their truth**: current-match is model-binding
equality, not fresh computation, current file verification or signed attestation.

Two atomic batch operations expose manual maintenance:
- `set_region_role {region_id,label}` requires nullable `label`; null clears only
  the reviewed role. Non-null labels bind current model source/bounds without
  computing/verifying audio or claiming classifier authority. Repeating an identical
  current-bound label is a no-op that preserves any existing measured-evidence SHA.
- `clear_region_features {region_id}` clears descriptors only, preserving the
  reviewed role and its historical binding. No arbitrary descriptor injection.

The shared core/UI manual-role action is pure metadata and can succeed with
missing media. MCP `set_region_role` runs inside ordinary `edit_batch`, whose
all-referenced-source pathname authorization/existence preflight remains in force:
it can refuse missing, symlinked or outside-root media before the metadata
transaction, without hashing or decoding audio. This is an intentional adapter
stage-policy difference, not identical UI/MCP failure chronology. Both catalog
inspectors remain metadata-only and can show historical records despite missing
or changed media.

Limits are independent:64 selected regions,256 catalog entries,30seconds per
region,16,777,216 total visited frames (overlap counted repeatedly),500,000,000
estimated work units, native8..384kHz, and existing strict import/serialized project
limits. This is bounded **synchronous** MCP work, not an async job/cancellation or
latency guarantee. No ML, automatic role authority, tempo inference or playback
change is introduced by this catalog.

## Reviewed-role seeded THIN v1

`leviathan_thin_preview` delegates to the shared `mutations::preview` and the real
pinned pattrns `ProbabilityGate`. It proposes **whole ordinary-cell deletion**, not
rotation, replacement, note movement or a second sequencer. The output is ordinary
editable Song content; no runtime random state or persistent mutation recipe is
introduced. Preview is nonmutating; source bytes, BPM and reviewed roles never
change.

All configuration is explicit:
```json
{
  "expected_revision": 12,
  "config": {
    "version": 1,
    "scope": {"pattern":1,"track":1,"row_start":0,"row_end":16,"columns":[0]},
    "seed": 42,
    "removal_percent": 35,
    "max_removed": 8,
    "protected_roles": ["kick","snare"]
  }
}
```
`max_removed` is a rejection cap0..4096: excess seeded removals reject the whole
preview, never stop early or take the first N.

`removal_percent` is an **integer0..100**, not a float or a desired exact removal
count. The shared summary exposes the actual effective f32 probability, canonical
gate calls/random draws and outcomes. There is one seeded gate decision per
eligible stored cell, not per occurrence: repeated patterns and aliases share that
ordinary source cell. Protected-role order is retained verbatim, with unique
members from kick/snare/closed_hat/open_hat_ride/crash. Empty explicitly disables
only those optional protections. **Unknown, Mixed, unreviewed and stale uses remain
protected**, never silently treated as a user-approved source.

Eligibility, source resolution, all-use protection and candidate proof are shared
core behavior, not MCP guesses from displayed notes. Every resolved unsuppressed ON use is
considered, including repeats/aliases, kit layers and melodic key zones. Alias
target writes reject. Phrase/effect/control cells and uncertain uses are retained
with explicit reasons. The `no_audible_use` wire reason means no evaluated unsuppressed ON, not measured
silence; zero-velocity and silent-source ONs still count. No downbeat/anchor or
role is inferred, and suggestions are not reviewed-role authority. Current role means **model source/bounds binding
matches**, not fresh audio verification or an attestation about loaded metadata.

The **whole base must be model-evaluable, even at0% or max_removed0**. This is a
conservative authoring prerequisite, not source decoding or proof that external
media is currently renderable. A candidate must preserve the shared retained-ON
attributes across all uses; selection-memory/velocity/round-robin drift rejects
atomically rather than being silently repaired. Retained Volume/Pan/Glide/Offset/Direction/
SampleStart/Cut controls must retain their origin, target, frame and value; new or
reassigned controls reject. This is not a claim that OFF,
choke, overlap, phrase replacement, voice lifetimes or final PCM remain identical.
Review the shared warnings and retained/removal reasons before committing.

`leviathan_thin_inspect` requires `preview_token` and one explicit kind:
- `decisions`: exact original source cell, row/column, use count, eligibility or
  retention reason and optional canonical gate index.
- `uses`: resolved unsuppressed ON occurrence/track/frame/instrument/source choice,
  pitch/velocity/pan, selection metadata and human review/current-match status.
- `contexts`: ordered affected blocks, including muted contexts.
- `changes`: exact before→null deletion records for reversing the proposal.

Each kind has its own total/start/next offset; default/max64 records and256KiB
structured data. Empty final pages are explicit, invalid ranges fail, oversized
pages ask for a smaller limit, and nothing is silently truncated. The bounded
record header exposes algorithm/gate/pattrns versions, original config, full
base/candidate and decision/use/change digests. It is transient provenance, not a
live document reference; inspect/save any desired review before consuming it.

`leviathan_thin_commit` accepts only the retained opaque token, not client cells or
candidate JSON. Both MCP and Editor revisions plus the exact full base guard one
undo transaction. Failed preview/inspection/commit preserves the previous review
and document; successful new preview supersedes the old token. Changed commit
invalidates pending reviews. True no-op consumes the token without dirty/revision/
history/redo changes. The complete bounded history/record response is pre-sized
before mutation and never echoes an arbitrary large title/path.

Shared THIN performs **no source-file IO, hash, decode or audio processing**.
MCP nevertheless retains its normal all-referenced-path authorization/existence
preflight at preview/inspection/commit; missing, symlinked or outside-root media
can refuse earlier than pure core/UI. `source_identity_verified:false` is explicit.
Changed bytes at an otherwise authorized path are not secretly hashed here;
evaluation/export retain their normal independent source rules.

Limits include4096 decision/use/change records and affected contexts, shared
model/evaluator/work/8MiB pretty-project constraints, and16MiB prospective source
path payload. Work is bounded synchronous MCP execution, not an async cancellation
or latency promise. No arbitrary Lua/code, hidden random seed, automatic role
assignment, per-occurrence randomization or broad composition-quality claim.

## Audition v1: prepared metadata, not headless playback

`leviathan_audition_prepare` is labeled **Prepare region or pad audition metadata
(no audio)** in `tools/list`. It calls the same pure bounded audition builder as
the native desktop; it does **not** invoke the desktop, open a device, render PCM,
publish a file or change Song/history/cursor/dirty/revision. Native desktop
exclusive audition and this isolated headless metadata inspection are different
adapters, not cross-session control. No MIDI capability is added.

Two explicit targets are available; `expected_revision` is always required:

```json
{"expected_revision":2,"target":{"kind":"region_draft","sample_id":7,"start_frame":0,"end_frame":48000,"root_note":60,"amp_envelope":null,"forward_loop":null}}
{"expected_revision":2,"target":{"kind":"pad","instrument_id":1,"pad_id":301}}
```

- **Region draft:** a registered SampleId and exact uncommitted inclusive start /
  exclusive end in original source frames. `root_note` is 0–127 and the synthetic
  ON uses that same note, so playback would be native pitch, independent of Song
  BPM. Both `amp_envelope` and `forward_loop` are **required nullable** fields:
  null explicitly selects the legacy/disabled option, not an inherited draft.
  No existing region or instrument is modified or allocated. Optional objects
  have the same strict units/limits as `set_region_playback`: ADSR integer
  milliseconds (attack/decay 0–10000, sustain_percent 0–100, release 0–500), and
  an at-least-two-frame forward loop wholly within the region. For example:

  ```json
  {"expected_revision":2,"target":{"kind":"region_draft","sample_id":7,"start_frame":10,"end_frame":48000,"root_note":72,"amp_envelope":{"attack_ms":2,"decay_ms":3,"sustain_percent":50,"release_ms":7},"forward_loop":{"start_frame":20,"end_frame":100}}}
  ```

- **Pad:** a committed kit-assigned InstrumentId and stable PadId, never a slot
  position or UI selection. The shared evaluator resolves the actual selector,
  gain, pan, native root, tune, mode, replacement/choke policy and selected
  region. Input velocity is fixed **1.0 / bin 127 before pad gain**, choosing the
  matching velocity zone and its **first** round-robin alternative. Every request
  starts a fresh isolated context; repeated requests do not advance persistent
  RR counters, inherit column memory, play neighbours, or apply a pad draft.

The complete `summary` exposes the shared version/target, 48 kHz clock,
**96000 musical + 24000 tail = 120000 total frames**, selected region bounds,
ADSR/loop and declared source SHA, ON gain/pan/sounding note and master gain.
`pad` is null for a region draft; otherwise it exposes exact committed pad
metadata, including selector, tune, mode and chosen layer. `layer_selection`
includes the input velocity/bin, zone bounds, stable layer ID, RR index and
ordinal. Region-draft summary IDs are **synthetic local IDs**, not new document
registrations; the requested original SampleId remains in `target`. Pad IDs and
selected region/source IDs retain their committed identity.

Preparation describes one ON followed by the existing final release at two
seconds. A long source/loop would be released at that gate and its tail bounded
to 0.5 seconds; a short source can exhaust earlier. These are snapshot semantics,
not evidence that any PCM was rendered or speakers played. Shared warnings
about native desktop Stop/focus/closing describe that other adapter only.

The result is explicit:

- `scope: "headless_metadata_only"`
- `source_paths_authorized_now: true` means the retained snapshot's lexical
  source paths passed the existing root/regular-file/no-symlink checks. All
  alternatives of the selected pad are retained and path-checked; unrelated
  document sources are not inspected by this tool.
- `source_identity_verified_now: false` and
  `summary.source_identity_verified: false`: stored SHA/native metadata is
  **declared**, not freshly hashed or decoded. Even changed/non-audio bytes at a
  regular authorized path can still yield a metadata preview.
- `audio_rendered: false`, `audio_device_opened: false`,
  `document_changed: false`.

`preview_id` is the shared synthetic snapshot digest. `review_ref` additionally
binds the exact current full base Song, typed request, both MCP/Editor revisions
and that snapshot ID. It is a **provenance reference only**, not an authorization,
retained workflow, commit/render token or authenticated source attestation.
Repeated unchanged requests return the same reference; a document edit/Undo ABA
changes it even if the audible snapshot returns to the same content. Invalid or
stale requests leave all document and history state untouched, including redo.

Both shared metadata and the complete structured result are bounded to 256 KiB;
source paths are at most 8192 bytes, and selected-pad mapping remains within the
existing 8 zones / 8 alternatives per zone / 32 alternatives per pad limits.
Names/path/bounds/options over the shared limits reject; responses are never
silently truncated. No pagination tool or inert server token is needed for this
single bounded summary.

There is deliberately **no `leviathan_audition_render`**, audition WAV publisher,
live GUI IPC or device-control MCP tool in this version. Ordinary
`leviathan_song_evaluate` / `leviathan_song_export` still operate on the current
Song, **not** on `review_ref`; they are not a hidden audition playback route.

## Accepted markers → a reviewed new pad bank

`leviathan_marker_bank_preview` and `leviathan_marker_bank_commit` expose the shared
**accepted-marker-bank-v1** allocator. This is not automatic slicing, a timing
compiler or pattern generation. It creates ordinary editable regions, one new
kit/pad bank and a newly assigned instrument from **already persisted accepted
markers**. No BPM is required, inferred or changed; no MIDI or ML is added.

### Preview an explicit selection, without source-file access

```json
{"expected_revision":3,"config":{"version":1,"marker_set":31,"interval_indices":[0,2],"name":"Reviewed marker bank"}}
```

The four config fields are required, and unknown fields reject:

- `version` must be `1`.
- `marker_set` is the stable nonzero ID of a persisted accepted marker set, not a
  cached detection result or a caller-provided boundary array.
- `interval_indices` contains **1–16 strictly ascending unique zero-based
  indices**. Index `i` selects exactly `[markers[i], markers[i+1])` in original
  source frames. Sparse selections such as `[0,2]` are allowed and remain two
  separate intervals. Empty, duplicate, descending, fractional, out-of-range or
  over-16 selections fail; nothing is sorted, merged or truncated silently.
- `name` is nonblank and at most **256 UTF-8 bytes**. It labels the fresh kit and
  instrument. Per-pad/per-region names are deterministic `Slice NNN` labels for
  the original interval, not inferred drum roles.

The preview is **pure metadata work**. It does not resolve/canonicalize/check
paths, hash or decode source bytes, touch audio devices, create output files, or
change the Song, cursor, history, dirty flag, clock or live transport. Missing,
changed or now-symlinked media does not prevent reviewing accepted metadata;
those conditions are checked at commit. Existing project open/import path policy
is unchanged: this is not a route to inject arbitrary source paths or identities.

The complete bounded result contains `preview_token`, shared `preview_id`,
`effective_config`, `source_path` and `summary`. The summary includes selected
marker/source identity and native metadata, all fresh kit/instrument/region IDs,
and every pad's interval index, exact bounds, slot, selector, root and playback
defaults. No additional pagination request is necessary for at most 16 pads.
`source_identity_verified_now:false`, `source_path_authorized_now:false`,
`audio_decoded:false` and `document_changed:false` describe this stage honestly.
The shared summary's `source_identity_verified:false` is always a property of
that pure plan, not a claim about a later commit's source check.

### Review the safe defaults and allocations

The shared allocator—not the MCP adapter—chooses checked fresh IDs strictly above
the existing maximum in each namespace. It never overwrites an occupied ID or
silently reuses a hole. Exhausted region/kit/instrument IDs, existing model errors,
prospective model limits and bounded-work failures reject atomically.

Within the **new** kit, selected intervals become stable PadIds `1..N`, slots
`0..N-1`, and note selectors `36..36+N-1`. Every new region/pad uses native
reference root `60`, tune `0`, unity gain, center pan, **OneShot / Overlap**, no
choke group, no velocity layers, and no inferred ADSR or forward loop. Root 60 is
an explicit new playback reference, **not detected source pitch**. Every region
retains its exact selected endpoint pair, including any tail before the next
marker; no extraction, fades, resampling or minimum-duration inference occurs.

Existing regions, kits/instruments, reviewed roles, immutable source metadata,
markers/locks/source-BPM, Song BPM/LPB, patterns, order, aliases and occurrence
mutes are preserved. No tracker notes or new pattern/order occurrences are
inserted. This is a one-time allocation, **not a live marker recipe**: later
marker changes do not regenerate the bank; its ordinary region/pad metadata can
be edited explicitly through the existing tools.

### Commit only the reviewed private plan

```json
{"expected_revision":3,"preview_token":"TOKEN_FROM_MARKER_BANK_PREVIEW"}
```

Commit accepts no candidate, IDs, path, source hash, marker array or altered
config. It checks the current retained token, both MCP and Editor revisions and
the exact shared base Song, including signed-zero scalar representations. It then
authorizes the selected marker set's **original lexical source path** under the
existing workspace-root/regular-file/no-symlink policy and freshly verifies the
stored SHA-256 with the existing **64 MiB encoded-source limit**. This is a
hash-only check, not audio decoding. The guards are checked again before the
shared Editor applies the reviewed additions as **one Undo transaction**.

Only this selected source is inspected: an unrelated missing source does not
invalidate this allocation. The receipt therefore reports
`source_identity_verified_now:true` and
`all_source_identities_verified_now:false`; it is **not** whole-project
save/render-readiness evidence. Normal Save/Open/export source policies remain
unchanged and can still reject unrelated bad media. No source or audio file is
written, and no playback starts. Path checks are not an OS sandbox against
hostile concurrent filesystem renames or a guarantee that bytes cannot change
after the check.

The commit response is pre-sized before mutation and returns the original shared
summary plus bounded `session` dirty/undo/redo indicators only. It omits arbitrary
document titles, project paths and counts (`session_scope:"history_only"`). A
successful commit consumes its review; another commit requires another explicit
preview. Undo removes the metadata additions together, and Redo restores them;
neither changes external source files or reruns the commit-time source hash check.
Later playback/Save still performs its normal source validation.

### Review lifetime and budgets

One private marker-bank plan is retained at a time. Successful new preview
supersedes the previous marker-bank token; failed preview or commit preserves the
current plan and document so a restored source can be retried. Changed document
revisions—including Undo/Redo, Save, New and Open—invalidate the token. A genuine
no-op edit preserves the pending review, revision and existing redo stack. An
edit/Undo ABA cannot revive an old token even when the Song returns to equal
content. Tokens are session-local review references, not portable signed
attestations or live GUI capabilities.

Shared preview uses bounded prospective validation: at most 65,536 visits for
ordered track blocks and resolved definition/ordered rows/cells, 32,768 phrase
definition cells, and 1,048,576 conservative prospective validation units. It
checks the existing and candidate pretty-project 8 MiB storage limits without
file access. MCP additionally enforces its 8 MiB compact candidate limit and
**256 KiB complete structured preview/commit payload**, before replacing a
review or committing. Names/path metadata cannot bypass that response bound;
oversized results fail without silent truncation. Retention is one bounded
base/candidate plan in this family, not an accumulating token history; the other
existing review families keep their separate bounded slots. These limits are
resource guards, not callback-latency or whole-arrangement render guarantees.

## Exact whole-cell rectangle Copy, Paste replace and Clear

These four tools share the native Editor's opaque document ownership and the pure
`whole-cell-rectangles-v1` core. They edit ordinary pattern cells only. They never
read paths or source files, hash/decode/render audio, use the system clipboard,
control an open GUI, or change transport/cursor state. Missing registered media
therefore does not block these operations; normal project Open/Save/export source
policies remain unchanged.

### Copy an explicit half-open rectangle

Call `leviathan_cell_block_copy` with all six scope fields:

```json
{"expected_revision":3,"scope":{"pattern":1,"track":1,"row_start":0,"row_end":2,"column_start":0,"column_end":3}}
```

Rows and columns are zero-based, with **exclusive** end bounds. The nonempty
rectangle must fit the existing pattern and track's note-column count; it may
contain at most **4,096 positions including holes**. There is no automatic growth,
cropping, merge, or hidden selection. Each rectangle addresses one pattern and one
logical TrackId, not an order occurrence or phrase definition.

Copy may read an alias. The summary records the selected logical source scope and
complete resolved source chain. Its frozen row-major cells retain every scalar,
effect and phrase field, including signed zero, OFF and parameter-only cells.
`null` is an absent cell; an explicit stored default Cell is present and remains
separate from a hole. Copy does not canonicalize, flatten phrases, infer missing
instrument memory, or promise identical audio after moving cells.

The result provides a private `clipboard_token`, shared `clipboard_id`, complete
bounded summary, historical copy revisions and `source_document_sha256`. That
digest describes the copied document metadata, **not verified source audio**.
Only one server-owned clipboard is retained. No tool accepts caller-provided
Cells, serialized snapshots, owner IDs or external clipboard contents.

### Preview Paste replace or Clear

Call `leviathan_cell_block_preview` with exactly one typed operation:

```json
{"expected_revision":3,"operation":{"kind":"paste_replace","clipboard_token":"TOKEN_FROM_COPY","destination":{"pattern":2,"track":1,"row_start":4,"column_start":0}}}
```

The destination is its upper-left position; dimensions come only from the retained
clipboard. A different pattern is allowed, but its logical TrackId **must equal
the copied logical TrackId**, including when Copy resolved an alias. The complete
rectangle must fit. Existing whole cells are replaced, and copied holes remove
existing destination cells. Overlap is safe because the source is frozen, not
reread during Paste. Raw copied instrument/phrase IDs resolve against current
same-document definitions: deleting or altering those definitions may reject or
change the meaning of a later Paste. There is no resource cloning or ID remapping.

```json
{"expected_revision":3,"operation":{"kind":"clear","scope":{"pattern":2,"track":1,"row_start":4,"row_end":6,"column_start":0,"column_end":3}}}
```

Clear removes stored cells only inside the explicit rectangle; it does not require
a clipboard. Both operations reject alias destinations without an implicit Make
unique or redirect. Review the affected aliases/occurrences when changing a local
block that other blocks share. Existing order, occurrence mutes, resources and
clock are otherwise unchanged. Empty row/track containers are not silently
canonicalized away.

Preview returns `preview_token`, shared `preview_id`, exact shared `request`,
summary and independent `record_counts`. Summary includes inserted/removed/replaced
counts, projected work and affected contexts/aliases. It is not a commit or audio
verification. Successful new preview replaces only the previous rectangle review;
failed preview preserves the current clipboard, review and document.

### Inspect bounded independent pages

Use `leviathan_cell_block_inspect`. Every request needs the **current** MCP revision.
`start` defaults to `0`; `limit` defaults to `64` and must be **1–64**.

```json
{"expected_revision":3,"kind":"clipboard","clipboard_token":"TOKEN_FROM_COPY","start":0,"limit":64}
{"expected_revision":3,"kind":"changes","preview_token":"TOKEN_FROM_PREVIEW","start":0,"limit":64}
{"expected_revision":3,"kind":"contexts","preview_token":"TOKEN_FROM_PREVIEW","start":0,"limit":64}
{"expected_revision":3,"kind":"aliases","preview_token":"TOKEN_FROM_PREVIEW","start":0,"limit":64}
```

Each result contains its header, `kind`, `records`, and
`page:{start,total,next_start}`. Each kind has its own offset and total; never reuse
a change count as a context offset. Clipboard records have row/column offsets and
nullable `cell`; changes have absolute destination row/column and exact nullable
`before`/`after`. Contexts identify affected order occurrences, patterns, tracks
and mute state. Aliases identify affected stored-content alias blocks. An empty
page at exactly `total` is allowed; offsets beyond it, wrong reference kinds and
unknown fields reject. Records are never silently truncated.

### Commit and separate clipboard/review lifetimes

Call `leviathan_cell_block_commit` after inspecting the private plan:

```json
{"expected_revision":3,"preview_token":"TOKEN_FROM_PREVIEW"}
```

Copy, Preview and Inspect are allowed without Edit Mode. Commit requires Edit Mode
and rechecks the **Editor-owned document identity**, both MCP/Editor revisions and
exact full base Song. No caller candidate is accepted. Changed content is one
shared Editor Undo transaction. A genuine no-op preserves dirty/history/redo and
both revisions; either successful commit consumes the review. The bounded receipt
contains the shared request/summary and history-only dirty/Undo/Redo state, not
arbitrary titles or project paths. It explicitly reports no file or audio work.

- The accepted clipboard survives ordinary edits, Save and Undo/Redo. Inspect or
  Paste uses its token with the latest MCP revision, not its historical copy
  revision. Content is frozen, not a live source selection.
- Successful **New/Open revokes the clipboard**, even if the replacement has
  byte-identical Song content. Failed New/Open preserves the existing authority.
  Tokens confer no authority in another server session or document.
- A successful Copy replaces the clipboard **and retires the rectangle preview**.
  Failed Copy preserves both. Token-counter exhaustion also fails atomically.
- Any changed document revision, including Save/Undo/Redo, invalidates the pending
  rectangle review; exact no-op edits preserve it. Edit/Undo ABA cannot revive it.
- Invalid/stale references, failed validation and rejected Commit do not consume
  a still-current review or clipboard and do not modify the Song/history.

### Resource bounds

Copy retains at most **1 MiB serialized clipboard data** and 4,096 positions.
The shared preview retains at most **2 MiB exact diff data**. Base/candidate
pretty-project storage is bounded at **8 MiB**, with the MCP's additional 8 MiB
compact-candidate check. Shared bounded-work checks cap resolved definition/order
visits at 65,536, phrase definition cells at 32,768 and conservative prospective
validation units at 1,048,576. A valid rectangle may still reject if the surrounding
project exceeds those work budgets.

Every complete structured response is checked against **256 KiB** before a
clipboard/review replacement or document commit; text mirroring and JSON-RPC add
transport overhead. Inspection checks the 64-record page limit before allocating
record pages. Retention is one clipboard plus one bounded review in this family,
not a growing token history; existing review families retain their own bounded
slots. Limits reject without partial edits or silent truncation and do not claim
a wall-clock deadline, automatic inherited-memory repair or audio equivalence.

## Persisted track balance — headless authoring, not live GUI control

`leviathan_track_mix_preview` and `leviathan_track_mix_commit` share the native
`track-balance-original-contributions-v1` implementation and Editor transaction
boundary. They store **gain, stereo balance, mute and solo** on this process's own
Song. They do not attach to a GUI, start a device, send live mailbox messages,
change transport, inspect physical output or claim that anything was heard.
Existing `song_export` uses the current persisted settings through the same Engine.
Saving settings makes them available to a later project load/start; there is no
MCP service or automatic synchronization of another open editor.

### Read stored settings or review a complete replacement

`leviathan_project_read` with `{"section":"track_mix"}` returns the persisted map
keyed by stable TrackId plus explicit neutral defaults. This metadata-only read
does **not** compile routing: `compiled_layout_checked:false` and
`live_update_applied:false`. Old schema-1 projects without `track_mix` load with an
empty set; omitted tracks use gain `1`, balance `0`, mute `false`, solo `false`.
An empty set remains omitted from stored project JSON.

Call `leviathan_track_mix_preview` with an explicit version and complete settings:

```json
{"expected_revision":3,"config":{"version":1,"settings":[{"track":1,"gain":0.5,"balance":-0.25,"mute":false,"solo":false},{"track":2,"gain":0.75,"balance":0.5,"mute":false,"solo":false}]}}
```

Every setting requires all five fields. The set contains at most **64 entries**;
TrackIds must exist and be unique. Unknown fields, duplicates and invalid values
reject instead of applying last-wins behavior. This is a **complete replacement**,
not a patch: omitted old entries return to neutral defaults. To reset all settings,
use `{"version":1,"settings":[]}`.

Gain is finite **0–1**, so this stage attenuates rather than amplifies. Balance is
finite **−1–1**, with negative values attenuating the right channel and positive
values attenuating the left. There is no equal-power pan, crossfeed, panning-law
conversion, normalization or master-gain change. Input range checks occur in f64
**before** f32 storage conversion, so an out-of-range input cannot become valid by
rounding. Stored f32 scalar bits, including signed zero, participate in exact
history; different inputs rounding to the same stored value can be true no-ops.
`requested_config` retains the validated f64 input, `stored_settings` shows the
actual f32 map, and the shared summary reports its effective coefficients.

Mute wins over solo. If **any configured track is solo**, non-solo tracks are
gated—even when the solo track is unused or silent. Mixing is applied to original
Engine contributions, not by deleting events or sources. Muted/solo-excluded
voices still advance and can participate in shared kit round-robin and choke
behavior. These settings are separate from per-occurrence arrangement mutes,
which retain their existing scheduling/voice-release semantics.

### Routing status is explicit, not inferred readiness

The pure preview performs bounded model/evaluator/control-routing work without
path authorization, filesystem reads, source hashing, decode or audio rendering.
It returns `preview_token`, shared `preview_id`, exact `requested_config`, narrowed `stored_settings`, complete
shared `summary`, and explicit false source-verification/audio/live-update flags.
The summary has at most 64 contributing-track rows, with effective L/R coefficients
and shared-legacy track/instrument constraints. Names and source paths are not
repeated in those rows.

Inspect `summary.layout_status.kind`:

- **`available`**: compiled layout available, with exact contributing-track and
  legacy-group counts. `live_ready` is only the shared layout's eligibility flag
  (false with zero contributions), **not evidence of an active device, matching
  native playback session, successful live delivery or heard output**. This MCP
  always reports `live_update_applied:false`.
- **`unavailable`**: the model is valid but cannot presently evaluate, with a
  reason. For example, an unfinished empty-order draft can still retain/save
  settings. There is no compiled layout or playback claim.
- **`unsupported`**: neutral playback may remain supported although the optional
  mix layout is unavailable, with an exact reason/count. More than 64 contributing
  tracks can retain effective-neutral settings without inventing a truncated
  64-track layout; nonneutral mixing in that case rejects.

Legacy samplers are owned by InstrumentId, not independently by every track. All
tracks sharing one legacy instrument must have **equal effective L/R targets**;
known conflicts reject both review and the typed setter. The Engine's original
sampler slots, smoothing and voice history remain intact. There is no automatic
sampler splitting, instrument duplication or source conversion. Explicitly import
and assign registered regions if independent controlled-voice balance is wanted;
controlled voices retain their existing independent routing. A silent/neutral
metadata result does not prove that sources exist or match stored hashes.

### Commit the private plan or use the aligned typed batch operation

```json
{"expected_revision":3,"preview_token":"TOKEN_FROM_TRACK_MIX_PREVIEW"}
```

Commit accepts only the current token/revision, not a candidate, layout, owner ID,
device target or alternate config. It rechecks the opaque **Editor document
owner**, both MCP/Editor revisions and exact base Song, then uses the shared
protected Editor transaction. Preview/read do not require Edit Mode; Commit does.
Changed settings are **one Undo**; a genuine no-op preserves both revisions,
redo/history and dirty state. Successful Commit consumes the review either way.
The complete history-only receipt is byte-checked before mutation and deliberately
omits arbitrary document titles/project paths.

There is one retained review. Successful new Preview supersedes it; failed
Preview/Commit preserves a still-current plan. Changed revisions—including
Save, Undo/Redo and New/Open—invalidate reviews; exact no-op edits preserve them.
Edit/Undo ABA cannot revive a token, and equal-content New/Open has a different
Editor owner. Tokens are session-local references, not portable attestations.

The existing `leviathan_edit_batch` additionally accepts:

```json
{"expected_revision":3,"operations":[{"op":"set_track_mix","config":{"version":1,"settings":[]}}]}
```

This complete-set setter invokes the **same shared pure preview/commit policy**;
it does not duplicate gain math or bypass routing validation. It can combine
settings with other ordered edits in one atomic Undo transaction. Routing is
validated at that ordered step: put `set_track_mix` last when the final batch
routing must be reviewed. Later ordinary source/arrangement edits can change
render readiness without invalidating the stored model. The Engine revalidates
actual routing before nonneutral rendering; no blanket evaluator requirement was
added to every generic edit. The existing
batch's all-source path preflight still applies, while the dedicated metadata
preview/commit deliberately performs no source IO. Missing/tampered/symlinked
media can therefore remain editable through the metadata tools but fail normal
Save/Open/export or a general batch; no whole-project readiness is implied.

Track settings are model references: `remove_track` refuses a track with a mix
entry, even with `clear_cells:true`. First use a complete `set_track_mix` excluding
that track, then remove it in the same batch if desired. Nothing silently resets
settings, and failed removal changes no history. Updating an existing TrackId's
name/columns preserves its mix entry.

### Offline output, native live behavior and limits

Ordinary offline export starts directly from the persisted target coefficients;
it does not replay fader gestures or native live ramps. Explicit neutral settings
retain old neutral PCM arithmetic and neutral manifest behavior. Effective
nonneutral rendering is recorded in the shared schema-11 event manifest without
rewriting original musical events. Export keeps existing bounded source
verification and paired WAV/manifest no-overwrite publication safeguards.

Native live Apply is a **separate desktop behavior** restricted to a matching
currently playing owner/content snapshot. Its bounded coalesced updates and
128-frame transitions are not stored automation, a headless control channel or
physical-latency/listening evidence. The MCP exposes no live transport/device
controls, live GUI meter readback, automation lanes, FX, stems, recording or MIDI.
The separate offline sample-peak tools below measure only this headless Song.

Shared preparation bounds the base/candidate project to 8 MiB and uses the shared
65,536-visit /32,768-phrase-cell /1,048,576-validation-unit work budgets. Declared
source paths are limited to 8,192 bytes to bound repeated manifest metadata, not
as a filesystem check. MCP additionally caps the compact candidate at 8 MiB and
every complete structured read/preview/commit response at **256 KiB**; JSON-RPC
and text mirroring add overhead. Only one bounded review is retained in this
family. Oversized work/responses fail without silent truncation or replacing the
current review, and there is no callback or wall-clock latency guarantee.

## Bounded offline Song sample peaks — not live GUI meters

`leviathan_sample_peaks_analyze` and `leviathan_sample_peaks_inspect` expose the
shared `original-contribution-sample-peaks-v1` measurement math through the same
Engine and 2,400-frame window accumulator used by native observation. **They do
not observe or control an already-open GUI, live transport, audio device, hardware
buffer or speaker.** Analysis renders bounded PCM internally, returns measurements
only, and never starts output, writes files, retains/returns PCM or edits the Song,
history, dirty state, revision or tracker cursor. Read Mode is allowed.

### Analyze an explicit interval

```json
{"expected_revision":3,"config":{"version":1,"start_frame":2400,"end_frame":24000}}
```

All three config fields are required; extra keys, a caller Song/layout/candidate,
source path overrides, device IDs and sample payloads are refused. Coordinates
are **48 kHz Song/Engine frames**, not source-file/native-asset frames:

- The selected interval is half-open `[start_frame,end_frame)` and nonempty.
- Start must be divisible by **2,400** (50 ms at 48 kHz).
- End must be divisible by 2,400 **or equal the exact natural Song musical+tail
  terminal frame**. It must not exceed either that terminal or **120,000** frames.
- There is no rounding, implicit snapping, duration extension or arbitrary
  unaligned crop. Only an exact natural terminal may produce `partial:true`, with
  its actual frame count; no zero padding is measured.
- The complete compiled Song and its scheduling context remain intact. Engine
  always advances from frame **0 through end_frame**. Earlier complete windows
  are discarded from the report, not skipped by a cold seek. Aliases, occurrence
  mutes, phrase/choke/control scheduling, incoming voices, source selection and
  round-robin context are therefore preserved. The range filters observations,
  not the arrangement or which full-compiled sources must be verified.

The result gives a session-local `analysis_token`, deterministic shared plan and
result IDs, exact config/shared summary, source verification count and the first
up to four selected windows. A no-trigger or otherwise unrenderable Song is refused by the shared evaluator;
no zero windows or source-byte verification are invented. The summary distinguishes processed prefix length,
observed interval, natural terminal, scheduled starts, conservative voice-frame
work, decoded-source budget and track layout. IDs are provenance, not portable
capabilities or promises that files will remain unchanged.

### What the numbers mean

Each `master_peak` is the maximum absolute **sample** amplitude in each channel
of the actual final stereo PCM. Each available track peak first sums that track's
existing contributions in the Engine's original f32 order **after track balance
and fixed master gain (0.5)**, then takes the maximum absolute sample per channel.
There is no RMS, loudness, true-peak/oversampling, normalization or listening claim.
Track regrouping need not reconstruct the master bit-for-bit.

`track_peaks` slots follow `summary.layout.tracks` (or the inspection page's
`layout.tracks`), using exact stable TrackIds. These are actually scheduled ON
owners, not every defined or configured track. `null` means **unavailable**, never
silence or a subtotal presented as a complete track. The shared observer refuses
whole-track attribution for shared legacy ownership and for meter-only nonfinite
subtotals. The first offline route deliberately rejects path-only legacy sources
before IO; native observation can encounter those ownership cases. If a neutral
Song exceeds bounded per-track layout capacity, the shared result may provide
master-only windows with empty track rows and an explicit `tracks_unavailable`
reason—not the first 64 tracks disguised as complete data.

Reports carry exact emitted bounds, first/last Song bounds, frame count, report
index, wrap count and first/last mix revision. This offline pass has no seek/loop
or live mix updates: its positions stay on the original Song timeline and wrap count remains zero.
`first_mix_revision`/`last_mix_revision` are zero by the shared helper convention
for static persisted settings applied from frame zero, **not** native GUI
accepted/enqueued revision counters.
Native live observation instead reports render-ahead full windows only, dropping
an incomplete terminal window; neither route claims device playback or hearing.

### Inspect exact pages without repeating IO

```json
{"expected_revision":3,"analysis_token":"TOKEN_FROM_ANALYZE","start":4,"limit":4}
```

`start` is a zero-based index **among selected result windows**, not a Song frame
or the report's one-based full-context `report_index`. `limit` is 1–4 (default 4). Every page
includes exact total, bounds and `next_start`; `start==total` yields an empty page,
while `start>total` fails. Nothing silently truncates a window or track list.
Inspect performs no path/hash/decode/render work and explicitly reports
`source_identity_verified_now:false`: source verification happened at Analyze.
A missing or changed file after Analyze does not turn a historical measurement
into a fresh-byte claim.

### Source authorization, budgets and failure atomicity

Shared **pure** preparation evaluates the complete exact Song route and checks
model, project/work, requested range and declared decoded-source budgets before
source IO. Every actually used source must be a **registered region route**
(including kit choices and multisample zones). Any used path-only legacy route is
refused with explicit import/register **and assign-region** guidance; merely
registering another reference to the same pathname does not change its playback
route or authorize an unbounded legacy decoder.

All selected original absolute lexical paths are checked against the startup
workspace's regular-file/no-traversal/no-symlink policy **before** Engine path
canonicalization, hashing and decoding. The shared Engine verifies declared SHA,
rate/channels/frames and current bytes. This includes a source first used later
than the requested prefix. Unused source files are not opened or byte-verified here. Full Song/model
validation and bounded metadata/path-length/work checks still apply to the
document. This narrow analysis does **not** bypass normal project
Open/Save/export path policy or claim that all project media are ready.

Bounds include: full base project 8 MiB; shared scheduling/validation work caps;
120,000 processed frames; conservative `scheduled_starts × end_frame ≤ 2^28`;
64 MiB per encoded source and 256 MiB aggregate encoded sources; 16 Mi sample
values/64 MiB aggregate decoded PCM including guard frames (shared identities are
handled by the same Engine); at most 64 attributable track rows; completed report
at most 1 MiB; at most four windows and **256 KiB structured payload** per response.
MCP text mirroring/envelopes add bounded wire overhead. Analysis is synchronous:
there is no callback latency guarantee, wall-clock SLA or MCP async-cancel handle.
The host can terminate its child process; no output file is left to publish.

Exactly **one** shared plan/base and one completed report are retained, with no
PCM or unbounded token history. The combined serialized footprint is **base up to
8 MiB plus result up to 1 MiB and bounded metadata**, not a 1 MiB total-cache claim.
A successful Analyze supersedes the old result only after complete rendering,
exact current-document checks and response-size validation. Any range, work,
source/path/hash/decode, clipping/nonfinite, rendering or response-size failure
preserves the previous complete report; no incomplete success is returned.

Inspection rechecks the opaque Editor document owner, both MCP/Editor revisions
and shared exact serialized Song equality, including signed zero and current mix.
Changed edits, Undo/Redo, Save and New/Open invalidate the report. Exact no-op edits
preserve it and redo history. Edit/Undo ABA cannot revive an old token, and an
equal-content document replacement has a new owner. No measurement is persisted
in the Song and no new typed edit operation is added.
