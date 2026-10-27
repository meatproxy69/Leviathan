# Direct agent control

LEVIATHAN exposes Rust actions and structured state through newline-delimited
JSON. No MCP setup or visual UI automation is required. This interface is in the
current source; the older Alpha 2 downloads do not include it.

## Running editor

Launch the editor as a child process with stdin/stdout pipes:

```powershell
.\target\release\leviathan.exe --agent-stdio --workspace-root C:\music
```

The workspace must already exist. An optional `--project PATH` opens a project.
Normal launches do not open the agent channel. In agent mode stdout is reserved
for one JSON reply per request line; diagnostics use stderr. Requests are limited
to 1 MiB and replies to 16 MiB. Malformed or oversized requests return errors;
the next line can still be processed. Keep the child and its pipes open between
requests.

```json
{"command":"discover"}
{"command":"app_state"}
{"command":"app_ui","arguments":{"edit_mode":true}}
{"command":"app_timing","arguments":{"action":"set_draft","bpm":140.0}}
{"command":"app_timing","arguments":{"action":"apply"}}
```

`discover` returns the 64 core operations with JSON input schemas, runtime
commands with argument schemas, and component names. Inspect a component's state
to obtain its actions, fields, selections, drafts, readiness, errors, and preview
results. The desktop owns the actual Editor and Session: edits share undo/redo,
dirty state, cursor, project identity, and GUI feedback. There is no second
headless copy of the open project.

## Components

Use `app_component` with a component name and an optional action object. Omitting
the action reads state. Action names and field schemas are supplied in state.

```json
{"command":"app_component","arguments":{"component":"samples"}}
{"command":"app_component","arguments":{"component":"samples","action":{"action":"load_preview","path":"break.wav"}}}
{"command":"app_component","arguments":{"component":"samples","action":{"action":"poll"}}}
{"command":"app_component","arguments":{"component":"samples","action":{"action":"new_region"}}}
{"command":"app_component","arguments":{"component":"samples","action":{"action":"patch_region_draft","fields":{"name":"First hit","start":0,"end":4000,"root":60},"map_on_apply":true,"instrument":1}}}
{"command":"app_component","arguments":{"component":"samples","action":{"action":"apply_region"}}}
```

Poll until the source load finishes before creating a region. Source frames are
integer, zero-based, with an exclusive end. Waveforms are returned as numeric
bins alongside source metadata. Playback, envelope, loop, marker, and break
drafts are available directly; no screen coordinates are used.

| Component | Controls |
| --- | --- |
| `tracker` | Cells, effects, phrase triggers and definitions, patterns, arrangement links/mutes, order, undo/redo |
| `samples` | Source loading, waveform, region trim/playback, markers, onset suggestions, tempo/pulse, catalogs, pad banks, break sequence preview/commit |
| `pads` | Kits, pads, mappings, velocity/round-robin layers, insertion and audition requests |
| `steps` | Drum step selection, draft fields, apply and navigation |
| `multisample` | Maps, zones, assignment, draft/apply, inspection and reload |
| `generation` | Generator configuration, previews and commit |
| `transform` | Transformation drafts, previews and commit |
| `groove` | Groove drafts, previews and commit |
| `thin` | Thinning drafts, previews and commit |
| `cells` | Cell-block drafts, previews and commit |
| `resample` | Prepare/print, output receipt, verified registration |
| `mix` | Track gain/balance/mute/solo, preview/apply, meter snapshot/follow |

Asynchronous operations expose busy/phase/readiness. Poll their state or `poll`
action as advertised. Verification and mutation are separate: where supplied,
use `commit_verified` only after verification is ready. Source changes, document
replacement, stale revisions, invalid drafts, and Read Mode retain the existing
editor checks. Component handoffs update the actual active view and selection;
replies include the resulting component state.

## Runtime commands

| Command | Purpose |
| --- | --- |
| `app_state` | Current document/revision, paths, cursor, views, confirmations, jobs, transport and mix delivery |
| `app_ui` | View, selected instrument, Edit Mode, Follow, Loop, step and octave |
| `app_cursor` | Exact occurrence, track, row, note column and field |
| `app_timing` | Inspect/set/reset/apply the transport BPM/LPB draft |
| `app_project` | File-path drafts, staged New/Open/Save, export and sample assignment |
| `app_play`, `app_stop`, `app_seek` | Existing live Song driver start, hard stop and cold seek |
| `app_audition` | Selected sample region or pad audition |
| `app_mix_apply` | Submit committed mix to the active Song transport |
| `app_confirm` | Resolve an existing project confirmation: cancel/discard/save/overwrite |
| `app_close` | Normal close request, respecting unsaved work |
| `app_window` | Typed window focus, size, minimization, maximization and fullscreen requests |
| `app_legacy` | Optional legacy motif/sample-loader/export controls |

The driver reports its real state and errors. Start and stop are asynchronous;
poll `app_state` to observe completion. Audition explicitly requested by the agent
does not require physical window focus. Playback uses a start snapshot; restart
to include later song edits. No API can retract audio already submitted to a
device. Large synchronous core operations may occupy the UI thread; component
analysis and preview actions use the existing background jobs.

## Shared core operations and headless use

```powershell
.\target\release\leviathan_agent.exe --workspace-root C:\music
```

This companion owns an isolated document and has no GUI or live audio device.
It accepts `help`, `discover`, and `call` through the same JSONL pipes. Core calls
also work in the running editor:

```json
{"command":"call","name":"leviathan_project_read","arguments":{}}
```

Discover the operation schema before mutation and supply its required revision
and other fields. Previews use existing validation and commit tokens. In the
desktop, manual edits or a new/opened document invalidate stale core tokens.
Ordinary file-path operations preserve the editor's existing confirmation rules.

## Paths and availability

The workspace root constrains the 64 structured core operations. Desktop file
controls have the same file authority as the GUI: relative paths use the project
directory or workspace as applicable, and explicit absolute paths may refer
outside it. The workspace flag is not a process-wide filesystem sandbox. Launch
this local channel only for an agent you authorize to operate your application.

Exposing an action preserves its implementation limits. In particular, existing
platform restrictions in resample publication and unsupported independent
pitch-preserving time-stretch remain visible as errors or unavailable states.
This interface does not add an audio algorithm or claim physical playback proof.
