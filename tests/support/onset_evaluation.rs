//! Small deterministic procedural fixtures. No dataset or external audio corpus.
use serde::Serialize;
#[derive(Clone)]
pub struct Fixture {
    pub name: &'static str,
    pub pcm: Vec<f32>,
    pub annotations: Vec<u64>,
}
fn noise(n: usize, amp: f64) -> Vec<f32> {
    let mut state = 0x9e3779b9u32;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            ((state as f64 / u32::MAX as f64 * 2.0 - 1.0) * amp) as f32
        })
        .collect()
}
fn burst(x: &mut [f32], rate: u32, start: usize, hz: f64, amp: f64, decay: f64) {
    for (i, s) in x
        .iter_mut()
        .skip(start)
        .take((rate as f64 * 0.12) as usize)
        .enumerate()
    {
        let t = i as f64 / rate as f64;
        let attack = (t / 0.00075).min(1.0);
        *s += (amp * attack * (-t / decay).exp() * (t * hz * std::f64::consts::TAU).cos()) as f32;
    }
}
pub fn fixtures(rate: u32) -> Vec<Fixture> {
    let n = rate as usize * 2;
    let starts = [0.12, 0.45, 0.81, 1.22, 1.67];
    let hz = [70.0, 600.0, 6000.0, 170.0, 3200.0];
    let annotations: Vec<_> = starts
        .iter()
        .map(|t| (t * rate as f64).round() as u64)
        .collect();
    let mut sparse = vec![0.0; n];
    for i in 0..5 {
        burst(
            &mut sparse,
            rate,
            annotations[i] as usize,
            hz[i],
            0.4,
            0.015,
        );
    }
    let mut masked = noise(n, 0.002);
    for (i, x) in masked.iter_mut().enumerate() {
        let t = i as f64 / rate as f64;
        *x += (0.10 * (t * 110.0 * std::f64::consts::TAU).sin()) as f32;
    }
    for i in 0..5 {
        burst(
            &mut masked,
            rate,
            annotations[i] as usize,
            hz[i],
            0.035,
            0.012,
        );
    }
    let mut dense = vec![0.0; n];
    let mut dense_annotations = Vec::new();
    for k in 0..12 {
        let start = ((0.20 + 0.011 * k as f64) * rate as f64).round() as usize;
        dense_annotations.push(start as u64);
        for i in 0..(rate as usize / 2000) {
            dense[start + i] = 0.8 * (1.0 - i as f32 / (rate as f32 / 2000.0));
        }
    }
    let mut noise_annotations = vec![0];
    noise_annotations.extend(annotations.iter().copied());
    let mut noisy = noise(n, 0.006);
    for i in 0..5 {
        burst(
            &mut noisy,
            rate,
            annotations[i] as usize,
            hz[i],
            if i == 2 { 0.025 } else { 0.2 },
            0.012,
        );
    }
    vec![
        Fixture {
            name: "sparse_three_bands",
            pcm: sparse,
            annotations: annotations.clone(),
        },
        Fixture {
            name: "quiet_under_tonal_wash",
            pcm: masked,
            annotations: noise_annotations.clone(),
        },
        Fixture {
            name: "dense_11ms",
            pcm: dense,
            annotations: dense_annotations,
        },
        Fixture {
            name: "noisy_weak_burst",
            pcm: noisy,
            annotations: noise_annotations,
        },
        Fixture {
            name: "stationary_noise",
            pcm: noise(n, 0.05),
            annotations: vec![0],
        },
    ]
}
#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub tolerance_ms: f64,
    pub true_positive: usize,
    pub false_positive: usize,
    pub false_negative: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub mean_abs_error_ms: Option<f64>,
    pub max_abs_error_ms: Option<f64>,
}
/// Ordered maximum-cardinality, then minimum-total-error one-to-one matching.
pub fn score(expected: &[u64], actual: &[u64], rate: u32, tolerance_ms: f64) -> Metrics {
    let tolerance = (tolerance_ms * rate as f64 / 1000.0).round() as u64;
    let width = actual.len() + 1;
    let mut dp = vec![(0usize, 0u64); (expected.len() + 1) * width];
    let better = |a: (usize, u64), b: (usize, u64)| {
        if a.0 > b.0 || (a.0 == b.0 && a.1 < b.1) {
            a
        } else {
            b
        }
    };
    for i in 1..=expected.len() {
        for j in 1..=actual.len() {
            let mut best = better(dp[(i - 1) * width + j], dp[i * width + j - 1]);
            let delta = expected[i - 1].abs_diff(actual[j - 1]);
            if delta <= tolerance {
                let prior = dp[(i - 1) * width + j - 1];
                best = better(best, (prior.0 + 1, prior.1 + delta));
            }
            dp[i * width + j] = best;
        }
    }
    let (mut i, mut j) = (expected.len(), actual.len());
    let mut errors = Vec::new();
    while i > 0 && j > 0 {
        let current = dp[i * width + j];
        let prior = dp[(i - 1) * width + j - 1];
        let delta = expected[i - 1].abs_diff(actual[j - 1]);
        if delta <= tolerance && current == (prior.0 + 1, prior.1 + delta) {
            errors.push(delta);
            i -= 1;
            j -= 1;
        } else if current == dp[(i - 1) * width + j] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    let tp = errors.len();
    let precision = if actual.is_empty() {
        if expected.is_empty() {
            1.0
        } else {
            0.0
        }
    } else {
        tp as f64 / actual.len() as f64
    };
    let recall = if expected.is_empty() {
        1.0
    } else {
        tp as f64 / expected.len() as f64
    };
    Metrics {
        tolerance_ms,
        true_positive: tp,
        false_positive: actual.len() - tp,
        false_negative: expected.len() - tp,
        precision,
        recall,
        f1: if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        },
        mean_abs_error_ms: if tp == 0 {
            None
        } else {
            Some(errors.iter().sum::<u64>() as f64 / tp as f64 * 1000.0 / rate as f64)
        },
        max_abs_error_ms: errors
            .iter()
            .max()
            .map(|x| *x as f64 * 1000.0 / rate as f64),
    }
}
