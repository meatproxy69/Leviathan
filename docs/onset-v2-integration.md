# Optional experimental onset v2 (2026-10-24)

**v1 remains the default.** V2 is an explicit experimental, review-only choice,
not a better detector for all sources. Candidates are not cuts; neither module
owns or changes manual markers. Confidence is a weighted heuristic, not a
probability, drum label, beat grid or BPM estimate. Source and target tempo are
not inferred by this service. No external datasets, models or neural processing.

## Version and application policy

`onset_v2::Config::preset` preserves the judged phase-on v2.0 preset values so
historical comparisons remain reproducible. The v2.1 algorithm makes one
contract fix: phase cannot authorize a candidate without positive enabled
flux, energy or fast-attack evidence. The judge found that phase-only custom
threshold 0.1/MAD 0.5 could previously trigger; a regression covers this.

`onset_service::experimental_config(preset)` is the explicit application policy
`experimental-phase-off-v1`: same preset values, but `enable_phase=false`.
The complete effective config and its SHA-256 are returned; this is not silently
presented as the original judged full detector. A phase-off tradeoff is not a
universal quality fix. FFT phase evidence still exists in raw cached curves;
the picker does not vote with it under this policy. Dependencies are exact
RealFFT 3.5.0 (MIT) / RustFFT 6.4.1 (MIT OR Apache-2.0); no custom FFT.

## Shared control-worker API

- `prepare_verified(asset, authorized_path, config) -> Prepared`: strict bounded
  production importer verifies encoded SHA-256 and native metadata. Analysis
  excludes the decoder interpolation guard. Returns `Arc<CachedSource>` and
  `SourceAnalysis`; never call on audio or UI paint callbacks.
- `repick(cache, expected_asset, config) -> SourceAnalysis`: no file I/O, decode
  or FFT. Raw weak novelty is retained. Threshold/MAD/gaps/ablations/refinement
  can change; spectral fields (including minimum RMS) require new extraction.
- `CachedSource` fields are private; not deserializable. SHA-256 extraction key
  binds algorithm, encoded identity, native metadata, stereo mode, bands, hop,
  minimum RMS and frequency max-filter. Full config SHA binds each result.
  Internal PCM FNV is accidental-mismatch protection, not asset authority.
- `curve_page(start, limit)` has a hard 2048-point cap, exact source-frame points
  and raw per-band/fast evidence. Raw curves are not normalized confidence.
  Normalized envelopes can be requested via config but are potentially large;
  frontends must bound payloads rather than serialize all of them by default.
- `analysis.detection.candidates[].peak_frame` is a coarse detector position;
  `analysis.suggestions[].attack_frame` is a separate refinement estimate.
  Both include evidence/config provenance. Refinement is optional and can worsen
  a boundary. No client-supplied Detection is accepted by this service.

A cache means a **verified immutable PCM snapshot**, not a claim that the file
is unchanged now. UI/MCP must reauthorize the current resolved path, call
`sample_asset::verify_identity` on their worker before reuse, and reject stale
source/document/revision/config completions. Extraction keys are content keys,
not permission or session tokens. Wrapper tests must establish these guards.

Core finite/mono/stereo/native-rate, duration, source-length, FFT, work, memory,
and output caps remain enforced. Retained PCM + raw cache has a separate 96 MiB
cap; raw cache is shared by Arc. Callers retain at most one cache per document
and one tracked worker; canceled workers remain tracked until they finish.
Replacing a cache must not create an unbounded list. Work estimates are bounded
proxies, not runtime guarantees. Marker conversion must independently honor the
257-boundary limit and never silently truncate proposals or overwrite locks.

## Required counterexample disclosures

These are deterministic authored development/diagnostic signals, not held-out
or generalized accuracy. No private audio or derived private plots/annotations
are included in tests or release files.

| Signal | v1 proposals | Phase-on v2 | Explicit phase-off v2 |
|---|---:|---:|---:|
| 2 s, 440 Hz, ±0.8 semitone 6 Hz vibrato | 1 | GhostHunter 39 | GhostHunter 23 |
| 2 s linear 100–6100 Hz chirp | 1 | GhostHunter 11 | GhostHunter 5 |
| 440+447 Hz beating tones | 15 | Balanced 31 / GhostHunter 52 | Balanced 17 / GhostHunter 50 |

Continuous pitch/beating can create false proposals. Alternating 0.8/0.15 pulses
11 ms apart retain only 4 of 8 at all presets; GhostHunter also misses the four
0.05 weak pulses at 20 ms. Phase-off does not fix these weak-hit misses.
Equal-strength 11 ms success therefore does not establish quiet-ghost recovery.

Across the five original authored fixtures at three rates (90 known events),
full GhostHunter gave TP89/FP5/FN1 (F1 .967), phase-off TP86/FP0/FN4 (F1 .977).
The higher specificity loses recall. Full Balanced TP64/FP0/FN26 versus v1
TP58/FP0/FN32 does not establish a general improvement. All 225 original
preset/ablation frame-output rows are kept in the numerical golden regression.
The exact one-to-one matcher and procedural fixtures live in test support.
These are developed-on fixtures, not an external benchmark.

Boundary refinement is not always better: one original quiet-under-tone case
at 44.1 kHz worsened matched-event MAE from .669 ms peaks to 1.224 ms attacks.
Keep both estimates and manual control. Counts alone establish no accuracy on
unannotated recordings. No listening or drum classification is claimed.
