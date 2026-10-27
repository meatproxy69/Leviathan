# Onset v2: research and source attribution (2026-10-24)

Historical staged research below; see onset-v2-integration.md for optional live
integration policy and subsequently discovered limitations. Not a default upgrade,
tempo estimator or instrument classifier.
No audio-device access, private audio redistribution, new runtime frameworks,
Python package installation, optional framework build, or model install is involved.
Only the explicitly approved FFT crates were fetched from the official registry.

## Official sources inspected

- [librosa onset implementation](https://github.com/librosa/librosa/blob/main/librosa/onset.py),
  [multi-band flux documentation, version 0.11.0](https://librosa.org/doc/0.11.0/generated/librosa.onset.onset_strength_multi.html),
  [ISC license](https://github.com/librosa/librosa/blob/main/LICENSE.md).
  The implementation half-rectifies lagged log-spectrum differences, optionally
  suppresses frequency-local variation with a maximum-filtered reference, and
  aggregates configured frequency bands. Its separate onset-backtrack function
  uses preceding energy minima. These are useful architecture references, not
  evidence that this prototype matches librosa's thresholds or accuracy.
- [aubio descriptor implementation](https://github.com/aubio/aubio/blob/master/src/spectral/specdesc.c),
  [onset pipeline](https://github.com/aubio/aubio/blob/master/src/onset/onset.c),
  [descriptor documentation](https://aubio.org/doc/latest/specdesc_8h.html),
  [GPL version 3 license](https://github.com/aubio/aubio/blob/master/COPYING).
  Descriptors include energy, HFC, complex-domain distance, phase deviation and
  positive spectral flux. Complex prediction extrapolates two prior phases;
  distance measures mismatch between the predicted and observed complex bins.
  Descriptor generation and final peak/spacing decisions are distinct concerns.
- [Essentia onset implementation](https://github.com/MTG/essentia/blob/master/src/algorithms/rhythm/onsetdetection.cpp),
  [official detector documentation](https://essentia.upf.edu/reference/streaming_OnsetDetection.html),
  [AGPL version 3 license](https://github.com/MTG/essentia/blob/master/COPYING.txt).
  It exposes energy/RMS, spectral flux and complex/phase detection functions,
  recommending a subsequent onset decision stage. Its documentation warns that
  simplified phase-based detection may over-detect percussion. Accordingly,
  phase here is supporting evidence and cannot independently trigger a marker.

These mutable branch pages were inspected on the date above; no source snapshot
was installed or vendored. This is an original Rust implementation of general
DSP equations with attribution, NOT translated library source, a linked library,
or a benchmark of those three packages. Exact upstream parity is not asserted.
Existing project dependencies serde/serde_json/hound are reused. The user explicitly
required proven FFT crates: this harness pins RealFFT 3.5.0 (MIT) and RustFFT 6.4.1
(MIT OR Apache-2.0), fetched from crates.io; no custom FFT is used. Band/evidence/
fusion/refinement code is authored here.

## Design

1. Borrow immutable native-rate interleaved PCM. Analyze channels separately;
   combine nonnegative evidence, never sum antiphase channels before analysis.
2. Default frequency ranges: low 20–180 Hz, mid 180–2000 Hz, high 2000–16000 Hz.
   These are acoustic bands, never kick/snare/hat labels. Upper bounds clamp to
   Nyquist with effective limits recorded. Long/medium/short Hann windows use the experimental reference sizes
   2048/1024/512 at 44.1 kHz, with hop256/128/64. Windows scale by source rate then
   round to the nearest power of two; the fast hop scales and rounds to the
   nearest source frame, with other hops integer multiples of it. No claim that a short FFT resolves
   the low end precisely; each effective size and bin count is reported.
3. Compute positive log spectral flux, positive band-energy change, magnitude-
   weighted complex phase-prediction residual, and separate short-window full-
   band attack change. Locally robust normalization uses median/MAD statistics.
4. Fuse auditable per-band/per-detector evidence into heuristic confidence.
   Phase alone is insufficient. Stable peak/spacing selection operates in
   integer source frames. Confidence is NOT calibrated event probability.
5. Keep peak_frame, attack_frame, raw strength, weighted heuristic confidence,
   per-detector votes and per-band evidence distinct. No probabilistic calibration
   is claimed. An immutable unthresholded novelty cache supports sensitivity/
   preset/ablation changes without rerunning FFT; it retains weak evidence.
   Keep the detected candidate and refined source boundary separately. Bounded
   waveform backtracking/refinement can be disabled and records its search
   bounds and displacement. Refinement does not invent detection confidence.
6. No manual-marker mutation API exists. Suggestions remain proposals requiring
   explicit acceptance through the host's existing revision/undo authority.
7. Deterministic work and memory estimates are checked before FFT allocation;
   capped input values, duration, FFT size, hop, detector count and output size.

## Evaluation policy

The copied v1 module and tests remain unchanged for paired baseline runs.
Synthetic annotations are exact construction timestamps with one-to-one matching
at declared tolerances; count, precision, recall and timing errors are reported.
A detector producing more points is not inherently better. Parameter sweeps and
ablation comparisons expose false positives and missed quiet attacks.

The two supplied WAVs are private, unannotated sources. Filename numbers are not
verified tempo. Any visual waveform audit is provisional, explicitly uncertain,
and not a precision/recall ground truth or listening test. No audio bytes or
extracted playable slices are copied into release artifacts.


## Additional requested leads, source-verified (DSP-only scope)

- [FluCoMa Rust status](https://github.com/emuell/flucoma-rs/blob/master/STATUS.md),
  [OnsetSlice wrapper](https://github.com/emuell/flucoma-rs/blob/master/src/onset_seg.rs),
  [AmpSlice wrapper](https://github.com/emuell/flucoma-rs/blob/master/src/amp_seg.rs),
  [TransientSlice wrapper](https://github.com/emuell/flucoma-rs/blob/master/src/transient_seg.rs),
  [build script](https://github.com/emuell/flucoma-rs/blob/master/flucoma-sys/build.rs),
  [BSD3 license](https://github.com/emuell/flucoma-rs/blob/master/LICENSE).
  Actual wrappers exist for spectral onsets, dual fast/slow amplitude envelopes,
  AR-model transient segmentation and HPSS; their presence is not a measured
  improvement here. The build invokes CMake and a C++ compilation/link path;
  README labels the bindings work-in-progress. Good OPTIONAL future comparison,
  not a lighter native default. Nothing installed or executed from this project.
- [SuperFlux reference source](https://github.com/CPJKU/SuperFlux/blob/master/SuperFlux.py).
  Actual source has a two-clause BSD-style header, a 200 fps default, frequency-
  maximum-filtered positive differences, and a log-frequency filterbank. Our
  three coarse bands plus linear-bin maximum filter are INSPIRED by this method,
  not a complete SuperFlux implementation.200 fps yields 5 ms feature spacing, not
  sample-accurate slicing; separate refinement remains necessary.
- [microdsp spectral-flux code](https://github.com/perarnia/microdsp/blob/main/src/sfnov/spectral_flux.rs),
  [MIT license](https://github.com/perarnia/microdsp/blob/main/LICENSE).
  The old stuffmatic URL redirects to perarnia. Source computes compressed power
  differences, keeps positive deltas and averages novelty. README's no_std claim
  still requires alloc; no_std does NOT mean allocation-free. Not installed.
- [playr onset code](https://github.com/shakfu/playr/blob/main/crates/playr-core/src/samples.rs),
  [source-frame cue writer](https://github.com/shakfu/playr/blob/main/crates/playr-core/src/sliced.rs),
  [MIT license](https://github.com/shakfu/playr/blob/main/LICENSE).
  Verified actual `onsets()` implementation: 5 ms RMS windows, half-window hops,
  dB-energy rises, a local-mean threshold, a fixed 50 ms gap, and backward search
  for the smallest absolute sample in the previous window. Source frame 0 is
  always inserted. It caches decoded mono audio for re-picking, not the novelty
  curves. Its channel-average mono conversion can cancel antiphase stereo; our
  Mid/Side or independent-channel evidence avoids copying that behavior. Its
  fixed gap is unsuitable for an 11 ms dense-event target. Useful structural
  reference, not an adopted implementation or accuracy benchmark.
- Essentia's actual first-frame zeroing is in the melflux and RMS branches. It is
  not a blanket fix applied to every descriptor, nor evidence that our separate
  v1 frame16 suggestion has the same cause. v2 initializes spectral history
  explicitly and tests startup silence, DC, noise and real leading attacks.

Per the latest user restriction, no further neural research, model/weight
integration or external-dataset work is undertaken. Previously inspected
licenses exposed a distinct noncommercial model/data license in madmom; those
files are not fetched/bundled. Optional future separation means conventional DSP
only, never implicit inference from a source-code license to model rights.

## Implemented subset / limitations

The bounded native prototype implements spectral/energy/complex evidence, fast
attack, local median+MAD with a mean-deviation safeguard when rectification makes
both median and MAD zero, weighted fusion, dense-gap checking and separate source
boundary refinement. Three presets are experimental engineering settings, not
calibrated accuracy claims. Optional HFC, FluCoMa votes, HPSS and rhythm hypotheses
are NOT implemented. No manual marker or event model is replaced or duplicated.
