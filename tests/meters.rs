use aqua_pattern_core::{
    audition,
    meter_windows::{MeterReport, MeterWindows},
    meters::{self, MeterAnalysisConfig, TrackMeterAvailability},
    sample_asset,
    song::*,
    song_render::Engine,
    track_mix::TrackMix,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    asset: SampleAsset,
}
impl Fixture {
    fn new(rate: u32, channels: u16, frames: usize, sign: f32) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-meters-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("authored.wav");
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
        for i in 0..frames {
            for channel in 0..channels {
                let value = if sign.abs() > 1.0 {
                    sign
                } else {
                    sign * (0.025 + (i % 7) as f32 * 0.001) * if channel == 0 { 1.0 } else { -0.4 }
                };
                writer.write_sample(value).unwrap();
            }
        }
        writer.finalize().unwrap();
        Self {
            asset: sample_asset::import(&path).unwrap().asset,
            dir,
        }
    }
    fn song(&self) -> Song {
        audition::region_preview(
            &self.asset,
            &SampleRegion {
                name: "Authored".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: self.asset.frames,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap()
        .song()
        .clone()
    }
    fn legacy(&self) -> Song {
        let mut song = self.song();
        let inst = song.instruments.get_mut(&InstrumentId(1)).unwrap();
        inst.region = None;
        inst.sample = Some(self.asset.path.clone());
        song
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn bytes<T: serde::Serialize + ?Sized>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|v| v.to_bits()).collect()
}
fn track(song: &mut Song, id: u64, columns: u16) {
    song.tracks.insert(
        TrackId(id),
        Track {
            name: format!("T{id}"),
            note_columns: columns,
        },
    );
    if !song.track_order.contains(&TrackId(id)) {
        song.track_order.push(TrackId(id));
    }
}
fn note(song: &mut Song, t: u64, row: u32, column: u16, instrument: u64) {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(t))
        .or_default()
        .entry(row)
        .or_default()
        .insert(
            column,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(instrument)),
                velocity: Some(1.0),
                pan: Some(0.0),
                ..Default::default()
            },
        );
}
fn add_region(song: &mut Song, asset: &SampleAsset, id: u64) {
    song.samples.insert(SampleId(id), asset.clone());
    song.regions.insert(
        RegionId(id),
        SampleRegion {
            name: format!("R{id}"),
            sample: SampleId(id),
            start_frame: 0,
            end_frame: asset.frames,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        },
    );
    song.instruments.insert(
        InstrumentId(id),
        Instrument {
            name: format!("I{id}"),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: Some(RegionId(id)),
        },
    );
}
fn read(engine: &mut Engine, end: u64, partition: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut buffer = vec![0.0; partition * 2];
    while engine.position < end {
        let count = engine.block(&mut buffer, end).unwrap();
        out.extend_from_slice(&buffer[..count * 2]);
    }
    out
}
fn observed(song: &Song, end: u64, partition: usize) -> (Vec<f32>, Vec<MeterReport>) {
    let mut engine = Engine::new(song, None).unwrap();
    let mut windows = MeterWindows::new(engine.enable_meters()).unwrap();
    let mut reports = Vec::new();
    let mut out = Vec::new();
    let mut buffer = vec![0.0; partition * 2];
    while engine.position < end {
        let start = engine.position;
        let count = engine.block(&mut buffer, end).unwrap();
        let block = engine.meter_block().unwrap();
        assert_eq!(block.song_start(), start);
        assert_eq!(block.frames(), count);
        for (i, pcm) in buffer[..count * 2].as_chunks::<2>().0.iter().enumerate() {
            assert_eq!(
                block.master_sample(i).unwrap().map(f32::to_bits),
                pcm.map(f32::to_bits)
            );
        }
        assert!(block.master_sample(count).is_none());
        assert!(block.track_sample(64, 0).is_none());
        windows.push_block(block, 0, |r| reports.push(r)).unwrap();
        out.extend_from_slice(&buffer[..count * 2]);
    }
    windows.finish_partial(|r| reports.push(r)).unwrap();
    (out, reports)
}
fn cfg(start: u64, end: u64) -> MeterAnalysisConfig {
    MeterAnalysisConfig {
        version: 1,
        start_frame: start,
        end_frame: end,
    }
}

#[test]
fn neutral_enabled_disabled_pcm_and_actual_master_are_exact_at_native_rates() {
    for (rate, channels) in [(8000, 1), (44100, 2), (48000, 2), (96000, 1)] {
        let f = Fixture::new(rate, channels, 900, 1.0);
        let song = f.song();
        let before = bytes(&song);
        let mut plain = Engine::new(&song, None).unwrap();
        let manifest = bytes(&plain.manifest);
        let expected = read(&mut plain, 7200, 512);
        let mut reference = None;
        for partition in [1, 37, 512] {
            let (actual, reports) = observed(&song, 7200, partition);
            assert_eq!(bits(&actual), bits(&expected));
            if let Some(previous) = &reference {
                assert_eq!(&bytes(&reports), previous);
            } else {
                reference = Some(bytes(&reports));
            }
            for r in reports {
                assert_eq!(r.track_peaks(), &[Some(r.master_peak)]);
                assert_eq!(r.frames(), 2400);
                assert!(!r.partial);
            }
        }
        let mut engine = Engine::new(&song, None).unwrap();
        engine.enable_meters();
        assert_eq!(bytes(&engine.manifest), manifest);
        engine.enable_meters();
        let mut output = [0.0; 74];
        engine.block(&mut output, 7200).unwrap();
        assert!(engine.meter_block().is_some());
        engine.disable_meters();
        assert!(engine.meter_layout().is_none());
        assert!(engine.meter_block().is_none());
        assert_eq!(bytes(&song), before);
    }
}
#[test]
fn same_track_sums_before_peak_and_other_tracks_can_cancel_master() {
    let pos = Fixture::new(48000, 2, 5000, 1.0);
    let neg = Fixture::new(48000, 2, 5000, -1.0);
    let single = pos.song();
    let (single_pcm, _) = observed(&single, 2400, 37);
    let mut doubled = single.clone();
    track(&mut doubled, 1, 2);
    note(&mut doubled, 1, 0, 1, 1);
    let (double_pcm, reports) = observed(&doubled, 2400, 37);
    for (actual, v) in double_pcm.iter().zip(&single_pcm) {
        assert_eq!(actual.to_bits(), (0.0 + v + v).to_bits());
    }
    assert_eq!(reports[0].track_peaks(), &[Some(reports[0].master_peak)]);
    assert!(reports[0].master_peak[0] > 0.03);
    let mut cancelling = single;
    add_region(&mut cancelling, &neg.asset, 2);
    track(&mut cancelling, 2, 1);
    note(&mut cancelling, 2, 0, 0, 2);
    let (pcm, reports) = observed(&cancelling, 2400, 37);
    assert!(pcm.iter().all(|v| *v == 0.0));
    assert_eq!(reports[0].master_peak, [0.0; 2]);
    assert_eq!(reports[0].track_peaks()[0], reports[0].track_peaks()[1]);
    assert!(reports[0].track_peaks()[0].unwrap()[0] > 0.0);
}
#[test]
fn shared_legacy_marks_entire_track_unavailable_including_controlled_subtotal() {
    let f = Fixture::new(48000, 2, 12000, 1.0);
    let mut song = f.legacy();
    track(&mut song, 2, 1);
    note(&mut song, 2, 0, 0, 1);
    add_region(&mut song, &f.asset, 2);
    track(&mut song, 1, 2);
    note(&mut song, 1, 0, 1, 2);
    track(&mut song, 3, 1);
    note(&mut song, 3, 0, 0, 2);
    let mut engine = Engine::new(&song, None).unwrap();
    let layout = engine.enable_meters().clone();
    for row in &layout.tracks[..2] {
        assert_eq!(
            row.availability,
            TrackMeterAvailability::SharedLegacy {
                instruments: vec![InstrumentId(1)]
            }
        );
    }
    assert_eq!(
        layout.tracks[2].availability,
        TrackMeterAvailability::Available
    );
    let (pcm, reports) = observed(&song, 2400, 37);
    assert_eq!(
        bits(&pcm),
        bits(&read(&mut Engine::new(&song, None).unwrap(), 2400, 512))
    );
    assert_eq!(&reports[0].track_peaks()[..2], &[None, None]);
    assert!(reports[0].track_peaks()[2].unwrap()[0] > 0.0);
    let unique = f.legacy();
    let (_, reports) = observed(&unique, 2400, 37);
    assert_eq!(reports[0].track_peaks(), &[Some(reports[0].master_peak)]);
}
#[test]
fn scheduled_muted_tracks_are_measured_zero_but_unused_tracks_are_absent() {
    let f = Fixture::new(48000, 2, 5000, 1.0);
    let mut song = f.song();
    track(&mut song, 2, 1);
    song.track_mix.insert(
        TrackId(1),
        TrackMix {
            mute: true,
            ..Default::default()
        },
    );
    song.track_mix.insert(
        TrackId(2),
        TrackMix {
            gain: 0.5,
            ..Default::default()
        },
    );
    let mut e = Engine::new(&song, None).unwrap();
    assert_eq!(e.enable_meters().tracks.len(), 1);
    let (pcm, reports) = observed(&song, 2400, 37);
    assert!(pcm.iter().all(|x| *x == 0.0));
    assert_eq!(reports[0].track_peaks(), &[Some([0.0; 2])]);
}
#[test]
fn sixty_four_rows_and_neutral_over_limit_master_only_do_not_limit_playback() {
    let f = Fixture::new(48000, 1, 5000, 0.25);
    let mut song = f.song();
    for id in 2..=64 {
        track(&mut song, id, 1);
        note(&mut song, id, 0, 0, 1);
    }
    let mut engine = Engine::new(&song, None).unwrap();
    assert_eq!(engine.enable_meters().tracks.len(), 64);
    track(&mut song, 65, 1);
    note(&mut song, 65, 0, 0, 1);
    let mut engine = Engine::new(&song, None).unwrap();
    let layout = engine.enable_meters();
    assert!(layout.tracks.is_empty());
    assert!(layout.tracks_unavailable.as_ref().unwrap().contains("64"));
    let (pcm, reports) = observed(&song, 2400, 37);
    assert_eq!(
        bits(&pcm),
        bits(&read(&mut Engine::new(&song, None).unwrap(), 2400, 512))
    );
    assert!(reports[0].track_peaks().is_empty());
    assert!(reports[0].master_peak[0] > 0.0);
}
#[test]
fn error_end_and_seek_invalidate_old_observation_without_audio_policy_changes() {
    let f = Fixture::new(48000, 2, 20000, 1.0);
    let song = f.song();
    let mut engine = Engine::new(&song, None).unwrap();
    engine.enable_meters();
    let mut buf = [0.0; 1024];
    engine.block(&mut buf, 2400).unwrap();
    assert!(engine.meter_block().is_some());
    assert!(engine.block(&mut [], 2400).is_err());
    assert!(engine.meter_block().is_none());
    engine.block(&mut buf, 2400).unwrap();
    engine.cold_seek(0).unwrap();
    assert!(engine.meter_block().is_none());
    read(&mut engine, 2400, 37);
    assert!(engine.meter_block().is_some());
    assert_eq!(engine.block(&mut buf, 2400).unwrap(), 0);
    assert!(engine.meter_block().is_none());
    let mut loud = song;
    track(&mut loud, 1, 100);
    for col in 1..100 {
        note(&mut loud, 1, 0, col, 1);
    }
    let mut plain = Engine::new(&loud, None).unwrap();
    let mut metered = Engine::new(&loud, None).unwrap();
    metered.enable_meters();
    assert_eq!(
        plain.block(&mut buf, 2400).unwrap_err(),
        metered.block(&mut buf, 2400).unwrap_err()
    );
    assert!(metered.meter_block().is_none());
}
#[test]
fn meter_only_track_overflow_never_changes_finite_cancelled_master() {
    let pos = Fixture::new(48000, 1, 120000, 3.0e38);
    let neg = Fixture::new(48000, 1, 120000, -3.0e38);
    let mut song = pos.song();
    add_region(&mut song, &neg.asset, 2);
    track(&mut song, 1, 4);
    track(&mut song, 2, 4);
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    for row in 0..8 {
        let id = if row % 2 == 0 { 1 } else { 2 };
        note(&mut song, id, row, (row / 2) as u16, id);
    }
    song.track_mix = BTreeMap::from([
        (
            TrackId(1),
            TrackMix {
                gain: 0.0,
                ..Default::default()
            },
        ),
        (
            TrackId(2),
            TrackMix {
                gain: 0.0,
                ..Default::default()
            },
        ),
    ]);
    let mut plain = Engine::new(&song, None).unwrap();
    let mut metered = Engine::new(&song, None).unwrap();
    metered.enable_meters();
    assert_eq!(
        bits(&read(&mut plain, 48000, 512)),
        bits(&read(&mut metered, 48000, 37))
    );
    let target = BTreeMap::new();
    plain
        .apply_mix_snapshot(&plain.mix_layout().unwrap().snapshot(&target).unwrap())
        .unwrap();
    metered
        .apply_mix_snapshot(&metered.mix_layout().unwrap().snapshot(&target).unwrap())
        .unwrap();
    let mut windows = MeterWindows::new(metered.meter_layout().unwrap()).unwrap();
    let mut reports = vec![];
    let mut pcm = vec![];
    let mut buf = [0.0; 1024];
    while metered.position < 50400 {
        let n = metered.block(&mut buf, 50400).unwrap();
        pcm.extend_from_slice(&buf[..n * 2]);
        windows
            .push_block(metered.meter_block().unwrap(), 1, |r| reports.push(r))
            .unwrap();
    }
    assert_eq!(bits(&pcm), bits(&read(&mut plain, 50400, 512)));
    assert!(pcm.iter().all(|s| *s == 0.0));
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].master_peak, [0.0; 2]);
    assert_eq!(reports[0].track_peaks(), &[None, None]);
}
#[test]
fn live_ramp_and_cold_seek_observations_preserve_pcm_at_every_partition() {
    let f = Fixture::new(48000, 2, 20000, 1.0);
    let song = f.song();
    for partition in [1, 37, 512] {
        let mut plain = Engine::new(&song, None).unwrap();
        let mut metered = Engine::new(&song, None).unwrap();
        metered.enable_meters();
        let settings = BTreeMap::from([(
            TrackId(1),
            TrackMix {
                gain: 0.5,
                balance: -0.25,
                ..Default::default()
            },
        )]);
        plain
            .apply_mix_snapshot(&plain.mix_layout().unwrap().snapshot(&settings).unwrap())
            .unwrap();
        metered
            .apply_mix_snapshot(&metered.mix_layout().unwrap().snapshot(&settings).unwrap())
            .unwrap();
        assert_eq!(
            bits(&read(&mut plain, 33, partition)),
            bits(&read(&mut metered, 33, partition))
        );
        plain.cold_seek(0).unwrap();
        metered.cold_seek(0).unwrap();
        assert_eq!(
            bits(&read(&mut plain, 2400, partition)),
            bits(&read(&mut metered, 2400, partition))
        );
        let b = metered.meter_block().unwrap();
        for frame in 0..b.frames() {
            assert_eq!(b.track_sample(0, frame), b.master_sample(frame));
        }
    }
}
#[test]
fn offline_prepare_is_pure_strict_aligned_bounded_and_refuses_legacy_route() {
    let f = Fixture::new(48000, 2, 900, 1.0);
    let mut song = f.song();
    song.samples.get_mut(&SampleId(1)).unwrap().path = f.dir.join("missing.wav");
    assert!(meters::prepare(&song, cfg(0, 120000)).is_ok());
    for request in [
        cfg(1, 2400),
        cfg(0, 2401),
        cfg(2400, 2400),
        cfg(0, 120001),
        MeterAnalysisConfig {
            version: 2,
            ..cfg(0, 2400)
        },
    ] {
        assert!(meters::prepare(&song, request).is_err());
    }
    song.samples.get_mut(&SampleId(1)).unwrap().path = "relative.wav".into();
    assert!(meters::prepare(&song, cfg(0, 2400))
        .unwrap_err()
        .contains("absolute"));
    let legacy = f.legacy();
    assert!(legacy.samples.contains_key(&SampleId(1)));
    assert!(meters::prepare(&legacy, cfg(0, 2400))
        .unwrap_err()
        .contains("assign a region"));
}
#[test]
fn offline_range_retains_earlier_context_and_only_whole_selected_windows() {
    let f = Fixture::new(48000, 2, 20000, 1.0);
    let song = f.song();
    let before = bytes(&song);
    let all = meters::analyze(
        &meters::prepare(&song, cfg(0, 7200)).unwrap(),
        |_| Ok(()),
        || false,
    )
    .unwrap();
    let selected = meters::analyze(
        &meters::prepare(&song, cfg(2400, 7200)).unwrap(),
        |_| Ok(()),
        || false,
    )
    .unwrap();
    assert_eq!(bytes(&selected.reports), bytes(&all.reports[1..]));
    assert_eq!(selected.reports[0].emitted_start, 2400);
    assert!(selected.reports[0].master_peak[0] > 0.0);
    assert_eq!(selected.summary.processed_frames, 7200);
    let mut cold = Engine::new(&song, None).unwrap();
    cold.cold_seek(2400).unwrap();
    assert!(read(&mut cold, 7200, 512).iter().all(|x| *x == 0.0));
    assert_eq!(bytes(&song), before);
    assert_eq!(selected.verified_sources, 1);
}
#[test]
fn offline_natural_end_partial_is_exact_and_no_zero_padding() {
    let f = Fixture::new(48000, 2, 1000, 1.0);
    let mut song = f.song();
    song.bpm = 123.0;
    let engine = Engine::new(&song, None).unwrap();
    let total = engine.manifest.musical_frames + engine.manifest.tail_frames;
    assert!(total < 120000);
    assert_ne!(total % 2400, 0);
    let result = meters::analyze(
        &meters::prepare(&song, cfg(0, total)).unwrap(),
        |_| Ok(()),
        || false,
    )
    .unwrap();
    let last = result.reports.last().unwrap();
    assert!(last.partial);
    assert_eq!(last.frames(), total % 2400);
    assert_eq!(last.emitted_end_exclusive, total);
    assert_eq!(last.last_song_frame_end_exclusive, total);
    assert!(meters::prepare(&song, cfg(0, total - 1)).is_err());
}
#[test]
fn offline_authorizes_all_lexical_sources_before_io_and_preserves_bytes_on_failure() {
    let f = Fixture::new(48000, 2, 20000, 1.0);
    let g = Fixture::new(44100, 1, 1000, 1.0);
    let mut song = f.song();
    add_region(&mut song, &g.asset, 2);
    note(&mut song, 1, 12, 0, 2); // after report range, still part of full Engine context
    song.samples.get_mut(&SampleId(2)).unwrap().path = g.dir.join("missing-later.wav");
    let plan = meters::prepare(&song, cfg(0, 2400)).unwrap();
    assert_eq!(plan.source_samples().len(), 2);
    let mut authorized = vec![];
    assert!(meters::analyze(
        &plan,
        |p| {
            authorized.push(p.to_owned());
            Ok(())
        },
        || false
    )
    .is_err());
    assert_eq!(
        authorized,
        vec![f.asset.path.clone(), g.dir.join("missing-later.wav")]
    );
    assert_eq!(
        meters::analyze(&plan, |_| Err("policy first".into()), || false).unwrap_err(),
        "policy first"
    );
    let song = f.song();
    let before = bytes(&song);
    let source = std::fs::read(&f.asset.path).unwrap();
    let plan = meters::prepare(&song, cfg(0, 2400)).unwrap();
    assert!(
        meters::analyze(&plan, |_| panic!("cancel before IO"), || true)
            .unwrap_err()
            .contains("cancelled")
    );
    let mut changed = source.clone();
    let n = changed.len();
    changed[n - 1] ^= 1;
    std::fs::write(&f.asset.path, changed).unwrap();
    assert!(meters::analyze(&plan, |_| Ok(()), || false)
        .unwrap_err()
        .contains("SHA-256"));
    std::fs::write(&f.asset.path, &source).unwrap();
    assert!(meters::analyze(&plan, |_| Ok(()), || false).is_ok());
    let mut checks = 0;
    assert!(meters::analyze(
        &plan,
        |_| Ok(()),
        || {
            checks += 1;
            checks == 4
        }
    )
    .unwrap_err()
    .contains("cancelled"));
    assert_eq!(bytes(&song), before);
    assert_eq!(std::fs::read(&f.asset.path).unwrap(), source);
}
#[test]
fn offline_decoded_budget_and_full_context_settings_validate_before_io() {
    let f = Fixture::new(48000, 1, 10, 1.0);
    let mut song = f.song();
    song.samples.get_mut(&SampleId(1)).unwrap().frames = sample_asset::MAX_SAMPLE_VALUES as u64 - 1;
    let plan = meters::prepare(&song, cfg(0, 2400)).unwrap();
    assert_eq!(
        plan.summary().source_sample_values,
        sample_asset::MAX_SAMPLE_VALUES as u64
    );
    let mut second = f.asset.clone();
    second.sha256 = "a".repeat(64);
    add_region(&mut song, &second, 2);
    note(&mut song, 1, 12, 0, 2);
    assert!(meters::prepare(&song, cfg(0, 2400))
        .unwrap_err()
        .contains("aggregate decoded"));
    let mut unchanged = f.song();
    unchanged.instruments.insert(
        InstrumentId(99),
        Instrument {
            name: "Unused legacy".into(),
            kind: InstrumentKind::Pitched,
            sample: Some("missing-unused.wav".into()),
            region: None,
        },
    );
    assert!(meters::analyze(
        &meters::prepare(&unchanged, cfg(0, 2400)).unwrap(),
        |_| Ok(()),
        || false
    )
    .is_ok());
}
#[test]
fn offline_clipping_returns_no_partial_result_and_report_is_bounded() {
    let f = Fixture::new(48000, 1, 20000, 1.0);
    let mut song = f.song();
    track(&mut song, 1, 100);
    for col in 1..100 {
        note(&mut song, 1, 1, col, 1);
    }
    let plan = meters::prepare(&song, cfg(0, 9600)).unwrap();
    assert!(meters::analyze(&plan, |_| Ok(()), || false)
        .unwrap_err()
        .contains("clipping"));
    let result = meters::analyze(
        &meters::prepare(&f.song(), cfg(0, 120000)).unwrap(),
        |_| Ok(()),
        || false,
    )
    .unwrap();
    assert_eq!(result.reports.len(), 50);
    assert!(bytes(&result).len() < meters::MAX_REPORT_BYTES);
    assert!(result.reports.iter().all(|r| !r.partial
        && r.wrap_count == 0
        && r.first_mix_revision == 0
        && r.last_mix_revision == 0));
}
