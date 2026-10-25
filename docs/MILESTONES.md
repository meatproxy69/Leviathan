# Integrated sequencer milestones

| # | Delivered |
|---|---|
| 1 | Validated sparse song content, typed IDs, tracks/instruments, variable patterns/order. |
| 2 | Editable focused tracker, note/OFF and independent attributes, Edit Mode/navigation. |
| 3 | Versioned save/open, asset resolution, guarded transitions, undo/redo/saved state. |
| 4 | Document-driven deterministic offline rendering, repeats vs independent clones. |
| 5 | Incremental transport, Play/Stop/Loop/Follow, cold seek and device/error handling. |
| 6 | Per-cell retrigger/cut/offset/reverse/glide with isolated voices and limits. |
| 7 | Instrument phrase pools, explicit selectors, transposition/scaling and parent gates. |
| 8 | Matrix, source-sharing aliases, hard-cut occurrence mutes, named section variation. |
| 9 | Pure/stale-checked transformation previews; committed groove and scoped edits. |
| 10 | Seeded pattrns rhythm/probability/chord generation into ordinary editable cells. |

The combined integration regression exercises these as one document, through
history, save/open, variable-length coda, export, snapshot streaming, loops, seek
and Stop. Other tests cover individual semantics and failures.

Aliases require equal row/column shapes and reject cycles/dangling/local conflicts.
Repeated references share changes; independent variations materialize content.
Transform/generation previews disclose sharing, and changed alias destinations
become local copies. Mutes ignore controller cells and cut voices/tails at entry.

Phrases are non-recursive one-shots. Trigger pitch transposes their root, launch
velocity/pan scale/offset local values, and parent note/OFF/cut gates compiled voices.
Groove stores visible cell delays; playback does not rerun generators or recipes.

Practical bounds include 4096 order positions, 64-block alias chains, 64 phrases per
instrument, 256 phrase launches, 2048 expanded note triggers, 32768 populated cells,
ten-minute songs, bounded source variants/voices, 8 MiB projects and 64 undo states.
Generation permits 32 saved recipes, 1024 rows/run and eight chord columns. Errors
reject invalid edits/outputs instead of silently clipping values or truncating data.

The original ten milestones are complete. This does not include MIDI/hosting,
recording, mixer/routing, automation, discovery or XRNS support.
