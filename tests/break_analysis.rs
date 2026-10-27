//! All fixtures below are procedural signals authored for these tests. No
//! third-party recordings, copyrighted breaks or external fixture downloads.
use aqua_pattern_core::break_analysis::*;
use std::time::Instant;

fn analyze(pcm: &[f32], channels: usize, rate: u32) -> TransientAnalysis {
    analyze_transients(pcm, channels, rate, &AnalysisConfig::default()).unwrap()
}

fn positions(result: &TransientAnalysis) -> Vec<u64> {
    result.suggestions.iter().map(|onset| onset.frame).collect()
}

fn noise(frames: usize, amplitude: f32) -> Vec<f32> {
    let mut state = 0x1234_5678u32;
    (0..frames)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32 * amplitude
        })
        .collect()
}

fn hit(pcm: &mut [f32], rate: u32, start: usize, amplitude: f32) {
    let duration = (rate / 25) as usize;
    for (offset, sample) in pcm.iter_mut().skip(start).take(duration).enumerate() {
        let t = offset as f64 / rate as f64;
        let envelope = (-t * 150.0).exp();
        *sample += (amplitude as f64
            * envelope
            * ((t * 1_400.0 * std::f64::consts::TAU).cos() * 0.7
                + (t * 3_100.0 * std::f64::consts::TAU).cos() * 0.3)) as f32;
    }
}

fn assert_near(actual: &[u64], expected: &[usize], tolerance: usize) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "actual {actual:?}; expected {expected:?}"
    );
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            actual.abs_diff(*expected as u64) <= tolerance as u64,
            "onset {actual}, expected {expected}, tolerance {tolerance}"
        );
    }
}

#[test]
fn exact_impulse_source_frames_at_multiple_rates_and_edges() {
    for rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
        let mut pcm = vec![0.0; rate as usize];
        let attacks = [
            0,
            rate as usize / 5 + 1,
            rate as usize / 2 + 17,
            pcm.len() - 1,
        ];
        for &frame in &attacks {
            pcm[frame] = 0.8;
        }
        let result = analyze(&pcm, 1, rate);
        assert_near(&positions(&result), &attacks, 0);
        assert_eq!(result.provenance.source_frames, pcm.len() as u64);
        assert_eq!(result.provenance.source_rate, rate);
    }
}

#[test]
fn noisy_and_quiet_attacks_have_bounded_timing_error_at_all_target_rates() {
    for rate in [44_100, 48_000, 96_000] {
        let mut pcm = noise(rate as usize, 0.002);
        // Leading silence avoids the intentional source-start edge suggestion.
        pcm[..(rate / 20) as usize].fill(0.0);
        let attacks = [
            rate as usize / 5 + 1,
            rate as usize / 2 + 17,
            rate as usize * 4 / 5,
        ];
        for (&frame, amplitude) in attacks.iter().zip([0.3, 0.025, 0.08]) {
            hit(&mut pcm, rate, frame, amplitude);
        }
        let result = analyze(&pcm, 1, rate);
        let found: Vec<_> = positions(&result)
            .into_iter()
            .filter(|frame| *frame > rate as u64 / 10)
            .collect();
        assert_near(&found, &attacks, (rate / 500) as usize);
        assert!(result
            .suggestions
            .iter()
            .all(|onset| onset.confidence.is_finite() && (0.0..=1.0).contains(&onset.confidence)));
    }
    let mut quiet = vec![0.0; 48_000];
    hit(&mut quiet, 48_000, 12_037, 0.0002);
    assert_near(&positions(&analyze(&quiet, 1, 48_000)), &[12_037], 96);
}

#[test]
fn stereo_antiphase_and_one_sided_audio_do_not_cancel() {
    let mut mono = vec![0.0; 48_000];
    for frame in [4_801, 19_237, 38_400] {
        hit(&mut mono, 48_000, frame, 0.6);
    }
    let antiphase: Vec<_> = mono.iter().flat_map(|value| [*value, -*value]).collect();
    let right_only: Vec<_> = mono.iter().flat_map(|value| [0.0, *value]).collect();
    let expected = positions(&analyze(&mono, 1, 48_000));
    assert_eq!(positions(&analyze(&antiphase, 2, 48_000)), expected);
    assert_eq!(positions(&analyze(&right_only, 2, 48_000)), expected);
}

#[test]
fn stationary_noise_and_sustained_tone_do_not_retrigger() {
    let pcm = noise(48_000 * 3, 0.05);
    let result = analyze(&pcm, 1, 48_000);
    assert!(
        result.suggestions.iter().all(|onset| onset.frame < 96),
        "{:?}",
        positions(&result)
    );
    let tone: Vec<_> = (0..48_000)
        .map(|frame| ((frame as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 0.3) as f32)
        .collect();
    let result = analyze(&tone, 1, 48_000);
    assert_eq!(result.suggestions.len(), 1, "{:?}", positions(&result));
    assert!(result.suggestions[0].frame < 96);
}

#[test]
fn silence_empty_short_and_incomplete_pcm_are_explicit() {
    assert!(analyze(&vec![0.0; 48_000], 1, 48_000)
        .suggestions
        .is_empty());
    assert!(analyze(&[0.0], 1, 48_000).suggestions.is_empty());
    assert_eq!(positions(&analyze(&[0.9], 1, 48_000)), vec![0]);
    assert_eq!(positions(&analyze(&[0.9, -0.9], 2, 48_000)), vec![0]);
    assert_eq!(
        analyze_transients(&[], 1, 48_000, &AnalysisConfig::default()),
        Err(AnalysisError::EmptyPcm)
    );
    assert_eq!(
        analyze_transients(&[0.0; 3], 2, 48_000, &AnalysisConfig::default()),
        Err(AnalysisError::IncompleteFrame)
    );
}

#[test]
fn sample_rate_channels_and_every_nonfinite_value_are_validated() {
    let config = AnalysisConfig::default();
    for channels in [0, 3, usize::MAX] {
        assert_eq!(
            analyze_transients(&[0.0], channels, 48_000, &config),
            Err(AnalysisError::InvalidChannels(channels))
        );
    }
    for rate in [0, MIN_SOURCE_RATE - 1, MAX_SOURCE_RATE + 1, u32::MAX] {
        assert_eq!(
            analyze_transients(&[0.0], 1, rate, &config),
            Err(AnalysisError::InvalidSampleRate(rate))
        );
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut pcm = vec![0.0; 1_000];
        pcm[999] = value;
        assert_eq!(
            analyze_transients(&pcm, 2, 48_000, &config),
            Err(AnalysisError::NonFiniteSample { sample_index: 999 })
        );
    }
}

#[test]
fn invalid_configuration_cannot_allocate_unbounded_work() {
    let mut invalid = Vec::new();
    for sensitivity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        invalid.push(AnalysisConfig {
            sensitivity,
            ..Default::default()
        });
    }
    for min_spacing_ms in [f32::NAN, f32::INFINITY, 0.0, 0.9, 1000.1] {
        invalid.push(AnalysisConfig {
            min_spacing_ms,
            ..Default::default()
        });
    }
    for minimum_rms in [f32::NAN, f32::NEG_INFINITY, -0.1, 1.1] {
        invalid.push(AnalysisConfig {
            minimum_rms,
            ..Default::default()
        });
    }
    for max_frames in [0, MAX_SAMPLE_VALUES as u64 + 1, u64::MAX] {
        invalid.push(AnalysisConfig {
            max_frames,
            ..Default::default()
        });
    }
    for max_suggestions in [0, MAX_SUGGESTIONS + 1, usize::MAX] {
        invalid.push(AnalysisConfig {
            max_suggestions,
            ..Default::default()
        });
    }
    invalid.push(AnalysisConfig {
        waveform_bins: MAX_WAVEFORM_BINS + 1,
        ..Default::default()
    });
    for config in invalid {
        assert!(matches!(
            analyze_transients(&[0.0], 1, 48_000, &config),
            Err(AnalysisError::InvalidConfig(_))
        ));
    }
}

#[test]
fn limits_refuse_oversized_input_and_output_without_truncation() {
    let config = AnalysisConfig {
        max_frames: 2,
        ..Default::default()
    };
    assert_eq!(
        analyze_transients(&[0.0; 3], 1, 48_000, &config),
        Err(AnalysisError::InputLimit)
    );
    let too_long = vec![0.0; (MIN_SOURCE_RATE as u64 * MAX_DURATION_SECONDS + 1) as usize];
    assert_eq!(
        analyze_transients(&too_long, 1, MIN_SOURCE_RATE, &AnalysisConfig::default()),
        Err(AnalysisError::InputLimit)
    );
    let too_large = vec![0.0; MAX_SAMPLE_VALUES + 2];
    assert_eq!(
        analyze_transients(&too_large, 2, 96_000, &AnalysisConfig::default()),
        Err(AnalysisError::InputLimit)
    );
    let mut pcm = vec![0.0; 48_000];
    for frame in [4_800, 14_400, 24_000] {
        pcm[frame] = 0.8;
    }
    let config = AnalysisConfig {
        max_suggestions: 2,
        ..Default::default()
    };
    assert_eq!(
        analyze_transients(&pcm, 1, 48_000, &config),
        Err(AnalysisError::SuggestionLimit { limit: 2 })
    );
}

#[test]
fn minimum_spacing_keeps_the_stronger_attack_and_rounds_up_source_frames() {
    let mut pcm = vec![0.0; 48_000];
    pcm[4_800] = 0.4;
    pcm[5_760] = 0.8;
    pcm[9_600] = 0.6;
    let config = AnalysisConfig {
        min_spacing_ms: 30.01,
        ..Default::default()
    };
    let result = analyze_transients(&pcm, 1, 48_000, &config).unwrap();
    assert_eq!(positions(&result), [5_760, 9_600]);
    assert_eq!(result.provenance.min_spacing_frames, 1_441);
    let config = AnalysisConfig {
        min_spacing_ms: 10.0,
        ..Default::default()
    };
    assert_eq!(
        positions(&analyze_transients(&pcm, 1, 48_000, &config).unwrap()),
        [4_800, 5_760, 9_600]
    );
}

#[test]
fn lower_absolute_floor_recovers_quiet_hits_without_changing_pcm() {
    let mut pcm = vec![0.0; 48_000];
    hit(&mut pcm, 48_000, 12_000, 0.0002);
    let before = pcm.clone();
    let high_floor = AnalysisConfig {
        minimum_rms: 0.01,
        ..Default::default()
    };
    assert!(analyze_transients(&pcm, 1, 48_000, &high_floor)
        .unwrap()
        .suggestions
        .is_empty());
    assert_eq!(analyze(&pcm, 1, 48_000).suggestions.len(), 1);
    assert_eq!(pcm, before);
}

#[test]
fn provenance_is_repeatable_and_changes_with_source_or_configuration() {
    let mut pcm = vec![0.0; 4_800];
    pcm[1_213] = 0.5;
    let first = analyze(&pcm, 1, 48_000);
    let second = analyze(&pcm, 1, 48_000);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    let encoded = serde_json::to_string(&first).unwrap();
    assert_eq!(
        serde_json::from_str::<TransientAnalysis>(&encoded).unwrap(),
        first
    );
    pcm[1_213] = 0.6;
    assert_ne!(
        analyze(&pcm, 1, 48_000).provenance.source_fingerprint,
        first.provenance.source_fingerprint
    );
    pcm[1_213] = 0.5;
    assert_ne!(
        analyze(&pcm, 1, 44_100).provenance.source_fingerprint,
        first.provenance.source_fingerprint
    );
    assert_ne!(
        analyze(&pcm, 2, 48_000).provenance.source_fingerprint,
        first.provenance.source_fingerprint
    );
    let config = AnalysisConfig {
        sensitivity: 0.9,
        ..Default::default()
    };
    let changed = analyze_transients(&pcm, 1, 48_000, &config).unwrap();
    assert_eq!(
        changed.provenance.source_fingerprint,
        first.provenance.source_fingerprint
    );
    assert_ne!(changed.provenance.config, first.provenance.config);
    assert_eq!(changed.provenance.algorithm, ALGORITHM);
}

#[test]
fn waveform_partitions_source_with_exact_per_channel_extrema() {
    let pcm = [0.1, -0.2, 0.9, -0.8, -0.5, 0.4, 0.7, -0.6, -0.3, 0.2];
    let config = AnalysisConfig {
        waveform_bins: 3,
        ..Default::default()
    };
    let result = analyze_transients(&pcm, 2, 48_000, &config).unwrap();
    assert_eq!(result.waveform.len(), 3);
    assert_eq!(
        (result.waveform[0].start_frame, result.waveform[0].end_frame),
        (0, 1)
    );
    assert_eq!(
        (result.waveform[1].start_frame, result.waveform[1].end_frame),
        (1, 3)
    );
    assert_eq!(
        (result.waveform[2].start_frame, result.waveform[2].end_frame),
        (3, 5)
    );
    assert_eq!(
        result.waveform[1].channels,
        [
            ChannelExtrema {
                min: -0.5,
                max: 0.9
            },
            ChannelExtrema {
                min: -0.8,
                max: 0.4
            }
        ]
    );
    let no_waveform = AnalysisConfig {
        waveform_bins: 0,
        ..Default::default()
    };
    assert!(analyze_transients(&pcm, 2, 48_000, &no_waveform)
        .unwrap()
        .waveform
        .is_empty());
    assert_eq!(analyze(&[0.2], 1, 48_000).waveform.len(), 1);
}

#[test]
fn large_finite_pcm_never_overflows_features_or_confidence() {
    let mut pcm = vec![0.0; 48_000];
    pcm[4_800] = f32::MAX;
    pcm[24_000] = -f32::MAX;
    let result = analyze(&pcm, 1, 48_000);
    assert_eq!(positions(&result), [4_800, 24_000]);
    assert!(result
        .suggestions
        .iter()
        .all(|onset| onset.confidence.is_finite()));
    assert!(serde_json::to_string(&result).is_ok());
}

#[test]
fn sixty_seconds_stereo_has_a_generous_offline_performance_guard() {
    let rate = 48_000;
    let mut mono = vec![0.0; rate * 60];
    for second in 0..60 {
        hit(&mut mono, rate as u32, second * rate + 123, 0.4);
    }
    let pcm: Vec<_> = mono.iter().flat_map(|value| [*value, -*value]).collect();
    let started = Instant::now();
    let result = analyze(&pcm, 2, rate as u32);
    let elapsed = started.elapsed();
    eprintln!(
        "60-second stereo transient analysis: {elapsed:?}, {} suggestions",
        result.suggestions.len()
    );
    assert_eq!(result.suggestions.len(), 60);
    // Offline regression guard, not an audio-callback latency guarantee.
    assert!(
        elapsed.as_secs_f64() < 15.0,
        "analysis exceeded the offline 15-second guard: {elapsed:?}"
    );
}

#[test]
fn sensitivity_controls_adaptive_threshold_on_a_weak_noise_burst() {
    let mut pcm = noise(48_000, 0.1);
    for value in &mut pcm[24_000..24_480] {
        *value *= 1.45;
    }
    let conservative = AnalysisConfig {
        sensitivity: 0.0,
        ..Default::default()
    };
    let sensitive = AnalysisConfig {
        sensitivity: 1.0,
        ..Default::default()
    };
    let low = analyze_transients(&pcm, 1, 48_000, &conservative).unwrap();
    let high = analyze_transients(&pcm, 1, 48_000, &sensitive).unwrap();
    assert!(
        !low.suggestions
            .iter()
            .any(|onset| (23_900..24_480).contains(&onset.frame)),
        "{:?}",
        positions(&low)
    );
    assert!(
        high.suggestions
            .iter()
            .any(|onset| (23_900..24_480).contains(&onset.frame)),
        "{:?}",
        positions(&high)
    );
}

#[test]
fn zero_floor_accepts_subnormal_finite_audio() {
    let pcm = [f32::from_bits(1)];
    let config = AnalysisConfig {
        minimum_rms: 0.0,
        ..Default::default()
    };
    let result = analyze_transients(&pcm, 1, 48_000, &config).unwrap();
    assert_eq!(positions(&result), [0]);
    assert!(result.suggestions[0].confidence.is_finite());
}

#[test]
fn deterministic_parameter_sweep_preserves_bounds_order_and_spacing() {
    let mut pcm = noise(48_000, 0.01);
    for frame in [101, 1_000, 4_001, 4_501, 24_101, 24_501, 47_900] {
        hit(&mut pcm, 48_000, frame, 0.1);
    }
    for sensitivity in [0.0, 0.25, 0.5, 0.75, 1.0] {
        for min_spacing_ms in [1.0, 2.1, 5.0, 25.0, 80.0, 1_000.0] {
            let config = AnalysisConfig {
                sensitivity,
                min_spacing_ms,
                ..Default::default()
            };
            let result = analyze_transients(&pcm, 1, 48_000, &config).unwrap();
            assert!(result
                .suggestions
                .iter()
                .all(|onset| onset.frame < pcm.len() as u64
                    && onset.confidence.is_finite()
                    && (0.0..=1.0).contains(&onset.confidence)
                    && (onset.energy_rise || onset.high_frequency_rise)));
            assert!(result
                .suggestions
                .windows(2)
                .all(|pair| pair[1].frame > pair[0].frame
                    && pair[1].frame - pair[0].frame >= result.provenance.min_spacing_frames));
        }
    }
}

#[test]
fn constant_dc_has_only_a_source_start_edge_and_preserves_overview() {
    for value in [-0.7, 0.7] {
        let pcm = vec![value; 48_000];
        let result = analyze(&pcm, 1, 48_000);
        assert_eq!(positions(&result), [0]);
        assert!(result.waveform.iter().all(|bin| bin.channels
            == [ChannelExtrema {
                min: value,
                max: value
            }]));
    }
}
