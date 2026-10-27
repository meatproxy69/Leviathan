use crate::{
    jobs::{Completed, Jobs, Request, SampleInfo},
    pattern_manifest, Event, Manifest, RATE,
};
use eframe::egui::{self, Color32, RichText, Vec2};
use std::{path::PathBuf, time::Duration};

fn native_sample_picker() -> Result<Option<PathBuf>, String> {
    Ok(rfd::FileDialog::new()
        .set_title("Load instrument sample")
        .add_filter("Audio samples", &["wav", "flac", "ogg", "aiff"])
        .add_filter("All files", &["*"])
        .pick_file())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Workspace {
    #[default]
    Tracker,
    Phrases,
    Arrangement,
    Transform,
    Generate,
    Audio,
}
impl Workspace {
    pub const ALL: [Self; 6] = [
        Self::Tracker,
        Self::Phrases,
        Self::Arrangement,
        Self::Transform,
        Self::Generate,
        Self::Audio,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Tracker => "Tracker",
            Self::Phrases => "Phrases",
            Self::Arrangement => "Arrange",
            Self::Transform => "Transform",
            Self::Generate => "Generate",
            Self::Audio => "Audio / Export",
        }
    }
}
fn sync_sample_form(
    song: &crate::song::Song,
    id: &mut u64,
    text: &mut String,
    seen: &mut Option<(u64, Option<PathBuf>)>,
) {
    if !song
        .instruments
        .contains_key(&crate::song::InstrumentId(*id))
    {
        *id = song.instruments.keys().next().map_or(0, |id| id.0);
    }
    let sample = song
        .instruments
        .get(&crate::song::InstrumentId(*id))
        .and_then(|i| i.sample.clone());
    let key = (*id, sample);
    if seen.as_ref() != Some(&key) {
        *text = key
            .1
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        *seen = Some(key);
    }
}
const MINT: Color32 = Color32::from_rgb(106, 227, 195);
pub struct Leviathan {
    workspace: Workspace,
    phrase_ui: crate::phrase_ui::PhraseUi,
    arrangement_ui: crate::arrangement_ui::ArrangementUi,
    transform_ui: crate::transform_ui::TransformUi,
    generation_ui: crate::generation_ui::GenerationUi,
    tracker: crate::tracker_ui::TrackerUi,
    legacy_tools: bool,
    session: crate::session::Session,
    project_path: String,
    pending_save_path: String,
    instrument_id: u64,
    asset_path: String,
    song_asset_seen: Option<(u64, Option<PathBuf>)>,
    tracker_sample_drop_rect: Option<egui::Rect>,
    song_output: String,
    transport: crate::transport::host::Transport,
    follow: bool,
    loop_song: bool,
    transport_smoke: bool,
    jobs: Jobs,
    sample: Option<SampleInfo>,
    sample_path: String,
    output_path: String,
    bpm: f64,
    bars: u32,
    manifest: Manifest,
    status: String,
    error: Option<String>,
    capture: Option<PathBuf>,
    capture_requested: bool,
    capture_frames: u8,
    export_finished: bool,
}
impl Leviathan {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(19, 23, 30);
        style.visuals.window_fill = Color32::from_rgb(25, 30, 39);
        style.visuals.selection.bg_fill = Color32::from_rgb(34, 95, 85);
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        cc.egui_ctx.set_style(style);
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = Self {
            workspace: Workspace::Tracker,
            phrase_ui: crate::phrase_ui::PhraseUi::default(),
            arrangement_ui: crate::arrangement_ui::ArrangementUi::default(),
            transform_ui: crate::transform_ui::TransformUi::default(),
            generation_ui: crate::generation_ui::GenerationUi::default(),
            tracker: crate::tracker_ui::TrackerUi::default(),
            legacy_tools: false,
            session: crate::session::Session::default(),
            project_path: String::new(),
            pending_save_path: String::new(),
            transport: crate::transport::host::Transport::default(),
            follow: true,
            loop_song: false,
            transport_smoke: false,
            instrument_id: 1,
            asset_path: String::new(),
            song_asset_seen: None,
            tracker_sample_drop_rect: None,
            song_output: root.join("song.wav").display().to_string(),
            jobs: Jobs::default(),
            sample: None,
            sample_path: String::new(),
            output_path: root.join("session.wav").display().to_string(),
            bpm: 120.0,
            bars: 2,
            manifest: pattern_manifest(120.0, 2).expect("valid defaults"),
            status: "Song editor ready · enable Edit Mode to enter notes".into(),
            error: None,
            capture: None,
            capture_requested: false,
            capture_frames: 0,
            export_finished: false,
        };
        let args: Vec<_> = std::env::args().collect();
        if args.len() == 3 && args[1] == "--project" {
            let path = PathBuf::from(&args[2]);
            let result = app.session.request(
                &mut app.tracker.editor,
                crate::session::Action::Open(path.clone()),
            );
            app.project_result(result);
            if app.error.is_none() {
                if app.tracker.editor.has_grid() {
                    let song = &app.tracker.editor.song;
                    let pattern = song.order[0].pattern;
                    let first = song.track_order.iter().copied().find(|track| {
                        song.block(pattern, *track)
                            .ok()
                            .flatten()
                            .and_then(|rows| rows.get(&0))
                            .is_some_and(|columns| {
                                columns
                                    .values()
                                    .any(|cell| matches!(cell.note, crate::song::Note::Note(_)))
                            })
                    });
                    if let Some(track) = first {
                        let _ = app.tracker.editor.select(0, track);
                    }
                }
                app.song_output = path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("next-export.wav")
                    .display()
                    .to_string();
                app.status = format!("Loaded {} · press Play song", app.tracker.editor.song.title);
                eprintln!("{}", app.status);
            }
        }

        if args.len() == 5 && args[1] == "--smoke-test" {
            app.sample_path = args[2].clone();
            app.output_path = args[3].clone();
            app.capture = Some(PathBuf::from(&args[4]));
            app.load();
        }
        if args.len() == 3 && args[1] == "--tracker-smoke" {
            app.capture = Some(PathBuf::from(&args[2]));
            app.export_finished = true;
            app.tracker.editor.edit_mode = true;
            app.tracker
                .editor
                .enter_note(crate::song::Note::Note(60))
                .expect("smoke edit");
            app.tracker
                .editor
                .select(2, crate::song::TrackId(1))
                .expect("smoke selection");
            app.tracker.editor.row = 0;
            assert_eq!(app.tracker.editor.cell().note, crate::song::Note::Note(60));
            eprintln!("Tracker smoke: edited A, verified shared A at occurrence 2");
        }
        if args.len() == 4 && args[1] == "--project-smoke" {
            let path = PathBuf::from(&args[2]);
            app.tracker.editor.edit_mode = true;
            app.tracker
                .editor
                .enter_note(crate::song::Note::Note(65))
                .expect("smoke edit");
            let expected = app.tracker.editor.song.clone();
            app.session
                .save_request(&mut app.tracker.editor, path.clone(), false)
                .expect("smoke save");
            assert!(
                app.session.overwrite.is_none(),
                "smoke requires a NEW project path"
            );
            app.tracker.editor.undo();
            assert!(app.tracker.editor.dirty);
            app.tracker.editor.redo();
            assert!(!app.tracker.editor.dirty);
            app.session
                .request(&mut app.tracker.editor, crate::session::Action::New)
                .expect("smoke new");
            app.session
                .request(
                    &mut app.tracker.editor,
                    crate::session::Action::Open(path.clone()),
                )
                .expect("smoke open");
            assert_eq!(app.tracker.editor.song, expected);
            app.project_path = path.display().to_string();
            app.status = "Project saved, undo/redo checked, then reopened successfully".into();
            app.capture = Some(PathBuf::from(&args[3]));
            app.export_finished = true;
            eprintln!("Project smoke: save, undo/redo saved-state, new/open roundtrip verified");
        }
        if args.len() == 5
            && matches!(
                args[1].as_str(),
                "--song-smoke"
                    | "--effects-smoke"
                    | "--phrase-smoke"
                    | "--arrangement-smoke"
                    | "--transform-smoke"
                    | "--generation-smoke"
                    | "--audio-smoke"
            )
        {
            app.session
                .request(
                    &mut app.tracker.editor,
                    crate::session::Action::Open(PathBuf::from(&args[2])),
                )
                .expect("smoke open song");
            app.project_path = args[2].clone();
            app.song_output = args[3].clone();
            app.capture = Some(PathBuf::from(&args[4]));
            if args[1] == "--audio-smoke" {
                app.workspace = Workspace::Audio;
                sync_sample_form(
                    &app.tracker.editor.song,
                    &mut app.instrument_id,
                    &mut app.asset_path,
                    &mut app.song_asset_seen,
                );
                assert!(!app.asset_path.is_empty());
                eprintln!("Audio smoke: loaded project sample field matches assigned sample");
            }
            if args[1] == "--generation-smoke" {
                use crate::{
                    generation::{Recipe, WriteMode},
                    song::TrackId,
                };
                let editor = &mut app.tracker.editor;
                editor.edit_mode = true;
                editor
                    .select(2, TrackId(4))
                    .expect("select generation target");
                editor.row = 0;
                let before = editor.song.clone();
                let recipe = Recipe {
                    rows: editor.pattern().rows.min(32),
                    instrument: crate::song::InstrumentId(4),
                    seed: 42,
                    probability: 0.75,
                    ..Default::default()
                };
                let preview = editor
                    .preview_generation("Native smoke", &recipe, WriteMode::Replace)
                    .expect("generation preview");
                assert!(preview.summary.changed_cells > 0);
                assert_eq!(editor.song, before);
                editor
                    .apply_generation(preview)
                    .expect("commit generated cells and recipe");
                assert_ne!(editor.song, before);
                editor.undo();
                assert_eq!(editor.song, before);
                editor.redo();
                assert_ne!(editor.song, before);
                editor.undo();
                assert_eq!(editor.song, before);
                app.workspace = Workspace::Generate;
                app.generation_ui
                    .preview_demo(editor)
                    .expect("show generation preview");
                eprintln!("Generation smoke: pure seeded preview, cell+recipe commit, undo/redo and restored snapshot export verified");
            }
            if args[1] == "--transform-smoke" {
                use crate::{
                    song::TrackId,
                    transform::{Scope, Transform},
                };
                let editor = &mut app.tracker.editor;
                editor.edit_mode = true;
                editor.select(0, TrackId(2)).expect("select hats");
                let before = editor.song.clone();
                let preview = editor
                    .preview_transform(
                        Scope::Track,
                        0..editor.pattern().rows,
                        &Transform::Groove(vec![0.0, 0.0, 0.375, 0.0]),
                    )
                    .expect("preview groove");
                assert!(preview.summary.changed_cells > 0);
                assert_eq!(editor.song, before);
                editor.apply_transform(preview).expect("commit groove");
                assert_ne!(editor.song, before);
                editor.undo();
                assert_eq!(editor.song, before);
                editor.redo();
                assert_ne!(editor.song, before);
                editor.undo();
                assert_eq!(editor.song, before);
                app.workspace = Workspace::Transform;
                app.transform_ui
                    .preview_track_groove(editor)
                    .expect("show preview");
                eprintln!("Transform smoke: pure preview, committed groove, undo/redo and restored snapshot verified");
            }
            if args[1] == "--arrangement-smoke" {
                let editor = &mut app.tracker.editor;
                editor.edit_mode = true;
                let before = editor.song.clone();
                editor.mark_section("Native smoke").expect("mark section");
                editor.mute_track(true).expect("mute occurrence track");
                editor.undo();
                editor.undo();
                assert_eq!(editor.song, before);
                editor.redo();
                assert_ne!(editor.song, before);
                editor.undo();
                assert_eq!(editor.song, before);
                app.workspace = Workspace::Arrangement;
                app.arrangement_ui.open = true;
                eprintln!("Arrangement smoke: section, mute and undo/redo verified; exporting restored snapshot");
            }
            if args[1] == "--phrase-smoke" {
                use crate::song::{Cell, InstrumentId, Note, Phrase, PhraseId};
                app.tracker.editor.edit_mode = true;
                app.tracker.editor.field = crate::editor::Field::Phrase;
                let before = app.tracker.editor.song.clone();
                let id = app
                    .tracker
                    .editor
                    .add_phrase(InstrumentId(1), Phrase::default())
                    .expect("create phrase");
                app.tracker
                    .editor
                    .set_phrase_cell(
                        InstrumentId(1),
                        id,
                        0,
                        0,
                        Cell {
                            note: Note::Note(64),
                            ..Default::default()
                        },
                    )
                    .expect("edit phrase cell");
                app.tracker.editor.undo();
                app.tracker.editor.undo();
                assert_eq!(app.tracker.editor.song, before);
                app.tracker.editor.redo();
                assert_ne!(app.tracker.editor.song, before);
                app.tracker.editor.undo();
                assert_eq!(app.tracker.editor.song, before);
                app.workspace = Workspace::Phrases;
                app.phrase_ui.select(InstrumentId(1), PhraseId(1));
                eprintln!("Phrase smoke: create, cell edit and undo/redo verified; exporting restored project");
            }
            if args[1] == "--effects-smoke" {
                app.tracker.editor.edit_mode = true;
                app.tracker.editor.field = crate::editor::Field::Effects;
                let before = app.tracker.editor.song.clone();
                app.tracker
                    .editor
                    .enter_value("offset=0.125 reverse cut=0.5")
                    .expect("effect edit");
                assert_ne!(app.tracker.editor.song, before);
                app.tracker.editor.undo();
                assert_eq!(app.tracker.editor.song, before);
                app.tracker.editor.redo();
                assert_ne!(app.tracker.editor.song, before);
                app.tracker.editor.undo();
                assert_eq!(app.tracker.editor.song, before);
                eprintln!("Effects smoke: guarded field edit / undo / redo verified; exporting restored project");
            }
            app.export_song();
        }
        if args.len() == 4 && args[1] == "--transport-smoke" {
            app.session
                .request(
                    &mut app.tracker.editor,
                    crate::session::Action::Open(PathBuf::from(&args[2])),
                )
                .expect("open transport smoke song");
            app.project_path = args[2].clone();
            app.capture = Some(PathBuf::from(&args[3]));
            app.transport_smoke = true;
            app.play_song();
        }
        app
    }
    fn project_result(&mut self, result: Result<(), String>) {
        match result {
            Err(error) => self.error = Some(error),
            Ok(()) => {
                self.error = None;
                if self.session.pending.is_none() && self.session.overwrite.is_none() {
                    self.status = self.session.notice.clone();
                    self.project_path = self
                        .session
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                }
            }
        }
    }
    fn request_action(&mut self, action: crate::session::Action) {
        self.pending_save_path = self
            .session
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let result = self.session.request(&mut self.tracker.editor, action);
        self.project_result(result);
    }
    fn project_controls(&mut self, ctx: &egui::Context) {
        let blocked = self.session.pending.is_some() || self.session.overwrite.is_some();
        egui::TopBottomPanel::top("project-file").show(ctx, |ui| {
            ui.add_enabled_ui(!blocked, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(if self.tracker.editor.dirty {
                        "PROJECT • unsaved"
                    } else {
                        "PROJECT"
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.project_path)
                            .desired_width((ui.available_width() - 480.0).max(110.0))
                            .hint_text("/path/to/song.leviathan.json"),
                    );
                    if ui.button("New").clicked() {
                        self.request_action(crate::session::Action::New);
                    }
                    if ui
                        .add_enabled(
                            !self.project_path.trim().is_empty(),
                            egui::Button::new("Open"),
                        )
                        .clicked()
                    {
                        self.request_action(crate::session::Action::Open(PathBuf::from(
                            self.project_path.trim(),
                        )));
                    }
                    if ui
                        .add_enabled(
                            !self.project_path.trim().is_empty(),
                            egui::Button::new("Save / Save as"),
                        )
                        .clicked()
                    {
                        let r = self.session.save_request(
                            &mut self.tracker.editor,
                            PathBuf::from(self.project_path.trim()),
                            false,
                        );
                        self.project_result(r);
                    }
                    if ui
                        .add_enabled(self.tracker.editor.can_undo(), egui::Button::new("Undo"))
                        .clicked()
                    {
                        self.tracker.editor.undo();
                    }
                    if ui
                        .add_enabled(self.tracker.editor.can_redo(), egui::Button::new("Redo"))
                        .clicked()
                    {
                        self.tracker.editor.redo();
                    }
                });
            });
        });
        if !blocked && ctx.memory(|m| m.has_focus(crate::tracker_ui::grid_id())) {
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                {
                    if modifiers.command || modifiers.ctrl {
                        match key {
                            egui::Key::N => self.request_action(crate::session::Action::New),
                            egui::Key::O if !self.project_path.trim().is_empty() => self
                                .request_action(crate::session::Action::Open(PathBuf::from(
                                    self.project_path.trim(),
                                ))),
                            egui::Key::S if !self.project_path.trim().is_empty() => {
                                let r = self.session.save_request(
                                    &mut self.tracker.editor,
                                    PathBuf::from(self.project_path.trim()),
                                    false,
                                );
                                self.project_result(r);
                            }
                            _ => (),
                        }
                    }
                }
            }
        }
        if let Some(overwrite) = &self.session.overwrite {
            let path = overwrite.path.display().to_string();
            egui::Modal::new(egui::Id::new("confirm-overwrite")).show(ctx, |ui| {
                ui.heading("Replace existing project?"); ui.label(path);
                ui.label("Only this explicitly confirmed file will be replaced. Changed targets are rejected.");
                if let Some(e) = &self.error { ui.colored_label(Color32::LIGHT_RED, e); }
                ui.horizontal(|ui| {
                    if ui.button("Replace file").clicked() { let r = self.session.confirm_overwrite(&mut self.tracker.editor); self.project_result(r); }
                    if ui.button("Cancel overwrite").clicked() { self.session.overwrite = None; }
                });
            });
        } else if self.session.pending.is_some() {
            egui::Modal::new(egui::Id::new("unsaved-song")).show(ctx, |ui| {
                ui.heading("Save changes before continuing?");
                ui.label("Save current song to:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.pending_save_path).desired_width(420.0),
                );
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.pending_save_path.trim().is_empty(),
                            egui::Button::new("Save and continue"),
                        )
                        .clicked()
                    {
                        let r = self.session.save_request(
                            &mut self.tracker.editor,
                            PathBuf::from(self.pending_save_path.trim()),
                            true,
                        );
                        self.project_result(r);
                    }
                    if ui.button("Discard and continue").clicked() {
                        let r = self.session.discard(&mut self.tracker.editor);
                        self.project_result(r);
                    }
                    if ui.button("Cancel").clicked() {
                        self.session.cancel();
                        self.error = None;
                    }
                });
            });
        }
    }
    fn play_song(&mut self) {
        let base = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_owned());
        match self.transport.start(self.tracker.editor.song.clone(), base) {
            Ok(()) => {
                self.error = None;
                self.status = "Transport preparing the immutable song snapshot".into();
                if self.loop_song {
                    if let Some(s) = &self.transport.shared {
                        s.looping.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn transport_controls(&mut self, ui: &mut egui::Ui) {
        use std::sync::atomic::Ordering;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.transport.busy()
                        && self.session.pending.is_none()
                        && self.session.overwrite.is_none(),
                    egui::Button::new("▶ Play song"),
                )
                .clicked()
            {
                self.play_song();
            }
            if ui
                .add_enabled(self.transport.busy(), egui::Button::new("■ Stop"))
                .clicked()
            {
                self.transport.stop();
            }
            if ui.checkbox(&mut self.loop_song, "Loop song").changed() && self.transport.busy() {
                if let (Some(state), Some(song)) =
                    (&self.transport.shared, &self.transport.snapshot)
                {
                    let rows: u64 = song
                        .order
                        .iter()
                        .map(|o| song.patterns[&o.pattern].rows as u64)
                        .sum();
                    let end = (rows as f64 * RATE as f64 * 60.0
                        / (song.bpm * song.lines_per_beat as f64))
                        .round() as u64;
                    state.set_loop(self.loop_song, end);
                }
            }
            ui.checkbox(&mut self.follow, "Follow");
            ui.separator();
            ui.label("BPM");
            let mut bpm = self.tracker.editor.song.bpm;
            let can_edit_bpm = self.tracker.editor.edit_mode
                && self.session.pending.is_none()
                && self.session.overwrite.is_none();
            let changed = ui
                .add_enabled_ui(can_edit_bpm, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut bpm)
                            .range(20.0..=400.0)
                            .speed(0.25),
                    )
                })
                .inner
                .changed();
            if changed {
                match self.tracker.editor.set_bpm(bpm) {
                    Ok(()) => {
                        self.error = None;
                        self.status = "Song tempo updated (undoable)".into();
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if ui
                .add_enabled(
                    self.transport.busy() && self.tracker.editor.has_grid(),
                    egui::Button::new("Seek to edit row"),
                )
                .on_hover_text("Cold seek: clears voices/tails; no reconstruction of earlier notes")
                .clicked()
            {
                if let (Some(state), Some(song)) =
                    (&self.transport.shared, &self.transport.snapshot)
                {
                    let occurrence = self.tracker.editor.occurrence;
                    if occurrence >= song.order.len()
                        || song.order[occurrence].pattern != self.tracker.editor.pattern_id()
                        || self.tracker.editor.row
                            >= song.patterns[&song.order[occurrence].pattern].rows
                    {
                        self.error = Some(
                            "Edit position differs from playback snapshot; restart playback first"
                                .into(),
                        );
                    } else {
                        let rows: u64 = song.order[..occurrence]
                            .iter()
                            .map(|o| song.patterns[&o.pattern].rows as u64)
                            .sum();
                        let frame =
                            ((rows + self.tracker.editor.row as u64) as f64 * RATE as f64 * 60.0
                                / (song.bpm * song.lines_per_beat as f64))
                                .round() as u64;
                        state.seek(frame);
                    }
                }
            }
        });
        if let (Some(state), Some(song)) = (&self.transport.shared, &self.transport.snapshot) {
            let frame = state.position.load(Ordering::Relaxed);
            let (occ, row) = crate::transport::location(song, frame);
            let status = if self.transport.busy() {
                if state.quit.load(Ordering::Relaxed) {
                    "STOPPING"
                } else if state.ready.load(Ordering::Relaxed) {
                    "PLAYING SNAPSHOT"
                } else {
                    "PREPARING / SEEKING"
                }
            } else if state.ended.load(Ordering::Relaxed) {
                "ENDED"
            } else {
                "STOPPED"
            };
            ui.label(format!(
                "{status} · position {} row {} · {:.2}s · {} underrun frames",
                occ + 1,
                row,
                frame as f64 / RATE as f64,
                state.underruns.load(Ordering::Relaxed)
            ));
        }
        ui.label(RichText::new("Playback help ⓘ").small().weak()).on_hover_text("Playback uses the start snapshot. Restart to hear edits. Seek / loop-toggle clears voices; loop restarts at row 0 without tail.");
        if let Some(error) = &self.transport.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
    fn export_song(&mut self) {
        if let Err(error) = self.tracker.editor.song.validate() {
            self.error = Some(error);
            return;
        }
        let song = self.tracker.editor.song.clone();
        let project_dir = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_owned());
        self.start(
            Request::SongExport {
                song,
                project_dir,
                output: PathBuf::from(self.song_output.trim()),
            },
            "Rendering song snapshot…",
        );
    }
    fn song_audio_controls(&mut self, ui: &mut egui::Ui) {
        sync_sample_form(
            &self.tracker.editor.song,
            &mut self.instrument_id,
            &mut self.asset_path,
            &mut self.song_asset_seen,
        );
        ui.heading("Song audio · instruments & offline export");
        ui.add_enabled_ui(self.session.pending.is_none() && self.session.overwrite.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    let before = self.instrument_id;
                    egui::ComboBox::from_id_salt("song-instrument").selected_text(format!("Instrument {}", self.instrument_id)).show_ui(ui, |ui| {
                        for (id, instrument) in &self.tracker.editor.song.instruments { ui.selectable_value(&mut self.instrument_id, id.0, format!("{} · {}", id.0, instrument.name)); }
                    });
                    if before != self.instrument_id { self.asset_path = self.tracker.editor.song.instruments.get(&crate::song::InstrumentId(self.instrument_id)).and_then(|i| i.sample.as_ref()).map(|p| p.display().to_string()).unwrap_or_default(); }
                    ui.add(egui::TextEdit::singleline(&mut self.asset_path).desired_width(230.0).hint_text("Sample path (blank unassigns)"));
                    if ui.add_enabled(self.tracker.editor.edit_mode, egui::Button::new("Assign sample")).clicked() {
                        let path = if self.asset_path.trim().is_empty() { None } else { Some(PathBuf::from(self.asset_path.trim())) };
                        let r = self.tracker.editor.assign_sample(crate::song::InstrumentId(self.instrument_id), path);
                        match r { Ok(()) => { self.error = None; self.status = "Instrument sample reference updated (undoable)".into(); }, Err(e) => self.error = Some(e) }
                    }
                });
                let assigned = self.tracker.editor.song.instruments.get(&crate::song::InstrumentId(self.instrument_id)).and_then(|i| i.sample.as_ref());
                ui.label(RichText::new(assigned.map(|p| format!("Assigned: {}", p.display())).unwrap_or_else(|| "No sample assigned".into())).small().weak());
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.song_output).desired_width(300.0).hint_text("New song WAV path"));
                    if ui.add_enabled(!self.jobs.busy() && !self.song_output.trim().is_empty(), egui::Button::new("Export song WAV")).clicked() { self.export_song(); }
                });
                ui.label(RichText::new("Exports current order snapshot. Edits after click affect the next export; transport is independent.").small().weak());
            });
    }
    fn tracker_instrument_controls(&mut self, ui: &mut egui::Ui) {
        sync_sample_form(
            &self.tracker.editor.song,
            &mut self.instrument_id,
            &mut self.asset_path,
            &mut self.song_asset_seen,
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Instrument sample").strong());
            if (self.tracker.editor.song.track_order.len() < 24 || self.tracker.editor.song.instruments.len() < 24)
                && ui.add_enabled(self.tracker.editor.edit_mode, egui::Button::new("Add 24 slots"))
                    .on_hover_text("Expand this song to at least 24 tracks and 24 instruments; existing content is preserved")
                    .clicked()
            {
                match self.tracker.editor.ensure_24_slots() {
                    Ok(()) => { self.error = None; self.status = "Added track and instrument slots (undoable)".into(); }
                    Err(error) => self.error = Some(error),
                }
            }
            let before = self.instrument_id;
            let selected_name = self
                .tracker
                .editor
                .song
                .instruments
                .get(&crate::song::InstrumentId(self.instrument_id))
                .map(|instrument| format!("{} · {}", self.instrument_id, instrument.name))
                .unwrap_or_else(|| format!("Instrument {}", self.instrument_id));
            egui::ComboBox::from_id_salt("tracker-instrument-sample")
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (id, instrument) in &self.tracker.editor.song.instruments {
                        ui.selectable_value(
                            &mut self.instrument_id,
                            id.0,
                            format!("{} · {}", id.0, instrument.name),
                        );
                    }
                });
            if before != self.instrument_id {
                sync_sample_form(
                    &self.tracker.editor.song,
                    &mut self.instrument_id,
                    &mut self.asset_path,
                    &mut self.song_asset_seen,
                );
            }
            self.tracker.selected_instrument = self
                .tracker
                .editor
                .song
                .instruments
                .contains_key(&crate::song::InstrumentId(self.instrument_id))
                .then_some(crate::song::InstrumentId(self.instrument_id));
            let path_response = ui.add(
                egui::TextEdit::singleline(&mut self.asset_path)
                    .desired_width(260.0)
                    .hint_text("Sample path · drop a file here"),
            );
            self.tracker_sample_drop_rect = Some(path_response.rect);
            if ui
                .add_enabled(self.tracker.editor.edit_mode, egui::Button::new("Assign"))
                .on_hover_text("Assign this sample path to the selected instrument (undoable)")
                .clicked()
            {
                self.assign_tracker_sample();
            }
            let can_browse = self.tracker.editor.edit_mode
                && self.session.pending.is_none()
                && self.session.overwrite.is_none();
            if ui
                .add_enabled(can_browse, egui::Button::new("Browse / Load sample"))
                .on_hover_text("Choose a sample file and assign it to this instrument")
                .clicked()
            {
                self.browse_tracker_sample();
            }
            let assigned = self
                .tracker
                .editor
                .song
                .instruments
                .get(&crate::song::InstrumentId(self.instrument_id))
                .and_then(|instrument| instrument.sample.as_ref());
            ui.label(
                RichText::new(
                    assigned
                        .map(|path| format!("Assigned: {}", path.display()))
                        .unwrap_or_else(|| "No sample assigned".into()),
                )
                .small()
                .weak(),
            );
        });
        ui.separator();
    }
    fn assign_tracker_sample(&mut self) {
        let path = if self.asset_path.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(self.asset_path.trim()))
        };
        match self
            .tracker
            .editor
            .assign_sample(crate::song::InstrumentId(self.instrument_id), path)
        {
            Ok(()) => {
                self.error = None;
                self.status = "Instrument sample reference updated (undoable)".into();
                self.song_asset_seen = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn browse_tracker_sample(&mut self) {
        if !self.tracker.editor.edit_mode
            || self.session.pending.is_some()
            || self.session.overwrite.is_some()
        {
            return;
        }
        match native_sample_picker() {
            Ok(Some(path)) => {
                self.asset_path = path.display().to_string();
                self.assign_tracker_sample();
            }
            Ok(None) => {}
            Err(error) => self.error = Some(error),
        }
    }
    fn start(&mut self, r: Request, status: &str) {
        match self.jobs.start(r) {
            Ok(()) => {
                self.status = status.into();
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn load(&mut self) {
        let path = PathBuf::from(self.sample_path.trim());
        self.start(Request::Load(path), "Loading sample…");
    }
    fn export(&mut self) {
        if let Some(sample) = &self.sample {
            self.start(
                Request::Export {
                    sample: sample.path.clone(),
                    output: PathBuf::from(self.output_path.trim()),
                    bpm: self.bpm,
                    bars: self.bars,
                },
                "Rendering offline…",
            );
        }
    }
    fn timeline(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Pattern");
            ui.label(RichText::new("C major seventh motif · 16 steps per bar").weak());
        });
        ui.label("Read-only pattern • C4 / E4 / G4 / B4 • 50% gate • alternating pan");
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(
                ui.available_width(),
                (ui.available_height() - 170.0).clamp(110.0, 220.0),
            ),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 8.0, Color32::from_rgb(13, 17, 23));
        let left = rect.left() + 42.0;
        let width = rect.width() - 54.0;
        let total = self.manifest.musical_frames as f32;
        for i in 0..=16 {
            let x = left + width * i as f32 / 16.0;
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + 27.0),
                    egui::pos2(x, rect.bottom() - 12.0),
                ],
                egui::Stroke::new(
                    1.0_f32,
                    if i % 4 == 0 {
                        Color32::from_gray(55)
                    } else {
                        Color32::from_gray(30)
                    },
                ),
            );
        }
        painter.text(
            egui::pos2(left, rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            format!(
                "01                                      {} bars / {:.2} s",
                self.bars,
                total / RATE as f32
            ),
            egui::FontId::monospace(12.0),
            Color32::GRAY,
        );
        for (row, (note, label)) in [(71, "B4"), (67, "G4"), (64, "E4"), (60, "C4")]
            .iter()
            .enumerate()
        {
            let row_height = (rect.height() - 48.0) / 4.0;
            let y = rect.top() + 36.0 + row as f32 * row_height;
            painter.text(
                egui::pos2(rect.left() + 10.0, y + 10.0),
                egui::Align2::LEFT_CENTER,
                *label,
                egui::FontId::monospace(12.0),
                Color32::GRAY,
            );
            for e in &self.manifest.events {
                if let Event::NoteOn { midi_note, .. } = e.event {
                    if midi_note == *note {
                        let x = left + e.frame as f32 / total * width;
                        let length = (width / (self.bars * 32) as f32).max(1.0);
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(x, y),
                                Vec2::new(length, (row_height - 6.0).clamp(8.0, 22.0)),
                            ),
                            3.0,
                            MINT,
                        );
                    }
                }
            }
        }
    }
}
impl eframe::App for Leviathan {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.transport.poll();
        if self.transport.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.tracker.playhead = None;
        if self.transport.busy() {
            if let (Some(state), Some(song)) = (&self.transport.shared, &self.transport.snapshot) {
                let (occ, row) = crate::transport::location(
                    song,
                    state.position.load(std::sync::atomic::Ordering::Relaxed),
                );
                if self
                    .tracker
                    .editor
                    .song
                    .order
                    .get(occ)
                    .is_some_and(|o| o.pattern == song.order[occ].pattern)
                {
                    self.tracker.playhead = Some((occ, row));
                }
            }
        }
        if self.transport_smoke {
            if let Some(error) = &self.transport.error {
                panic!("Transport smoke failed: {error}");
            }
            if let Some(state) = &self.transport.shared {
                if state
                    .audible_frames
                    .load(std::sync::atomic::Ordering::Relaxed)
                    > 24000
                    && !self.capture_requested
                {
                    self.export_finished = true;
                }
            }
            if self.capture_requested && self.capture.is_none() {
                self.transport.stop();
            }
        }
        if let Some(result) = self.jobs.poll() {
            match result {
                Ok(Completed::Loaded(info)) => {
                    self.status = format!(
                        "Loaded {}",
                        info.path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    self.sample = Some(info);
                    if self.capture.is_some() {
                        self.export();
                    }
                }
                Ok(Completed::Exported { path, stats }) => {
                    self.status = format!(
                        "Exported {} · {} frames · peak {:.3}",
                        path.display(),
                        stats.frames,
                        stats.peak
                    );
                    self.export_finished = true;
                }
                Err(error) => {
                    self.status = "Job failed".into();
                    eprintln!("Leviathan job error: {error}");
                    self.error = Some(error);
                    if self.capture.is_some() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = self.capture.take() {
                    let saved = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let file = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)?;
                        let pixels: Vec<u8> =
                            image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        let rgba = image::RgbaImage::from_raw(
                            image.width() as u32,
                            image.height() as u32,
                            pixels,
                        )
                        .ok_or("invalid screenshot")?;
                        image::DynamicImage::ImageRgba8(rgba).write_to(
                            &mut std::io::BufWriter::new(file),
                            image::ImageFormat::Png,
                        )?;
                        Ok(())
                    })();
                    eprintln!("UI smoke capture {}: {:?}", path.display(), saved);
                    self.transport.stop();
                    self.session.close_ready = true;
                }
            }
        }
        if self.export_finished && self.capture.is_some() && !self.capture_requested {
            self.capture_frames += 1;
            if self.capture_frames >= 3 {
                self.capture_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.request_repaint();
        }
        if self.jobs.busy() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.capture_requested
            && !self.session.close_ready
        {
            self.transport.stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.session.pending.is_none() && self.session.overwrite.is_none() {
                self.request_action(crate::session::Action::Close);
            }
        }
        if self.session.close_ready {
            if self.jobs.busy() || self.transport.busy() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !self.jobs.busy() {
            if let Some((path, position)) = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .first()
                    .and_then(|file| Some((file.path.clone()?, i.pointer.hover_pos()?)))
            }) {
                if self.legacy_tools {
                    self.sample_path = path.display().to_string();
                    self.load();
                } else if self.workspace == Workspace::Tracker
                    && self.session.pending.is_none()
                    && self.session.overwrite.is_none()
                    && !(self.follow && self.transport.busy())
                    && self
                        .tracker_sample_drop_rect
                        .is_some_and(|rect| rect.contains(position))
                {
                    self.asset_path = path.display().to_string();
                    self.assign_tracker_sample();
                }
            }
        }
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("LEVIATHAN").size(19.0).strong().color(MINT));
                ui.separator();
                ui.label("PATTERN STUDIO");
                ui.checkbox(&mut self.legacy_tools, "Legacy motif export tools");
            });
        });
        self.project_controls(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                if self.jobs.busy() || self.transport.busy() {
                    ui.spinner();
                } else {
                    ui.colored_label(MINT, "Ready");
                }
                ui.label(&self.status);
            });
            if self.session.close_ready && self.jobs.busy() {
                ui.label("Finishing the active job before closing…");
            }
            if let Some(e) = &self.error {
                ui.colored_label(Color32::from_rgb(255, 145, 145), e);
            }
            ui.add_space(5.0);
        });
        if self.legacy_tools {
            egui::SidePanel::left("source").exact_width(290.0).show(ctx,|ui|{
   ui.add_space(16.0);ui.heading("Fixed-motif source");ui.label("Drop an audio file here, or enter its path.");
   ui.add_enabled_ui(!self.jobs.busy(),|ui|{ui.add(egui::TextEdit::singleline(&mut self.sample_path).hint_text("/path/to/sample.wav").desired_width(f32::INFINITY));if ui.add_enabled(!self.sample_path.trim().is_empty(),egui::Button::new("Load sample").min_size(Vec2::new(260.0,32.0))).clicked(){self.load();}});
   if let Some(s)=&self.sample{ui.add_space(6.0);ui.colored_label(MINT,s.path.file_name().unwrap_or_default().to_string_lossy());ui.label(format!("{} Hz · {} channels · {:.2} s",s.rate,s.channels,s.frames as f64/s.rate as f64));ui.label(RichText::new(s.path.display().to_string()).small().weak());}else{ui.label(RichText::new("No sample loaded").weak());}
   ui.add_space(22.0);ui.separator();ui.heading("Legacy motif export");ui.colored_label(MINT,"Tracker edits are NOT rendered here.");ui.label("New WAV destination");ui.add_enabled_ui(!self.jobs.busy(),|ui|{ui.add(egui::TextEdit::singleline(&mut self.output_path).desired_width(f32::INFINITY));});
   ui.label(RichText::new("Writes WAV + event JSON. Existing files are never replaced.").small().weak());
   if ui.add_enabled(self.sample.is_some()&&!self.jobs.busy(),egui::Button::new(RichText::new("Export WAV").strong()).fill(Color32::from_rgb(35,107,89)).min_size(Vec2::new(260.0,40.0))).clicked(){self.export();}
   ui.add_space(24.0);ui.separator();ui.label(RichText::new("ENGINE").small().strong());ui.label("phonic 0.16  +  pattrns 0.9.3");ui.label(RichText::new("Sample playback and deterministic rendering are connected. MIDI, plugins and discovery are not installed features.").small().weak());
  });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.legacy_tools {
                self.transport_controls(ui);
                ui.horizontal_wrapped(|ui| {
                    for workspace in Workspace::ALL {
                        if ui
                            .selectable_value(&mut self.workspace, workspace, workspace.label())
                            .changed()
                        {
                            ui.ctx()
                                .memory_mut(|m| m.surrender_focus(crate::tracker_ui::grid_id()));
                        }
                    }
                });
                ui.separator();
                if self.workspace != Workspace::Tracker {
                    ui.add_enabled_ui(
                        self.session.pending.is_none() && self.session.overwrite.is_none(),
                        |ui| match self.workspace {
                            Workspace::Phrases => {
                                self.phrase_ui.contents(ui, &mut self.tracker.editor)
                            }
                            Workspace::Arrangement => {
                                self.arrangement_ui.contents(ui, &mut self.tracker.editor)
                            }
                            Workspace::Transform => {
                                self.transform_ui.contents(ui, &mut self.tracker.editor)
                            }
                            Workspace::Generate => {
                                egui::ScrollArea::vertical()
                                    .id_salt("generation-workspace")
                                    .show(ui, |ui| {
                                        self.generation_ui.contents(ui, &mut self.tracker.editor)
                                    });
                            }
                            Workspace::Audio => {
                                ui.checkbox(&mut self.tracker.editor.edit_mode, "Edit Mode");
                                self.song_audio_controls(ui);
                            }
                            Workspace::Tracker => (),
                        },
                    );
                    return;
                }
                if self.follow && self.transport.busy() {
                    if let (Some(state), Some(song)) =
                        (&self.transport.shared, &self.transport.snapshot)
                    {
                        let (occ, row) = crate::transport::location(
                            song,
                            state.position.load(std::sync::atomic::Ordering::Relaxed),
                        );
                        crate::tracker_ui::playback_view(ui, song, occ, row);
                        return;
                    }
                }
                ui.add_enabled_ui(
                    self.session.pending.is_none() && self.session.overwrite.is_none(),
                    |ui| {
                        self.tracker_instrument_controls(ui);
                        self.tracker.show(ui);
                    },
                );
                return;
            }
            ui.colored_label(
                MINT,
                "LEGACY FIXED MOTIF — independent of the editable song",
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.heading("Transport");
                ui.label(
                    RichText::new(if self.jobs.busy() {
                        "WORKING"
                    } else {
                        "STOPPED"
                    })
                    .color(MINT),
                );
            });
            ui.horizontal(|ui| {
                ui.add_enabled(false, egui::Button::new("▶ Live play"))
                    .on_hover_text("Live pattern scheduling is not implemented");
                ui.label("Offline session · no live playback");
            });
            ui.add_space(6.0);
            let mut changed = false;
            ui.add_enabled_ui(!self.jobs.busy(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("BPM");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.bpm)
                                .range(20.0..=400.0)
                                .speed(0.25),
                        )
                        .changed();
                    ui.label("Bars");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.bars).range(1..=256))
                        .changed();
                    ui.label("4 / 4");
                    ui.separator();
                    ui.label(format!(
                        "{} frames + 0.5 s tail",
                        self.manifest.musical_frames
                    ));
                });
            });
            if changed {
                match pattern_manifest(self.bpm, self.bars) {
                    Ok(m) => self.manifest = m,
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            ui.add_space(18.0);
            self.timeline(ui);
            ui.add_space(15.0);
            ui.horizontal(|ui| {
                ui.heading("Event inspector");
                ui.label(format!(
                    "{} deterministic events",
                    self.manifest.events.len()
                ));
            });
            ui.label(
                RichText::new("FRAME          TIME        EVENT / NOTE / VOICE")
                    .monospace()
                    .weak(),
            );
            egui::ScrollArea::vertical()
                .id_salt("events")
                .auto_shrink([false, false])
                .show_rows(ui, 22.0, self.manifest.events.len(), |ui, range| {
                    for i in range {
                        let e = &self.manifest.events[i];
                        let text = match e.event {
                            Event::NoteOn { id, midi_note, .. } => {
                                format!("ON    MIDI {midi_note:<3}  voice {id}")
                            }
                            Event::NoteOff { id } => format!("OFF             voice {id}"),
                            Event::AllNotesOff => "ALL NOTES OFF".into(),
                        };
                        ui.label(
                            RichText::new(format!(
                                "{:>8}     {:>7.3}s     {}",
                                e.frame,
                                e.frame as f64 / RATE as f64,
                                text
                            ))
                            .monospace(),
                        );
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_form_tracks_loaded_documents_and_undo_without_erasing_a_draft() {
        let mut song = crate::song::example_song();
        let mut id = 999;
        let mut text = String::new();
        let mut seen = None;
        song.instruments
            .get_mut(&crate::song::InstrumentId(1))
            .unwrap()
            .sample = Some("/tmp/first.wav".into());
        sync_sample_form(&song, &mut id, &mut text, &mut seen);
        assert_eq!(id, 1);
        assert_eq!(text, "/tmp/first.wav");
        text = "/tmp/draft.wav".into();
        sync_sample_form(&song, &mut id, &mut text, &mut seen);
        assert_eq!(text, "/tmp/draft.wav");
        song.instruments
            .get_mut(&crate::song::InstrumentId(1))
            .unwrap()
            .sample = Some("/tmp/opened.wav".into());
        sync_sample_form(&song, &mut id, &mut text, &mut seen);
        assert_eq!(text, "/tmp/opened.wav");
        song.instruments
            .get_mut(&crate::song::InstrumentId(1))
            .unwrap()
            .sample = None;
        sync_sample_form(&song, &mut id, &mut text, &mut seen);
        assert!(text.is_empty());
    }
}
