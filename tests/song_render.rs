use aqua_pattern_core::{
    generate_sample,
    jobs::{Jobs, Request},
    project::{self, SaveMode},
    song::*,
    song_render::{self, Command, Release},
    RATE,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    song: Song,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "leviathan-m4-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let sample = dir.join("sample.wav");
        generate_sample(&sample).unwrap();
        let mut song = example_song();
        for (id, pattern) in &mut song.patterns {
            pattern.tracks.clear();
            pattern.rows = if id.0 == 1 { 4 } else { 6 };
        }
        for instrument in song.instruments.values_mut() {
            instrument.sample = Some(sample.clone());
        }
        put(
            &mut song,
            1,
            1,
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                velocity: Some(0.2),
                pan: Some(-0.25),
                ..Default::default()
            },
        );
        Self { dir, song }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn put(song: &mut Song, pattern: u64, track: u64, row: u32, cell: Cell) {
    song.patterns
        .get_mut(&PatternId(pattern))
        .unwrap()
        .tracks
        .entry(TrackId(track))
        .or_default()
        .entry(row)
        .or_default()
        .insert(0, cell);
}
fn audio(path: &std::path::Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect()
}
#[test]
fn sustained_voice_crosses_pattern_boundary_and_off_differs_from_song_end() {
    let mut f = Fixture::new();
    put(
        &mut f.song,
        2,
        1,
        3,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    let (stats, m) = song_render::export(&f.song, None, &f.path("sustain.wav")).unwrap();
    assert_eq!(stats.frames, 108000);
    assert!(!m.events.iter().any(|e| e.frame == 24000));
    assert!(m.events.iter().any(|e| e.frame == 42000
        && matches!(
            e.command,
            Command::Off {
                reason: Release::ExplicitOff,
                ..
            }
        )));
    assert!(m.events.iter().any(|e| e.frame == 84000
        && matches!(
            e.command,
            Command::Off {
                reason: Release::SongEnd,
                ..
            }
        )));
    let sustained = audio(&f.path("sustain.wav"));
    assert!(sustained[48000..60000].iter().map(|v| v.abs()).sum::<f32>() > 1.0);
    put(
        &mut f.song,
        1,
        1,
        3,
        Cell {
            note: Note::Off,
            ..Default::default()
        },
    );
    song_render::export(&f.song, None, &f.path("early-off.wav")).unwrap();
    let stopped = audio(&f.path("early-off.wav"));
    assert!(
        sustained[48000..60000]
            .iter()
            .zip(&stopped[48000..60000])
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            > 1.0
    );
}
#[test]
fn tracks_instruments_omitted_fields_and_parameter_only_rows_are_independent() {
    let mut f = Fixture::new();
    put(
        &mut f.song,
        1,
        2,
        0,
        Cell {
            note: Note::Note(72),
            instrument: Some(InstrumentId(2)),
            velocity: Some(0.15),
            pan: Some(1.0),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        1,
        0,
        Cell {
            velocity: Some(0.1),
            pan: Some(0.5),
            ..Default::default()
        },
    );
    put(
        &mut f.song,
        2,
        1,
        1,
        Cell {
            note: Note::Note(64),
            ..Default::default()
        },
    );
    let (stats, m) = song_render::export(&f.song, None, &f.path("routed.wav")).unwrap();
    assert!(stats.energy > 0.0 && stats.peak < 1.0);
    assert_eq!(m.assets.len(), 2);
    assert!(m.events.iter().any(|e| e.frame == 24000
        && e.instrument == InstrumentId(1)
        && matches!(e.command, Command::Volume { value, .. } if value == 0.1)));
    assert!(m.events.iter().any(|e| e.frame == 30000 && e.instrument == InstrumentId(1) && matches!(e.command, Command::On { pitch:64, volume, pan, .. } if volume == 0.1 && pan == 0.5)));
    let mixed = audio(&f.path("routed.wav"));
    f.song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .remove(&TrackId(2));
    song_render::export(&f.song, None, &f.path("solo.wav")).unwrap();
    assert_ne!(mixed, audio(&f.path("solo.wav")));
}
#[test]
fn shared_a_edits_repeat_and_cloned_b_changes_are_local() {
    let mut f = Fixture::new();
    f.song.clone_pattern(PatternId(1), PatternId(3)).unwrap();
    f.song.order[1].pattern = PatternId(3);
    put(
        &mut f.song,
        3,
        1,
        0,
        Cell {
            note: Note::Note(67),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    let m = song_render::evaluate(&f.song, None).unwrap();
    let notes: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On { pitch, .. } = e.command {
                Some((e.position.unwrap().0, pitch))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(notes, [(0, 60), (1, 67), (2, 60)]);
    put(
        &mut f.song,
        1,
        1,
        0,
        Cell {
            note: Note::Note(62),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        },
    );
    let m = song_render::evaluate(&f.song, None).unwrap();
    let pitches: Vec<_> = m
        .events
        .iter()
        .filter_map(|e| {
            if let Command::On { pitch, .. } = e.command {
                Some(pitch)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(pitches, [62, 67, 62]);
}
#[test]
fn fractional_timing_uses_absolute_rows_and_late_delay_stays_before_end() {
    let mut f = Fixture::new();
    f.song.bpm = 123.45;
    f.song.lines_per_beat = 7;
    put(
        &mut f.song,
        1,
        1,
        3,
        Cell {
            note: Note::Off,
            delay: Some(0.999999999),
            ..Default::default()
        },
    );
    let m = song_render::evaluate(&f.song, None).unwrap();
    let row = RATE as f64 * 60.0 / (123.45 * 7.0);
    assert_eq!(m.musical_frames, (14.0 * row).round() as u64);
    let onset: Vec<_> = m
        .events
        .iter()
        .filter(|e| matches!(e.command, Command::On { .. }))
        .map(|e| e.frame)
        .collect();
    assert_eq!(onset, [0, (10.0 * row).round() as u64]);
    let last_off = m
        .events
        .iter()
        .find(|e| e.position == Some((2, 3)))
        .unwrap();
    assert_eq!(last_off.frame, m.musical_frames - 1);
}
#[test]
fn saved_reopened_song_renders_identical_audio_and_manifest() {
    let f = Fixture::new();
    let project_path = f.path("song.json");
    project::save(&f.song, &project_path, SaveMode::CreateNew).unwrap();
    let (restored, _) = project::load(&project_path).unwrap();
    song_render::export(&f.song, None, &f.path("first.wav")).unwrap();
    song_render::export(&restored, None, &f.path("second.wav")).unwrap();
    assert_eq!(
        std::fs::read(f.path("first.wav")).unwrap(),
        std::fs::read(f.path("second.wav")).unwrap()
    );
    assert_eq!(
        std::fs::read(f.path("first.events.json")).unwrap(),
        std::fs::read(f.path("second.events.json")).unwrap()
    );
    let r = hound::WavReader::open(f.path("first.wav")).unwrap();
    assert_eq!(r.spec().sample_rate, 48000);
    assert_eq!(r.spec().channels, 2);
    assert_eq!(r.duration(), 108000);
    assert!(audio(&f.path("first.wav"))
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 1.0));
}
#[test]
fn missing_invalid_empty_and_no_clobber_failures_are_explicit() {
    let mut f = Fixture::new();
    let original = f.song.clone();
    f.song.order.clear();
    assert!(song_render::evaluate(&f.song, None).is_err());
    f.song = original.clone();
    f.song.patterns.values_mut().for_each(|p| p.tracks.clear());
    assert!(song_render::evaluate(&f.song, None).is_err());
    f.song = original.clone();
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = None;
    assert!(song_render::evaluate(&f.song, None)
        .unwrap_err()
        .contains("no sample"));
    f.song = original.clone();
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.path("missing.wav"));
    assert!(song_render::evaluate(&f.song, None).is_err());
    f.song = original.clone();
    f.song.bpm = f64::NAN;
    assert!(song_render::evaluate(&f.song, None).is_err());
    f.song = original;
    std::fs::write(f.path("bad.wav"), b"not audio").unwrap();
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.path("bad.wav"));
    assert!(song_render::export(&f.song, None, &f.path("bad-output.wav")).is_err());
    assert!(!f.path("bad-output.events.json").exists());
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.path("sample.wav"));
    std::fs::write(f.path("keep.wav"), b"keep").unwrap();
    assert!(song_render::export(&f.song, None, &f.path("keep.wav")).is_err());
    assert_eq!(std::fs::read(f.path("keep.wav")).unwrap(), b"keep");
    assert!(!f.path("keep.events.json").exists());
    std::fs::write(f.path("reserved.events.json"), b"keep manifest").unwrap();
    assert!(song_render::export(&f.song, None, &f.path("reserved.wav")).is_err());
    assert!(!f.path("reserved.wav").exists());
}
#[test]
fn background_export_owns_snapshot_while_editor_changes() {
    let mut f = Fixture::new();
    let mut jobs = Jobs::default();
    jobs.start(Request::SongExport {
        song: Box::new(f.song.clone()),
        project_dir: None,
        output: f.path("snapshot.wav"),
    })
    .unwrap();
    put(
        &mut f.song,
        1,
        1,
        0,
        Cell {
            note: Note::Note(90),
            ..Default::default()
        },
    );
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = jobs.poll() {
            result.unwrap();
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    let m: song_render::Manifest =
        serde_json::from_slice(&std::fs::read(f.path("snapshot.events.json")).unwrap()).unwrap();
    assert!(m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::On { pitch: 60, .. })));
    assert!(!m
        .events
        .iter()
        .any(|e| matches!(e.command, Command::On { pitch: 90, .. })));
}
#[test]
fn relative_paths_require_project_base_and_unselected_instrument_is_error() {
    let mut f = Fixture::new();
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some(PathBuf::from("sample.wav"));
    assert!(song_render::evaluate(&f.song, None).is_err());
    song_render::evaluate(&f.song, Some(&f.dir)).unwrap();
    f.song.patterns.get_mut(&PatternId(1)).unwrap().tracks = BTreeMap::new();
    put(
        &mut f.song,
        1,
        1,
        0,
        Cell {
            note: Note::Note(60),
            ..Default::default()
        },
    );
    assert!(song_render::evaluate(&f.song, Some(&f.dir))
        .unwrap_err()
        .contains("no selected instrument"));
}

#[test]
fn sample_assignment_is_protected_validated_and_undoable() {
    use aqua_pattern_core::editor::Editor;
    let f = Fixture::new();
    let mut e = Editor::default();
    assert!(e
        .assign_sample(InstrumentId(1), Some(f.path("sample.wav")))
        .is_err());
    e.edit_mode = true;
    assert!(e
        .assign_sample(InstrumentId(1), Some(f.path("missing.wav")))
        .is_err());
    e.assign_sample(InstrumentId(1), Some(f.path("sample.wav")))
        .unwrap();
    assert!(e.dirty);
    assert_eq!(
        e.song.instruments[&InstrumentId(1)].sample,
        Some(f.path("sample.wav"))
    );
    e.undo();
    assert_eq!(e.song.instruments[&InstrumentId(1)].sample, None);
    e.redo();
    assert!(e.song.instruments[&InstrumentId(1)].sample.is_some());
}

#[test]
fn volume_only_row_changes_the_running_audio() {
    let mut f = Fixture::new();
    song_render::export(&f.song, None, &f.path("normal.wav")).unwrap();
    put(
        &mut f.song,
        1,
        1,
        1,
        Cell {
            velocity: Some(0.0),
            ..Default::default()
        },
    );
    song_render::export(&f.song, None, &f.path("muted.wav")).unwrap();
    let normal = audio(&f.path("normal.wav"));
    let muted = audio(&f.path("muted.wav"));
    assert!(normal[24000..36000].iter().map(|v| v.abs()).sum::<f32>() > 1.0);
    assert!(muted[24000..36000].iter().all(|v| v.abs() < 0.00001));
}
#[test]
fn clipping_failure_cleans_only_new_outputs() {
    let mut f = Fixture::new();
    let loud = f.path("loud.wav");
    let mut writer = hound::WavWriter::create(
        &loud,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..RATE * 2 {
        writer.write_sample(0.9f32).unwrap();
    }
    writer.finalize().unwrap();
    f.song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(loud);
    for track in 1..=4 {
        let cells = f
            .song
            .patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(track))
            .or_default()
            .entry(0)
            .or_default();
        for column in 0..2 {
            cells.insert(
                column,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(1)),
                    velocity: Some(1.0),
                    ..Default::default()
                },
            );
        }
    }
    assert!(song_render::export(&f.song, None, &f.path("clip.wav"))
        .unwrap_err()
        .contains("clipping"));
    assert!(!f.path("clip.wav").exists());
    assert!(!f.path("clip.events.json").exists());
    assert!(f.path("loud.wav").exists());
}
