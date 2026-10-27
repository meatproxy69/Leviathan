# LEVIATHAN

LEVIATHAN is a Rust tracker for writing patterns, arranging songs, and shaping samples. The current source combines the public Alpha 2 work with the recovered October 25 sampler and break-workflow source.

## Get started with samples

1. Open **Samples** and choose **Browse…** to select a WAV or FLAC file. LEVIATHAN loads a preview and displays its waveform.
2. Choose **Regions**, turn on **Edit mode**, set the start and end, and apply the region. You can map it to an instrument without changing the original audio file.
3. Use **Break workflow** to review suggested markers, edit boundaries, create regions and pads, or compile a break into editable sequence data. Tempo and marker suggestions need review.
4. Return to **Song** to enter notes. A new project starts with 24 tracks and 24 instruments; more can be added.

The recovered sampler also includes region playback settings, looping, an envelope, pad and step editors, track mix controls, and sample peak meters. Break reconstruction supports repitching and slicing. Independent pitch-preserving time-stretch is not implemented in this source.

## Build and run

Install Rust 1.92 or newer and a working native compiler. From the repository:

```sh
cargo build --locked --features desktop-audio --bin leviathan
cargo test --locked --features desktop-audio
```

On Windows, run `target\debug\leviathan.exe`. On Linux, run `target/debug/leviathan`; the build requires ALSA and desktop/graphics development libraries. The Linux file picker uses an XDG desktop portal. Pass `--project PATH` to open a project, or use `--help` for command-line options. The integration was built on Windows; Linux execution remains to be verified.

The manual GitHub Actions workflow builds audio-enabled Windows and Linux artifacts. It does not publish a release automatically.

## Existing releases

The [Alpha 2 release](https://github.com/meatproxy69/Leviathan/releases/tag/v0.1.0-alpha.2) provides existing Windows and Linux downloads. Those packages predate this recovered-source integration; build the current source to try these changes. No samples are included.

## Project compatibility

Keep a backup of existing projects. The recovered source and Alpha 2 both used version 1 project files but represented some musical features differently. The importer preserves supported common data, section markers, and named generation recipes; some Alpha 2 instrument-owned phrases and step effects are rejected rather than silently changing their playback. Some older unmarked effect-only projects are ambiguous and also require further migration work. New ambiguous effect-bearing files include a dialect marker, which older builds may not recognize.

Projects reference audio files rather than embedding them, so keep your sample files available when moving or reopening a project.

## More information

- [Integration notes](docs/RECOVERED-INTEGRATION.md) record the recovered source and compatibility limits.
- [Recovered feature status](FEATURE-STATUS.md) describes accepted capabilities and remaining work.
- [MCP interface](MCP.md) documents the optional headless integration.
- [Release notes](RELEASE-NOTES.md) describe earlier releases.
- The source is licensed under [AGPL-3.0-only](LICENSE); see [third-party notices](THIRD-PARTY-NOTICES.md) and [licenses](licenses/).
