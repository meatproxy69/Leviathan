#[path = "support/onset_evaluation.rs"]
mod evaluation;
use aqua_pattern_core::onset_v2::*;
fn frames(a: &Analysis) -> Vec<u64> {
    a.suggestions.iter().map(|s| s.attack_frame).collect()
}
#[test]
fn sparse_synthetic_precision_recall_and_refinement_are_measured_at_three_rates() {
    for rate in [44100, 48000, 96000] {
        let fixture = evaluation::fixtures(rate).remove(0);
        let a = analyze(&fixture.pcm, 1, rate, &Config::default()).unwrap();
        for tolerance in [5.0, 10.0] {
            let m = evaluation::score(&fixture.annotations, &frames(&a), rate, tolerance);
            assert_eq!(m.f1, 1.0, "rate{rate}: {m:?}");
            assert!(m.max_abs_error_ms.unwrap() < 1.0);
        }
        assert!(a
            .suggestions
            .iter()
            .all(|s| s.search_start <= s.attack_frame && s.attack_frame < s.search_end));
    }
}
#[test]
fn ghost_preset_can_retain_dense_11ms_attacks_without_disabling_hard_gap() {
    for rate in [44100, 48000, 96000] {
        let fixture = evaluation::fixtures(rate).remove(2);
        let c = Config::preset(Preset::GhostHunter);
        let a = analyze(&fixture.pcm, 1, rate, &c).unwrap();
        let m = evaluation::score(&fixture.annotations, &frames(&a), rate, 5.0);
        assert!(m.precision >= 0.9 && m.recall >= 0.9, "rate{rate}: {m:?}");
        assert!(a
            .suggestions
            .windows(2)
            .all(|s| s[1].attack_frame - s[0].attack_frame
                >= (c.dense_gap_ms * rate as f64 / 1000.0).ceil() as u64));
    }
}
#[test]
fn antiphase_stereo_is_not_silence_and_all_modes_preserve_source() {
    let fixture = evaluation::fixtures(44100).remove(0);
    let mono = analyze(&fixture.pcm, 1, 44100, &Config::default()).unwrap();
    let stereo: Vec<_> = fixture.pcm.iter().flat_map(|x| [*x, -*x]).collect();
    let before = stereo.clone();
    for mode in [
        StereoMode::MidSide,
        StereoMode::LeftRight,
        StereoMode::MidSideAndLeftRight,
    ] {
        let c = Config {
            stereo: mode,
            ..Default::default()
        };
        let result = analyze(&stereo, 2, 44100, &c).unwrap();
        assert_eq!(frames(&result), frames(&mono));
        assert_eq!(stereo, before);
    }
}
#[test]
fn silence_dc_and_startup_do_not_create_fft_history_artifacts() {
    for pcm in [vec![0.0; 44100], vec![0.3; 44100], vec![-0.3; 44100]] {
        let a = analyze(&pcm, 1, 44100, &Config::default()).unwrap();
        assert!(frames(&a).iter().all(|f| *f == 0), "{:?}", frames(&a));
        let c = Config {
            enable_fast_attack: false,
            ..Default::default()
        };
        let a = analyze(&pcm, 1, 44100, &c).unwrap();
        assert!(a.suggestions.is_empty());
    }
    let f = evaluation::fixtures(44100).pop().unwrap();
    assert_eq!(
        frames(&analyze(&f.pcm, 1, 44100, &Config::default()).unwrap()),
        [0]
    );
}
#[test]
fn source_start_ramp_short_inputs_and_last_frame_are_not_fabricated_fft_events() {
    for pcm in [
        vec![0.0],
        vec![0.8],
        vec![0.0, 0.3, 0.8],
        vec![0.0, 0.0, 0.8],
    ] {
        let a = analyze(&pcm, 1, 44100, &Config::default()).unwrap();
        assert!(frames(&a).iter().all(|f| *f < pcm.len() as u64));
    }
    let mut pcm = vec![0.0; 44100];
    for (i, x) in pcm.iter_mut().take(500).enumerate() {
        *x = (i as f32 / 16.0).min(1.0) * (-(i as f32) / 100.0).exp();
    }
    let a = analyze(&pcm, 1, 44100, &Config::default()).unwrap();
    assert_eq!(a.suggestions.len(), 1);
    assert!(a.suggestions[0].attack_frame < 8, "{:?}", frames(&a));
}
#[test]
fn cache_keeps_weak_evidence_and_repicks_without_mutating_it() {
    let fixture = evaluation::fixtures(44100).remove(1);
    let cache = extract(&fixture.pcm, 1, 44100, &Config::default()).unwrap();
    let before = cache.clone();
    let low = detect_cached(&cache, &Config::preset(Preset::Conservative)).unwrap();
    let high = detect_cached(&cache, &Config::preset(Preset::GhostHunter)).unwrap();
    assert!(high.candidates.len() > low.candidates.len());
    assert_eq!(cache, before);
    assert_eq!(
        detect_cached(&cache, &Config::preset(Preset::GhostHunter)).unwrap(),
        high
    );
    assert_eq!(
        cache.provenance().fft_backend,
        "realfft-3.5.0/rustfft-6.4.1"
    );
    let changed = Config {
        max_filter_bins: 1,
        ..Default::default()
    };
    assert!(matches!(
        detect_cached(&cache, &changed),
        Err(Error::InvalidConfig(_))
    ));
}
#[test]
fn refinement_is_separate_disableable_and_source_bound() {
    let fixture = evaluation::fixtures(44100).remove(0);
    let mut c = Config::default();
    c.refinement.enabled = false;
    let d = detect(&fixture.pcm, 1, 44100, &c).unwrap();
    let (s, _) = refine(&fixture.pcm, 1, 44100, &d).unwrap();
    assert!(s
        .iter()
        .all(|s| s.attack_frame == s.candidate.peak_frame && s.refinement_shift == 0));
    let mut changed = fixture.pcm.clone();
    changed[0] = 0.1;
    assert_eq!(refine(&changed, 1, 44100, &d), Err(Error::MismatchedSource));
    let mut invalid = d.clone();
    invalid.candidates[0].peak_frame = u64::MAX;
    assert!(matches!(
        refine(&fixture.pcm, 1, 44100, &invalid),
        Err(Error::InvalidInput(_))
    ));
}
#[test]
fn invalid_nonfinite_and_budget_inputs_are_rejected() {
    let c = Config::default();
    assert!(matches!(
        extract(&[], 1, 44100, &c),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        extract(&[0.0; 3], 2, 44100, &c),
        Err(Error::InvalidInput(_))
    ));
    for rate in [0, 7999, 384001] {
        assert!(extract(&[0.0], 1, rate, &c).is_err());
    }
    for channels in [0, 3, usize::MAX] {
        assert!(extract(&[0.0], channels, 44100, &c).is_err());
    }
    for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(extract(&[0.0, x], 2, 44100, &c), Err(Error::NonFinite(1)));
    }
    for c in [
        Config {
            max_work_units: 1,
            ..Default::default()
        },
        Config {
            max_working_bytes: 1,
            ..Default::default()
        },
        Config {
            max_frames: 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            extract(&[0.0; 2000], 1, 44100, &c),
            Err(Error::Budget(_))
        ));
    }
    for c in [
        Config {
            threshold: f64::NAN,
            ..Default::default()
        },
        Config {
            minimum_rms: 0.0,
            ..Default::default()
        },
        Config {
            dense_gap_ms: 100.0,
            ..Default::default()
        },
        Config {
            max_suggestions: 0,
            ..Default::default()
        },
        Config {
            max_fft_size: usize::MAX,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            extract(&[0.0; 10], 1, 44100, &c),
            Err(Error::InvalidConfig(_))
        ));
    }
}
#[test]
fn output_limits_evidence_serialization_and_phase_only_are_explicit() {
    let fixture = evaluation::fixtures(44100).remove(0);
    let cache = extract(&fixture.pcm, 1, 44100, &Config::default()).unwrap();
    assert_eq!(
        detect_cached(
            &cache,
            &Config {
                max_suggestions: 1,
                ..Default::default()
            }
        ),
        Err(Error::OutputLimit)
    );
    let c = Config {
        enable_flux: false,
        enable_energy: false,
        enable_fast_attack: false,
        ..Default::default()
    };
    assert!(detect_cached(&cache, &c).unwrap().candidates.is_empty());
    let d = detect_cached(&cache, &Config::default()).unwrap();
    assert!(d.candidates.iter().all(|s| s.confidence.is_finite()
        && s.strength.is_finite()
        && (0.0..=1.0).contains(&s.confidence)
        && s.detector_votes.iter().any(|x| *x)));
    let text = serde_json::to_string(&d).unwrap();
    let restored = serde_json::from_str::<Detection>(&text).unwrap();
    assert_eq!(restored.provenance, d.provenance);
    assert_eq!(restored.candidates.len(), d.candidates.len());
    for (a, b) in restored.candidates.iter().zip(&d.candidates) {
        assert_eq!(a.peak_frame, b.peak_frame);
        assert_eq!(a.detector_votes, b.detector_votes);
        assert!((a.confidence - b.confidence).abs() < 1e-12);
    }
    assert_eq!(
        serde_json::to_string(&detect_cached(&cache, &Config::default()).unwrap()).unwrap(),
        text
    );
}
#[test]
fn explicit_rate_policy_and_nyquist_bands_are_recorded() {
    for rate in [8000, 44100, 48000, 96000, 384000] {
        let c = Config {
            max_work_units: 2_000_000_000,
            ..Default::default()
        };
        let a = extract(&[0.0; 128], 1, rate, &c).unwrap();
        for band in &a.provenance().effective_bands {
            assert!(band.high_hz <= rate as f64 / 2.0);
            assert!(band.window_frames.is_power_of_two());
            assert_eq!(band.alignment_frames, band.window_frames / 4);
        }
        if rate == 44100 {
            assert_eq!(
                a.provenance()
                    .effective_bands
                    .iter()
                    .map(|b| (b.window_frames, b.hop_frames))
                    .collect::<Vec<_>>(),
                [(2048, 256), (1024, 128), (512, 64)]
            );
        }
    }
}
#[test]
fn one_to_one_metrics_cannot_credit_duplicate_predictions() {
    let m = evaluation::score(&[100, 200], &[99, 100, 101, 200], 1000, 5.0);
    assert_eq!(m.true_positive, 2);
    assert_eq!(m.false_positive, 2);
    assert_eq!(m.false_negative, 0);
    assert_eq!(m.mean_abs_error_ms, Some(0.0));
}

#[test]
fn isolated_first_and_last_sample_boundaries_are_exact_after_refinement() {
    for pcm in [vec![0.8], vec![0.0, 0.0, 0.8]] {
        let a = analyze(&pcm, 1, 44100, &Config::default()).unwrap();
        assert_eq!(frames(&a), [pcm.len() as u64 - 1]);
    }
    for rate in [44100, 48000, 96000] {
        let mut pcm = vec![0.0; rate as usize];
        pcm[0] = 0.8;
        pcm[rate as usize - 1] = 0.8;
        let a = analyze(&pcm, 1, rate, &Config::default()).unwrap();
        assert_eq!(frames(&a), [0, rate as u64 - 1]);
    }
}

#[test]
fn cached_normalization_rechecks_the_changed_work_budget() {
    let fixture = evaluation::fixtures(44100).remove(0);
    let cache = extract(&fixture.pcm, 1, 44100, &Config::default()).unwrap();
    let c = Config {
        median_radius_ms: 250.0,
        max_work_units: cache.provenance().estimated_work_units,
        ..Default::default()
    };
    assert!(matches!(detect_cached(&cache, &c), Err(Error::Budget(_))));
    let c = Config {
        max_work_units: 2_000_000_000,
        ..c
    };
    assert!(
        detect_cached(&cache, &c)
            .unwrap()
            .provenance
            .estimated_work_units
            > cache.provenance().estimated_work_units
    );
}

#[test]
fn phase_is_never_standalone_evidence_even_at_lowest_valid_threshold() {
    let fixture = evaluation::fixtures(44100).remove(0);
    let cache = extract(&fixture.pcm, 1, 44100, &Config::default()).unwrap();
    for threshold in [0.1, 0.11, 0.12, 0.43] {
        let config = Config {
            enable_flux: false,
            enable_energy: false,
            enable_fast_attack: false,
            enable_phase: true,
            threshold,
            mad_multiplier: 0.5,
            retain_envelope: true,
            ..Config::preset(Preset::GhostHunter)
        };
        let result = detect_cached(&cache, &config).unwrap();
        assert!(result.candidates.is_empty());
        assert!(result.envelope.iter().all(|e| e.confidence == 0.0));
        assert!(result
            .envelope
            .iter()
            .any(|e| e.bands.iter().any(|b| b.phase > 0.0)));
    }
}

#[test]
fn judged_default_and_ablation_source_frame_outputs_remain_unchanged() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/onset-v2-judged-frames.json")).unwrap();
    let mut checked = 0;
    for rate in [44100, 48000, 96000] {
        for fixture in evaluation::fixtures(rate) {
            let cache = extract(&fixture.pcm, 1, rate, &Config::default()).unwrap();
            let baseline = aqua_pattern_core::break_analysis::analyze_transients(
                &fixture.pcm,
                1,
                rate,
                &Default::default(),
            )
            .unwrap();
            let baseline_frames: Vec<_> = baseline.suggestions.iter().map(|s| s.frame).collect();
            for row in rows
                .iter()
                .filter(|r| r["rate"] == rate && r["fixture"] == fixture.name)
            {
                let preset: Preset = serde_json::from_value(row["preset"].clone()).unwrap();
                let mut c = Config::preset(preset);
                let ablation = row["ablation"].as_str().unwrap();
                c.enable_flux = ablation != "without_flux";
                c.enable_energy = ablation != "without_energy";
                c.enable_phase = ablation != "without_phase";
                c.enable_fast_attack = ablation != "without_fast";
                let d = detect_cached(&cache, &c).unwrap();
                let (s, collisions) = refine(&fixture.pcm, 1, rate, &d).unwrap();
                assert_eq!(
                    serde_json::json!(d
                        .candidates
                        .iter()
                        .map(|c| c.peak_frame)
                        .collect::<Vec<_>>()),
                    row["peak_frames"]
                );
                assert_eq!(
                    serde_json::json!(s.iter().map(|s| s.attack_frame).collect::<Vec<_>>()),
                    row["attack_frames"]
                );
                assert_eq!(serde_json::json!(baseline_frames), row["v1_frames"]);
                assert_eq!(serde_json::json!(collisions), row["collisions"]);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 225);
}
