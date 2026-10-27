use aqua_pattern_core::{
    onset_service, onset_v2, sample_asset,
    song::*,
    tempo::{self, CachedRequest, EvidenceKind, MarkerRequest, PointBasis, TempoConfig},
};
use std::sync::atomic::{AtomicU64, Ordering};

fn fixture(rate: u32, frames: u64, points: &[u64]) -> Song {
    let mut s = example_song();
    s.samples.insert(
        SampleId(1),
        SampleAsset {
            path: "/absent-tempo/no-io.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: rate,
            channels: 1,
            frames,
        },
    );
    let mut markers = vec![0];
    markers.extend(points.iter().copied().filter(|&p| p != 0 && p != frames));
    markers.push(frames);
    s.marker_sets.insert(
        MarkerSetId(1),
        MarkerSet {
            name: "Declared points".into(),
            sample: SampleId(1),
            source_bpm: None,
            markers,
            locked_frames: vec![],
        },
    );
    s.validate().unwrap();
    s
}
fn request(song: &Song) -> MarkerRequest {
    MarkerRequest {
        marker_set: MarkerSetId(1),
        start_index: 0,
        end_index: (song.marker_sets[&MarkerSetId(1)].markers.len() - 1) as u16,
        include_start: false,
    }
}
fn fixed(b: u16) -> TempoConfig {
    TempoConfig {
        version: 1,
        min_bpm_tenths: b,
        max_bpm_tenths: b,
    }
}
fn periodic(rate: u32, b: u16, phase: u64, n: usize) -> Vec<u64> {
    (0..n)
        .map(|k| {
            phase
                + ((k as u128 * u128::from(rate) * 600 + u128::from(b) / 2) / u128::from(b)) as u64
        })
        .collect()
}
#[test]
fn marker_only_is_pure_deterministic_and_retains_harmonic_ambiguity() {
    let points = periodic(48000, 700, 17, 12);
    let s = fixture(48000, 600000, &points);
    let a = tempo::analyze_markers(&s, request(&s), TempoConfig::default()).unwrap();
    let b = tempo::analyze_markers(&s, request(&s), TempoConfig::default()).unwrap();
    assert_eq!(a.id(), b.id());
    assert_eq!(a.analysis(), b.analysis());
    assert_eq!(a.base(), &s);
    assert_eq!(a.points(), points);
    let c = a
        .analysis()
        .candidates
        .iter()
        .find(|c| c.bpm_tenths == 700)
        .unwrap();
    assert!(c.phase_fit > 0.99);
    assert!(!c.aliases.is_empty());
    assert!(c.aliases.iter().any(|a| a.bpm_tenths == 1400));
    assert!(a.analysis().candidates.len() <= 8);
    assert!(a
        .analysis()
        .binding
        .identity_basis
        .contains("no source file"));
}
#[test]
fn source_bpm_names_and_paths_do_not_influence_scores() {
    let mut s = fixture(48000, 300000, &periodic(48000, 1200, 100, 10));
    let a = tempo::analyze_markers(&s, request(&s), TempoConfig::default()).unwrap();
    let set = s.marker_sets.get_mut(&MarkerSetId(1)).unwrap();
    set.source_bpm = Some(20.0);
    set.name = "400BPM sure".into();
    s.samples.get_mut(&SampleId(1)).unwrap().path = "/missing/200bpm.wav".into();
    let b = tempo::analyze_markers(&s, request(&s), TempoConfig::default()).unwrap();
    assert_eq!(a.analysis().candidates, b.analysis().candidates);
    assert_ne!(a.id(), b.id());
}
#[test]
fn exact_rational_pulses_all_rates_fractional_bpm_and_window_prefixes() {
    for rate in [8000, 44100, 48000, 96000, 384000] {
        for bpm in [200, 700, 1373, 4000] {
            let prefix = 101;
            let points = periodic(rate, bpm, prefix + 17, 8);
            let end = points[7] + u64::from(rate) / 2;
            let mut all = vec![prefix];
            all.extend(&points);
            let s = fixture(rate, end, &all);
            let mut r = request(&s);
            r.start_index = 1;
            let a = tempo::analyze_markers(&s, r, fixed(bpm)).unwrap();
            let c = &a.analysis().candidates[0];
            assert_eq!(c.bpm_tenths, bpm);
            assert!(c.phase_fit > 0.99);
            let page = a.pulse_page(0, 0, 256).unwrap();
            assert_eq!(page.total, c.pulse_count);
            for (k, &f) in page.frames.iter().enumerate() {
                let num = u128::from(c.phase_numerator) + k as u128 * u128::from(rate) * 600 * 128;
                let den = u128::from(c.phase_denominator);
                let expected = prefix + (num / den + u128::from(num % den * 2 >= den)) as u64;
                assert_eq!(f, expected);
                assert!(f < end);
            }
        }
    }
}
#[test]
fn window_end_excluded_start_explicit_and_empty_insufficient() {
    let s = fixture(48000, 96000, &[24000, 48000, 72000]);
    let mut r = request(&s);
    let a = tempo::analyze_markers(&s, r, TempoConfig::default()).unwrap();
    assert_eq!(a.points(), [24000, 48000, 72000]);
    assert!(a.analysis().candidates.is_empty());
    assert!(a.analysis().insufficient_reason.is_some());
    assert!(a.use_bpm(MarkerSetId(1), 0).is_err());
    r.include_start = true;
    let a = tempo::analyze_markers(&s, r, fixed(1200)).unwrap();
    assert_eq!(a.points(), [0, 24000, 48000, 72000]);
    assert_eq!(a.analysis().candidates[0].bpm_tenths, 1200);
    r.start_index = 1;
    r.end_index = 2;
    r.include_start = false;
    let empty = tempo::analyze_markers(&s, r, TempoConfig::default()).unwrap();
    assert!(empty.points().is_empty());
}
#[test]
fn use_bpm_changes_only_target_metadata_with_full_base_guard() {
    let mut s = fixture(48000, 300000, &periodic(48000, 1200, 17, 10));
    s.marker_sets
        .get_mut(&MarkerSetId(1))
        .unwrap()
        .locked_frames = vec![17];
    let mut target = s.marker_sets[&MarkerSetId(1)].clone();
    target.name = "Other target same source".into();
    s.marker_sets.insert(MarkerSetId(2), target);
    let a = tempo::analyze_markers(&s, request(&s), fixed(1200)).unwrap();
    let p = a.use_bpm(MarkerSetId(2), 0).unwrap();
    let mut expected = s.clone();
    expected
        .marker_sets
        .get_mut(&MarkerSetId(2))
        .unwrap()
        .source_bpm = Some(120.0);
    assert_eq!(p.candidate(), &expected);
    assert_eq!(p.commit(&s).unwrap(), expected);
    let mut changed = s.clone();
    changed.title.push('x');
    assert!(p.commit(&changed).unwrap_err().contains("stale"));
    let a2 = tempo::analyze_markers(&expected, request(&expected), fixed(1200)).unwrap();
    assert_eq!(
        a2.use_bpm(MarkerSetId(2), 0).unwrap().candidate(),
        &expected
    );
    let mut foreign = s.samples[&SampleId(1)].clone();
    foreign.sha256 = "b".repeat(64);
    s.samples.insert(SampleId(2), foreign);
    s.marker_sets.get_mut(&MarkerSetId(2)).unwrap().sample = SampleId(2);
    let a = tempo::analyze_markers(&s, request(&s), fixed(1200)).unwrap();
    assert!(a.use_bpm(MarkerSetId(2), 0).is_err());
}
#[test]
fn sparse_syncopated_and_near_coincident_points_report_support_not_probability() {
    let s = fixture(48000, 240000, &[17, 18, 24017, 72017, 120017, 168017]);
    let a = tempo::analyze_markers(&s, request(&s), fixed(1200)).unwrap();
    let c = &a.analysis().candidates[0];
    assert_eq!(c.multiple_points_per_pulse, 1);
    assert_eq!(c.occupied_pulses, 5);
    assert!(c.pulse_coverage < 0.75);
    assert!(c.mean_error_beats > 0.0);
    assert_ne!(c.strength, tempo::EvidenceStrength::Strong);
    assert!(a
        .analysis()
        .warnings
        .iter()
        .any(|w| w.contains("not full raw-audio")));
}
#[test]
fn no_in_range_votes_is_inspectable_without_guess_or_use() {
    let s = fixture(48000, 1000, &[1, 2, 3, 4, 5, 6]);
    let a = tempo::analyze_markers(&s, request(&s), TempoConfig::default()).unwrap();
    assert!(a.analysis().candidates.is_empty());
    assert!(a
        .analysis()
        .insufficient_reason
        .as_ref()
        .unwrap()
        .contains("No in-range"));
    assert!(a.pulse_page(0, 0, 10).is_err());
}
#[test]
fn local_rate_window_version_and_page_limits_are_explicit() {
    let mut s = fixture(48000, 300000, &periodic(48000, 1200, 17, 10));
    let r = request(&s);
    for c in [
        TempoConfig {
            version: 2,
            ..Default::default()
        },
        fixed(199),
        fixed(4001),
        TempoConfig {
            version: 1,
            min_bpm_tenths: 2000,
            max_bpm_tenths: 1000,
        },
    ] {
        assert!(tempo::analyze_markers(&s, r, c).is_err());
    }
    let a = tempo::analyze_markers(&s, r, TempoConfig::default()).unwrap();
    assert!(a.pulse_page(0, 0, 0).is_err());
    assert!(a.pulse_page(0, 0, 257).is_err());
    assert!(a.pulse_page(0, usize::MAX, 1).is_err());
    s.samples.get_mut(&SampleId(1)).unwrap().sample_rate = 1;
    assert!(tempo::analyze_markers(&s, r, TempoConfig::default())
        .unwrap_err()
        .contains("8000"));
    let s = fixture(8000, 2400001, &[1, 8001, 16001, 24001]);
    assert!(
        tempo::analyze_markers(&s, request(&s), TempoConfig::default())
            .unwrap_err()
            .contains("300 seconds")
    );
}
#[test]
fn historical_preview_has_no_persistent_analysis_schema() {
    let s = fixture(48000, 300000, &periodic(48000, 1200, 17, 10));
    let before = serde_json::to_vec(&s).unwrap();
    let a = tempo::analyze_markers(&s, request(&s), fixed(1200)).unwrap();
    assert_eq!(serde_json::to_vec(&s).unwrap(), before);
    let p = a.use_bpm(MarkerSetId(1), 0).unwrap();
    let bytes = serde_json::to_vec(p.candidate()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("timing-point-periodicity"));
    assert_eq!(
        serde_json::from_slice::<Song>(&bytes).unwrap(),
        *p.candidate()
    );
}
static NEXT: AtomicU64 = AtomicU64::new(0);
#[test]
fn cached_route_derives_complete_points_from_opaque_cache_and_exact_config_without_io() {
    let path = std::env::temp_dir().join(format!(
        "tempo-cache-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for f in 0..192000 {
        writer
            .write_sample(if f % 24000 == 2400 { 0.8f32 } else { 0.0 })
            .unwrap();
    }
    writer.finalize().unwrap();
    let imported = sample_asset::import(&path).unwrap();
    let config = onset_service::experimental_config(onset_v2::Preset::Balanced);
    let prepared = onset_service::prepare_verified(&imported.asset, &path, &config).unwrap();
    let mut s = fixture(48000, 192000, &[2400, 26400, 50400, 74400]);
    s.samples.insert(SampleId(1), imported.asset.clone());
    // Core cached mode is snapshot-only. A real adapter must authorize and verify before this call.
    std::fs::remove_file(&path).unwrap();
    assert!(sample_asset::verify_identity(&imported.asset, &path).is_err());
    for basis in [PointBasis::Peak, PointBasis::RefinedAttack] {
        let request = CachedRequest {
            start_frame: 0,
            end_frame: 192000,
            basis,
        };
        let a = tempo::analyze_cached(
            &s,
            SampleId(1),
            &prepared.cache,
            &config,
            request,
            TempoConfig::default(),
        )
        .unwrap();
        let expected: Vec<_> = match basis {
            PointBasis::Peak => prepared
                .result
                .analysis
                .detection
                .candidates
                .iter()
                .map(|x| x.peak_frame)
                .collect(),
            PointBasis::RefinedAttack => prepared
                .result
                .analysis
                .suggestions
                .iter()
                .map(|x| x.attack_frame)
                .collect(),
        };
        assert_eq!(a.points(), expected);
        assert!(matches!(
            a.analysis().binding.kind,
            EvidenceKind::CachedV2 { .. }
        ));
        assert!(a.analysis().binding.identity_basis.contains("snapshot"));
        let b = tempo::analyze_cached(
            &s,
            SampleId(1),
            &prepared.cache,
            &config,
            request,
            TempoConfig::default(),
        )
        .unwrap();
        assert_eq!(a.id(), b.id());
    }
    // A legal threshold/evidence repick changes effective config identity even
    // when the immutable extraction cache and Song revision stay the same.
    let mut repicked_config = config.clone();
    repicked_config.enable_flux = !repicked_config.enable_flux;
    let request = CachedRequest {
        start_frame: 0,
        end_frame: 192000,
        basis: PointBasis::Peak,
    };
    let original = tempo::analyze_cached(
        &s,
        SampleId(1),
        &prepared.cache,
        &config,
        request,
        TempoConfig::default(),
    )
    .unwrap();
    let repicked = tempo::analyze_cached(
        &s,
        SampleId(1),
        &prepared.cache,
        &repicked_config,
        request,
        TempoConfig::default(),
    )
    .unwrap();
    assert_ne!(original.id(), repicked.id());
    match (
        &original.analysis().binding.kind,
        &repicked.analysis().binding.kind,
    ) {
        (
            EvidenceKind::CachedV2 {
                extraction_key: a,
                onset_config_sha256: ac,
                ..
            },
            EvidenceKind::CachedV2 {
                extraction_key: b,
                onset_config_sha256: bc,
                ..
            },
        ) => {
            assert_eq!(a, b);
            assert_ne!(ac, bc);
        }
        _ => panic!("wrong evidence kind"),
    }
    let mut wrong = config.clone();
    wrong.minimum_rms *= 2.0;
    assert!(tempo::analyze_cached(
        &s,
        SampleId(1),
        &prepared.cache,
        &wrong,
        CachedRequest {
            start_frame: 0,
            end_frame: 192000,
            basis: PointBasis::Peak
        },
        TempoConfig::default()
    )
    .is_err());
    wrong = config;
    wrong.max_suggestions = 4097;
    assert!(tempo::analyze_cached(
        &s,
        SampleId(1),
        &prepared.cache,
        &wrong,
        CachedRequest {
            start_frame: 0,
            end_frame: 192000,
            basis: PointBasis::Peak
        },
        TempoConfig::default()
    )
    .unwrap_err()
    .contains("4096"));
}

#[test]
fn decimal_tenth_use_persists_existing_binary_clock_not_advisory_phase() {
    let s = fixture(48000, 300000, &periodic(48000, 1373, 17, 10));
    let a = tempo::analyze_markers(&s, request(&s), fixed(1373)).unwrap();
    let p = a.use_bpm(MarkerSetId(1), 0).unwrap();
    assert_eq!(
        p.candidate().marker_sets[&MarkerSetId(1)]
            .source_bpm
            .unwrap()
            .to_bits(),
        (1373.0f64 / 10.0).to_bits()
    );
    let loaded: Song = serde_json::from_slice(&serde_json::to_vec(p.candidate()).unwrap()).unwrap();
    assert_eq!(loaded, *p.candidate());
    assert!(p
        .summary()
        .warnings
        .iter()
        .any(|w| w.contains("f64 approximation")));
}

#[test]
fn translated_window_preserves_relative_hypotheses_and_exact_grid() {
    let points = periodic(44100, 1373, 117, 12);
    let end = 300000;
    let mut p = vec![100];
    p.extend(&points);
    let s = fixture(44100, end, &p);
    let mut r = request(&s);
    r.start_index = 1;
    let a = tempo::analyze_markers(&s, r, TempoConfig::default()).unwrap();
    let shift = 123456;
    let p: Vec<_> = p.iter().map(|&p| p + shift).collect();
    let s2 = fixture(44100, end + shift, &p);
    let mut r2 = request(&s2);
    r2.start_index = 1;
    let b = tempo::analyze_markers(&s2, r2, TempoConfig::default()).unwrap();
    assert_eq!(a.analysis().candidates, b.analysis().candidates);
    for i in 0..a.analysis().candidates.len() {
        let x = a.pulse_page(i, 0, 256).unwrap();
        let y = b.pulse_page(i, 0, 256).unwrap();
        assert_eq!(x.total, y.total);
        assert_eq!(
            x.frames.iter().map(|f| f + shift).collect::<Vec<_>>(),
            y.frames
        );
    }
}
#[test]
fn authored_irregular_points_return_only_finite_bounded_heuristic_components() {
    let s = fixture(
        48000,
        480000,
        &[
            23, 4051, 8703, 19301, 56003, 92013, 151047, 231731, 299983, 430001,
        ],
    );
    let a = tempo::analyze_markers(&s, request(&s), TempoConfig::default()).unwrap();
    assert!(!a.analysis().candidates.is_empty());
    for (i, c) in a.analysis().candidates.iter().enumerate() {
        assert!(c.score.is_finite());
        assert!((0.0..=1.0).contains(&c.score));
        assert!((0.0..=1.0).contains(&c.phase_fit));
        assert!((0.0..=1.0).contains(&c.pulse_coverage));
        assert!((0.0..=1.0).contains(&c.relative_interval_vote));
        assert!(c.mean_error_beats <= 0.5);
        assert!(c.occupied_pulses <= c.pulse_count);
        assert!(c.matched_points + c.outside_window_matches <= a.points().len());
        assert_eq!(
            c.multiple_points_per_pulse,
            c.matched_points - c.occupied_pulses
        );
        assert!(a.analysis().candidates[..i]
            .iter()
            .all(|o| o.bpm_tenths.abs_diff(c.bpm_tenths) >= 10));
        assert_ne!(c.strength, tempo::EvidenceStrength::Strong);
    }
}
