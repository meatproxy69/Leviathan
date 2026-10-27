# Recovered sampler integration

The main source combines Alpha 2's Windows support and 24-slot startup with the
accepted sample-peak meters source dated October 25, 2026. The later stems
checkpoint in the handoff was marked in review and was not used as the base.

Source archive: `leviathan-meters-v1-source-2026-10-25.zip`.
SHA-256: `28661b9e229c374f7d8e86662c150d9c0f6eb4c7479931c02b06557f2fb24215`.

The integration restores source registration, waveform and region editing,
region playback and audition, reviewed break markers and sequence compilation,
marker-to-pad banks, pad/step editing, tempo review, slice analysis, mix controls,
and peak meters. Native WAV/FLAC file selection is available from Samples.

Windows changes include platform-specific window-system dependencies, direct
CPAL playback without requiring the separate ASIO probe, stable atomic APIs,
and consistent safe DOS/UNC path spelling across project IO, sample assignment
and headless workspace checks. Long paths and special Windows path components
retain the namespace they require. Release CI uses `desktop-audio` on both
Windows and Linux.

The earlier Alpha 2-only implementation files are retained in
`legacy-alpha2-reference/` for migration work. The recovered format has different
phrase ownership and effect semantics; unsupported Alpha 2 projects fail
explicitly. Section markers and named generation recipes are retained as
validated metadata. Ambiguous newly saved effect-bearing documents include a
format discriminator. Older readers may reject that field.

Independent pitch-preserving time-stretch is absent from the recovered source.
Linux execution and physical audio-device playback have not been verified for
this integration. Historical cloud acceptance counts in FEATURE-STATUS.md refer
to the recovered checkpoint, not these modifications.

## Windows validation on October 27, 2026

- The full locked/offline `desktop-audio` run exercised 1,167 checks: 1,156
  passed initially; the 11 failing cases were corrected and passed in affected
  suite or individual reruns. Three added workspace-containment regressions
  passed separately. This records a full run plus focused reruns, rather than a
  single uninterrupted all-green run.
- The final main-tree `cargo build --locked --offline --features desktop-audio
  --bin leviathan` passed with Rust 1.92.0. The development executable's
  `--version` and native `--tracker-smoke` run passed; the latter verified a
  shared-pattern edit and captured the rendered tracker window.
- An external local test sample imported at 44,100 Hz stereo / 235,200 source
  frames, mapped through a registered region to an instrument, and rendered
  216,000 frames with nonzero audio and peak 0.403539. Analysis produced 1,024
  waveform bins and 57 in-bounds suggestions. The source was read only and is
  not included in the repository. This was offline output, not device playback.
- The dependency inventory matches all 467 locked packages; all 466 linked
  third-party notices resolve. The authored FLAC test fixture is explicitly
  exempted from the repository's sample-file ignore rule.
