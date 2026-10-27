#![cfg(unix)]
use aqua_pattern_core::{
    resample::{self, Access, ResampleConfig, TailPolicy},
    sample_asset,
    song::*,
    song_render::Engine,
    RATE,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    song: Song,
}
fn note(instrument: u64) -> Cell {
    Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(instrument)),
        velocity: Some(1.),
        pan: Some(0.),
        ..Default::default()
    }
}
fn put(song: &mut Song, pattern: u64, track: u64, row: u32, column: u16, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(pattern))
        .unwrap()
        .tracks
        .entry(TrackId(track))
        .or_default()
        .entry(row)
        .or_default()
        .insert(column, cell);
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-resample-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("source.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..4096 {
            wav.write_sample(if i % 2 == 0 { 1.8f32 } else { -1.8 })
                .unwrap();
            wav.write_sample((i % 17) as f32 * 0.01 - 0.08).unwrap();
        }
        wav.finalize().unwrap();
        let mut song = example_song();
        song.bpm = 120.;
        song.lines_per_beat = 100;
        song.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                name: "Print".into(),
                rows: 4,
                tracks: Default::default(),
                track_aliases: Default::default(),
                generation: None,
            },
        )]);
        song.order = vec![Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        }];
        song.register_sample(SampleId(1), sample_asset::import(&path).unwrap().asset)
            .unwrap();
        song.upsert_region(
            RegionId(1),
            SampleRegion {
                name: "Input".into(),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 4096,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        song.assign_region(InstrumentId(1), Some(RegionId(1)))
            .unwrap();
        put(&mut song, 1, 1, 0, 0, note(1));
        Self { dir, song }
    }
    fn config(&self, name: &str, tail: TailPolicy) -> ResampleConfig {
        ResampleConfig {
            version: 1,
            occurrence: 0,
            name: "Printed".into(),
            root_note: 60,
            tail,
            output: self.dir.join(name),
        }
    }
    fn print(&self, name: &str, tail: TailPolicy) -> resample::ResamplePreview {
        let p = resample::prepare(&self.song, self.config(name, tail)).unwrap();
        resample::render(&p, |_| Ok(()), || false).unwrap()
    }
    fn temp_count(&self) -> usize {
        std::fs::read_dir(&self.dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".leviathan-resample-")
            })
            .count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn pcm(song: &Song, end: u64, block: usize) -> Vec<f32> {
    let mut engine = Engine::new(song, None).unwrap();
    let mut out = Vec::new();
    let mut b = vec![0.; block * 2];
    while engine.position < end {
        let n = engine.block(&mut b, end).unwrap();
        out.extend_from_slice(&b[..n * 2]);
    }
    out
}
fn wav(path: &std::path::Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect()
}
fn replay(result: &resample::ResamplePreview, rows: u32) -> Song {
    let mut song = result.candidate().clone();
    song.patterns.get_mut(&PatternId(1)).unwrap().rows = rows;
    song.patterns.get_mut(&PatternId(1)).unwrap().tracks.clear();
    put(&mut song, 1, 1, 0, 0, note(result.summary().instrument.0));
    song
}
#[test]
fn prepare_is_pure_and_checked_without_source_or_output_parent_io() {
    let mut f = Fixture::new();
    f.song.samples.get_mut(&SampleId(1)).unwrap().path = f.dir.join("missing.wav");
    let mut config = f.config("new.wav", TailPolicy::ReleaseTail);
    config.output = f.dir.join("missing-parent/new.wav");
    let before = f.song.clone();
    let p = resample::prepare(&f.song, config).unwrap();
    assert_eq!(p.summary().printed_frames, 24960);
    assert_eq!(p.base(), &before);
    assert!(resample::render(&p, |_| Ok(()), || false)
        .unwrap_err()
        .contains("parent"));
    assert_eq!(f.song, before);
    assert_eq!(f.temp_count(), 0);
}
#[test]
fn pre_master_float_peaks_and_two_neutral_generations_are_exact() {
    let f = Fixture::new();
    let original = pcm(&f.song, 960, 512);
    let first = f.print("first.wav", TailPolicy::MusicalOnly);
    assert!(first.receipt().asset_peak > 1.);
    assert!(first.receipt().post_master_peak <= 1.);
    assert_eq!(
        wav(&first.asset().path),
        original.iter().map(|x| x * 2.).collect::<Vec<_>>()
    );
    let imported = sample_asset::import(&first.asset().path).unwrap();
    assert!(imported.buffer.buffer().iter().any(|x| x.abs() > 1.));
    let replay1 = replay(&first, 4);
    for block in [1, 37, 512] {
        assert_eq!(pcm(&replay1, 960, block), original);
    }
    let config = ResampleConfig {
        output: f.dir.join("second.wav"),
        ..first.config().clone()
    };
    let p = resample::prepare(&replay1, config).unwrap();
    let second = resample::render(&p, |_| Ok(()), || false).unwrap();
    assert_eq!(
        std::fs::read(&first.asset().path).unwrap(),
        std::fs::read(&second.asset().path).unwrap()
    );
    assert_eq!(pcm(&replay(&second, 4), 960, 37), original);
    sample_asset::verify_identity(
        &f.song.samples[&SampleId(1)],
        &f.song.samples[&SampleId(1)].path,
    )
    .unwrap();
}
#[test]
fn release_tail_is_exact_engine_tail_and_whole_printed_region() {
    let mut f = Fixture::new();
    f.song.regions.get_mut(&RegionId(1)).unwrap().forward_loop = Some(ForwardLoop {
        start_frame: 0,
        end_frame: 100,
    });
    f.song.regions.get_mut(&RegionId(1)).unwrap().amp_envelope = Some(AmpEnvelope {
        attack_ms: 0,
        decay_ms: 0,
        sustain_percent: 100,
        release_ms: 500,
    });
    let expected = pcm(&f.song, 24960, 512);
    let r = f.print("tail.wav", TailPolicy::ReleaseTail);
    assert_eq!(r.summary().musical_frames, 960);
    assert_eq!(r.summary().tail_frames, 24000);
    assert_eq!(r.asset().frames, 24960);
    assert_eq!(r.candidate().regions[&r.summary().region].end_frame, 24960);
    assert_eq!(
        wav(&r.asset().path),
        expected.iter().map(|x| x * 2.).collect::<Vec<_>>()
    );
    assert_eq!(pcm(&replay(&r, 104), 24960, 37), expected);
    assert_ne!(
        pcm(&replay(&r, 4), 24960, 37),
        expected,
        "early SongEnd releases the already baked tail again"
    );
    assert_eq!(
        r.candidate().regions[&r.summary().region].amp_envelope,
        None
    );
    assert_eq!(
        r.candidate().regions[&r.summary().region].forward_loop,
        None
    );
}
#[test]
fn cold_occurrence_is_not_crop_and_never_borrows_preceding_memory() {
    let mut f = Fixture::new();
    let mut second = f.song.patterns[&PatternId(1)].clone();
    second.tracks.clear();
    f.song.patterns.insert(PatternId(2), second);
    f.song.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: Default::default(),
        section: None,
    });
    put(
        &mut f.song,
        2,
        1,
        0,
        0,
        Cell {
            note: Note::Note(60),
            ..Default::default()
        },
    );
    let mut config = f.config("cold.wav", TailPolicy::MusicalOnly);
    config.occurrence = 1;
    assert!(resample::prepare(&f.song, config.clone())
        .unwrap_err()
        .contains("no selected instrument"));
    put(&mut f.song, 2, 1, 0, 0, note(1));
    let plan = resample::prepare(&f.song, config).unwrap();
    assert_eq!(plan.summary().starts, 1);
    let mut isolated = f.song.clone();
    isolated.order = vec![f.song.order[1].clone()];
    let r = resample::render(&plan, |_| Ok(()), || false).unwrap();
    assert_eq!(
        wav(&r.asset().path),
        pcm(&isolated, 960, 512)
            .iter()
            .map(|x| x * 2.)
            .collect::<Vec<_>>()
    );
    assert_eq!(r.candidate().order, f.song.order);
    assert_eq!(r.candidate().patterns, f.song.patterns);
}
#[test]
fn alias_all_tracks_and_all_muted_silence_use_same_engine() {
    let mut f = Fixture::new();
    f.song
        .set_track_alias(
            PatternId(1),
            TrackId(2),
            Some(TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            }),
        )
        .unwrap();
    // Mute both actual and alias tracks; no selected source is read.
    f.song.order[0]
        .muted_tracks
        .extend([TrackId(1), TrackId(2)]);
    std::fs::remove_file(&f.song.samples[&SampleId(1)].path).unwrap();
    let p = resample::prepare(&f.song, f.config("silent.wav", TailPolicy::MusicalOnly)).unwrap();
    assert_eq!(p.summary().starts, 0);
    assert!(p.summary().source_samples.is_empty());
    let r = resample::render(
        &p,
        |a| {
            assert!(!matches!(a, Access::ReadSource(_)));
            Ok(())
        },
        || false,
    )
    .unwrap();
    assert_eq!(wav(&r.asset().path), vec![0.; 1920]);
}
#[test]
fn registered_keyzone_and_phrase_are_printed_without_remapping() {
    let mut f = Fixture::new();
    f.song
        .upsert_multisample(
            MultisampleId(1),
            MultisampleInstrument {
                name: "Map".into(),
                zones: BTreeMap::from([(
                    KeyZoneId(9),
                    KeyZone {
                        name: "zone".into(),
                        note_min: 0,
                        note_max: 127,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(1),
                        root_note: 72,
                    },
                )]),
            },
        )
        .unwrap();
    f.song
        .assign_multisample(InstrumentId(1), Some(MultisampleId(1)))
        .unwrap();
    f.song.phrases.insert(
        PhraseId(1),
        Phrase {
            name: "phrase".into(),
            instrument: InstrumentId(1),
            rows: 8,
            lines_per_beat: 100,
            note_columns: 1,
            root_note: 60,
            cells: BTreeMap::from([
                (
                    0,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Note(72),
                            velocity: Some(0.5),
                            ..Default::default()
                        },
                    )]),
                ),
                (
                    6,
                    BTreeMap::from([(
                        0,
                        Cell {
                            note: Note::Note(72),
                            ..Default::default()
                        },
                    )]),
                ),
            ]),
        },
    );
    let mut trigger = note(1);
    trigger.phrase = Some(PhraseTrigger {
        phrase: PhraseId(1),
        cycles: 1,
    });
    put(&mut f.song, 1, 1, 0, 0, trigger);
    let p =
        resample::prepare(&f.song, f.config("map-phrase.wav", TailPolicy::ReleaseTail)).unwrap();
    assert_eq!(p.summary().starts, 1); // later child beyond selected end does not start
    let expected = pcm(&f.song, p.summary().printed_frames, 512);
    let r = resample::render(&p, |_| Ok(()), || false).unwrap();
    assert_eq!(
        wav(&r.asset().path),
        expected.iter().map(|x| x * 2.).collect::<Vec<_>>()
    );
    assert_eq!(r.receipt().source_identities.len(), 1);
}
#[test]
fn legacy_and_relative_used_sources_reject_but_unused_legacy_is_unchanged() {
    let mut f = Fixture::new();
    f.song.instruments.get_mut(&InstrumentId(2)).unwrap().sample =
        Some(PathBuf::from("unused.wav"));
    let plan = resample::prepare(&f.song, f.config("okay.wav", TailPolicy::MusicalOnly)).unwrap();
    assert_eq!(plan.base(), &f.song);
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().region = None;
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some(f.song.samples[&SampleId(1)].path.clone());
    assert!(
        resample::prepare(&f.song, f.config("legacy.wav", TailPolicy::MusicalOnly))
            .unwrap_err()
            .contains("import/register")
    );
    f.song
        .assign_region(InstrumentId(1), Some(RegionId(1)))
        .unwrap();
    f.song.samples.get_mut(&SampleId(1)).unwrap().path = PathBuf::from("relative.wav");
    assert!(resample::prepare(
        &f.song,
        f.config("relative-out.wav", TailPolicy::MusicalOnly)
    )
    .unwrap_err()
    .contains("absolute"));
}
#[test]
fn cancellation_before_and_during_work_removes_only_owned_temporary() {
    let f = Fixture::new();
    let p = resample::prepare(&f.song, f.config("cancel.wav", TailPolicy::ReleaseTail)).unwrap();
    assert!(resample::render(&p, |_| Ok(()), || true)
        .unwrap_err()
        .contains("cancelled"));
    let mut calls = 0;
    assert!(resample::render(
        &p,
        |_| Ok(()),
        || {
            calls += 1;
            calls >= 5
        }
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(!p.config().output.exists());
    assert_eq!(f.temp_count(), 0);
    assert!(f.song.samples[&SampleId(1)].path.exists());
}
#[test]
fn no_clobber_existing_and_racing_destination_never_removed() {
    let f = Fixture::new();
    let p = resample::prepare(&f.song, f.config("existing.wav", TailPolicy::MusicalOnly)).unwrap();
    std::fs::write(&p.config().output, b"existing").unwrap();
    assert!(resample::render(&p, |_| Ok(()), || false)
        .unwrap_err()
        .contains("already exists"));
    assert_eq!(std::fs::read(&p.config().output).unwrap(), b"existing");
    let p = resample::prepare(&f.song, f.config("racing.wav", TailPolicy::MusicalOnly)).unwrap();
    let mut publications = 0;
    let error = resample::render(
        &p,
        |a| {
            if let Access::PublishOutput(path) = a {
                publications += 1;
                if publications == 2 {
                    std::fs::write(path, b"competitor").unwrap();
                }
            }
            Ok(())
        },
        || false,
    )
    .unwrap_err();
    assert!(error.contains("already exists"));
    assert_eq!(std::fs::read(&p.config().output).unwrap(), b"competitor");
    assert_eq!(f.temp_count(), 0);
}
#[test]
fn authorization_is_before_decode_and_rechecked_before_publication() {
    let f = Fixture::new();
    let p = resample::prepare(&f.song, f.config("denied.wav", TailPolicy::MusicalOnly)).unwrap();
    let source = &f.song.samples[&SampleId(1)].path;
    std::fs::write(source, b"malformed").unwrap();
    assert_eq!(
        resample::render(
            &p,
            |a| match a {
                Access::ReadSource(path) => {
                    assert_eq!(path, source);
                    Err("denied source".into())
                }
                _ => Ok(()),
            },
            || false
        )
        .unwrap_err(),
        "denied source"
    );
    assert!(!p.config().output.exists());
    assert_eq!(f.temp_count(), 0);
    let g = Fixture::new();
    let p = resample::prepare(
        &g.song,
        g.config("late-denied.wav", TailPolicy::MusicalOnly),
    )
    .unwrap();
    let mut n = 0;
    let e = resample::render(
        &p,
        |a| {
            if matches!(a, Access::PublishOutput(_)) {
                n += 1;
                if n == 2 {
                    return Err("late denial".into());
                }
            }
            Ok(())
        },
        || false,
    )
    .unwrap_err();
    assert_eq!(e, "late denial");
    assert_eq!(g.temp_count(), 0);
    assert!(!p.config().output.exists());
}
#[test]
fn published_artifact_survives_stale_and_verification_detects_tamper() {
    let f = Fixture::new();
    let r = f.print("durable.wav", TailPolicy::MusicalOnly);
    resample::verify_artifact(&r, |_| Ok(())).unwrap();
    let mut changed = f.song.clone();
    changed.bpm = 123.;
    assert!(r.commit(&changed).unwrap_err().contains("stale"));
    assert!(r.asset().path.exists());
    assert_eq!(r.commit(&f.song).unwrap(), *r.candidate());
    let mut data = std::fs::read(&r.asset().path).unwrap();
    let end = data.len() - 1;
    data[end] ^= 1;
    std::fs::write(&r.asset().path, &data).unwrap();
    assert!(resample::verify_artifact(&r, |_| Ok(()))
        .unwrap_err()
        .contains("SHA-256"));
    drop(r);
    assert!(f.dir.join("durable.wav").exists());
}
#[test]
fn deterministic_wav_no_companion_and_fresh_ids_preserve_every_old_definition() {
    let f = Fixture::new();
    let a = f.print("a.wav", TailPolicy::MusicalOnly);
    let b = f.print("b.wav", TailPolicy::MusicalOnly);
    assert_eq!(
        std::fs::read(&a.asset().path).unwrap(),
        std::fs::read(&b.asset().path).unwrap()
    );
    assert_eq!(a.receipt().sha256, b.receipt().sha256);
    assert!(!a.asset().path.with_extension("events.json").exists());
    assert_eq!(a.summary().sample, SampleId(2));
    assert_eq!(a.summary().region, RegionId(2));
    assert_eq!(a.summary().instrument, InstrumentId(5));
    let mut candidate = a.candidate().clone();
    candidate.samples.remove(&a.summary().sample);
    candidate.regions.remove(&a.summary().region);
    candidate.instruments.remove(&a.summary().instrument);
    assert_eq!(candidate, f.song);
    let json = serde_json::to_string(a.candidate()).unwrap();
    assert_eq!(serde_json::from_str::<Song>(&json).unwrap(), *a.candidate());
}
#[test]
fn preflight_ids_frames_and_voice_work_reject_before_any_output() {
    let mut f = Fixture::new();
    f.song.instruments.insert(
        InstrumentId(u64::MAX),
        Instrument {
            name: "Last".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: None,
        },
    );
    assert!(
        resample::prepare(&f.song, f.config("overflow.wav", TailPolicy::MusicalOnly))
            .unwrap_err()
            .contains("fresh instrument")
    );
    f.song.instruments.remove(&InstrumentId(u64::MAX));
    f.song.lines_per_beat = 1;
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 400;
    assert!(
        resample::prepare(&f.song, f.config("long.wav", TailPolicy::MusicalOnly))
            .unwrap_err()
            .contains("printed-frame")
    );
    f.song.lines_per_beat = 4;
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 256;
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 32;
    for row in 0..64 {
        for col in 0..32 {
            put(&mut f.song, 1, 1, row, col, note(1));
        }
    }
    assert!(
        resample::prepare(&f.song, f.config("work.wav", TailPolicy::MusicalOnly))
            .unwrap_err()
            .contains("voice-frames")
    );
    assert_eq!(f.temp_count(), 0);
}
#[test]
fn engine_clipping_or_changed_source_never_publishes_partial_wav() {
    let mut f = Fixture::new();
    f.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 2;
    put(&mut f.song, 1, 1, 0, 1, note(1));
    let p = resample::prepare(&f.song, f.config("clip.wav", TailPolicy::MusicalOnly)).unwrap();
    assert!(resample::render(&p, |_| Ok(()), || false).is_err());
    assert!(!p.config().output.exists());
    assert_eq!(f.temp_count(), 0);
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .remove(&1);
    let p = resample::prepare(&f.song, f.config("changed.wav", TailPolicy::MusicalOnly)).unwrap();
    std::fs::write(&f.song.samples[&SampleId(1)].path, b"changed").unwrap();
    assert!(resample::render(&p, |_| Ok(()), || false).is_err());
    assert!(!p.config().output.exists());
    assert_eq!(f.temp_count(), 0);
}
#[cfg(unix)]
#[test]
fn symlink_destination_parent_and_parent_replacement_reject() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(&f.song.samples[&SampleId(1)].path, f.dir.join("link.wav")).unwrap();
    let p = resample::prepare(&f.song, f.config("link.wav", TailPolicy::MusicalOnly)).unwrap();
    assert!(resample::render(&p, |_| Ok(()), || false).is_err());
    assert!(f
        .dir
        .join("link.wav")
        .symlink_metadata()
        .unwrap()
        .is_symlink());
    std::fs::create_dir(f.dir.join("real")).unwrap();
    symlink(f.dir.join("real"), f.dir.join("alias")).unwrap();
    let p = resample::prepare(&f.song, f.config("alias/out.wav", TailPolicy::MusicalOnly)).unwrap();
    assert!(resample::render(&p, |_| Ok(()), || false)
        .unwrap_err()
        .contains("canonical"));
    let p = resample::prepare(&f.song, f.config("real/out.wav", TailPolicy::MusicalOnly)).unwrap();
    let mut n = 0;
    let e = resample::render(
        &p,
        |a| {
            if matches!(a, Access::PublishOutput(_)) {
                n += 1;
                if n == 2 {
                    std::fs::rename(f.dir.join("real"), f.dir.join("moved")).unwrap();
                    std::fs::create_dir(f.dir.join("real")).unwrap();
                }
            }
            Ok(())
        },
        || false,
    )
    .unwrap_err();
    assert!(e.contains("parent changed"));
    assert!(!p.config().output.exists());
}

#[cfg(unix)]
#[test]
fn replaced_owned_temp_is_not_published_or_deleted_as_foreign_file() {
    let f = Fixture::new();
    let p = resample::prepare(
        &f.song,
        f.config("foreign-temp.wav", TailPolicy::MusicalOnly),
    )
    .unwrap();
    let mut n = 0;
    let mut foreign = None;
    let e = resample::render(
        &p,
        |a| {
            if matches!(a, Access::PublishOutput(_)) {
                n += 1;
                if n == 2 {
                    let path = std::fs::read_dir(&f.dir)
                        .unwrap()
                        .map(Result::unwrap)
                        .map(|e| e.path())
                        .find(|p| {
                            p.file_name()
                                .unwrap()
                                .to_string_lossy()
                                .starts_with(".leviathan-resample-")
                        })
                        .unwrap();
                    std::fs::remove_file(&path).unwrap();
                    std::fs::write(&path, b"foreign replacement").unwrap();
                    foreign = Some(path);
                }
            }
            Ok(())
        },
        || false,
    )
    .unwrap_err();
    assert!(e.contains("identity changed"));
    assert!(e.contains("owned temporary path"));
    assert_eq!(
        std::fs::read(foreign.unwrap()).unwrap(),
        b"foreign replacement"
    );
    assert!(!p.config().output.exists());
}

#[test]
fn fractional_prefix_is_rebased_not_cropped_and_selected_order_mute_is_kept() {
    let mut f = Fixture::new();
    f.song.bpm = 137.;
    f.song.lines_per_beat = 7;
    f.song.patterns.get_mut(&PatternId(1)).unwrap().rows = 1;
    let mut selected = f.song.patterns[&PatternId(1)].clone();
    selected.rows = 2;
    selected.tracks.clear();
    f.song.patterns.insert(PatternId(2), selected);
    let mut cell = note(1);
    cell.delay = Some(0.23703703703703694);
    put(&mut f.song, 2, 1, 0, 0, cell);
    f.song.order.push(Occurrence {
        pattern: PatternId(2),
        muted_tracks: Default::default(),
        section: None,
    });
    let mut config = f.config("fractional.wav", TailPolicy::MusicalOnly);
    config.occurrence = 1;
    let plan = resample::prepare(&f.song, config).unwrap();
    let mut isolated = f.song.clone();
    isolated.order = vec![f.song.order[1].clone()];
    let engine = Engine::new(&isolated, None).unwrap();
    assert_eq!(
        plan.summary().musical_frames,
        engine.manifest.musical_frames
    );
    let expected = pcm(&isolated, plan.summary().printed_frames, 37);
    let result = resample::render(&plan, |_| Ok(()), || false).unwrap();
    assert_eq!(
        wav(&result.asset().path),
        expected.iter().map(|v| v * 2.).collect::<Vec<_>>()
    );
    assert_eq!(result.candidate().patterns, f.song.patterns);
    assert_eq!(result.config().occurrence, 1);
}

#[test]
fn registration_preflight_checks_unused_sources_without_undoing_publication() {
    let mut f = Fixture::new();
    f.song.order[0].muted_tracks.insert(TrackId(1));
    std::fs::remove_file(&f.song.samples[&SampleId(1)].path).unwrap();
    let result = f.print("printed-silence.wav", TailPolicy::MusicalOnly);
    let error = resample::verify_artifact(&result, |_| Ok(())).unwrap_err();
    assert!(error.contains("missing/unreadable"));
    assert!(error.contains("published WAV retained"));
    assert_eq!(wav(&result.asset().path), vec![0.; 1920]);
    let g = Fixture::new();
    let printed = g.print("authorized.wav", TailPolicy::MusicalOnly);
    let source = &g.song.samples[&SampleId(1)].path;
    let error = resample::verify_artifact(&printed, |access| match access {
        Access::ReadSource(path) if path == source => {
            Err("policy denied unused/used candidate source".into())
        }
        _ => Ok(()),
    })
    .unwrap_err();
    assert!(error.contains("denied"));
    assert!(printed.asset().path.exists());
}

#[test]
fn cold_print_restarts_round_robin_instead_of_retaining_previous_occurrence_counter() {
    let mut f = Fixture::new();
    f.song.regions.get_mut(&RegionId(1)).unwrap().end_frame = 100;
    let mut region = f.song.regions[&RegionId(1)].clone();
    region.start_frame = 1;
    region.end_frame = 101;
    f.song.upsert_region(RegionId(2), region).unwrap();
    f.song
        .upsert_drum_kit(
            DrumKitId(1),
            DrumKitInstrument {
                name: "RR".into(),
                pads: BTreeMap::from([(
                    PadId(1),
                    DrumPad {
                        slot: 0,
                        name: "Pad".into(),
                        note: 60,
                        region: None,
                        velocity_zones: vec![VelocityZone {
                            min_velocity: 0,
                            max_velocity: 127,
                            alternatives: vec![
                                PadLayer {
                                    id: PadLayerId(17),
                                    region: RegionId(1),
                                },
                                PadLayer {
                                    id: PadLayerId(9),
                                    region: RegionId(2),
                                },
                            ],
                        }],
                        root_note: 60,
                        tune_semitones: 0,
                        gain: 1.,
                        pan: 0.,
                        mode: PadMode::Gated,
                        new_note: PadNewNote::Overlap,
                        choke_group: None,
                    },
                )]),
            },
        )
        .unwrap();
    f.song
        .assign_kit(InstrumentId(1), Some(DrumKitId(1)))
        .unwrap();
    f.song.order.push(f.song.order[0].clone());
    let full = Engine::new(&f.song, None).unwrap();
    assert_eq!(
        full.manifest
            .pad_voices
            .values()
            .map(|p| p.layer_selection.as_ref().unwrap().layer)
            .collect::<Vec<_>>(),
        vec![PadLayerId(17), PadLayerId(9)]
    );
    let full_pcm = pcm(&f.song, 1920, 512);
    assert!(full_pcm[1920] < 0.);
    let mut config = f.config("cold-rr.wav", TailPolicy::MusicalOnly);
    config.occurrence = 1;
    let plan = resample::prepare(&f.song, config).unwrap();
    let printed = resample::render(&plan, |_| Ok(()), || false).unwrap();
    assert!(wav(&printed.asset().path)[0] > 0.);
    assert_eq!(printed.summary().starts, 1);
    assert_eq!(printed.candidate().drum_kits, f.song.drum_kits);
}
