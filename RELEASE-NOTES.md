# LEVIATHAN Alpha 2 — v0.1.0-alpha.2

**Early alpha. Keep backups of your projects.**

LEVIATHAN is an agent-first Rust tracker and sequencer for editable patterns,
instrument phrases, arrangement, groove, and seeded generation. Local agents can
work with `.leviathan.json` projects and the included command-line tools; no MCP
server, plugin, or configuration is required. There is no embedded AI agent.

## Downloads

Both platform packages are available together:

| Platform | Package |
| --- | --- |
| Windows x86_64 | [`leviathan-v0.1.0-alpha.2-windows-x86_64.zip`](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-windows-x86_64.zip) |
| Linux x86_64 | [`leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz`](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz) |

Both
packages contain executables under `bin/`; the Linux package also includes
`run-leviathan.sh` as its desktop entry point. Check the release page for current
availability and package details.

## Highlights

- Windows x86_64 package alongside the Linux x86_64 package.
- 24 tracker channels and instrument slots in new songs; existing songs can be expanded with an undoable action.
- Instrument sample assignment from the editor, including the native file picker.
- Editable sparse song model, stable IDs, and variable-length ordered patterns.
- Focused keyboard tracker, explicit Edit Mode, typed attributes, repeat and clone.
- Validated project save/open, replacement confirmation, and bounded undo/redo.
- Shared offline and streaming sampler engine, snapshot Play/Stop/Loop/Follow/cold Seek.
- Retrigger, hard cut, sample offset, reverse, and pitch glide.
- Instrument-owned one-shot polyphonic phrases with their own LPB/root/FX.
- Connected arrangement matrix, shared aliases, occurrence mutes, and section variations.
- Previewed scoped groove/transpose/velocity/pan/row reversal.
- Seeded pattrns note/chord generation committed into editable cells with saved recipes.
- CLI tools for synthetic project generation and offline WAV rendering.

## Validation and limits

Do not treat the Alpha 1 validation record as Alpha 2 release verification. This
release has not been verified for live audio playback or Linux GUI interaction.
Offline render commands and automated tests are distinct from those checks. See
the [project README](README.md) for build instructions and the
[validation notes](docs/VALIDATION.md) for the currently recorded evidence.

No samples or recordings are bundled. Samples remain external references. No
MIDI I/O, VST/CLAP hosting, recording, automation, or XRNS compatibility is included.

AGPL-3.0-only. Retain dependency notices and license texts when redistributing
the binaries. Release assets contain no user samples, recordings, sample-backed
projects, or private machine configuration.
