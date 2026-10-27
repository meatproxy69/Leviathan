//! Marker-bank registration is one protected ordinary-document transaction.
use aqua_pattern_core::{
    editor::{Editor, Field},
    marker_bank::{self, MarkerBankConfig},
    project::{self, SaveMode},
    sample_asset,
    song::*,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn config() -> MarkerBankConfig {
    MarkerBankConfig {
        version: 1,
        marker_set: MarkerSetId(5),
        interval_indices: vec![1, 4, 7],
        name: "Reviewed sparse bank".into(),
    }
}
fn song() -> Song {
    let mut s = example_song();
    s.title = "Occupied arrangement; preserve everything".into();
    s.bpm = 137.3;
    s.lines_per_beat = 7;
    s.samples.insert(
        SampleId(11),
        SampleAsset {
            // Pure preview/Editor has no IO or process-CWD interpretation.
            path: PathBuf::from("unresolved-relative-native.wav"),
            sha256: "1".repeat(64),
            sample_rate: 8_000,
            channels: 1,
            frames: 128,
        },
    );
    for (id, start, end) in [(7, 0, 8), (90, 8, 16)] {
        s.regions.insert(
            RegionId(id),
            SampleRegion {
                name: format!("Existing region {id}"),
                sample: SampleId(11),
                start_frame: start,
                end_frame: end,
                root_note: 45,
                amp_envelope: None,
                forward_loop: None,
            },
        );
    }
    s.assign_region(InstrumentId(1), Some(RegionId(90)))
        .unwrap();
    s.instruments.insert(
        InstrumentId(70),
        Instrument {
            name: "Unused existing instrument".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: Some(RegionId(7)),
        },
    );
    s.drum_kits.insert(
        DrumKitId(40),
        DrumKitInstrument {
            name: "Existing empty kit".into(),
            pads: BTreeMap::new(),
        },
    );
    s.marker_sets.insert(
        MarkerSetId(5),
        MarkerSet {
            name: "Accepted manual markers".into(),
            sample: SampleId(11),
            source_bpm: None,
            markers: (0..=8).map(|n| n * 16).collect(),
            locked_frames: vec![16, 64],
        },
    );
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .insert(
            7,
            BTreeMap::from([(
                1,
                Cell {
                    velocity: Some(-0.0),
                    pan: Some(-0.0),
                    delay: Some(-0.0),
                    ..Cell::default()
                },
            )]),
        );
    let mut alias = s.patterns[&PatternId(1)].clone();
    alias.name = "Existing alias".into();
    alias.tracks.remove(&TrackId(1));
    alias.track_aliases.insert(
        TrackId(1),
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        },
    );
    s.patterns.insert(PatternId(99), alias);
    s.order.push(Occurrence {
        pattern: PatternId(99),
        muted_tracks: BTreeSet::from([TrackId(2)]),
        section: None,
    });
    s.validate().unwrap();
    s
}
fn editor() -> Editor {
    let mut e = Editor::default();
    e.replace_document(song()).unwrap();
    e.edit_mode = true;
    e.occurrence = e.song.order.len() - 1;
    e.track = TrackId(1);
    e.column = 1;
    e.row = 7;
    e.field = Field::Pan;
    e.edit_step = 4;
    e.octave = 6;
    e
}
fn with_redo() -> Editor {
    let mut e = editor();
    e.set_timing(139.5, 7).unwrap();
    e.mark_saved();
    e.set_timing(140.25, 7).unwrap();
    e.undo();
    assert!(e.can_undo() && e.can_redo() && !e.dirty);
    e
}
#[derive(Debug, PartialEq)]
struct State {
    song: Vec<u8>,
    history: String,
    revision: u64,
    cursor: (usize, TrackId, u16, u32, Field),
    options: (bool, u32, u8, bool),
}
fn state(e: &Editor) -> State {
    State {
        song: serde_json::to_vec(&e.song).unwrap(),
        history: format!("{:?}", e.history),
        revision: e.revision(),
        cursor: (e.occurrence, e.track, e.column, e.row, e.field),
        options: (e.edit_mode, e.edit_step, e.octave, e.dirty),
    }
}
fn zero_cell(s: &mut Song) -> &mut Cell {
    s.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&7)
        .unwrap()
        .get_mut(&1)
        .unwrap()
}

#[test]
fn marker_bank_preview_is_pure_sparse_exact_and_needs_no_bpm_or_source_io() {
    let e = with_redo();
    let before = state(&e);
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    assert_eq!(state(&e), before);
    assert!(!p.summary().source_identity_verified);
    assert_eq!(
        p.source().path,
        PathBuf::from("unresolved-relative-native.wav")
    );
    assert_eq!(p.summary().kit, DrumKitId(41));
    assert_eq!(p.summary().instrument, InstrumentId(71));
    assert_eq!(p.summary().pads.len(), 3);
    for (slot, (interval, start, end)) in [(1, 16, 32), (4, 64, 80), (7, 112, 128)]
        .into_iter()
        .enumerate()
    {
        let pad = &p.summary().pads[slot];
        assert_eq!(pad.interval_index, interval);
        assert_eq!(pad.pad, PadId(slot as u64 + 1));
        assert_eq!(pad.region, RegionId(91 + slot as u64));
        assert_eq!((pad.start_frame, pad.end_frame), (start, end));
        assert_eq!(pad.slot, slot as u8);
        assert_eq!(pad.selector, 36 + slot as u8);
        assert_eq!(pad.root_note, 60);
        assert_eq!(pad.mode, PadMode::OneShot);
        assert_eq!(pad.new_note, PadNewNote::Overlap);
    }
    let mut stripped = p.candidate().clone();
    stripped.instruments.remove(&p.summary().instrument);
    stripped.drum_kits.remove(&p.summary().kit);
    for pad in &p.summary().pads {
        stripped.regions.remove(&pad.region);
    }
    assert_eq!(serde_json::to_vec(&stripped).unwrap(), before.song);
    assert_eq!(p.candidate().marker_sets[&MarkerSetId(5)].source_bpm, None);
    assert_eq!(p.candidate().marker_sets, e.song.marker_sets);
    assert_eq!(p.candidate().samples, e.song.samples);
    assert_eq!(
        marker_bank::preview(&e.song, &config()).unwrap().id(),
        p.id()
    );
}

#[test]
fn marker_bank_read_mode_refuses_then_commit_is_one_undo_with_exact_cursor_and_ids() {
    let mut e = editor();
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    let revision = e.revision();
    e.edit_mode = false;
    let read = state(&e);
    assert!(e.commit_marker_bank_preview(revision, &p).is_err());
    assert_eq!(state(&e), read);
    e.edit_mode = true;
    let base = state(&e);
    e.commit_marker_bank_preview(revision, &p).unwrap();
    assert_eq!(
        serde_json::to_vec(&e.song).unwrap(),
        serde_json::to_vec(p.candidate()).unwrap()
    );
    assert_eq!(state(&e).cursor, base.cursor);
    assert_eq!(e.revision(), revision + 1);
    assert!(e.dirty && e.can_undo() && !e.can_redo());
    let committed = state(&e);
    assert!(e.commit_marker_bank_preview(e.revision(), &p).is_err());
    assert_eq!(state(&e), committed);
    e.undo();
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), base.song);
    assert_eq!(state(&e).cursor, base.cursor);
    assert!(!e.dirty && !e.can_undo() && e.can_redo());
    e.redo();
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), committed.song);
    assert_eq!(state(&e).cursor, base.cursor);
    assert_eq!(
        zero_cell(&mut e.song).pan.unwrap().to_bits(),
        (-0.0_f32).to_bits()
    );
}

#[test]
fn marker_bank_stale_revision_aba_open_and_unrelated_content_preserve_all_state() {
    let mut e = with_redo();
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    let revision = e.revision();
    e.set_timing(142.0, 7).unwrap();
    e.undo();
    assert_eq!(&e.song, p.base());
    let before = state(&e);
    assert!(e.commit_marker_bank_preview(revision, &p).is_err());
    assert_eq!(state(&e), before);
    e.replace_document(p.base().clone()).unwrap();
    e.edit_mode = true;
    let before = state(&e);
    assert!(e.commit_marker_bank_preview(revision, &p).is_err());
    assert_eq!(state(&e), before);
    // An unchecked content mutation must also fail even with the current revision.
    e.song.order[0].muted_tracks.insert(TrackId(4));
    let before = state(&e);
    assert!(e.commit_marker_bank_preview(e.revision(), &p).is_err());
    assert_eq!(state(&e), before);
}

#[test]
fn marker_bank_signed_zero_base_change_is_stale_without_a_revision_change() {
    let mut e = with_redo();
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    let revision = e.revision();
    zero_cell(&mut e.song).pan = Some(0.0);
    assert_eq!(
        &e.song,
        p.base(),
        "PartialEq alone cannot detect this change"
    );
    assert_ne!(
        serde_json::to_vec(&e.song).unwrap(),
        serde_json::to_vec(p.base()).unwrap()
    );
    assert!(!p.is_current(&e.song).unwrap());
    let before = state(&e);
    assert!(e.commit_marker_bank_preview(revision, &p).is_err());
    assert_eq!(state(&e), before);
    assert_eq!(
        zero_cell(&mut e.song).pan.unwrap().to_bits(),
        0.0_f32.to_bits()
    );
}

#[test]
fn marker_bank_failed_previews_and_true_existing_noop_keep_review_and_redo() {
    let mut e = with_redo();
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    let revision = e.revision();
    let before = state(&e);
    let mut bad = Vec::new();
    for indices in [vec![], vec![1, 1], vec![4, 1], vec![8], (0..17).collect()] {
        let mut c = config();
        c.interval_indices = indices;
        bad.push(c);
    }
    let mut c = config();
    c.version = 2;
    bad.push(c);
    for name in ["".into(), " ".into(), "é".repeat(129)] {
        let mut c = config();
        c.name = name;
        bad.push(c);
    }
    for c in bad {
        assert!(marker_bank::preview(&e.song, &c).is_err());
        assert_eq!(state(&e), before);
    }
    // There is no successful empty bank/no-op fabrication. An actual unrelated
    // no-op edit leaves this reviewed plan current and preserves redo exactly.
    e.set_timing(e.song.bpm, e.song.lines_per_beat).unwrap();
    assert_eq!(state(&e), before);
    assert!(p.is_current(&e.song).unwrap());
    e.commit_marker_bank_preview(revision, &p).unwrap();
    assert!(!e.can_redo());
    e.undo();
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), before.song);
}

#[test]
fn marker_bank_id_exhaustion_and_dangling_existing_refs_cannot_partially_allocate() {
    for which in 0..3 {
        let mut s = song();
        match which {
            0 => {
                let region = s.regions[&RegionId(7)].clone();
                s.regions.insert(RegionId(u64::MAX), region);
            }
            1 => {
                let kit = s.drum_kits[&DrumKitId(40)].clone();
                s.drum_kits.insert(DrumKitId(u64::MAX), kit);
            }
            _ => {
                let instrument = s.instruments[&InstrumentId(70)].clone();
                s.instruments.insert(InstrumentId(u64::MAX), instrument);
            }
        }
        s.validate().unwrap();
        let mut e = editor();
        e.replace_document(s).unwrap();
        let before = state(&e);
        assert!(marker_bank::preview(&e.song, &config()).is_err());
        assert_eq!(state(&e), before);
    }
    let mut e = with_redo();
    zero_cell(&mut e.song).instrument = Some(InstrumentId(71));
    let before = state(&e);
    assert!(marker_bank::preview(&e.song, &config()).is_err());
    assert_eq!(state(&e), before);
    assert!(!e.song.instruments.contains_key(&InstrumentId(71)));
}

#[test]
fn marker_bank_new_references_protect_deletion_and_undo_restores_old_graph() {
    let mut e = editor();
    let before = state(&e);
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    e.commit_marker_bank_preview(e.revision(), &p).unwrap();
    let committed = state(&e);
    assert!(e.remove_region(p.summary().pads[0].region).is_err());
    assert_eq!(state(&e), committed);
    assert!(e.remove_sample(SampleId(11)).is_err());
    assert_eq!(state(&e), committed);
    assert!(e.remove_drum_kit(p.summary().kit).is_err());
    assert_eq!(state(&e), committed);
    e.undo();
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), before.song);
    assert!(!e.song.drum_kits.contains_key(&p.summary().kit));
    assert!(!e.song.regions.contains_key(&p.summary().pads[0].region));
    assert!(!e.song.instruments.contains_key(&p.summary().instrument));
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "leviathan-marker-bank-editor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn marker_bank_save_reopen_is_ordinary_old_schema_and_keeps_source_bytes_exact() {
    let dir = Fixture::new();
    let source = dir.0.join("authored.wav");
    let mut wav = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..128 {
        wav.write_sample((frame as f32 - 64.0) / 512.0).unwrap();
    }
    wav.finalize().unwrap();
    let original = std::fs::read(&source).unwrap();
    let mut s = song();
    s.samples
        .insert(SampleId(11), sample_asset::import(&source).unwrap().asset);
    let old_path = dir.0.join("before.leviathan.json");
    project::save(&s, &old_path, SaveMode::CreateNew).unwrap();
    let old_bytes = std::fs::read(&old_path).unwrap();
    let mut e = editor();
    e.replace_document(project::load(&old_path).unwrap().0)
        .unwrap();
    e.edit_mode = true;
    let before = state(&e);
    let p = marker_bank::preview(&e.song, &config()).unwrap();
    e.commit_marker_bank_preview(e.revision(), &p).unwrap();
    let new_path = dir.0.join("bank.leviathan.json");
    project::save(&e.song, &new_path, SaveMode::CreateNew).unwrap();
    let reopened = project::load(&new_path).unwrap().0;
    assert_eq!(
        serde_json::to_vec(&reopened).unwrap(),
        serde_json::to_vec(&e.song).unwrap()
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&new_path).unwrap()).unwrap();
    assert_eq!(saved["song"]["samples"]["11"]["path"], "authored.wav");
    assert_eq!(saved["song"]["schema"], 1);
    assert!(!saved["song"]
        .as_object()
        .unwrap()
        .contains_key("marker_bank"));
    assert!(!saved["song"]
        .as_object()
        .unwrap()
        .contains_key("bank_recipe"));
    assert_eq!(std::fs::read(&source).unwrap(), original);
    e.undo();
    assert_eq!(serde_json::to_vec(&e.song).unwrap(), before.song);
    let undone_path = dir.0.join("undone.leviathan.json");
    project::save(&e.song, &undone_path, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(&undone_path).unwrap(), old_bytes);
    // Legacy default JSON still contains no optional bank-specific state.
    let legacy = serde_json::to_vec(&example_song()).unwrap();
    let roundtrip: Song = serde_json::from_slice(&legacy).unwrap();
    assert_eq!(serde_json::to_vec(&roundtrip).unwrap(), legacy);
}
