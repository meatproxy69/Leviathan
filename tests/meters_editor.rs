//! Meter preparation, offline analysis and observation never become Editor edits.
use aqua_pattern_core::{
    audition,
    cell_blocks::RectScope,
    editor::{Editor, Field},
    meter_windows::MeterWindows,
    meters::{self, MeterAnalysisConfig},
    project::{self, SaveMode},
    sample_asset,
    song::*,
    song_render::Engine,
    track_mix::{TrackMixConfig, TrackMixSetting},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    asset: SampleAsset,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-meter-editor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("authored.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..6_000 {
            let value = if frame % 7 == 0 {
                -0.0
            } else {
                (frame % 17) as f32 / 1024.0
            };
            writer.write_sample(value).unwrap();
            writer.write_sample(-value * 0.5).unwrap();
        }
        writer.finalize().unwrap();
        let asset = sample_asset::import(&path).unwrap().asset;
        Self { dir, asset }
    }
    fn song(&self, mix: bool) -> Song {
        let region = SampleRegion {
            name: "Authored meter input".into(),
            sample: SampleId(1),
            start_frame: 0,
            end_frame: self.asset.frames,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        };
        let mut song = audition::region_preview(&self.asset, &region)
            .unwrap()
            .song()
            .clone();
        let cell = song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .get_mut(&0)
            .unwrap()
            .get_mut(&0)
            .unwrap();
        cell.pan = Some(-0.0);
        cell.delay = Some(-0.0);
        if mix {
            song.set_track_mix(TrackMixConfig {
                version: 1,
                settings: vec![TrackMixSetting {
                    track: TrackId(1),
                    gain: 1.0,
                    balance: -0.0,
                    mute: false,
                    solo: false,
                }],
            })
            .unwrap();
        }
        song.validate().unwrap();
        song
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn config() -> MeterAnalysisConfig {
    MeterAnalysisConfig {
        version: 1,
        start_frame: 0,
        end_frame: 4_800,
    }
}
fn editor(song: Song) -> Editor {
    let mut editor = Editor::default();
    editor.replace_document(song).unwrap();
    editor.edit_mode = true;
    let mut renamed = editor.song.clone();
    renamed.title = "Read-only meter observation".into();
    editor.apply_document_edit(renamed).unwrap();
    editor.mark_saved();
    editor.set_timing(123.0, 4).unwrap();
    editor.undo();
    assert!(editor.can_undo() && editor.can_redo());
    editor.row = 3;
    editor.column = 0;
    editor.field = Field::Pan;
    editor.edit_mode = false;
    editor
}
#[derive(Debug, PartialEq)]
struct State {
    song: Vec<u8>,
    history: String,
    revision: u64,
    occurrence: usize,
    track: TrackId,
    row: u32,
    column: u16,
    field: Field,
    edit_mode: bool,
    dirty: bool,
    mix_revision: Option<u64>,
}
fn state(e: &Editor) -> State {
    State {
        song: bytes(&e.song),
        history: format!("{:?}", e.history),
        revision: e.revision(),
        occurrence: e.occurrence,
        track: e.track,
        row: e.row,
        column: e.column,
        field: e.field,
        edit_mode: e.edit_mode,
        dirty: e.dirty,
        mix_revision: e.track_mix_change_revision(),
    }
}
fn clipboard(e: &Editor) -> aqua_pattern_core::editor::DocumentClipboard {
    let copy = e
        .capture_cell_document()
        .unwrap()
        .copy_cells(&RectScope {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 0,
            row_end: 1,
            column_start: 0,
            column_end: 1,
        })
        .unwrap();
    e.validate_cell_copy(&copy).unwrap();
    copy
}

#[test]
fn pure_meter_preparation_in_read_mode_preserves_exact_history_and_clipboard() {
    let fixture = Fixture::new();
    let e = editor(fixture.song(true));
    let copy = clipboard(&e);
    let owner = e.document_owner();
    let before = state(&e);
    // The pure stage uses captured metadata, not pathname IO or fresh-byte proof.
    std::fs::remove_file(&fixture.asset.path).unwrap();
    let a = meters::prepare(&e.song, config()).unwrap();
    let b = meters::prepare(&e.song, config()).unwrap();
    assert_eq!(a.id(), b.id());
    assert_eq!(bytes(a.song()), before.song);
    assert_eq!(bytes(a.summary()), bytes(b.summary()));
    assert_eq!(a.source_samples(), &[SampleId(1)]);
    assert_eq!(
        a.song().track_mix[&TrackId(1)].balance.to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(state(&e), before);
    assert!(e.owns_document(&owner));
    assert!(e.cell_clipboard_is_current_document(&copy));
    e.validate_cell_copy(&copy).unwrap();
}

#[test]
fn successful_offline_analysis_is_read_only_and_only_authorizes_compiled_sources() {
    let fixture = Fixture::new();
    let mut song = fixture.song(true);
    let mut unused = fixture.asset.clone();
    unused.path = fixture.dir.join("unused-and-missing.wav");
    song.register_sample(SampleId(91), unused).unwrap();
    let e = editor(song);
    let copy = clipboard(&e);
    let owner = e.document_owner();
    let before = state(&e);
    let source = std::fs::read(&fixture.asset.path).unwrap();
    let plan = meters::prepare(&e.song, config()).unwrap();
    let mut authorized = Vec::new();
    let a = meters::analyze(
        &plan,
        |path| {
            authorized.push(path.to_owned());
            Ok(())
        },
        || false,
    )
    .unwrap();
    let b = meters::analyze(&plan, |_| Ok(()), || false).unwrap();
    assert_eq!(authorized, vec![fixture.asset.path.clone()]);
    assert_eq!(a.verified_sources, 1);
    assert_eq!(a.reports.len(), 2);
    assert!(a.reports.iter().any(|r| r.master_peak[0] > 0.0));
    assert_eq!(bytes(&a), bytes(&b));
    assert_eq!(state(&e), before);
    assert!(e.owns_document(&owner));
    assert!(e.cell_clipboard_is_current_document(&copy));
    assert_eq!(std::fs::read(&fixture.asset.path).unwrap(), source);
    assert_eq!(std::fs::read_dir(&fixture.dir).unwrap().count(), 1);
}

#[test]
fn rejected_cancelled_denied_and_tampered_meter_analysis_leave_no_edit_or_report() {
    let fixture = Fixture::new();
    let e = editor(fixture.song(true));
    let before = state(&e);
    for c in [
        MeterAnalysisConfig {
            version: 2,
            ..config()
        },
        MeterAnalysisConfig {
            start_frame: 1,
            ..config()
        },
        MeterAnalysisConfig {
            end_frame: 4_799,
            ..config()
        },
        MeterAnalysisConfig {
            end_frame: 120_001,
            ..config()
        },
    ] {
        assert!(meters::prepare(&e.song, c).is_err());
        assert_eq!(state(&e), before);
    }
    let plan = meters::prepare(&e.song, config()).unwrap();
    let mut authorized = 0;
    assert!(meters::analyze(
        &plan,
        |_| {
            authorized += 1;
            Ok(())
        },
        || true
    )
    .is_err());
    assert_eq!(authorized, 0);
    assert!(
        meters::analyze(&plan, |_| Err("test policy denial".into()), || false)
            .unwrap_err()
            .contains("test policy denial")
    );
    let mut checks = 0;
    assert!(meters::analyze(
        &plan,
        |_| Ok(()),
        || {
            checks += 1;
            checks >= 4
        }
    )
    .unwrap_err()
    .contains("cancelled"));
    assert_eq!(checks, 4);
    let source = std::fs::read(&fixture.asset.path).unwrap();
    let mut changed = source.clone();
    changed.push(7);
    std::fs::write(&fixture.asset.path, &changed).unwrap();
    assert!(meters::analyze(&plan, |_| Ok(()), || false).is_err());
    assert_eq!(std::fs::read(&fixture.asset.path).unwrap(), changed);
    std::fs::write(&fixture.asset.path, &source).unwrap();
    meters::analyze(&plan, |_| Ok(()), || false).unwrap();
    assert_eq!(state(&e), before);
}

#[test]
fn meter_enable_read_disable_and_window_serialization_preserve_editor_and_pcm() {
    let fixture = Fixture::new();
    let e = editor(fixture.song(true));
    let before = state(&e);
    let owner = e.document_owner();
    let mut measured = Engine::new(&e.song, None).unwrap();
    let mut reference = Engine::new(&e.song, None).unwrap();
    assert!(measured.meter_layout().is_none() && measured.meter_block().is_none());
    let layout = measured.enable_meters().clone();
    assert_eq!(measured.enable_meters(), &layout);
    assert!(measured.meter_block().is_none());
    let mut windows = MeterWindows::new(&layout).unwrap();
    let mut reports = Vec::new();
    let mut a = [0.0; 1_024];
    let mut b = [0.0; 1_024];
    while measured.position < 4_800 {
        let n = measured.block(&mut a, 4_800).unwrap();
        assert_eq!(reference.block(&mut b, 4_800).unwrap(), n);
        assert_eq!(
            a[..n * 2].iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            b[..n * 2].iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
        for _ in 0..3 {
            let block = measured.meter_block().unwrap();
            assert_eq!(block.frames(), n);
            assert_eq!(block.master_sample(0).unwrap()[0].to_bits(), a[0].to_bits());
            assert!(block.master_sample(n).is_none());
            assert_eq!(measured.meter_layout(), Some(&layout));
        }
        windows
            .push_block(measured.meter_block().unwrap(), 0, |r| reports.push(r))
            .unwrap();
    }
    assert_eq!(reports.len(), 2);
    assert!(!bytes(&reports).is_empty());
    measured.disable_meters();
    assert!(measured.meter_layout().is_none() && measured.meter_block().is_none());
    assert_eq!(state(&e), before);
    assert!(e.owns_document(&owner));
}

#[test]
fn meter_analysis_preserves_saved_bytes_relative_paths_and_omitted_old_json() {
    let fixture = Fixture::new();
    for mix in [false, true] {
        let before_path = fixture.dir.join(format!("before-{mix}.json"));
        let after_path = fixture.dir.join(format!("after-{mix}.json"));
        project::save(&fixture.song(mix), &before_path, SaveMode::CreateNew).unwrap();
        let original = std::fs::read(&before_path).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let object = doc["song"].as_object().unwrap();
        assert!(!object.contains_key("meters") && !object.contains_key("meter_reports"));
        assert_eq!(object.contains_key("track_mix"), mix);
        assert_eq!(doc["song"]["samples"]["1"]["path"], "authored.wav");
        let loaded = project::load(&before_path).unwrap().0;
        assert!(loaded.samples[&SampleId(1)].path.is_absolute());
        let mut raw_relative = loaded.clone();
        raw_relative.samples.get_mut(&SampleId(1)).unwrap().path = PathBuf::from("authored.wav");
        assert!(meters::prepare(&raw_relative, config())
            .unwrap_err()
            .contains("absolute"));
        let mut e = Editor::default();
        e.replace_document(loaded).unwrap();
        let before = state(&e);
        let plan = meters::prepare(&e.song, config()).unwrap();
        meters::analyze(&plan, |_| Ok(()), || false).unwrap();
        project::save(&e.song, &after_path, SaveMode::CreateNew).unwrap();
        assert_eq!(std::fs::read(&after_path).unwrap(), original);
        assert_eq!(bytes(&project::load(&after_path).unwrap().0), before.song);
        assert_eq!(state(&e), before);
        if mix {
            assert_eq!(
                e.song.track_mix[&TrackId(1)].balance.to_bits(),
                (-0.0f32).to_bits()
            );
        }
    }
}

#[test]
fn historical_meter_results_cannot_restore_old_document_or_change_undo_redo() {
    let fixture = Fixture::new();
    let mut e = editor(fixture.song(true));
    let mut never_observed = e.clone();
    let owner = e.document_owner();
    let copy = clipboard(&e);
    let plan = meters::prepare(&e.song, config()).unwrap();
    let plan_bytes = bytes(plan.song());
    let report = meters::analyze(&plan, |_| Ok(()), || false).unwrap();
    e.redo();
    never_observed.redo();
    assert_eq!(state(&e), state(&never_observed));
    e.undo();
    never_observed.undo();
    assert_eq!(state(&e), state(&never_observed));
    let same_bytes = e.song.clone();
    e.replace_document(same_bytes.clone()).unwrap();
    never_observed.replace_document(same_bytes).unwrap();
    assert!(!e.owns_document(&owner));
    assert!(!e.cell_clipboard_is_current_document(&copy));
    let before = state(&e);
    // A detached analysis may still describe its historical snapshot. It has no
    // commit method or document-owner authority; adapters own freshness labels.
    assert_eq!(
        bytes(&meters::analyze(&plan, |_| Ok(()), || false).unwrap()),
        bytes(&report)
    );
    assert_eq!(bytes(plan.song()), plan_bytes);
    drop(report);
    drop(plan);
    assert_eq!(state(&e), before);
    assert_eq!(state(&e), state(&never_observed));
    assert!(e.track_mix_change_revision().is_none());
}

#[test]
fn live_meter_reader_and_polling_are_read_only_not_implicit_mix_submission() {
    use aqua_pattern_core::transport::{Producer, Shared};
    use std::sync::Arc;
    let fixture = Fixture::new();
    let mut e = editor(fixture.song(true));
    let copy = clipboard(&e);
    let before = state(&e);
    let owner = e.document_owner();
    let shared = Arc::new(Shared::default());
    assert!(shared.meter_reader(&owner, &e.song).is_err());
    shared.bind_live_mix(owner.clone(), &e.song).unwrap();
    let mut producer = Producer::new(&e.song, None, Arc::clone(&shared)).unwrap();
    let reader = shared.meter_reader(&owner, &e.song).unwrap();
    assert!(shared.poll_meters(&reader).unwrap().reports.is_empty());
    let mut saw_report = false;
    let mut output = [0.0; 1_024];
    for _ in 0..12 {
        producer.pump().unwrap();
        shared.fill(&mut output);
        let counters = (
            shared.position(),
            shared.callbacks.load(Ordering::SeqCst),
            shared.underruns.load(Ordering::SeqCst),
            shared.audible_frames.load(Ordering::SeqCst),
        );
        for _ in 0..3 {
            let poll = shared.poll_meters(&reader).unwrap();
            saw_report |= !poll.reports.is_empty();
            assert_eq!(
                counters,
                (
                    shared.position(),
                    shared.callbacks.load(Ordering::SeqCst),
                    shared.underruns.load(Ordering::SeqCst),
                    shared.audible_frames.load(Ordering::SeqCst),
                )
            );
            assert_eq!(shared.live_mix_status().accepted_revision, 0);
            assert_eq!(shared.live_mix_status().applied_revision, 0);
            assert_eq!(state(&e), before);
        }
    }
    assert!(saw_report);
    assert!(e.cell_clipboard_is_current_document(&copy));
    e.replace_document(e.song.clone()).unwrap();
    assert!(!e.owns_document(&owner));
    assert!(shared.meter_reader(&e.document_owner(), &e.song).is_err());
    // Controllers explicitly revoke the old session on document replacement;
    // detached tokens cannot discover a new Editor implicitly.
    shared.revoke_live_mix();
    let replaced = state(&e);
    assert!(shared.poll_meters(&reader).is_err());
    assert_eq!(state(&e), replaced);
    assert_eq!(shared.live_mix_status().accepted_revision, 0);
    shared.stop();
}
