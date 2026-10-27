use aqua_pattern_core::song::*;

fn cell(song: &mut Song) -> &mut Cell {
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .get_mut(&0)
        .unwrap()
}
#[test]
fn fixture_and_serialization_preserve_content_and_ids() {
    let mut song = example_song();
    song.validate().unwrap();
    assert_eq!(song.tracks.len(), 4);
    assert_eq!(song.patterns[&PatternId(1)].rows, 32);
    assert_eq!(song.patterns[&PatternId(2)].rows, 64);
    assert_eq!(
        song.order.iter().map(|o| o.pattern.0).collect::<Vec<_>>(),
        [1, 2, 1]
    );
    song.track_order.reverse();
    song.tracks.get_mut(&TrackId(1)).unwrap().name = "Renamed".into();
    song.validate().unwrap();
    let json = serde_json::to_string(&song).unwrap();
    let restored: Song = serde_json::from_str(&json).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, song);
    assert_eq!(serde_json::to_string(&restored).unwrap(), json);
}

#[test]
fn new_song_template_has_24_blank_single_column_channels_and_instruments() {
    let song = new_song_template();
    song.validate().unwrap();
    assert_eq!(song.tracks.len(), 24);
    assert_eq!(song.instruments.len(), 24);
    assert_eq!(song.track_order.len(), 24);
    assert!(song.tracks.values().all(|track| track.note_columns == 1));
    assert!(song.patterns[&PatternId(1)].tracks.is_empty());
    assert_eq!(song.order.len(), 1);
    assert_eq!(example_song().tracks.len(), 4);
}

#[test]
fn startup_song_expands_fixture_without_changing_its_content_or_layout() {
    let fixture = example_song();
    let startup = startup_song();
    startup.validate().unwrap();
    assert_eq!(startup.tracks.len(), 24);
    assert_eq!(startup.instruments.len(), 24);
    assert_eq!(startup.track_order[..4], fixture.track_order);
    for id in fixture.track_order {
        assert_eq!(startup.tracks[&id], fixture.tracks[&id]);
    }
    for (id, pattern) in fixture.patterns {
        assert_eq!(startup.patterns[&id], pattern);
    }
    assert_eq!(startup.order, fixture.order);
}
#[test]
fn repeat_shares_content_clone_is_independent_and_errors_are_atomic() {
    let mut song = example_song();
    song.append_repeat(PatternId(1)).unwrap();
    song.clone_pattern(PatternId(1), PatternId(3)).unwrap();
    let original = song.patterns[&PatternId(3)].clone();
    cell(&mut song).note = Note::Note(72);
    assert_eq!(song.patterns[&PatternId(3)], original);
    for index in [0, 2, 3] {
        assert_eq!(
            song.patterns[&song.order[index].pattern].tracks[&TrackId(1)][&0][&0].note,
            Note::Note(72)
        );
    }
    let before = song.clone();
    assert!(song.clone_pattern(PatternId(1), PatternId(2)).is_err());
    assert!(song.clone_pattern(PatternId(1), PatternId(0)).is_err());
    assert!(song.clone_pattern(PatternId(99), PatternId(4)).is_err());
    assert!(song.append_repeat(PatternId(99)).is_err());
    assert_eq!(song, before);
}
#[test]
fn optional_fields_are_independent_of_note_and_instrument_presence() {
    for note in [Note::Empty, Note::Note(0), Note::Note(127), Note::Off] {
        let mut song = example_song();
        *cell(&mut song) = Cell {
            note,
            instrument: None,
            velocity: Some(0.0),
            pan: Some(-1.0),
            delay: Some(255.0 / 256.0),
            ..Default::default()
        };
        song.validate().unwrap();
        let restored: Song = serde_json::from_str(&serde_json::to_string(&song).unwrap()).unwrap();
        assert_eq!(song, restored);
        assert_ne!(Cell::default(), *cell(&mut song));
    }
}
#[test]
fn rejects_dangling_references_and_invalid_track_order() {
    let base = example_song();
    let mut song = base.clone();
    song.instruments.remove(&InstrumentId(1));
    assert!(song.validate().is_err());
    let mut song = base.clone();
    song.patterns.remove(&PatternId(1));
    assert!(song.validate().is_err());
    let mut song = base.clone();
    song.tracks.remove(&TrackId(1));
    song.track_order.remove(0);
    assert!(song.validate().is_err());
    let mut song = base.clone();
    song.track_order.push(TrackId(1));
    assert!(song.validate().is_err());
    let mut song = base.clone();
    song.track_order[0] = TrackId(99);
    assert!(song.validate().is_err());
    let mut song = base.clone();
    song.track_order.pop();
    assert!(song.validate().is_err());
    let mut song = base;
    song.instruments.insert(
        InstrumentId(0),
        Instrument {
            phrases: Default::default(),
            name: String::new(),
            sample: None,
        },
    );
    assert!(song.validate().is_err());
}
#[test]
fn rejects_nonfinite_and_out_of_range_musical_values() {
    for value in [f64::NAN, f64::INFINITY, 19.9, 400.1] {
        let mut song = example_song();
        song.bpm = value;
        assert!(song.validate().is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        let mut song = example_song();
        cell(&mut song).velocity = Some(value);
        assert!(song.validate().is_err());
    }
    for value in [f32::NAN, f32::NEG_INFINITY, -1.01, 1.01] {
        let mut song = example_song();
        cell(&mut song).pan = Some(value);
        assert!(song.validate().is_err());
    }
    for value in [f64::NAN, f64::INFINITY, -0.01, 1.0] {
        let mut song = example_song();
        cell(&mut song).delay = Some(value);
        assert!(song.validate().is_err());
    }
    let mut song = example_song();
    cell(&mut song).note = Note::Note(128);
    assert!(song.validate().is_err());
    for value in [0, 257] {
        let mut song = example_song();
        song.lines_per_beat = value;
        assert!(song.validate().is_err());
        let mut song = example_song();
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = value;
        assert!(song.validate().is_err());
    }
}
#[test]
fn validates_sparse_geometry_and_schema_even_for_unused_patterns() {
    for rows in [0, 65537] {
        let mut song = example_song();
        song.patterns.get_mut(&PatternId(1)).unwrap().rows = rows;
        assert!(song.validate().is_err());
    }
    let mut song = example_song();
    song.schema = 2;
    assert!(song.validate().is_err());
    let mut song = example_song();
    song.patterns.get_mut(&PatternId(1)).unwrap().rows = 31;
    assert!(song.validate().is_err());
    let mut song = example_song();
    song.clone_pattern(PatternId(1), PatternId(3)).unwrap();
    song.patterns
        .get_mut(&PatternId(3))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .insert(2, Cell::default());
    assert!(song.validate().is_err());
    let mut song = example_song();
    song.patterns.clear();
    song.order.clear();
    song.instruments.clear();
    song.validate().unwrap(); // Empty project content is legitimate.
}
