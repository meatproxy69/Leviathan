# LEVIATHAN Alpha 1 — v0.1.0-alpha.1

First prerelease of the integrated native tracker/sequencer.

- Editable sparse song model, stable IDs, variable-length ordered patterns.
- Focused keyboard tracker, explicit Edit Mode, typed attributes, repeat/clone.
- Validated project save/open, replacement confirmation and bounded undo/redo.
- Shared offline/streaming sampler engine, snapshot Play/Stop/Loop/Follow/cold Seek.
- Retrigger, hard cut, sample offset, reverse and pitch glide.
- Instrument-owned one-shot polyphonic phrases with their own LPB/root/FX.
- Connected arrangement matrix, shared aliases, occurrence mutes and section variations.
- Previewed scoped groove/transpose/velocity/pan/row reversal.
- Seeded pattrns note/chord generation committed into editable cells with saved recipes.
- Consolidated workspaces and integrated regression coverage.
- Release portability fixes: user-runtime transport lock and working-directory export defaults.

## Assets

- Linux x86_64 GNU binaries, launcher, notices and documentation.
- Complete source ZIP and tar.gz, including tests and pinned dependency lockfile.
- SHA256SUMS for release-asset verification.

**No samples, recordings, sample-backed projects, caches, personal configuration,
private library paths or internal handoff bundles are included.** Bring your own
samples, or use the optional synthetic test-tone generator locally.

The Linux binary requires glibc 2.43+ and a compatible Linux desktop/audio/graphics
runtime; source is supplied for other systems. No Windows/macOS binary is verified.
This is an early alpha; retain backups. See README for supported behavior and limits.
