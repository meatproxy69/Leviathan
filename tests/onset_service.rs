use aqua_pattern_core::{
    onset_service::*,
    onset_v2::{self, Config, Preset},
    sample_asset,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    pcm: Vec<f32>,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "onset-service-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut pcm = vec![0.0; 22050];
        for at in [0, 4410, 11025, 22049] {
            pcm[at] = 0.7;
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for &sample in &pcm {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        Self { path, pcm }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
#[test]
fn verified_shared_service_matches_core_and_repick_exactly() {
    let fixture = Fixture::new();
    let imported = sample_asset::import(&fixture.path).unwrap();
    let c = experimental_config(Preset::Balanced);
    assert!(!c.enable_phase);
    assert!(
        Config::preset(Preset::Balanced).enable_phase,
        "Judged historical preset is explicit, not silently retuned"
    );
    let prepared = prepare_verified(&imported.asset, &fixture.path, &c).unwrap();
    assert_eq!(
        prepared.result.analysis,
        onset_v2::analyze(&fixture.pcm, 1, 44100, &c).unwrap()
    );
    assert_eq!(prepared.result.binding.source_sha256, imported.asset.sha256);
    assert_eq!(
        prepared.result.binding.source_frames,
        fixture.pcm.len() as u64
    );
    assert_eq!(
        prepared.cache.provenance().source_frames,
        fixture.pcm.len() as u64
    );
    assert_eq!(prepared.result.binding.extraction_key.len(), 64);
    assert!(prepared.cache.retained_bytes() <= MAX_RETAINED_BYTES);
    for preset in [Preset::Conservative, Preset::Balanced, Preset::GhostHunter] {
        for radius in [20.0, 80.0, 250.0] {
            let mut config = experimental_config(preset);
            config.median_radius_ms = radius;
            config.retain_envelope = true;
            let repicked = repick(&prepared.cache, &imported.asset, &config).unwrap();
            let fresh = prepare_verified(&imported.asset, &fixture.path, &config).unwrap();
            assert_eq!(repicked, fresh.result);
            assert_eq!(
                repicked.analysis,
                onset_v2::analyze(&fixture.pcm, 1, 44100, &config).unwrap()
            );
            assert_eq!(repicked.binding, prepared.result.binding);
            if config != c {
                assert_ne!(repicked.config_sha256, prepared.result.config_sha256);
            }
        }
    }
}
#[test]
fn cached_snapshot_does_not_claim_disk_reverification_and_rejects_wrong_authority() {
    let fixture = Fixture::new();
    let imported = sample_asset::import(&fixture.path).unwrap();
    let config = experimental_config(Preset::Balanced);
    let prepared = prepare_verified(&imported.asset, &fixture.path, &config).unwrap();
    let mut wrong = imported.asset.clone();
    wrong.sha256 = "0".repeat(64);
    assert!(repick(&prepared.cache, &wrong, &config).is_err());
    assert!(prepare_verified(&wrong, &fixture.path, &config).is_err());
    wrong = imported.asset.clone();
    wrong.frames -= 1;
    assert!(repick(&prepared.cache, &wrong, &config).is_err());
    assert!(prepare_verified(&wrong, &fixture.path, &config).is_err());
    let mut spectral = config.clone();
    spectral.minimum_rms *= 2.0;
    assert!(repick(&prepared.cache, &imported.asset, &spectral).is_err());
    let mut budget = config.clone();
    budget.max_frames = 10;
    assert!(repick(&prepared.cache, &imported.asset, &budget).is_err());
    // Disk replacement does not mutate the retained immutable snapshot.
    std::fs::write(&fixture.path, b"replaced source").unwrap();
    assert!(sample_asset::verify_identity(&imported.asset, &fixture.path).is_err());
    // The already verified immutable snapshot is deliberately still usable.
    // Protocol/UI guards, NOT repick, must reject external-file changes.
    assert_eq!(
        repick(&prepared.cache, &imported.asset, &config).unwrap(),
        prepared.result
    );
}
#[test]
fn inspectable_curves_are_bounded_exact_source_frames_and_never_mutate_raw_cache() {
    let fixture = Fixture::new();
    let imported = sample_asset::import(&fixture.path).unwrap();
    let prepared = prepare_verified(
        &imported.asset,
        &fixture.path,
        &experimental_config(Preset::Balanced),
    )
    .unwrap();
    let cache = &prepared.cache;
    let before = cache.raw_curves().to_vec();
    let page = cache.curve_page(3, 17).unwrap();
    assert_eq!(page.points.len(), 17);
    for (i, point) in page.points.iter().enumerate() {
        assert_eq!(point.evidence, before[3 + i]);
        assert_eq!(point.source_frame, (3 + i) as u64 * page.hop_frames as u64);
        assert!(point.source_frame < imported.asset.frames);
    }
    assert_eq!(cache.curve_page(0, 1).unwrap().points[0].source_frame, 0);
    assert!(cache.curve_page(0, 0).is_err());
    assert!(cache.curve_page(0, MAX_CURVE_PAGE + 1).is_err());
    assert!(cache.curve_page(usize::MAX, 1).is_err());
    assert!(cache.curve_page(before.len(), 1).unwrap().points.is_empty());
    assert_eq!(cache.raw_curves(), before);
    let mut config = experimental_config(Preset::GhostHunter);
    config.refinement.enabled = false;
    let picked = repick(cache, &imported.asset, &config).unwrap();
    assert!(picked
        .analysis
        .suggestions
        .iter()
        .all(|s| s.attack_frame == s.candidate.peak_frame));
    assert_eq!(cache.raw_curves(), before);
}

#[test]
fn all_extraction_fields_invalidate_cache_and_tighter_budgets_reject() {
    let fixture = Fixture::new();
    let imported = sample_asset::import(&fixture.path).unwrap();
    let c = experimental_config(Preset::Balanced);
    let prepared = prepare_verified(&imported.asset, &fixture.path, &c).unwrap();
    for field in 0..5 {
        let mut changed = c.clone();
        match field {
            0 => changed.stereo = onset_v2::StereoMode::LeftRight,
            1 => changed.bands[1].low_hz += 1.0,
            2 => changed.fast_hop_at_44100 *= 2,
            3 => changed.minimum_rms *= 2.0,
            _ => changed.max_filter_bins = 5,
        }
        assert!(repick(&prepared.cache, &imported.asset, &changed).is_err());
    }
    for field in 0..4 {
        let mut changed = c.clone();
        match field {
            0 => changed.max_frames = 1,
            1 => changed.max_fft_size = 256,
            2 => changed.max_work_units = 1,
            _ => changed.max_working_bytes = 1,
        }
        assert!(repick(&prepared.cache, &imported.asset, &changed).is_err());
    }
    for field in 0..3 {
        let mut asset = imported.asset.clone();
        match field {
            0 => asset.sample_rate = 48000,
            1 => asset.channels = 2,
            _ => asset.frames = u64::MAX,
        }
        assert!(prepare_verified(&asset, &fixture.path, &c).is_err());
        assert!(repick(&prepared.cache, &asset, &c).is_err());
    }
}
