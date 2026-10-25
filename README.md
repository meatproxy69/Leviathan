<div align="center">

# LEVIATHAN

### Agent-first music creation. No MCP needed.

A native Rust tracker and sequencer for editable patterns, phrases, arrangement, groove, and seeded generation.

[![Alpha 1](https://img.shields.io/badge/release-Alpha_1-66ddcc)](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.1)
[![Linux x86_64](https://img.shields.io/badge/binary-Linux_x86__64-82aaff)](#download-alpha-1)
[![AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-aaaaaa)](LICENSE)

**[Download Alpha 1](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.1)** · **[Install](#install-on-linux)** · **[Try a demo](#try-a-song-in-three-commands)** · **[Agent workflow](#agent-first-no-mcp-required)**

![LEVIATHAN Alpha 1 tracker showing editable notes and the song order](docs/images/tracker.png)

*Real Alpha 1 screenshots with locally generated synthetic tones.*

</div>

## Make patterns. Shape a song.

Write notes directly in the tracker, build instrument phrases, and arrange shared
patterns into sections. Preview groove and seeded generation, then commit the
results as ordinary editable cells. Play the song in the native editor or render
it from the command line.

| Workspace | What you can do |
| --- | --- |
| **Tracker** | Enter notes, velocity, pan, delay, step effects, and phrase triggers. |
| **Phrases** | Build instrument-owned, one-shot polyphonic phrases. |
| **Arrange** | Repeat, clone, alias, mute occurrences, and create section variations. |
| **Transform** | Preview transpose, velocity/pan edits, row reversal, and groove. |
| **Generate** | Preview seeded Euclidean/probability patterns and save recipes. |
| **Audio / Export** | Assign your samples and export WAV audio plus an event manifest. |

<details>
<summary><strong>See arrangement and generation screenshots</strong></summary>

### Arrange sections and shared patterns

![Arrangement workspace with Verse, Bridge, a return variation, aliases, and an occurrence mute](docs/images/arrangement.png)

### Preview generation before committing

![Generation workspace showing a seeded sequence and the affected cells](docs/images/generation.png)

</details>

## Download Alpha 1

**Version `v0.1.0-alpha.1` · Early alpha · Keep backups of your projects.**

| Download | Use it for |
| --- | --- |
| **[Linux x86_64 binary (.tar.gz)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.1/leviathan-v0.1.0-alpha.1-linux-x86_64.tar.gz)** | Run the native editor and bundled CLI tools. |
| [Source (.tar.gz)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.1/leviathan-v0.1.0-alpha.1-source.tar.gz) · [Source (.zip)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.1/leviathan-v0.1.0-alpha.1-source.zip) | Build Alpha 1 yourself; tests and Cargo.lock included. |
| [SHA256SUMS](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.1/SHA256SUMS) | Verify your download. |

**Linux binary requirements:** x86_64 GNU/Linux, **glibc 2.43 or newer**, ALSA,
an OpenGL/EGL-capable driver, and a Wayland/X11 desktop. Check glibc with
`ldd --version`. Older distributions should [build from source](#build-from-source).
Windows/macOS binaries are not provided or verified in this alpha.

**No samples or recordings are bundled.** Bring your own audio, or generate the
optional synthetic demo locally using the commands below.

## Install on Linux

Download and extract into a writable directory. No system installation is required.
Run these commands in a new directory:

```sh
curl -fLO https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.1/leviathan-v0.1.0-alpha.1-linux-x86_64.tar.gz
curl -fLO https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.1/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS
# Continue only when the Linux archive reports OK.
tar -xzf leviathan-v0.1.0-alpha.1-linux-x86_64.tar.gz
cd leviathan-v0.1.0-alpha.1-linux-x86_64
./run-leviathan.sh
```

You can also download the two files with your browser and extract the archive
with your file manager. Open an existing project with:

```sh
./run-leviathan.sh --project /absolute/path/to/song.leviathan.json
```

Keep `LICENSE`, `THIRD-PARTY-NOTICES.md`, `licenses/`, and the bundled documentation
with redistributed binaries. The application does not configure accounts,
audio-device settings, or system services.

## Try a song in three commands

From the extracted package directory:

```sh
./bin/song_demo my-first-song --generate
./bin/render_song my-first-song/first-current.leviathan.json my-first-song/listen.wav
./bin/leviathan --project my-first-song/first-current.leviathan.json
```

The first command creates synthetic kick, hat, bass, and lead sounds **on your
machine**, plus an editable project and reference render. The second renders a
fresh WAV; the third opens the project in the editor. Press **Play song** to listen.
Use a new demo directory and new output filename each time; existing destinations
are refused. No personal or commercial sample library is downloaded or distributed.

## Agent-first: no MCP required

LEVIATHAN is **agent-first**: local coding agents can work with saved
`.leviathan.json` projects and invoke the included command-line tools directly.
**No MCP server, MCP plugin, or MCP configuration is needed to use LEVIATHAN.**
The native editor works on the same project files.

An agent can generate a reproducible example with `song_demo`, inspect and edit
its project JSON, render it with `render_song`, and open it for human review with
`leviathan --project`. Projects are validated on load; keep the sample references
valid and render to a new output path. Seeded generation commits notes into cells,
so playback does not rerun the generator.

| Command in the extracted package | Purpose |
| --- | --- |
| `./bin/leviathan --help` | Editor launch options. |
| `./bin/leviathan --project PROJECT.json` | Open a saved project. |
| `./bin/render_song PROJECT.json NEW_OUTPUT.wav` | Render a project without modifying it. |
| `./bin/song_demo NEW_DIRECTORY --generate` | Create a synthetic example with committed generation. |
| `./bin/transport_probe PROJECT.json` | Run bounded playback/seek/loop/stop diagnostics. |

`song_demo` also accepts `--effects`, `--phrases`, `--arrangement`, or `--groove`.
The package retains `aqua-pattern-core`, a legacy fixed-motif CLI independent of
the editable project. Alpha 1 provides files and commands for agent workflows;
it does not include an embedded AI agent or agent orchestration service.

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
