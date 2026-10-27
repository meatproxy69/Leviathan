use aqua_pattern_core::{
    onset_service, onset_v2, sample_asset,
    slice_features::{
        self as features, Articulation, FeatureRequest, PercussionRole, RoleChoice, RoleLabel,
        SuggestionStatus,
    },
    song::*,
    song_render,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    song: Song,
}
impl Fixture {
    fn new(
        rate: u32,
        channels: u16,
        frames: usize,
        mut value: impl FnMut(usize, usize) -> f32,
    ) -> Self {
        let path = std::env::temp_dir().join(format!(
            "slice-features-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for f in 0..frames {
            for c in 0..usize::from(channels) {
                writer.write_sample(value(f, c)).unwrap();
            }
        }
        writer.finalize().unwrap();
        let asset = sample_asset::import(&path).unwrap().asset;
        let mut song = example_song();
        song.samples.insert(SampleId(1), asset);
        song.regions.insert(
            RegionId(1),
            SampleRegion {
                name: "Measured native source".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: frames as u64,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        );
        Self { path, song }
    }
    fn preview(&self) -> features::FeaturePreview {
        features::analyze_verified(
            &features::prepare(&self.song, request(vec![RegionId(1)])).unwrap(),
            &self.path,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn request(regions: Vec<RegionId>) -> FeatureRequest {
    FeatureRequest {
        version: 1,
        sample: SampleId(1),
        regions,
    }
}
fn label(role: PercussionRole) -> RoleLabel {
    RoleLabel {
        role,
        articulation: Articulation::Unspecified,
        note: "Human-authored, not classifier confidence".into(),
    }
}
fn close(a: f64, b: f64, t: f64) {
    assert!((a - b).abs() <= t, "{a} != {b} (tolerance{t})");
}
#[test]
fn raw_levels_dc_and_antiphase_preserve_native_energy_without_normalizing() {
    let f = Fixture::new(48000, 2, 4800, |_, c| if c == 0 { 1.5 } else { -1.5 });
    let p = f.preview();
    let m = &p.records()[0].snapshot.metrics;
    assert_eq!(m.rms, 1.5);
    assert_eq!(m.peak, 1.5);
    assert_eq!(m.dc_per_channel, [1.5, -1.5]);
    assert_eq!(m.peak_frame, 0);
    assert_eq!(m.analysed_power, None);
    assert_eq!(
        p.records()[0].snapshot.suggestion_status,
        SuggestionStatus::Unknown
    );
    let a = Fixture::new(48000, 1, 12000, |i, _| {
        (0.5 * (std::f64::consts::TAU * 1000.0 * i as f64 / 48000.0).sin()) as f32
    });
    let b = Fixture::new(48000, 2, 12000, |i, c| {
        ((if c == 0 { 0.5 } else { -0.5 })
            * (std::f64::consts::TAU * 1000.0 * i as f64 / 48000.0).sin()) as f32
    });
    let a = a.preview();
    let b = b.preview();
    let a = &a.records()[0].snapshot.metrics;
    let b = &b.records()[0].snapshot.metrics;
    close(a.rms, b.rms, 1e-13);
    close(a.analysed_power.unwrap(), b.analysed_power.unwrap(), 1e-13);
    close(a.centroid_hz.unwrap(), b.centroid_hz.unwrap(), 1e-9);
}
#[test]
fn spectral_tones_have_declared_native_bands_and_effective_layout_across_rates() {
    for rate in [8000, 44100, 48000, 96000, 384000] {
        let n = (u64::from(rate) * 32)
            .div_ceil(1000)
            .next_power_of_two()
            .clamp(256, 16384) as usize;
        let f = Fixture::new(rate, 1, n * 8, |i, _| {
            (0.6 * (std::f64::consts::TAU * 1000.0 * i as f64 / f64::from(rate)).sin()) as f32
        });
        let p = f.preview();
        let s = &p.records()[0].snapshot;
        assert_eq!(s.layout.fft_size, n as u32);
        assert_eq!(s.layout.hop_frames, n as u32 / 4);
        assert_eq!(s.layout.envelope_bin_frames, (rate + 500) / 1000);
        assert_eq!(s.metrics.spectral_windows, 32);
        close(s.metrics.centroid_hz.unwrap(), 1000.0, 20.0);
        assert!(s.metrics.mid_fraction.unwrap() > 0.97);
        assert!(s.metrics.flatness.unwrap() < 0.01);
    }
    for (hz, which) in [(125.0, 0), (1000.0, 1), (3000.0, 2)] {
        let f = Fixture::new(8000, 1, 8192, |i, _| {
            (0.6 * (std::f64::consts::TAU * hz * i as f64 / 8000.0).sin()) as f32
        });
        let p = f.preview();
        let m = &p.records()[0].snapshot.metrics;
        assert!([m.low_fraction, m.mid_fraction, m.high_fraction][which].unwrap() > 0.97);
    }
}
#[test]
fn silence_single_frame_subnormal_and_above_unity_stay_finite_or_explicit_none() {
    for value in [0.0, f32::from_bits(1), 2.0, -2.0] {
        let f = Fixture::new(48000, 1, 1, |_, _| value);
        let p = f.preview();
        let s = &p.records()[0].snapshot;
        assert_eq!(s.metrics.peak, f64::from(value).abs());
        assert_eq!(s.metrics.rms, f64::from(value).abs());
        assert_eq!(s.metrics.analysed_power, None);
        assert_eq!(s.metrics.shape_change_mean, None);
        assert_eq!(s.suggestion_status, SuggestionStatus::Unknown);
        let c = p.catalog_preview(vec![]).unwrap();
        c.candidate().validate().unwrap();
        assert_eq!(
            serde_json::from_slice::<Song>(&serde_json::to_vec(c.candidate()).unwrap()).unwrap(),
            *c.candidate()
        );
    }
}
#[test]
fn envelope_frames_are_native_absolute_quantized_and_censoring_is_not_invented_decay() {
    let bins = [
        0.0, 0.0625, 0.125, 0.5, 0.9375, 1.0, 0.5, 0.125, 0.0625, 0.0,
    ];
    let mut f = Fixture::new(48000, 1, bins.len() * 48, |i, _| bins[i / 48]);
    let p = f.preview();
    let m = &p.records()[0].snapshot.metrics;
    assert_eq!(m.envelope_peak_frame, Some(5 * 48));
    assert_eq!(m.attack_10_90_frames, Some(2 * 48));
    assert!(!m.attack_left_censored);
    assert_eq!(m.tail_20db_frames, Some(3 * 48));
    assert!(!m.tail_right_censored);
    assert_eq!(m.envelope_rises, 1);
    let r = f.song.regions.get_mut(&RegionId(1)).unwrap();
    r.start_frame = 2 * 48;
    r.end_frame = 8 * 48;
    let p = f.preview();
    let m = &p.records()[0].snapshot.metrics;
    assert_eq!(m.envelope_peak_frame, Some(5 * 48));
    assert!(m.attack_left_censored);
    assert_eq!(m.attack_10_90_frames, None);
    assert!(m.tail_right_censored);
    assert_eq!(m.tail_20db_frames, None);
    assert_eq!(m.tail_lower_bound_frames, Some(2 * 48));
}
#[test]
fn complete_bounds_ignore_outside_samples_and_caches_do_not_read_source_again() {
    let mut f = Fixture::new(48000, 2, 6000, |i, c| {
        if i < 100 {
            9.0
        } else if i >= 4900 {
            -12.0
        } else if c == 0 {
            0.25
        } else {
            -0.25
        }
    });
    let r = f.song.regions.get_mut(&RegionId(1)).unwrap();
    r.start_frame = 100;
    r.end_frame = 4900;
    let p = f.preview();
    let m = &p.records()[0].snapshot.metrics;
    assert_eq!(m.peak, 0.25);
    assert_eq!(m.rms, 0.25);
    assert_eq!(m.peak_frame, 100);
    assert_eq!(m.dc_per_channel, [0.25, -0.25]);
    let config = onset_service::experimental_config(onset_v2::Preset::Balanced);
    let cached =
        onset_service::prepare_verified(&f.song.samples[&SampleId(1)], &f.path, &config).unwrap();
    let plan = features::prepare(&f.song, request(vec![RegionId(1)])).unwrap();
    let c = features::analyze_cached(&plan, &cached.cache).unwrap();
    assert_eq!(p.records(), c.records());
    assert_eq!(p.id(), c.id());
    std::fs::remove_file(&f.path).unwrap();
    assert!(sample_asset::verify_identity(plan.asset(), &f.path).is_err());
    assert_eq!(
        features::analyze_cached(&plan, &cached.cache)
            .unwrap()
            .records(),
        p.records()
    );
    assert!(features::analyze_verified(&plan, &f.path).is_err());
}
#[test]
fn snapshots_and_explicit_labels_are_independent_and_geometry_stales_only_binding() {
    let mut f = Fixture::new(48000, 1, 4800, |i, _| if i % 400 < 40 { 0.4 } else { 0.0 });
    let p = f.preview();
    assert!(f.song.region_catalog.is_empty());
    let stored = p.catalog_preview(vec![]).unwrap();
    assert!(stored.candidate().region_catalog[&RegionId(1)]
        .reviewed_role
        .is_none());
    let choice = RoleChoice {
        region: RegionId(1),
        label: RoleLabel {
            role: PercussionRole::Mixed,
            articulation: Articulation::Ghost,
            note: "Manual contextual ghost with bleed".into(),
        },
    };
    f.song = p
        .catalog_preview(vec![choice])
        .unwrap()
        .commit(&f.song)
        .unwrap();
    let old = f.song.region_catalog[&RegionId(1)]
        .reviewed_role
        .clone()
        .unwrap();
    let r = f.song.regions.get_mut(&RegionId(1)).unwrap();
    r.name = "Rename".into();
    r.root_note = 12;
    r.amp_envelope = Some(AmpEnvelope {
        attack_ms: 10,
        decay_ms: 20,
        sustain_percent: 50,
        release_ms: 30,
    });
    r.forward_loop = Some(ForwardLoop {
        start_frame: 100,
        end_frame: 200,
    });
    let s = features::inspect_catalog(&f.song, RegionId(1)).unwrap();
    assert_eq!(s.features_current_match, Some(true));
    assert_eq!(s.reviewed_role_current_match, Some(true));
    f.song.regions.get_mut(&RegionId(1)).unwrap().end_frame = 4700;
    let s = features::inspect_catalog(&f.song, RegionId(1)).unwrap();
    assert_eq!(s.features_current_match, Some(false));
    assert_eq!(s.reviewed_role_current_match, Some(false));
    let fresh = f.preview();
    f.song = fresh
        .catalog_preview(vec![])
        .unwrap()
        .commit(&f.song)
        .unwrap();
    assert_eq!(f.song.region_catalog[&RegionId(1)].reviewed_role, Some(old));
    let s = features::inspect_catalog(&f.song, RegionId(1)).unwrap();
    assert_eq!(s.features_current_match, Some(true));
    assert_eq!(s.reviewed_role_current_match, Some(false));
    f.song
        .set_region_role(RegionId(1), Some(label(PercussionRole::Unknown)))
        .unwrap();
    assert_eq!(
        features::inspect_catalog(&f.song, RegionId(1))
            .unwrap()
            .reviewed_role_current_match,
        Some(true)
    );
    assert_eq!(
        f.song.region_catalog[&RegionId(1)]
            .reviewed_role
            .as_ref()
            .unwrap()
            .based_on_feature_sha256,
        None
    );
}
#[test]
fn explicit_clear_prunes_empty_metadata_and_region_delete_is_atomic() {
    let mut f = Fixture::new(48000, 1, 2400, |_, _| 0.0);
    f.song
        .set_region_role(RegionId(1), Some(label(PercussionRole::Unknown)))
        .unwrap();
    assert!(f.song.region_catalog.contains_key(&RegionId(1)));
    f.song.clear_region_features(RegionId(1)).unwrap();
    assert!(f.song.region_catalog.contains_key(&RegionId(1)));
    f.song.set_region_role(RegionId(1), None).unwrap();
    assert!(f.song.region_catalog.is_empty());
    f.song = f
        .preview()
        .catalog_preview(vec![])
        .unwrap()
        .commit(&f.song)
        .unwrap();
    f.song
        .set_region_role(RegionId(1), Some(label(PercussionRole::Kick)))
        .unwrap();
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().region = Some(RegionId(1));
    let before = f.song.clone();
    assert!(f.song.remove_region(RegionId(1)).is_err());
    assert_eq!(f.song, before);
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().region = None;
    f.song.remove_region(RegionId(1)).unwrap();
    assert!(f.song.region_catalog.is_empty());
}
#[test]
fn metadata_only_catalog_does_not_change_evaluator_or_default_serialization() {
    let mut f = Fixture::new(48000, 1, 4800, |i, _| 0.2 * (i as f32 * 0.2).sin());
    for p in f.song.patterns.values_mut() {
        p.tracks.clear();
    }
    f.song.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: Default::default(),
        section: None,
    }];
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(0)
        .or_default()
        .insert(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        );
    f.song
        .assign_region(InstrumentId(1), Some(RegionId(1)))
        .unwrap();
    assert!(!serde_json::to_string(&f.song)
        .unwrap()
        .contains("region_catalog"));
    let before = song_render::evaluate(&f.song, None).unwrap();
    let p = f.preview();
    f.song = p
        .catalog_preview(vec![RoleChoice {
            region: RegionId(1),
            label: label(PercussionRole::Crash),
        }])
        .unwrap()
        .commit(&f.song)
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&before).unwrap(),
        serde_json::to_vec(&song_render::evaluate(&f.song, None).unwrap()).unwrap()
    );
    sample_asset::verify_identity(&f.song.samples[&SampleId(1)], &f.path).unwrap();
}
#[test]
fn early_limits_charge_overlapping_regions_fft_and_channels_without_source_io() {
    let mut s = example_song();
    s.samples.insert(
        SampleId(1),
        SampleAsset {
            path: "/missing/features.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: 8000,
            channels: 1,
            frames: 240000,
        },
    );
    for i in 1..=65 {
        s.regions.insert(
            RegionId(i),
            SampleRegion {
                name: format!("Overlap{i}"),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 240000,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        );
    }
    let p = features::prepare(&s, request((1..=50).map(RegionId).collect())).unwrap();
    assert_eq!(p.summary().visited_frames, 12000000);
    assert_eq!(p.summary().estimated_work, 492000000);
    let error = features::prepare(&s, request((1..=51).map(RegionId).collect())).unwrap_err();
    assert!(error.contains("501840000"));
    assert!(features::prepare(&s, request((1..=65).map(RegionId).collect())).is_err());
    assert!(features::prepare(&s, request(vec![RegionId(2), RegionId(1)])).is_err());
    assert!(features::prepare(&s, request(vec![RegionId(1), RegionId(1)])).is_err());
    assert!(features::prepare(&s, request(vec![])).is_err());
    s.regions.get_mut(&RegionId(1)).unwrap().end_frame = 240001;
    s.samples.get_mut(&SampleId(1)).unwrap().frames = 240001;
    assert!(features::prepare(&s, request(vec![RegionId(1)]))
        .unwrap_err()
        .contains("30 seconds"));
    // Last whole-hop region below the work cap; adding one frame introduces
    // another full FFT window and must fail before opening the missing path.
    s.regions.retain(|id, _| *id == RegionId(1));
    let asset = s.samples.get_mut(&SampleId(1)).unwrap();
    asset.sample_rate = 384000;
    asset.frames = 7_692_289;
    s.regions.get_mut(&RegionId(1)).unwrap().end_frame = 7_692_288;
    let plan = features::prepare(&s, request(vec![RegionId(1)])).unwrap();
    assert_eq!(plan.summary().estimated_work, 499_998_720);
    s.regions.get_mut(&RegionId(1)).unwrap().end_frame += 1;
    assert!(features::prepare(&s, request(vec![RegionId(1)]))
        .unwrap_err()
        .contains("500260865"));
}
#[test]
fn source_paths_hashes_and_manual_notes_reject_without_hidden_repair() {
    let mut f = Fixture::new(48000, 1, 2400, |_, _| 0.1);
    let p = features::prepare(&f.song, request(vec![RegionId(1)])).unwrap();
    assert!(
        features::analyze_verified(&p, std::path::Path::new("relative.wav"))
            .unwrap_err()
            .contains("absolute")
    );
    let before = f.song.clone();
    let mut bad = label(PercussionRole::Kick);
    bad.note = "é".repeat(129);
    assert!(f.song.set_region_role(RegionId(1), Some(bad)).is_err());
    assert_eq!(f.song, before);
    f.song.samples.get_mut(&SampleId(1)).unwrap().sha256 = "b".repeat(64);
    let wrong = features::prepare(&f.song, request(vec![RegionId(1)])).unwrap();
    assert!(features::analyze_verified(&wrong, &f.path).is_err());
}
#[test]
fn malformed_stored_numbers_layout_null_groups_and_rules_are_rejected() {
    let f = Fixture::new(48000, 1, 4800, |i, _| 0.3 * (i as f32 * 0.3).sin());
    let p = f.preview();
    let base = p.catalog_preview(vec![]).unwrap().candidate().clone();
    for case in 0..11 {
        let mut s = base.clone();
        let f = s
            .region_catalog
            .get_mut(&RegionId(1))
            .unwrap()
            .features
            .as_mut()
            .unwrap();
        match case {
            0 => f.metrics.rms = f64::NAN,
            1 => f.metrics.low_fraction = Some(-1.0),
            2 => f.layout.fft_size = 1,
            3 => f.metrics.peak_frame = f.binding.end_frame,
            4 => f.metrics.flatness = None,
            5 => f.suggestions[0].rule_support = 1.1,
            6 => f.warnings.push("signed truth".into()),
            7 => f.binding.source_sha256 = "x".repeat(64),
            8 => f.metrics.dc_per_channel.push(0.0),
            9 => f.metrics.supported_shape_windows = f.metrics.nonzero_spectral_windows + 1,
            _ => f.layout.shape_support_floor_relative = 0.0,
        }
        assert!(s.validate().is_err(), "case{case}");
        assert!(
            features::inspect_catalog(&s, RegionId(1)).is_err(),
            "inspect{case}"
        );
    }
    let mut stale = base;
    stale.title.push('x');
    assert!(p
        .catalog_preview(vec![])
        .unwrap()
        .commit(&stale)
        .unwrap_err()
        .contains("stale"));
}

#[test]
fn shape_aggregate_roundoff_does_not_reject_a_valid_normalized_summary() {
    let f = Fixture::new(48000, 1, 4800, |i, _| 0.3 * (i as f32 * 0.3).sin());
    let p = f.preview();
    let mut s = p.catalog_preview(vec![]).unwrap().candidate().clone();
    let m = &mut s
        .region_catalog
        .get_mut(&RegionId(1))
        .unwrap()
        .features
        .as_mut()
        .unwrap()
        .metrics;
    m.shape_comparisons = 3;
    m.shape_change_max = Some(0.1);
    m.shape_change_mean = Some((0.1 + 0.1 + 0.1) / 3.0);
    assert!(m.shape_change_mean.unwrap() > m.shape_change_max.unwrap());
    s.validate().unwrap();
    features::inspect_catalog(&s, RegionId(1)).unwrap();
}

#[test]
fn long_finite_extreme_level_uses_stable_energy_sum_without_false_peak_rejection() {
    let f = Fixture::new(384000, 1, 2_000_000, |_, _| f32::MAX);
    let p = f.preview();
    let m = &p.records()[0].snapshot.metrics;
    assert_eq!(m.peak, f64::from(f32::MAX));
    close(m.rms / m.peak, 1.0, 2.0 * f64::EPSILON);
    close(m.crest_factor.unwrap(), 1.0, 2.0 * f64::EPSILON);
    assert_eq!(m.analysed_power, None);
    p.catalog_preview(vec![])
        .unwrap()
        .candidate()
        .validate()
        .unwrap();
}

#[test]
fn repeating_an_identical_manual_label_preserves_its_historical_evidence_sha() {
    let f = Fixture::new(48000, 1, 4800, |i, _| 0.2 * (i as f32 * 0.1).sin());
    let p = f.preview();
    let chosen = label(PercussionRole::Mixed);
    let mut s = p
        .catalog_preview(vec![RoleChoice {
            region: RegionId(1),
            label: chosen.clone(),
        }])
        .unwrap()
        .commit(&f.song)
        .unwrap();
    let before = s.clone();
    assert!(s.region_catalog[&RegionId(1)]
        .reviewed_role
        .as_ref()
        .unwrap()
        .based_on_feature_sha256
        .is_some());
    s.set_region_role(RegionId(1), Some(chosen.clone()))
        .unwrap();
    assert_eq!(s, before);
    // Direct model callers cannot use an identical-label no-op to bypass
    // validation of an otherwise malformed document.
    s.bpm = 0.0;
    let malformed = s.clone();
    assert!(s.set_region_role(RegionId(1), Some(chosen)).is_err());
    assert_eq!(s, malformed);
}

#[test]
fn out_of_band_roundoff_cannot_be_normalized_into_spectral_shape() {
    let mut reference = None;
    // Power-of-two source scales prove this is relative numerical support,
    // not an absolute quiet-source cutoff or normalization of the samples.
    for amplitude in [0.25f32, 2.0f32.powi(-80), 2.0f32.powi(80)] {
        let f = Fixture::new(48000, 1, 48000, |i, _| {
            if i.is_multiple_of(2) {
                amplitude
            } else {
                -amplitude
            }
        });
        let p = f.preview();
        let snapshot = &p.records()[0].snapshot;
        let m = &snapshot.metrics;
        assert_eq!(snapshot.layout.shape_support_floor_relative, 1e-24);
        assert_eq!(m.spectral_windows, 94);
        // Only the four genuinely truncated edge windows leak meaningful
        // Nyquist energy into the <=20 kHz analysed band.
        assert_eq!(m.supported_shape_windows, 4);
        assert!(m.nonzero_spectral_windows > m.supported_shape_windows);
        assert_eq!(m.shape_comparisons, 2);
        assert!(m.analysed_power.unwrap() > 0.0);
        assert!(snapshot
            .warnings
            .iter()
            .any(|w| w.contains("numerical shape support")));
        let shape = (m.shape_change_mean.unwrap(), m.shape_change_max.unwrap());
        if let Some((mean, max)) = reference {
            close(shape.0, mean, 1e-12);
            close(shape.1, max, 1e-12);
        } else {
            reference = Some(shape);
        }
        p.catalog_preview(vec![])
            .unwrap()
            .candidate()
            .validate()
            .unwrap();
    }
    // Conversely, a genuinely quiet in-band AC source remains supported.
    let tiny = Fixture::new(48000, 1, 4800, |i, _| {
        (1e-40f64 * (std::f64::consts::TAU * 1000.0 * i as f64 / 48000.0).sin()) as f32
    });
    let p = tiny.preview();
    let m = &p.records()[0].snapshot.metrics;
    assert_eq!(m.supported_shape_windows, m.spectral_windows);
    assert_eq!(m.shape_comparisons, m.spectral_windows - 1);
}
