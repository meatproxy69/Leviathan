# LEVIATHAN Alpha 1 — v0.1.0-alpha.1

**Agent-first music creation. No MCP needed.**

The first public alpha of LEVIATHAN: a native Rust tracker and sequencer with
editable patterns, instrument phrases, arrangement, groove, and seeded generation.

Local agents can work directly with `.leviathan.json` projects and the bundled
CLI tools. **No MCP server, plugin, or configuration is required.** Open the same
projects in the native editor for hands-on editing and review. Alpha 1 provides
files and commands for agent workflows; it does not include an embedded AI agent.

![LEVIATHAN Alpha 1 tracker — editable notes and song order](https://raw.githubusercontent.com/meatproxy69/Leviathan/e8cf2dbd60efcf27df567bfc5c1eafe6d1a377cd/docs/images/tracker.png)

*Actual Alpha 1 screenshots using locally generated synthetic tones. No samples
or recordings are included in the release.*

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

## Build from source

Extract a source archive above and open the directory containing `Cargo.toml`.
Use Rust/Cargo **1.99.1 or newer** (1.99.1 verified), a C toolchain, pkg-config,
ALSA development files, libxkbcommon, and suitable Linux desktop/graphics libraries.
Network access is required for the first dependency fetch.

```sh
cargo fetch --locked
cargo build --release --locked --all-features --bins
./target/release/leviathan
```

Dependency versions and Git revisions are pinned in `Cargo.lock`. See the
[README](https://github.com/meatproxy69/Leviathan#readme) for the editing workflow,
CLI reference, validation commands, and supported scope.

## What's in Alpha 1

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

## Arrange and generate

**Arrange** — sections, shared aliases, occurrence mutes, and independent variations.

![LEVIATHAN arrangement workspace](https://raw.githubusercontent.com/meatproxy69/Leviathan/e8cf2dbd60efcf27df567bfc5c1eafe6d1a377cd/docs/images/arrangement.png)

**Generate** — preview seeded note/chord sequences, then commit editable cells and a saved recipe.

![LEVIATHAN seeded generation preview](https://raw.githubusercontent.com/meatproxy69/Leviathan/e8cf2dbd60efcf27df567bfc5c1eafe6d1a377cd/docs/images/generation.png)

## Alpha status and license

This is a prerelease; keep project backups. Samples remain external references.
Playback uses a start snapshot: restart playback to hear edits. No MIDI I/O,
VST/CLAP hosting, recording, automation, or XRNS compatibility is included.
See the [validation notes](https://github.com/meatproxy69/Leviathan/blob/main/docs/VALIDATION.md)
for tested behavior and limits.

AGPL-3.0-only. Binaries include dependency notices and license texts; retain them
when redistributing. Release assets contain no user samples, recordings,
sample-backed projects, or private machine configuration.
