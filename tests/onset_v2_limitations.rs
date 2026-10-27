//! Authored diagnostic counterexamples retained as disclosures, not held-out
//! accuracy tests. A higher event count is not a quality improvement.
use aqua_pattern_core::{break_analysis, onset_service, onset_v2::*};
use std::f64::consts::TAU;
fn count(pcm: &[f32], config: &Config) -> usize {
    analyze(pcm, 1, 44100, config).unwrap().suggestions.len()
}
#[test]
fn continuous_pitch_changes_and_beating_tones_still_create_false_proposals() {
    for (name, v1_count, phase_on, phase_off) in [
        ("vibrato", 1, [39, 39], [23, 23]),
        ("chirp", 1, [11, 11], [5, 5]),
        ("beating", 15, [31, 52], [17, 50]),
    ] {
        let mut phase = 0.0;
        let pcm: Vec<f32> = (0..88200)
            .map(|i| {
                let t = i as f64 / 44100.0;
                let frequency = match name {
                    "vibrato" => 440.0 * 2f64.powf(0.8 * (TAU * 6.0 * t).sin() / 12.0),
                    "chirp" => 100.0 + 6000.0 * t / 2.0,
                    _ => 440.0,
                };
                phase += TAU * frequency / 44100.0;
                if name == "beating" {
                    (0.1 * (TAU * 440.0 * t).cos() + 0.1 * (TAU * 447.0 * t).cos()) as f32
                } else {
                    (0.2 * phase.cos()) as f32
                }
            })
            .collect();
        let baseline =
            break_analysis::analyze_transients(&pcm, 1, 44100, &Default::default()).unwrap();
        assert_eq!(baseline.suggestions.len(), v1_count, "{name}");
        let presets = if name == "beating" {
            [Preset::Balanced, Preset::GhostHunter]
        } else {
            [Preset::GhostHunter, Preset::GhostHunter]
        };
        for (i, preset) in presets.into_iter().enumerate() {
            assert_eq!(
                count(&pcm, &Config::preset(preset)),
                phase_on[i],
                "{name}: historical phase-on"
            );
            assert_eq!(
                count(&pcm, &onset_service::experimental_config(preset)),
                phase_off[i],
                "{name}: explicit phase-off policy"
            );
        }
    }
}
#[test]
fn unequal_dense_pulses_do_not_establish_weak_hit_recovery() {
    for (gap, weak, expected) in [(0.011, 0.15, [4, 4, 4]), (0.020, 0.05, [2, 4, 4])] {
        let mut pcm = vec![0f32; 88200];
        for k in 0..8 {
            let start = ((0.25 + k as f64 * gap) * 44100.0).round() as usize;
            let amplitude = if k % 2 == 0 { 0.8 } else { weak };
            for j in 0..22 {
                pcm[start + j] += amplitude * (1.0 - j as f32 / (44100.0 / 2000.0));
            }
        }
        for (preset, count_expected) in
            [Preset::Conservative, Preset::Balanced, Preset::GhostHunter]
                .into_iter()
                .zip(expected)
        {
            assert_eq!(count(&pcm, &Config::preset(preset)), count_expected);
            assert_eq!(
                count(&pcm, &onset_service::experimental_config(preset)),
                count_expected
            );
        }
    }
}
