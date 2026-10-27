use aqua_pattern_core::{
    project,
    song::{example_song, Song},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "leviathan-alpha2-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self) -> PathBuf {
        self.0.join("alpha2.json")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn core_song() -> Song {
    let mut song = example_song();
    song.track_order = song.tracks.keys().copied().collect();
    song.validate().unwrap();
    song
}

#[test]
fn recovered_effect_fields_identify_an_unmarked_historical_project() {
    let dir = TempDir::new();
    let path = dir.file();
    let mut song = core_song();
    let cell = song
        .patterns
        .get_mut(&aqua_pattern_core::song::PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&aqua_pattern_core::song::TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap();
    cell.effects.release = Some(0.0);
    let data = json!({"format": "leviathan-song", "version": 1, "song": song});
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let (loaded, _) = project::load(&path).unwrap();
    assert_eq!(loaded, song);
}

fn expected_song() -> Song {
    let mut song = core_song();
    let pattern = song
        .patterns
        .get_mut(&aqua_pattern_core::song::PatternId(1))
        .unwrap();
    pattern.tracks.remove(&aqua_pattern_core::song::TrackId(2));
    pattern.track_aliases.insert(
        aqua_pattern_core::song::TrackId(2),
        aqua_pattern_core::song::TrackAlias {
            pattern: aqua_pattern_core::song::PatternId(1),
            track: aqua_pattern_core::song::TrackId(1),
        },
    );
    song.validate().unwrap();
    song
}

fn alpha2() -> Value {
    let mut song = serde_json::to_value(expected_song()).unwrap();
    let pattern = song["patterns"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .find(|pattern| pattern.get("track_aliases").is_some())
        .unwrap();
    let object = pattern.as_object_mut().unwrap();
    let aliases = object.remove("track_aliases").unwrap();
    object.insert("aliases".into(), aliases);
    json!({ "format": "leviathan-song", "version": 1, "song": song })
}

#[test]
fn migrates_shared_alpha2_core_without_changing_song_content() {
    let dir = TempDir::new();
    let path = dir.file();
    let data = alpha2();
    std::fs::write(&path, serde_json::to_vec_pretty(&data).unwrap()).unwrap();

    let (loaded, loaded_path) = project::load(&path).unwrap();
    assert_eq!(loaded, expected_song());
    assert_eq!(loaded_path.file_name(), path.file_name());
}

#[test]
fn rejects_unsupported_alpha2_phrases_and_effects_before_asset_policy() {
    let dir = TempDir::new();
    let path = dir.file();
    let data = alpha2();

    for (name, mutate, expected) in [
        (
            "phrase",
            Box::new(
                |data: &mut Value| data["song"]["instruments"]["1"]["phrases"]["4"] = json!({"name":"P","rows":4,"lines_per_beat":4,"note_columns":1,"root_note":60,"cells":{}}),
            ) as Box<dyn Fn(&mut Value)>,
            "instrument-owned phrases",
        ),
        (
            "effects",
            Box::new(|data: &mut Value| {
                let cell = data["song"]["patterns"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()["tracks"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap();
                cell["effects"] = json!({"retrigger":4});
            }),
            "effect semantics",
        ),
    ] {
        let mut case = data.clone();
        mutate(&mut case);
        std::fs::write(&path, serde_json::to_vec(&case).unwrap()).unwrap();
        let before = std::fs::read(&path).unwrap();
        let calls = std::cell::Cell::new(0);
        let error = project::load_with_asset_policy(&path, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains(expected), "{name}: {error}");
        assert_eq!(
            calls.get(),
            0,
            "{name}: asset access must follow conversion"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "{name}");
    }
}

#[test]
fn alpha2_aliases_keep_their_source_pattern_and_track_identity() {
    let dir = TempDir::new();
    let path = dir.file();
    let data = alpha2();
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let (loaded, _) = project::load(&path).unwrap();
    assert_eq!(
        loaded.patterns[&aqua_pattern_core::song::PatternId(1)].track_aliases
            [&aqua_pattern_core::song::TrackId(2)],
        aqua_pattern_core::song::TrackAlias {
            pattern: aqua_pattern_core::song::PatternId(1),
            track: aqua_pattern_core::song::TrackId(1),
        }
    );
}

#[test]
fn ambiguous_delayed_cut_is_rejected_even_though_it_deserializes_as_candidate() {
    let dir = TempDir::new();
    let path = dir.file();
    let mut data = json!({
        "format": "leviathan-song",
        "version": 1,
        "song": core_song(),
    });
    let cell = data.pointer_mut("/song/patterns/1/tracks/1/0/0").unwrap();
    cell["delay"] = json!(0.25);
    cell["effects"] = json!({ "cut": 0.5 });

    // This is valid candidate JSON too; without a schema-specific marker, its
    // Alpha 2 cut meaning cannot be inferred from wrapper/schema version 1.
    let native_parse: Song = serde_json::from_value(data["song"].clone()).unwrap();
    native_parse.validate().unwrap();
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let error = project::load(&path).unwrap_err();
    assert!(
        error.contains("Ambiguous schema-1 effect semantics"),
        "{error}"
    );
}

#[test]
fn effectful_candidate_checkpoint_loads_when_candidate_identity_is_proven() {
    let dir = TempDir::new();
    let path = dir.file();
    let mut song = expected_song();
    let cell = song
        .patterns
        .get_mut(&aqua_pattern_core::song::PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&aqua_pattern_core::song::TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap();
    cell.delay = Some(0.25);
    cell.effects.cut = Some(0.5);
    song.validate().unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "format": "leviathan-song",
            "version": 1,
            "song": song,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(project::load(&path).unwrap().0, song);
}

#[test]
fn newly_saved_ambiguous_effects_get_a_dialect_marker_and_reopen() {
    let dir = TempDir::new();
    let path = dir.file();
    let core_path = dir.0.join("core.json");
    std::fs::write(&core_path, b"old core document").unwrap();
    let core_snapshot = std::fs::read(&core_path).unwrap();
    project::save(
        &core_song(),
        &core_path,
        project::SaveMode::ReplaceConfirmed(core_snapshot),
    )
    .unwrap();
    let ordinary: Value = serde_json::from_slice(&std::fs::read(&core_path).unwrap()).unwrap();
    assert!(ordinary.get("dialect").is_none());

    let mut song = core_song();
    let cell = song
        .patterns
        .get_mut(&aqua_pattern_core::song::PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&aqua_pattern_core::song::TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap();
    cell.delay = Some(0.25);
    cell.effects.cut = Some(0.5);
    song.validate().unwrap();

    std::fs::write(&path, b"old candidate document").unwrap();
    let snapshot = std::fs::read(&path).unwrap();
    project::save(&song, &path, project::SaveMode::ReplaceConfirmed(snapshot)).unwrap();
    let encoded: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(encoded["dialect"], "recovered-meters-v1");
    assert_eq!(project::load(&path).unwrap().0, song);
}
