<div align="center">

# LEVIATHAN

### Agent-first music creation. No MCP needed.

A native Rust tracker and sampler for editable patterns, waveform regions, break slicing, phrases, and seeded generation.

[![Alpha 2](https://img.shields.io/badge/release-Alpha_2-66ddcc)](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2)
[![Windows x86_64](https://img.shields.io/badge/Windows-x86__64-0078D4)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-windows-x86_64.zip)
[![Linux x86_64](https://img.shields.io/badge/Linux-x86__64-82aaff)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz)
[![AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-aaaaaa)](LICENSE)

**[Download for Windows](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-windows-x86_64.zip)** · **[Download for Linux](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz)** · **[Build from source](#build-from-source)** · **[Try a demo](#try-a-song-in-three-commands)** · **[Agent workflow](#agent-first-no-mcp-required)**

![LEVIATHAN Alpha 1 tracker showing editable notes and the song order](docs/images/tracker.png)

*Real Alpha 1 screenshots with locally generated synthetic tones.*

</div>

## Make patterns. Shape a song.

Write notes directly in the tracker, shape samples in the waveform editor, and
turn breaks into editable patterns. Review slice markers, map regions to pads,
and preview groove or seeded generation before committing ordinary notes.
Start with 24 tracks and 24 instruments, then add more as needed.

| Workspace | What you can do |
| --- | --- |
| **Tracker** | Enter notes, velocity, pan, delay, step commands, and phrase triggers. |
| **Samples / Regions** | Browse WAV/FLAC files, inspect waveforms, trim regions, and set playback, loop, and envelope controls. |
| **Break workflow** | Review markers and tempo suggestions, create slice regions or pad banks, and compile editable sequences. |
| **Pads / Steps** | Map samples to pads and edit drum patterns as ordinary tracker cells. |
| **Phrases** | Define reusable polyphonic phrases and explicit tracker triggers. |
| **Arrange** | Repeat, clone, alias blocks, and mute tracks in individual song positions. |
| **Transform / Generate** | Preview groove, scoped edits, and seeded patterns before committing them. |
| **Mix / Export** | Set track gain, balance, mute/solo; inspect peak meters and render WAV audio plus an event manifest. |

<details>
<summary><strong>See arrangement and generation screenshots</strong></summary>

*These screenshots show the earlier Alpha interface.*

### Arrange sections and shared patterns

![Arrangement workspace with Verse, Bridge, a return variation, aliases, and an occurrence mute](docs/images/arrangement.png)

### Preview generation before committing

![Generation workspace showing a seeded sequence and the affected cells](docs/images/generation.png)

</details>

## Alpha 2 downloads

**Version `v0.1.0-alpha.2` · Early alpha · Keep backups of your projects.**

The published packages predate the recovered sampler and break-tool integration.
[Build the current source](#build-from-source) to try those additions.
The Windows and Linux packages are available together on the
[Alpha 2 release page](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2).

| Download | Package |
| --- | --- |
| [Windows x86_64 (.zip)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-windows-x86_64.zip) | `leviathan-v0.1.0-alpha.2-windows-x86_64.zip` |
| [Linux x86_64 (.tar.gz)](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz) | `leviathan-v0.1.0-alpha.2-linux-x86_64.tar.gz` |

Both packages place the command-line executables under `bin/`. The Linux package
also includes `run-leviathan.sh` as its editor launcher. Alpha 1 remains the
earlier Linux-only release; see its [historical release page](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.1).

**No samples or recordings are bundled.** Bring your own audio, or generate the
optional synthetic demo locally using the commands below.

### Run on Windows

Download the Windows x86_64 ZIP above and extract it to a folder. Open the
extracted `bin` folder and run `leviathan.exe`. No separate installation is
required. To open an existing project from PowerShell, run:

```powershell
.\bin\leviathan.exe --project "C:\path\to\song.leviathan.json"
```

The [SHA256SUMS](https://github.com/meatproxy69/Leviathan/releases/download/v0.1.0-alpha.2/SHA256SUMS)
file contains checksums for both platform archives. Windows builds and automated
tests passed; live audio playback has not yet been verified.

## Try a song in three commands

From the extracted Linux package directory:

```sh
./bin/song_demo my-first-song
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
.\bin\song_demo.exe my-first-song
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
| `./bin/song_demo NEW_DIRECTORY` | Create a synthetic example project and reference renders. |

An optional headless MCP interface is also available in the source build; see
[MCP.md](MCP.md). The editor and command-line workflow work without MCP.

## Make a first song

1. Click **New** for 24 empty tracks and 24 unassigned instruments.
2. Open **Samples**, choose **Browse…**, and select a WAV or FLAC file. Its waveform
   appears in **Regions**.
3. Enable **Edit mode**, set the region's start and end, choose an instrument, and
   apply it. The original audio file stays intact. Expand **Sampler playback**
   for the region's playback, loop, and envelope settings.
4. For a break, open **Break workflow**. Review the markers and source tempo, then
   create regions, a pad bank, or an editable sequence. Suggestions need review.
5. Return to **Song**, enable **Edit mode**, and enter notes. Z–M enter notes,
   arrows/Tab navigate, backtick enters OFF, Delete clears a field, and
   Shift+Delete clears a cell. Use **Pads** or **Steps** for drum editing.
6. Set BPM in the transport bar and press **Play**. Playback uses its start
   snapshot; restart to hear edits. Turn **Follow** off to return to editing.
7. Save your project or expand **Song audio · instruments & offline export** to
   render a new WAV and event manifest. Undo/Redo is available for edits.

Samples remain external references; keep them available when reopening or moving
a project. Break reconstruction supports repitching and slicing. Independent
pitch-preserving time-stretch is not implemented in this source.

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
cargo build --release --locked --features desktop-audio --bin leviathan
cargo test --locked --features desktop-audio
cargo test --locked --no-default-features
cargo fmt --all -- --check
```

The desktop executable is `target\release\leviathan.exe`; command-line tools
can be built separately. The optional `realtime-probe` feature enables phonic’s
ASIO backend and requires the ASIO SDK and libclang; the desktop app uses CPAL
directly and does not require that feature. A prebuilt Windows Alpha 2 package
is available in the download section above.
When real-time playback runs, its per-user lease is stored under
`%LOCALAPPDATA%\Leviathan\transport.lock` (falling back to `%APPDATA%`, then the
system temporary directory). A stale lease is refused; remove it only after
confirming no LEVIATHAN transport is active.

### Linux

Linux builds additionally need a C toolchain, pkg-config, ALSA development files,
libxkbcommon, and suitable Linux desktop/graphics libraries. The native file
picker uses an XDG desktop portal.

```sh
cargo fetch --locked
cargo build --release --locked --features desktop-audio --bins
./target/release/leviathan
cargo test --locked --features desktop-audio
cargo test --locked --no-default-features
cargo fmt --all -- --check
```

For offline builds add `--offline` after populating the Cargo cache. Headless tests
and rendering do not require the optional desktop feature.

## Compatibility and validation

The current source includes the recovered October 25 sampler and break tools.
Windows build, native tracker smoke, and offline sample rendering have been
checked; Linux execution and physical device playback remain unverified for this
integration. See [integration notes](docs/RECOVERED-INTEGRATION.md) for the exact
source and test evidence, and [feature status](FEATURE-STATUS.md) for remaining work.

Keep backups of older projects. Alpha 2 and the recovered source used different
phrase ownership and effect semantics despite sharing a version number. The
importer retains supported common data, section markers, and named recipes;
some older phrase/effect projects still need migration. Projects that cannot be
converted safely are rejected with an error.

## License

AGPL-3.0-only; see [LICENSE](LICENSE).
