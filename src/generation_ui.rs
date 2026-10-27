//! Bounded control-side pattrns preview. Committing appends ordinary editable Song cells.
//! This inspector owns neither a second sequencer nor an audio/audition path.
use crate::{
    editor::{note_label, Editor, Field},
    generation::{self, GenerationConfig, GenerationNote, GenerationPreset, GenerationPreview},
    song::{InstrumentId, InstrumentKind, Note, PatternId, TrackId},
    tracker_ui::{grid_id, AQUA},
};
use eframe::egui::{self, Color32, RichText};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const AMBER: Color32 = Color32::from_rgb(229, 185, 112);

#[derive(Clone, PartialEq, Eq)]
struct Context {
    occurrence: usize,
    pattern: Option<PatternId>,
    track: TrackId,
    row: u32,
    column: u16,
    field: Field,
}
impl Context {
    fn of(editor: &Editor) -> Self {
        Self {
            occurrence: editor.occurrence,
            pattern: editor
                .song
                .order
                .get(editor.occurrence)
                .map(|order| order.pattern),
            track: editor.track,
            row: editor.row,
            column: editor.column,
            field: editor.field,
        }
    }
}
#[derive(Clone, PartialEq)]
struct NoteDraft {
    note: String,
    velocity: String,
}
#[derive(Clone, PartialEq)]
struct Draft {
    context: Context,
    revision: u64,
    name: String,
    instrument: InstrumentId,
    bars: String,
    seed: String,
    steps: String,
    pulses: String,
    rotation: String,
    probability: String,
    sequence: Vec<NoteDraft>,
}
fn integer(text: &str, min: u64, max: u64, label: &str) -> Result<u64, String> {
    let value = text.parse::<u64>().map_err(|_| {
        format!("{label} must be an integer in {min}..{max}; no rounding or clamping")
    })?;
    if !(min..=max).contains(&value) {
        return Err(format!("{label} must be in {min}..{max}; no clamping"));
    }
    Ok(value)
}
fn unit_float(text: &str, label: &str) -> Result<f32, String> {
    let value = text
        .parse::<f64>()
        .map_err(|_| format!("{label} must be a finite decimal in 0..1"))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!("{label} must be in 0..1; no clamping"));
    }
    Ok(value as f32)
}
impl Draft {
    fn effective_config(&self) -> Result<GenerationConfig, String> {
        let steps = integer(&self.steps, 1, 128, "Steps per bar")? as u16;
        let pulses = integer(&self.pulses, 1, u64::from(steps), "Pulses")? as u16;
        let rotation = integer(&self.rotation, 0, u64::from(steps - 1), "Rotation")? as u16;
        let mut sequence = Vec::with_capacity(self.sequence.len());
        for (index, draft) in self.sequence.iter().enumerate() {
            sequence.push(GenerationNote {
                note: integer(&draft.note, 12, 127, &format!("MIDI {:02}", index + 1))? as u8,
                velocity: unit_float(&draft.velocity, &format!("Velocity {:02}", index + 1))?,
            });
        }
        Ok(GenerationConfig {
            version: generation::VERSION,
            name: self.name.clone(),
            bars: integer(&self.bars, 1, 16, "Bars")? as u16,
            instrument: self.instrument,
            seed: integer(&self.seed, 0, u64::MAX, "Seed")?,
            preset: GenerationPreset::Euclidean {
                steps,
                pulses,
                rotation,
                probability: unit_float(&self.probability, "Keep probability")?,
            },
            sequence,
        })
    }
}
struct Pending {
    epoch: u64,
    revision: u64,
    receiver: Receiver<Result<GenerationPreview, String>>,
}
struct Cached {
    revision: u64,
    preview: GenerationPreview,
    reveal: bool,
}
#[derive(Default)]
pub(crate) struct GenerationUi {
    draft: Option<Draft>,
    pending: Option<Pending>,
    cached: Option<Cached>,
    epoch: u64,
    error: Option<String>,
    notice: Option<String>,
}
impl GenerationUi {
    pub(crate) fn open(&mut self, editor: &Editor) {
        self.reset_document();
        self.draft = Some(Draft {
            context: Context::of(editor),
            revision: editor.revision(),
            name: "Euclidean sequence".into(),
            instrument: editor
                .cell()
                .instrument
                .or_else(|| editor.song.instruments.keys().next().copied())
                .unwrap_or(InstrumentId(0)),
            bars: "1".into(),
            seed: "1".into(),
            steps: "16".into(),
            pulses: "5".into(),
            rotation: "0".into(),
            probability: "1".into(),
            sequence: [60, 64, 67, 71]
                .into_iter()
                .map(|note| NoteDraft {
                    note: note.to_string(),
                    velocity: "0.7".into(),
                })
                .collect(),
        });
    }
    pub(crate) fn is_open(&self) -> bool {
        self.draft.is_some()
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn reset_document(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.draft = None;
        self.cached = None;
        self.error = None;
        self.notice = None;
        // Keep canceled workers tracked until completion: no unbounded replacement jobs.
    }
    fn invalidate(&mut self, notice: &str) {
        self.epoch = self.epoch.wrapping_add(1);
        self.cached = None;
        self.error = None;
        self.notice = Some(notice.into());
    }
    pub(crate) fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Generation worker disconnected; no document changed".into())
            }
        };
        let pending = self.pending.take().unwrap();
        if pending.epoch != self.epoch || self.draft.is_none() {
            return;
        }
        match result {
            Ok(preview) => {
                self.cached = Some(Cached {
                    revision: pending.revision,
                    preview,
                    reveal: true,
                });
                self.notice = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn start_preview(&mut self, editor: &Editor) {
        if self.pending.is_some() {
            return;
        }
        let Some(draft) = &self.draft else {
            return;
        };
        let config = match draft.effective_config() {
            Ok(config) => config,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let revision = editor.revision();
        let song = editor.song.clone();
        self.cached = None;
        self.error = None;
        self.notice = None;
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-generation-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    generation::preview(&song, config)
                }))
                .unwrap_or_else(|_| Err("Generation worker panicked; no document changed".into()));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    revision,
                    receiver,
                })
            }
            Err(error) => self.error = Some(format!("Could not start generation preview: {error}")),
        }
    }
    fn current(&self, editor: &Editor) -> bool {
        let (Some(draft), Some(cached)) = (&self.draft, &self.cached) else {
            return false;
        };
        Context::of(editor) == draft.context
            && cached.revision == editor.revision()
            && cached.preview.base() == &editor.song
            && draft
                .effective_config()
                .is_ok_and(|config| &config == cached.preview.config())
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_preview(&self, editor: &Editor) -> Option<&GenerationPreview> {
        self.current(editor)
            .then(|| &self.cached.as_ref().unwrap().preview)
    }
    /// True only after a successful explicit Commit & show; the caller reveals the song grid.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
        let Some(mut draft) = self.draft.take() else {
            return false;
        };
        if Context::of(editor) != draft.context {
            self.reset_document();
            return false;
        }
        if draft.revision != editor.revision() {
            draft.revision = editor.revision();
            self.invalidate("Document changed: prepare a new preview before appending.");
        }
        let allowed = ui.is_enabled();
        let writable = allowed && editor.edit_mode;
        let old_draft = draft.clone();
        let mut open = true;
        let mut preview = false;
        let mut commit = false;
        let mut close = false;
        let ctx = ui.ctx().clone();
        let body_height = (ctx.content_rect().height() - 405.0).max(140.0);
        egui::Window::new("Seeded pattern generation")
            .id(egui::Id::new("seeded-generation"))
            .default_pos(egui::pos2(220.0, 135.0))
            .default_width(620.0).min_width(500.0).resizable(false).collapsible(false).open(&mut open)
            .show(&ctx, |ui| {
                ui.set_max_width(620.0);
                ui.label(RichText::new("PATTRNS · PREVIEW -> ORDINARY EDITABLE CELLS").strong().color(AQUA));
                ui.label(RichText::new(format!("{} · v{} · document revision {}", generation::ALGORITHM, generation::VERSION, draft.revision)).small().color(MUTED));
                ui.label(RichText::new(format!("Song clock unchanged: {} BPM / {} LPB · four beats per bar", editor.song.bpm, editor.song.lines_per_beat)).small());
                egui::ScrollArea::vertical().id_salt("generation-body").max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add_enabled_ui(allowed, |ui| config_fields(ui, editor, &mut draft));
                    ui.separator();
                    ui.label(RichText::new("A fresh track, pattern and final order occurrence are appended. Existing cells and timing stay unchanged. Extending SongEnd can reveal truncated phrase child notes and continue old voices/tails. No audition or arbitrary code.").small().color(MUTED));
                    if let Some(cached) = &mut self.cached { preview_details(ui, &cached.preview, cached.reveal); cached.reveal = false; }
                });
                ui.separator();
                if let Some(cached) = &self.cached {
                    let summary = cached.preview.summary();
                    ui.label(RichText::new(format!("READY · append POS {:02} / P{} / T{} · {} rows / {} columns · {} hits", summary.occurrence, summary.pattern.0, summary.track.0, summary.rows, summary.columns, summary.hits)).small().color(AQUA));
                    ui.label(RichText::new(config_summary(cached.preview.config())).small().color(MUTED));
                    ui.label(RichText::new(format!("Frames [{}, {}) · {} output frames (~{:.3} s)", summary.start_frame, summary.end_frame, summary.end_frame.saturating_sub(summary.start_frame), summary.end_frame.saturating_sub(summary.start_frame) as f64 / f64::from(crate::RATE))).small().color(MUTED));
                }
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
                if let Some(notice) = &self.notice { ui.label(RichText::new(notice).small().color(AMBER)); }
                if self.pending.is_some() { ui.label(RichText::new("Generation worker running · no document mutation").small().color(MUTED)); }
                if !editor.edit_mode { ui.label(RichText::new("READ MODE · preview is allowed; enable Edit mode to commit").small().color(AMBER)); }
                let current = self.cached.as_ref().is_some_and(|cached| cached.revision == editor.revision() && cached.preview.base() == &editor.song && draft.effective_config().is_ok_and(|config| &config == cached.preview.config()));
                ui.horizontal(|ui| {
                    preview = ui.add_enabled(allowed && self.pending.is_none(), egui::Button::new("Preview generation")).clicked();
                    commit = ui.add_enabled(writable && current && self.pending.is_none(), egui::Button::new("Commit & show")).clicked();
                    close = ui.button("Cancel / close").clicked();
                });
                ui.label(RichText::new("Commit appends once; Undo restores the document. Output is ordinary notes, not a live generator. Settings changes discard the preview.").small().color(MUTED));
            });
        ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        if !open || close {
            self.reset_document();
            return false;
        }
        if old_draft != draft {
            self.invalidate("Settings changed: preview the exact new generation.");
        }
        self.draft = Some(draft);
        if preview && allowed {
            self.start_preview(editor);
        }
        if commit && writable && self.current(editor) {
            let cached = self.cached.as_ref().unwrap();
            let occurrence = cached.preview.summary().occurrence;
            let track = cached.preview.summary().track;
            match editor.commit_generation_preview(cached.revision, &cached.preview) {
                Ok(()) => {
                    // These IDs are validated members of the exact committed candidate.
                    if let Err(error) = editor.select(occurrence, track) {
                        self.error = Some(error);
                        return false;
                    }
                    editor.row = 0;
                    editor.column = 0;
                    editor.field = Field::Note;
                    self.reset_document();
                    return true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        false
    }
}
fn number(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(width));
}
fn config_fields(ui: &mut egui::Ui, editor: &Editor, draft: &mut Draft) {
    ui.label(RichText::new("EUCLIDEAN GATES / EXPLICIT NOTE SEQUENCE").strong());
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(240.0));
    });
    ui.horizontal(|ui| {
        ui.label("Instrument");
        let label = editor.song.instruments.get(&draft.instrument).map_or_else(
            || format!("I{} · missing", draft.instrument.0),
            |instrument| {
                format!(
                    "I{} · {} · {}",
                    draft.instrument.0,
                    instrument.name,
                    match instrument.kind {
                        InstrumentKind::Pitched => "pitched",
                        InstrumentKind::DrumKit { .. } => "kit selectors",
                        InstrumentKind::Multisample { .. } => "melodic key zones",
                    }
                )
            },
        );
        egui::ComboBox::from_id_salt("generation-instrument")
            .width(330.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (id, instrument) in &editor.song.instruments {
                    ui.selectable_value(
                        &mut draft.instrument,
                        *id,
                        format!(
                            "I{} · {} · {}",
                            id.0,
                            instrument.name,
                            match instrument.kind {
                                InstrumentKind::Pitched => "pitched",
                                InstrumentKind::DrumKit { .. } => "kit selectors",
                                InstrumentKind::Multisample { .. } => "melodic key zones",
                            }
                        ),
                    );
                }
            });
    });
    if let Some(instrument) = editor.song.instruments.get(&draft.instrument) {
        let assignment = if let Some(region) = instrument.region {
            format!("Region R{}", region.0)
        } else if let Some(path) = &instrument.sample {
            format!("Sample {}", path.display())
        } else if matches!(instrument.kind, InstrumentKind::DrumKit { .. }) {
            "Kit pad regions checked by preview".into()
        } else if let InstrumentKind::Multisample { map } = instrument.kind {
            format!(
                "Melodic map M{} · actual note/velocity coverage checked by evaluation",
                map.0
            )
        } else {
            "UNASSIGNED · assign a sample/region before generating".into()
        };
        ui.add(egui::Label::new(RichText::new(&assignment).small().color(MUTED)).truncate())
            .on_hover_text(assignment);
    }
    if editor
        .song
        .instruments
        .get(&draft.instrument)
        .is_some_and(|instrument| matches!(instrument.kind, InstrumentKind::DrumKit { .. }))
    {
        ui.label(RichText::new("Kit MIDI values select mapped pads, not pitch. Every configured selector must have an assigned region; pad tuning/policies remain authoritative.").small().color(AMBER));
    }
    ui.horizontal(|ui| {
        number(ui, "Seed", &mut draft.seed, 175.0);
        number(ui, "Bars", &mut draft.bars, 40.0);
        ui.label(RichText::new("1..16").small().color(MUTED));
    });
    ui.horizontal(|ui| {
        number(ui, "Steps / bar", &mut draft.steps, 45.0);
        number(ui, "Pulses", &mut draft.pulses, 45.0);
        number(ui, "Rotation", &mut draft.rotation, 45.0);
        ui.label(RichText::new("steps 1..128").small().color(MUTED));
    });
    ui.horizontal(|ui| {
        number(ui, "Keep probability", &mut draft.probability, 85.0);
        ui.label(
            RichText::new("0..1 · all-closed results reject, no fallback")
                .small()
                .color(MUTED),
        );
    });
    ui.label(RichText::new("One seeded gate stream spans all bars; the sequence advances only on open gates. Keep 1 retains every pulse. No pattrns note-duration import: normal column replacement / kit policies apply. Decode/clipping checks remain at export/playback.").small().color(MUTED));
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("SEQUENCE · {} / 16 entries", draft.sequence.len())).strong(),
        );
        if ui
            .add_enabled(draft.sequence.len() < 16, egui::Button::new("+ Note"))
            .clicked()
        {
            draft.sequence.push(NoteDraft {
                note: "60".into(),
                velocity: "0.7".into(),
            });
        }
        if ui
            .add_enabled(draft.sequence.len() > 1, egui::Button::new("Remove last"))
            .clicked()
        {
            draft.sequence.pop();
        }
    });
    egui::Grid::new("generation-sequence")
        .num_columns(5)
        .spacing(egui::vec2(8.0, 3.0))
        .show(ui, |ui| {
            for (index, note) in draft.sequence.iter_mut().enumerate() {
                ui.label(format!("MIDI {:02}", index + 1));
                ui.add(egui::TextEdit::singleline(&mut note.note).desired_width(50.0));
                ui.label(
                    RichText::new(
                        note.note
                            .parse::<u8>()
                            .ok()
                            .filter(|n| (12..=127).contains(n))
                            .map_or_else(|| "invalid".into(), |n| note_label(Note::Note(n))),
                    )
                    .small()
                    .color(MUTED),
                );
                ui.label(format!("Velocity {:02}", index + 1));
                ui.add(egui::TextEdit::singleline(&mut note.velocity).desired_width(80.0));
                ui.end_row();
            }
        });
    ui.label(RichText::new("MIDI 12..127 · explicit velocity 0..1 · explicit instrument and pan 0. No inherited note fields in generated starts.").small().color(MUTED));
}
fn config_summary(config: &GenerationConfig) -> String {
    let GenerationPreset::Euclidean {
        steps,
        pulses,
        rotation,
        probability,
    } = &config.preset;
    format!("v{} · seed {} · {} bars · I{} · Euclidean {pulses}/{steps} rot {rotation} keep {probability} · {} sequence entries", config.version, config.seed, config.bars, config.instrument.0, config.sequence.len())
}
fn preview_details(ui: &mut egui::Ui, preview: &GenerationPreview, reveal: bool) {
    let summary = preview.summary();
    ui.separator();
    ui.label(
        RichText::new(format!(
            "APPEND PREVIEW · {} ordinary starts · {} pattrns ticks",
            summary.hits, summary.raw_ticks
        ))
        .strong()
        .color(AQUA),
    );
    ui.label(
        RichText::new(format!(
            "New P{} / T{} / POS {:02} · {} rows · {} note columns",
            summary.pattern.0, summary.track.0, summary.occurrence, summary.rows, summary.columns
        ))
        .small(),
    );
    ui.label(
        RichText::new(format!(
            "Output frames [{}, {}) · {} frames · {} BPM / {} LPB",
            summary.start_frame,
            summary.end_frame,
            summary.end_frame.saturating_sub(summary.start_frame),
            summary.bpm,
            summary.lines_per_beat
        ))
        .small(),
    );
    ui.label(
        RichText::new("Historical provenance · hover for exact version/config/hash record")
            .small()
            .color(MUTED),
    )
    .on_hover_ui(|ui| provenance_details(ui, preview.provenance()));
    ui.label(
        RichText::new(format!(
            "Raw SHA {} · cells SHA {}",
            &summary.raw_sha256[..12.min(summary.raw_sha256.len())],
            &summary.content_sha256[..12.min(summary.content_sha256.len())]
        ))
        .small()
        .color(MUTED),
    )
    .on_hover_text(format!(
        "Raw pattrns SHA256 {}\nGenerated content SHA256 {}\nPreview ID {}",
        summary.raw_sha256,
        summary.content_sha256,
        preview.id()
    ));
    ui.label(RichText::new("Append only; Song clock stays fixed. A later SongEnd can reveal old phrase notes/tails; kit choke can affect held voices.").small().color(AMBER));
    ui.label(RichText::new("Ordinary playback lengths apply, not pattrns durations. No source decoding or sound/clipping check here; recipe is historical, not live.").small().color(AMBER));
    let details = egui::CollapsingHeader::new(format!(
        "Full preview caveats ({} details)",
        summary.warnings.len()
    ))
    .id_salt("generation-full-caveats")
    .default_open(false)
    .show(ui, |ui| {
        for warning in &summary.warnings {
            ui.colored_label(AMBER, warning);
        }
    });
    if details.header_response.clicked() {
        details.header_response.request_focus();
    }
    ui.label(
        RichText::new(
            "DIFF · new cells only; all existing patterns/tracks/order entries remain unchanged",
        )
        .strong(),
    );
    ui.monospace("STEP    OUTPUT FRAME      ROW / COL    NOTE  VEL    DELAY");
    let cells = preview.cells();
    let table = egui::ScrollArea::vertical().id_salt("generation-cells").max_height(145.0).show_rows(ui, 19.0, cells.len(), |ui, range| {
        for index in range {
            let event = &cells[index];
            let velocity = event.cell.velocity.map_or_else(|| "unset".into(), |v| v.to_string());
            let delay = event.cell.delay.map_or_else(|| "unset".into(), |v| v.to_string());
            ui.add(egui::Label::new(RichText::new(format!("{:04}  {:12}  {:04} / N{:<2}  {:>4}  {:<5}  {}", event.step, event.frame, event.row, event.column + 1, note_label(event.cell.note), velocity, delay)).monospace()).truncate())
                .on_hover_text(format!("Generated step {} · output frame {} · P{} / T{} / row {} / N{}\nRaw pattrns virtual ticks {} (1024 per step; not Song seconds)\nExact stored cell {:?}", event.step, event.frame, summary.pattern.0, summary.track.0, event.row, event.column + 1, event.virtual_time, event.cell));
        }
    });
    if reveal {
        ui.scroll_to_rect_animation(
            table.inner_rect,
            Some(egui::Align::BOTTOM),
            egui::style::ScrollAnimation::none(),
        );
    }
}

pub(crate) fn provenance_details(ui: &mut egui::Ui, provenance: &generation::GenerationProvenance) {
    ui.set_max_width(560.0);
    ui.label(RichText::new("HISTORICAL GENERATION PROVENANCE").strong());
    egui::ScrollArea::vertical().id_salt("generation-provenance").max_height(240.0).show(ui, |ui| {
        ui.label(format!("Algorithm {} · config v{}", provenance.algorithm, provenance.config.version));
        ui.monospace(format!("pattrns {}", provenance.pattrns_revision));
        ui.label(format!("Gate {}", provenance.gate_algorithm));
        ui.label(format!("Name {}", provenance.config.name));
        ui.label(config_summary(&provenance.config));
        ui.label(format!("Clock {} BPM / {} LPB · prefix {} rows · {} raw ticks", provenance.bpm, provenance.lines_per_beat, provenance.prefix_rows, provenance.raw_ticks));
        ui.label(format!("Sequence MIDI:velocity {:?}", provenance.config.sequence.iter().map(|note|(note.note,note.velocity)).collect::<Vec<_>>()));
        ui.monospace(format!("Raw SHA256 {}", provenance.raw_sha256));
        ui.monospace(format!("Content SHA256 {}", provenance.content_sha256));
        ui.label("Recorded at bake time, not live playback authority. Later ordinary cell edits do not regenerate or rewrite this history.");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canceled_generation_stays_tracked_and_late_result_is_not_installed() {
        let mut editor = Editor::default();
        for instrument in editor.song.instruments.values_mut() {
            instrument.sample = Some("generation-unit-fixture.wav".into());
        }
        let mut ui = GenerationUi::default();
        ui.open(&editor);
        let preview = generation::preview(
            &editor.song,
            ui.draft.as_ref().unwrap().effective_config().unwrap(),
        )
        .unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        ui.pending = Some(Pending {
            epoch: ui.epoch,
            revision: editor.revision(),
            receiver,
        });
        ui.reset_document();
        ui.open(&editor);
        let current_epoch = ui.epoch;
        ui.start_preview(&editor);
        assert!(ui.busy());
        assert_ne!(ui.pending.as_ref().unwrap().epoch, current_epoch);
        sender.send(Ok(preview)).unwrap();
        ui.poll();
        assert!(!ui.busy());
        assert!(ui.cached.is_none());
        assert!(ui.is_open());
    }
}
