use aqua_pattern_core::{
    generation::Recipe,
    project,
    song::{example_song, Occurrence, PatternId, Section, SectionId, Song},
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
            "leviathan-legacy-metadata-{}-{}",
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

fn alpha2_with_metadata() -> (Value, Recipe, Section) {
    let mut song = serde_json::to_value(example_song()).unwrap();
    let patterns = song["patterns"].as_object_mut().unwrap();
    for pattern in patterns.values_mut() {
        let aliases = pattern
            .as_object_mut()
            .unwrap()
            .remove("track_aliases")
            .unwrap_or_else(|| json!({}));
        pattern["aliases"] = aliases;
    }
    let recipe = Recipe {
        schema: 1,
        revision: aqua_pattern_core::pattern::REVISION.into(),
        rows: 16,
        step_rows: 1,
        cycle_steps: 8,
        hits: 3,
        rotation: 0,
        probability: 0.75,
        seed: 22,
        sequence: vec![
            vec![aqua_pattern_core::song::Note::Note(60)],
            vec![aqua_pattern_core::song::Note::Note(64)],
        ],
        instrument: aqua_pattern_core::song::InstrumentId(1),
        velocity: 0.4,
        pan: -0.25,
    };
    let section = Section {
        id: SectionId(9),
        name: "Chorus".into(),
    };
    song["generation_recipes"] = json!({"lead": recipe});
    song["order"][0]["section"] = json!(section);
    (
        json!({"format":"leviathan-song", "version":1, "song":song}),
        recipe,
        section,
    )
}

#[test]
fn alpha2_recipes_and_sections_round_trip_without_loss() {
    let (document, recipe, section) = alpha2_with_metadata();
    let dir = TempDir::new();
    let path = dir.file();
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();

    let (loaded, _) = project::load(&path).unwrap();
    assert_eq!(loaded.generation_recipes.get("lead"), Some(&recipe));
    assert_eq!(loaded.order[0].section.as_ref(), Some(&section));
    loaded.validate().unwrap();
}

#[test]
fn section_range_preserves_alpha2_marker_boundaries() {
    let mut song: Song = example_song();
    song.order[0].section = Some(Section {
        id: SectionId(1),
        name: "Intro".into(),
    });
    song.order[2].section = Some(Section {
        id: SectionId(2),
        name: "Verse".into(),
    });
    assert_eq!(song.section_range(0).unwrap(), 0..2);
    assert_eq!(song.section_range(1).unwrap(), 0..2);
    assert_eq!(song.section_range(2).unwrap(), 2..3);
    assert!(Occurrence::new(PatternId(1)).section.is_none());
    song.validate().unwrap();
}

#[test]
fn generation_recipe_validation_rejects_bad_names_and_instruments() {
    let mut song = example_song();
    let (_, recipe, _) = alpha2_with_metadata();
    song.generation_recipes
        .insert(" lead".into(), recipe.clone());
    assert!(song.validate().unwrap_err().contains("recipe name"));

    song.generation_recipes.clear();
    let mut bad = recipe;
    bad.instrument = aqua_pattern_core::song::InstrumentId(99);
    song.generation_recipes.insert("lead".into(), bad);
    assert!(song
        .validate()
        .unwrap_err()
        .contains("Unknown generation recipe instrument"));
}
