<div align="center">

# LEVIATHAN

### Agent-first music creation. No MCP needed.

A native Rust tracker and sequencer for editable patterns, phrases, arrangement, groove, and seeded generation.

[![Alpha 2](https://img.shields.io/badge/release-Alpha_2-66ddcc)](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2)
[![Windows and Linux builds](https://img.shields.io/badge/builds-Windows_%2B_Linux-82aaff)](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2)
[![AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-aaaaaa)](LICENSE)

**[Alpha 2 downloads](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2)** · **[Build from source](#build-from-source)** · **[Try a demo](#try-a-song-in-three-commands)** · **[Agent workflow](#agent-first-no-mcp-required)**

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

## Alpha 2 downloads

**Version `v0.1.0-alpha.2` · Early alpha · Keep backups of your projects.**
The Windows and Linux packages are intended to be published together on the
[Alpha 2 release page](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2).
This page and its asset links may not be available until the release is published.

| Download | Package |
| --- | --- |
| [Windows x86_64 (.zip)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-windows-x86_64.zip) | `leviathan-v0.1.0-alpha.2-windows-x86_64.zip` |
| [Linux x86_64 (.tar.gz)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz) | `leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz` |

Both packages place the command-line executables under `bin/`. The Linux package
also includes `run-leviathan.sh` as its editor launcher. Alpha 1 remains the
earlier Linux-only release; see its [historical release page](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.1).

**No samples or recordings are bundled.** Bring your own audio, or generate the
optional synthetic demo locally using the commands below.

## Try a song in three commands

From the extracted Linux package directory:

```sh
./bin/song_demo my-first-song --generate
./bin/render_song my-first-song/first-current.leviathan.json my-first-song/listen.wav
./run-leviathan.sh --project my-first-song/first-current.leviathan.json
```

The first command creates synthetic kick, hat, bass, and lead sounds **on your
machine**, plus an editable project and reference render. The second renders a
fresh WAV; the third starts the editor with the project. No claim about Linux
GUI validation or live audio is made here. Use a new demo directory and new
output filename each time; existing destinations are refused.

On Windows, run the corresponding commands from PowerShell in the extracted
package directory:

```powershell
.\bin\song_demo.exe my-first-song --generate
.\bin\render_song.exe my-first-song\first-current.leviathan.json my-first-song\listen.wav
.\bin\leviathan.exe --project my-first-song\first-current.leviathan.json
```

No personal or commercial sample library is downloaded or distributed.

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
the editable project. The release provides files and commands for agent workflows;
it does not include an embedded AI agent or agent orchestration service.

## Make a first song

1. Select **Tracker** and enable **Edit Mode**. **New** creates 24 empty tracks
   and 24 unassigned instruments. No sample files are embedded.
2. Select **Tracker**, enable **Edit Mode**, and choose an instrument in the
   **Instrument sample** row. Click **Browse / Load sample** to pick a local audio
   file. You can also enter a path or drop a file onto the path field, then click
   **Assign**. Repeat for other instruments; the assigned path is shown.
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
BPM is available in the transport bar in Edit Mode; LPB is stored in project content. Playback uses its start snapshot, so restart to hear tempo changes.

## Build from source

Use Rust/Cargo **1.92.0 or newer** (1.92.0 verified for the Windows desktop build). Dependency versions and
revisions are fixed in Cargo.lock. Network access is needed for the first fetch;
the pattrns Git revision is pinned explicitly.

### Windows

Build with the `x86_64-pc-windows-msvc` Rust target, Visual Studio 2022 Build
Tools with the **Desktop development with C++** workload, and a Windows 10/11
SDK. Install Rust's MSVC toolchain through rustup. No separate ALSA or
pkg-config packages are needed. The desktop backend uses the native Windows
window system and audio output through CPAL's Windows host.

In PowerShell, from the repository root:

```powershell
cargo fetch --locked
cargo build --release --locked --features desktop --bin leviathan
cargo test --locked --features desktop
cargo test --locked --no-default-features
cargo fmt --all --check
```

The desktop executable is `target\release\leviathan.exe`; command-line tools
can be built separately. The optional `realtime-probe` feature enables phonic’s
ASIO backend and requires the ASIO SDK and libclang; the desktop app uses CPAL
directly and does not require that feature. Alpha 1 did not include a Windows
binary. The Alpha 2 package link is above; its asset becomes available when the
release is published.
When real-time playback runs, its per-user lease is stored under
`%LOCALAPPDATA%\Leviathan\transport.lock` (falling back to `%APPDATA%`, then the
system temporary directory). A stale lease is refused; remove it only after
confirming no LEVIATHAN transport is active.

### Linux

Linux builds additionally need a C toolchain, pkg-config, ALSA development files,
libxkbcommon, and suitable Linux desktop/graphics libraries.

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

Only one live LEVIATHAN transport per user runtime is permitted. Linux stores
its lease at `$XDG_RUNTIME_DIR/leviathan/transport.lock`, falling back to
XDG_CACHE_HOME or HOME/.cache. Windows uses `%LOCALAPPDATA%\Leviathan`, falling
back to `%APPDATA%` or the system temporary directory. A stale lease is refused;
the error includes its path. Only remove it after confirming no LEVIATHAN
transport is active. The release does not embed a build-machine checkout path
for the lock or default exports.

## License

AGPL-3.0-only; see [LICENSE](LICENSE). The pinned engines are pattrns 0.9.3 at
`8505bfe09111d4123a202af5222466f55021db9f` and phonic 0.16.0. Dependency metadata,
upstream notices and license texts are included in DEPENDENCIES.tsv,
THIRD-PARTY-NOTICES.md and licenses/. No private samples or local machine settings
are part of this repository or the release assets.
