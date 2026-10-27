//! Audition snapshots are playback inputs, never Editor candidates/transactions.
use aqua_pattern_core::{
    audition::{self, AuditionTarget},
    editor::{Editor, Field},
    project::{self, SaveMode},
    sample_asset,
    song::*,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn asset() -> SampleAsset {
    SampleAsset {
        path: std::env::temp_dir().join("audition-editor-metadata-only-missing.wav"),
        sha256: "1".repeat(64),
        sample_rate: 8_000,
        channels: 1,
        frames: 128,
    }
}
fn region() -> SampleRegion {
    SampleRegion {
        name: "Saved region".into(),
        sample: SampleId(11),
        start_frame: 0,
        end_frame: 64,
        root_note: 60,
        amp_envelope: None,
        forward_loop: None,
    }
}
fn pad() -> DrumPad {
    DrumPad {
        slot: 7,
        name: "Committed pad".into(),
        note: 36,
        region: Some(RegionId(21)),
        velocity_zones: vec![],
        root_note: 64,
        tune_semitones: 7,
        gain: 0.625,
        pan: -0.0,
        mode: PadMode::Gated,
        new_note: PadNewNote::Overlap,
        choke_group: Some(3),
    }
}
fn song() -> Song {
    let mut s = example_song();
    s.title = "Do not change for audition".into();
    s.tracks = BTreeMap::from([(
        TrackId(1),
        Track {
            name: "Current editor track".into(),
            note_columns: 2,
        },
    )]);
    s.track_order = vec![TrackId(1)];
    let pattern = Pattern {
        name: "Source".into(),
        rows: 16,
        tracks: BTreeMap::from([(
            TrackId(1),
            BTreeMap::from([(
                3,
                BTreeMap::from([(
                    1,
                    Cell {
                        velocity: Some(-0.0),
                        pan: Some(-0.0),
                        delay: Some(-0.0),
                        ..Cell::default()
                    },
                )]),
            )]),
        )]),
        track_aliases: BTreeMap::new(),
        generation: None,
    };
    let mut alias = pattern.clone();
    alias.name = "Read-only alias".into();
    alias.tracks.clear();
    alias.track_aliases.insert(
        TrackId(1),
        TrackAlias {
            pattern: PatternId(1),
            track: TrackId(1),
        },
    );
    s.patterns = BTreeMap::from([(PatternId(1), pattern), (PatternId(2), alias)]);
    s.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        },
        Occurrence {
            pattern: PatternId(2),
            muted_tracks: Default::default(),
            section: None,
        },
    ];
    s.instruments = BTreeMap::from([
        (
            InstrumentId(1),
            Instrument {
                name: "Unassigned ordinary instrument".into(),
                sample: None,
                region: None,
                kind: InstrumentKind::Pitched,
            },
        ),
        (
            InstrumentId(99),
            Instrument {
                name: "Kit owner".into(),
                sample: None,
                region: None,
                kind: InstrumentKind::DrumKit { kit: DrumKitId(77) },
            },
        ),
    ]);
    s.samples.insert(SampleId(11), asset());
    s.regions.insert(RegionId(21), region());
    let mut second = region();
    second.name = "High zone source".into();
    second.start_frame = 64;
    second.end_frame = 128;
    second.root_note = 45;
    s.regions.insert(RegionId(22), second);
    s.drum_kits.insert(
        DrumKitId(77),
        DrumKitInstrument {
            name: "Unchanged mapping".into(),
            pads: BTreeMap::from([(PadId(501), pad())]),
        },
    );
    s.validate().unwrap();
    s
}
fn editor() -> Editor {
    let mut e = Editor::default();
    e.replace_document(song()).unwrap();
    e.occurrence = 1;
    e.track = TrackId(1);
    e.column = 1;
    e.row = 3;
    e.field = Field::Pan;
    e.edit_step = 4;
    e.octave = 6;
    e
}
fn editor_with_redo() -> Editor {
    let mut e = editor();
    e.edit_mode = true;
    e.set_timing(121.25, 4).unwrap();
    e.mark_saved();
    e.set_timing(122.5, 4).unwrap();
    e.undo();
    e.edit_mode = false;
    assert!(e.can_undo() && e.can_redo());
    assert!(!e.dirty);
    e
}
#[derive(Debug, PartialEq)]
struct ExactEditorState {
    song: Vec<u8>,
    // Includes both private history stacks and saved baseline, not merely flags.
    history: String,
    revision: u64,
    cursor: (usize, TrackId, u16, u32, Field),
    options: (bool, u32, u8, bool),
}
fn state(e: &Editor) -> ExactEditorState {
    ExactEditorState {
        song: serde_json::to_vec(&e.song).unwrap(),
        history: format!("{:?}", e.history),
        revision: e.revision(),
        cursor: (e.occurrence, e.track, e.column, e.row, e.field),
        options: (e.edit_mode, e.edit_step, e.octave, e.dirty),
    }
}
fn assert_unchanged(e: &Editor, before: &ExactEditorState) {
    assert_eq!(&state(e), before);
    let c = &e.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&3][&1];
    assert_eq!(c.velocity.unwrap().to_bits(), (-0.0_f32).to_bits());
    assert_eq!(c.pan.unwrap().to_bits(), (-0.0_f32).to_bits());
    assert_eq!(c.delay.unwrap().to_bits(), (-0.0_f64).to_bits());
}

#[test]
fn unsaved_region_audition_is_read_only_even_in_read_mode_on_an_alias() {
    let e = editor();
    let before = state(&e);
    let mut source = asset();
    source.path = std::env::temp_dir().join("audition-new-unregistered-metadata.wav");
    let mut draft = region();
    draft.sample = SampleId(999);
    draft.name = "Unsaved bounds and sampler controls".into();
    draft.start_frame = 8;
    draft.end_frame = 96;
    draft.root_note = 69;
    draft.amp_envelope = Some(AmpEnvelope {
        attack_ms: 2,
        decay_ms: 3,
        sustain_percent: 70,
        release_ms: 40,
    });
    draft.forward_loop = Some(ForwardLoop {
        start_frame: 30,
        end_frame: 64,
    });
    let source_bytes = serde_json::to_vec(&source).unwrap();
    let draft_bytes = serde_json::to_vec(&draft).unwrap();
    let preview = audition::region_preview(&source, &draft).unwrap();
    assert_eq!(preview.summary().target, AuditionTarget::RegionDraft);
    assert_eq!(preview.summary().sounding_note, 69);
    assert_eq!(preview.summary().total_frames, 120_000);
    assert!(!preview.summary().source_identity_verified);
    assert_eq!(preview.song().samples.len(), 1);
    assert_eq!(preview.song().regions.len(), 1);
    assert_eq!(serde_json::to_vec(&source).unwrap(), source_bytes);
    assert_eq!(serde_json::to_vec(&draft).unwrap(), draft_bytes);
    drop(preview);
    assert_unchanged(&e, &before);
}

#[test]
fn committed_pad_and_zoned_rr_auditions_preserve_exact_redo_and_mapping() {
    let mut e = editor_with_redo();
    let before = state(&e);
    let first = audition::pad_preview(&e.song, InstrumentId(99), PadId(501)).unwrap();
    assert_eq!(first.summary().input_velocity.to_bits(), 1.0_f32.to_bits());
    assert_eq!(first.summary().velocity_bin, 127);
    assert_eq!(first.summary().sounding_note, 71);
    assert_eq!(first.summary().on_gain.to_bits(), 0.625_f32.to_bits());
    assert!(first.summary().layer_selection.is_none());
    assert_unchanged(&e, &before);
    // Commit the test mapping BEFORE measuring audition; preview itself never edits.
    e.edit_mode = true;
    e.set_pad_sources(
        DrumKitId(77),
        PadId(501),
        None,
        vec![
            VelocityZone {
                min_velocity: 0,
                max_velocity: 63,
                alternatives: vec![PadLayer {
                    id: PadLayerId(900),
                    region: RegionId(21),
                }],
            },
            VelocityZone {
                min_velocity: 64,
                max_velocity: 127,
                alternatives: vec![
                    PadLayer {
                        id: PadLayerId(901),
                        region: RegionId(22),
                    },
                    PadLayer {
                        id: PadLayerId(902),
                        region: RegionId(21),
                    },
                ],
            },
        ],
    )
    .unwrap();
    e.mark_saved();
    e.set_timing(133.75, 4).unwrap();
    e.undo();
    e.edit_mode = false;
    let before = state(&e);
    let a = audition::pad_preview(&e.song, InstrumentId(99), PadId(501)).unwrap();
    let b = audition::pad_preview(&e.song, InstrumentId(99), PadId(501)).unwrap();
    assert_eq!(a.id(), b.id());
    assert_eq!(
        serde_json::to_vec(a.song()).unwrap(),
        serde_json::to_vec(b.song()).unwrap()
    );
    assert_eq!(a.summary(), b.summary());
    let selected = a.summary().layer_selection.as_ref().unwrap();
    assert_eq!(selected.layer, PadLayerId(901));
    assert_eq!(selected.round_robin_index, 0);
    assert_eq!(selected.round_robin_ordinal, 0);
    assert_eq!(selected.velocity_bin, 127);
    assert_eq!(a.summary().sounding_note, 71);
    assert_unchanged(&e, &before);
}

#[test]
fn invalid_audition_targets_preserve_document_cursor_saved_state_and_redo() {
    let e = editor_with_redo();
    let before = state(&e);
    for (instrument, pad) in [
        (InstrumentId(404), PadId(501)),
        (InstrumentId(1), PadId(501)),
        (InstrumentId(99), PadId(404)),
    ] {
        assert!(audition::pad_preview(&e.song, instrument, pad).is_err());
        assert_unchanged(&e, &before);
    }
    let mut bad_bounds = region();
    bad_bounds.end_frame = asset().frames + 1;
    assert!(audition::region_preview(&asset(), &bad_bounds).is_err());
    let mut bad_source = asset();
    bad_source.path = PathBuf::new();
    assert!(audition::region_preview(&bad_source, &region()).is_err());
    bad_source.path = PathBuf::from("unresolved-relative.wav");
    assert!(audition::region_preview(&bad_source, &region())
        .unwrap_err()
        .contains("absolute"));
    let mut unmapped = e.song.clone();
    unmapped
        .set_pad_sources(DrumKitId(77), PadId(501), None, vec![])
        .unwrap();
    assert!(audition::pad_preview(&unmapped, InstrumentId(99), PadId(501)).is_err());
    assert_unchanged(&e, &before);
}

#[test]
fn audition_snapshot_clone_drop_and_document_replacement_never_write_back() {
    let mut e = editor_with_redo();
    let mut expected = e.clone();
    let before = state(&e);
    let preview = audition::pad_preview(&e.song, InstrumentId(99), PadId(501)).unwrap();
    let bytes = serde_json::to_vec(preview.song()).unwrap();
    let copy = preview.clone();
    drop(preview);
    assert_unchanged(&e, &before);
    // Existing undo/redo is exactly that of an Editor never auditioned.
    e.redo();
    expected.redo();
    assert_eq!(state(&e), state(&expected));
    e.undo();
    expected.undo();
    assert_eq!(state(&e), state(&expected));
    e.edit_mode = true;
    expected.edit_mode = true;
    e.set_timing(139.5, 8).unwrap();
    expected.set_timing(139.5, 8).unwrap();
    assert_eq!(state(&e), state(&expected));
    assert_eq!(serde_json::to_vec(copy.song()).unwrap(), bytes);
    e.replace_document(song()).unwrap();
    expected.replace_document(song()).unwrap();
    drop(copy);
    assert_eq!(state(&e), state(&expected));
    // UI owns stale intent/worker cancellation. Pure snapshots have no commit API.
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct AuthoredProject(PathBuf);
impl AuthoredProject {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "leviathan-audition-persistence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for AuthoredProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn audition_save_reopen_preserves_relative_project_assets_bytes_and_original_pcm() {
    let fixture = AuthoredProject::new();
    let source = fixture.0.join("authored.wav");
    let mut writer = hound::WavWriter::create(
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
        writer.write_sample((frame as f32 - 64.0) / 512.0).unwrap();
    }
    writer.finalize().unwrap();
    let original_pcm = std::fs::read(&source).unwrap();
    let mut song = song();
    song.samples
        .insert(SampleId(11), sample_asset::import(&source).unwrap().asset);
    song.validate().unwrap();
    let first_path = fixture.0.join("before.leviathan.json");
    project::save(&song, &first_path, SaveMode::CreateNew).unwrap();
    let original_bytes = std::fs::read(&first_path).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&original_bytes).unwrap();
    assert_eq!(document["song"]["samples"]["11"]["path"], "authored.wav");
    let loaded = project::load(&first_path).unwrap().0;
    assert!(loaded.samples[&SampleId(11)].path.is_absolute());
    let mut e = Editor::default();
    e.replace_document(loaded).unwrap();
    e.edit_mode = true;
    e.set_timing(123.75, 4).unwrap();
    e.undo();
    e.edit_mode = false;
    let before = state(&e);
    audition::region_preview(
        &e.song.samples[&SampleId(11)],
        &e.song.regions[&RegionId(21)],
    )
    .unwrap();
    audition::pad_preview(&e.song, InstrumentId(99), PadId(501)).unwrap();
    assert_unchanged(&e, &before);
    let after_path = fixture.0.join("after.leviathan.json");
    project::save(&e.song, &after_path, SaveMode::CreateNew).unwrap();
    assert_eq!(std::fs::read(&after_path).unwrap(), original_bytes);
    assert_eq!(
        serde_json::to_vec(&project::load(&after_path).unwrap().0).unwrap(),
        before.song
    );
    assert_eq!(std::fs::read(&source).unwrap(), original_pcm);
    assert!(!document["song"]
        .as_object()
        .unwrap()
        .contains_key("audition"));
    assert_unchanged(&e, &before);
}
