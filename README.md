# LEVIATHAN — Alpha 1

A native Rust tracker/sequencer with editable patterns, instrument phrases,
arrangement aliases, committed groove and seeded pattrns generation.

**Version: 0.1.0-alpha.1.** Early alpha: keep backups and expect rough edges.
This release contains **no audio samples, recordings or sample-backed demo projects**.
Use your own audio files. No accounts, audio-device settings or system services
are configured by the application.

## Download and run

The release provides a Linux x86_64 GNU build and complete source archives.
The prebuilt Linux binary requires **glibc 2.43 or newer**, ALSA, an OpenGL/EGL-capable
driver and a Wayland/X11 desktop. Tested on a recent Arch-based Linux host with
Rust 1.99.1. Older Linux distributions should build from source; Windows/macOS
binaries are not provided or verified in this alpha.

Extract the Linux archive into a writable directory, then:

```sh
./run-leviathan.sh
# Or open your own saved project:
./run-leviathan.sh --project /path/to/song.leviathan.json
```

`SHA256SUMS` accompanies the release assets. Keep LICENSE, THIRD-PARTY-NOTICES.md,
licenses/ and the other bundled documentation with redistributed binaries.

## Make a first song

1. Select **Tracker** and enable **Edit Mode**. **New** creates four empty tracks
   and four unassigned instruments. No sample files are embedded.
2. Select **Audio / Export**, choose an instrument, enter the path of your own audio
   file and click **Assign sample**. Repeat as needed. The assigned path is shown.
3. Return to **Tracker**. Click a note field to focus it. Z–M enter notes, arrows/Tab
   navigate, backtick enters OFF, Delete clears a field and Shift+Delete clears a cell.
   Numeric entry uses MIDI notes/IDs and normalized velocity/pan/delay values.
4. Use **Arrange** to select positions/blocks, repeat or clone, set occurrence mutes,
   alias blocks, mark sections, repeat sections or create independent variations.
5. **Phrases** edits an instrument's one-shot phrase pool. A pitched tracker cell's
   Phr field explicitly launches a phrase owned by its selected instrument.
6. **Transform** previews scoped transpose, explicit velocity/pan edits, row reversal
   and groove committed into cell delays. **Generate** previews seeded Euclidean/
   probability note/chord generation and commits cells plus a named recipe.
7. Save to a `.leviathan.json` path. Existing projects require replacement confirmation.
   Use Undo/Redo for edits. Relative sample references resolve against the project.
8. Press **Play song**. Playback/export uses an immutable start snapshot; restart to
   hear edits. **Follow** is read-only, and turning it off preserves the edit cursor.
   **Audio / Export** writes a new WAV and event JSON; existing outputs are refused.

Generated cells are ordinary editable content. Samples stay external references;
keep them available when reopening a project. There is no automatic sample copying.
BPM/LPB are stored in project content; dedicated song-tempo controls are not yet exposed.

## Build from source

Use Rust/Cargo **1.99.1 or newer** (1.99.1 verified), a C toolchain, pkg-config,
ALSA development files, libxkbcommon, and suitable Linux desktop/graphics libraries.
Dependency versions/revisions are fixed in Cargo.lock. Network access is needed
for the first fetch; the pattrns Git revision is pinned explicitly.

```sh
cargo fetch --locked
cargo build --release --locked --all-features --bins
./target/release/leviathan
cargo test --locked --all-features
cargo test --locked --no-default-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

For offline builds add `--offline` after populating the Cargo cache. Headless tests
and rendering do not require the optional desktop feature.

## Included command-line tools

- `leviathan`: native editor; `--project PATH`, `--help`, `--version`.
- `render_song PROJECT.json NEW_OUTPUT.wav`: render your saved song without changing it.
- `song_demo NEW_DIRECTORY [--effects | --phrases | --arrangement | --groove | --generate]`:
  locally generate synthetic test tones and a reproducible editable example. No
  commercial/user sample library is distributed. Every destination must be new.
- `transport_probe PROJECT`: bounded diagnostic playback/seek/loop/stop check.
- `aqua-pattern-core`: legacy fixed-motif CLI retained for compatibility; independent
  of the editable song project. Source also includes diagnostic/example tools.

## Architecture and supported scope

All ten sequencer milestones are integrated: content model, tracker, persistence/
history, offline rendering, transport, step FX, instrument phrases, arrangement/
sections/aliases, scoped transformations/groove, and reproducible committed generation.
See [docs/MILESTONES.md](docs/MILESTONES.md) and [docs/VALIDATION.md](docs/VALIDATION.md).
The phonic sampler is shared by offline and incremental playback. Device callbacks
copy bounded PCM/access atomics; generation/DSP/file loading stays off the callback.

Alpha limits include stereo float32 48 kHz device support, fixed render tail,
cold seeks without note reconstruction, finite voice/event/sample-variant limits,
external samples and synchronous bounded project I/O. Instrument phrases are
one-shot/non-recursive; sample/phrase playback is not Renoise command emulation.
No XRNS compatibility, MIDI I/O, VST/CLAP hosting, recording, mixer/routing editor,
automation, discovery, or automatic time-stretch is claimed.

Only one live LEVIATHAN transport per user runtime is permitted. Its lease is
`$XDG_RUNTIME_DIR/leviathan/transport.lock`, falling back to XDG_CACHE_HOME or
HOME/.cache. A stale lease is refused; the error includes its path. Only remove
it after confirming no LEVIATHAN transport is active. Alpha 1 no longer embeds a
build-machine checkout path for the lock or default exports.

## License

AGPL-3.0-only; see [LICENSE](LICENSE). The pinned engines are pattrns 0.9.3 at
`8505bfe09111d4123a202af5222466f55021db9f` and phonic 0.16.0. Dependency metadata,
upstream notices and license texts are included in DEPENDENCIES.tsv,
THIRD-PARTY-NOTICES.md and licenses/. No private samples or local machine settings
are part of this repository or the release assets.
