//! Explicit seeded deletion of ordinary cells, never a runtime sequencer or classifier.
//! All eligibility, randomness, role authority and retained-ON proof live in `mutations`.
use crate::{
    editor::{note_label, Editor, Field},
    mutations::{self, ThinConfig, ThinDecision, ThinPreview, ThinReason, ThinSource},
    slice_features::PercussionRole,
    song::{PatternId, TrackId},
    tracker_ui::{grid_id, AQUA},
    transforms::TransformScope,
};
use eframe::egui::{self, Color32, RichText};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const WARN: Color32 = Color32::from_rgb(229, 185, 112);
const ROLES: [PercussionRole; 5] = [
    PercussionRole::Kick,
    PercussionRole::Snare,
    PercussionRole::ClosedHat,
    PercussionRole::OpenHatRide,
    PercussionRole::Crash,
];
#[derive(Clone, PartialEq, Eq)]
struct Context {
    occurrence: usize,
    pattern: PatternId,
    track: TrackId,
    row: u32,
    column: u16,
    field: Field,
}
impl Context {
    fn of(e: &Editor) -> Option<Self> {
        e.has_grid().then(|| Self {
            occurrence: e.occurrence,
            pattern: e.pattern_id(),
            track: e.track,
            row: e.row,
            column: e.column,
            field: e.field,
        })
    }
}
#[derive(Clone, PartialEq)]
struct Draft {
    context: Context,
    revision: u64,
    pattern: PatternId,
    track: TrackId,
    columns: Vec<u16>,
    start: String,
    end: String,
    seed: String,
    percent: String,
    cap: String,
    protected: Vec<PercussionRole>,
}
fn integer(text: &str, max: u64, label: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|_| format!("{label} must be an integer in 0..{max}; no rounding or clamping"))?;
    if value > max {
        return Err(format!("{label} must be in 0..{max}; no clamping"));
    }
    Ok(value)
}
impl Draft {
    fn config(&self) -> Result<ThinConfig, String> {
        Ok(ThinConfig {
            version: mutations::VERSION,
            scope: TransformScope {
                pattern: self.pattern,
                track: self.track,
                row_start: integer(&self.start, u64::from(u32::MAX), "Row start")? as u32,
                row_end: integer(&self.end, u64::from(u32::MAX), "End exclusive")? as u32,
                columns: self.columns.clone(),
            },
            seed: integer(&self.seed, u64::MAX, "Seed")?,
            removal_percent: integer(&self.percent, 100, "Removal percent")? as u8,
            max_removed: integer(&self.cap, 4096, "Removal cap")? as u16,
            protected_roles: self.protected.clone(),
        })
    }
}
struct Pending {
    epoch: u64,
    revision: u64,
    receiver: Receiver<Result<ThinPreview, String>>,
}
struct Cached {
    revision: u64,
    preview: ThinPreview,
    reveal: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Decisions,
    Changes,
    Uses,
}
#[derive(Default)]
pub(crate) struct ThinUi {
    draft: Option<Draft>,
    pending: Option<Pending>,
    cached: Option<Cached>,
    epoch: u64,
    error: Option<String>,
    notice: Option<String>,
    selected: Option<usize>,
    selected_use: Option<usize>,
    reveal_use: bool,
    view: View,
    reveal_error: bool,
    #[cfg(any(test, feature = "ui-capture"))]
    body_layout: Option<(egui::Rect, f32)>,
}
impl ThinUi {
    pub(crate) fn open(&mut self, e: &Editor) {
        let Some(context) = Context::of(e) else {
            return;
        };
        self.reset_document();
        self.draft = Some(Draft {
            context,
            revision: e.revision(),
            pattern: e.pattern_id(),
            track: e.track,
            columns: vec![e.column],
            start: e.row.to_string(),
            end: (e.row + 1).to_string(),
            seed: "0".into(),
            percent: "25".into(),
            cap: "4096".into(),
            protected: vec![PercussionRole::Kick, PercussionRole::Snare],
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
        self.selected = None;
        self.selected_use = None;
        self.reveal_use = false;
        self.view = View::Decisions;
        self.reveal_error = false; /* A canceled worker remains tracked until completion. */
    }
    fn invalidate(&mut self, message: &str) {
        self.epoch = self.epoch.wrapping_add(1);
        // Keep one last successful review for explicit stale/read-only inspection.
        self.error = None;
        self.notice = Some(message.into());
    }
    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.notice = None;
        self.reveal_error = true;
    }
    pub(crate) fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("THIN worker disconnected; document unchanged".into())
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
                self.error = None;
                self.selected = None;
                self.selected_use = None;
                self.reveal_use = false;
                self.view = View::Decisions;
            }
            Err(error) => self.fail(error),
        }
    }
    fn start(&mut self, e: &Editor) {
        if self.pending.is_some() {
            return;
        }
        let Some(draft) = &self.draft else { return };
        let config = match draft.config() {
            Ok(value) => value,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let revision = e.revision();
        let song = e.song.clone();
        self.error = None;
        self.notice = None;
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-thin-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mutations::preview(&song, config)
                }))
                .unwrap_or_else(|_| Err("THIN worker panicked; document unchanged".into()));
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    revision,
                    receiver,
                })
            }
            Err(error) => self.fail(format!("Could not start THIN preview: {error}")),
        }
    }
    fn current(&self, e: &Editor) -> bool {
        let (Some(draft), Some(cached)) = (&self.draft, &self.cached) else {
            return false;
        };
        Context::of(e).as_ref() == Some(&draft.context)
            && cached.revision == e.revision()
            && cached.preview.base() == &e.song
            && draft.config().is_ok_and(|c| &c == cached.preview.config())
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_preview(&self, e: &Editor) -> Option<&ThinPreview> {
        self.current(e)
            .then(|| &self.cached.as_ref().unwrap().preview)
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_state(&self, e: &Editor) -> serde_json::Value {
        serde_json::json!({"body_layout":self.body_layout.map(|(r,y)|serde_json::json!({"rect":[r.min.x,r.min.y,r.max.x,r.max.y],"offset_y":y})),"open":self.is_open(),"busy":self.busy(),"current":self.current(e),"error":self.error,"notice":self.notice,"selected_decision":self.selected,"selected_use":self.selected_use,"exact_use":self.cached.as_ref().and_then(|c|self.selected_use.and_then(|index|c.preview.uses().get(index))),"draft":self.draft.as_ref().map(|d|serde_json::json!({"revision":d.revision,"pattern":d.pattern,"track":d.track,"columns":d.columns,"row_start":d.start,"row_end":d.end,"seed":d.seed,"removal_percent":d.percent,"max_removed":d.cap,"protected_roles":d.protected,"parsed":d.config().ok()})),"preview":self.cached.as_ref().map(|c|serde_json::json!({"revision":c.revision,"id":c.preview.id(),"config":c.preview.config(),"summary":c.preview.summary(),"decisions":c.preview.decisions(),"uses":c.preview.uses(),"contexts":c.preview.contexts(),"changes":c.preview.changes(),"record":c.preview.record(),"candidate":c.preview.candidate()}))})
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, e: &mut Editor) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        if Context::of(e).as_ref() != Some(&draft.context) {
            self.reset_document();
            return;
        }
        if draft.revision != e.revision() {
            draft.revision = e.revision();
            self.invalidate("Document changed; preview the exact operation again.");
        }
        let allowed = ui.is_enabled();
        let writable = allowed && e.edit_mode;
        let before = draft.clone();
        let mut open = true;
        let (mut preview, mut commit, mut close) = (false, false, false);
        let ctx = ui.ctx().clone();
        let extra_lines = usize::from(self.error.is_some())
            + usize::from(self.notice.is_some())
            + usize::from(self.pending.is_some())
            + usize::from(!e.edit_mode);
        let body_height =
            (ctx.content_rect().height() - 380.0 - extra_lines as f32 * 18.0).clamp(150.0, 420.0);
        egui::Window::new("THIN · reviewed-role deletion").id(egui::Id::new("thin-window")).default_pos(egui::pos2(216.0,100.0)).default_width(640.0).min_width(500.0).resizable(false).collapsible(false).open(&mut open).show(&ctx,|ui|{
            ui.set_max_width(640.0);ui.label(RichText::new("ORDINARY CELLS · SEEDED PREVIEW / EXPLICIT COMMIT").strong().color(AQUA));
            ui.label(RichText::new("Deletes the WHOLE chosen cell, including its stored instrument, velocity, pan and delay. Retained ON/control proof is not tail or audio invariance.").small().color(WARN));
            ui.label(RichText::new(format!("Revision {} · cursor POS {:02} / P{} / T{} · version {}",draft.revision,draft.context.occurrence,draft.context.pattern.0,draft.context.track.0,mutations::VERSION)).small().color(MUTED));
            let body = egui::ScrollArea::vertical().id_salt("thin-body").max_height(body_height).auto_shrink([false,true]).show(ui,|ui|{
                ui.add_enabled_ui(allowed,|ui|{scope(ui,e,&mut draft);ui.separator();settings(ui,&mut draft);});
                ui.label(RichText::new("The entire base and candidate must be model-evaluable with assigned sources, even at 0%. No source bytes are verified or decoded here.").small().color(WARN));
                if let Err(error)=e.song.require_local_track(draft.pattern,draft.track){ui.colored_label(WARN,error);}
                ui.label(RichText::new("This edits a shared pattern block, not one occurrence. Repeated and linked uses are reviewed together; no implicit make-unique or automatic role assignment.").small().color(MUTED));
                if let Some(cached)=&mut self.cached{let current=cached.revision==e.revision()&&cached.preview.base()==&e.song&&draft.config().is_ok_and(|c|&c==cached.preview.config());let response=ui.label(RichText::new(if current{"EXACT SHARED PREVIEW"}else{"PREVIOUS REVIEW · STALE FOR THIS DRAFT / DOCUMENT"}).strong().color(if current{AQUA}else{WARN}));if cached.reveal{response.scroll_to_me(Some(egui::Align::Min));cached.reveal=false;}preview_body(ui,&cached.preview,&mut self.selected,&mut self.selected_use,&mut self.reveal_use,&mut self.view);}
                if let Some(error)=&self.error{ui.separator();let response=ui.colored_label(Color32::LIGHT_RED,error);if self.reveal_error{response.scroll_to_me(Some(egui::Align::Center));self.reveal_error=false;}}
            });
            #[cfg(any(test,feature="ui-capture"))] { self.body_layout=Some((body.inner_rect,body.state.offset.y)); }
            #[cfg(not(any(test,feature="ui-capture")))] let _=body;
            ui.separator();
            if let Some(cached)=&self.cached{let current=cached.revision==e.revision()&&cached.preview.base()==&e.song&&draft.config().is_ok_and(|c|&c==cached.preview.config());ready_summary(ui,&cached.preview,current);}
            if self.error.is_some(){ui.label(RichText::new("REFUSED · exact error in the scrollable body; no document change").small().color(Color32::LIGHT_RED));}
            if let Some(notice)=&self.notice{ui.label(RichText::new(notice).small().color(WARN));}
            if self.pending.is_some(){ui.label(RichText::new("Preview worker running · no mutation; cancel keeps it tracked until completion").small().color(MUTED));}
            if !e.edit_mode{ui.label(RichText::new("READ MODE · preview allowed; commit protected").small().color(WARN));}
            let current=self.cached.as_ref().is_some_and(|c|c.revision==e.revision()&&c.preview.base()==&e.song&&draft.config().is_ok_and(|config|&config==c.preview.config()));let removed=self.cached.as_ref().map_or(0,|c|c.preview.summary().removed_cells);
            ui.horizontal(|ui|{preview=ui.add_enabled(allowed&&self.pending.is_none(),egui::Button::new("Preview THIN")).clicked();commit=ui.add_enabled(writable&&current&&removed>0&&self.pending.is_none(),egui::Button::new("Commit THIN")).clicked();close=ui.button("Cancel / close").clicked();});
            ui.label(RichText::new("One undo restores deleted cells. A prior review is read-only unless the exact config/base/revision match. No live regeneration recipe.").small().color(MUTED));
        });
        ui.memory_mut(|m| m.surrender_focus(grid_id()));
        if !open || close {
            self.reset_document();
            return;
        }
        if before != draft {
            self.invalidate(if self.cached.is_some() {
                "Draft changed; check the current/stale status of the retained review."
            } else {
                "Draft changed; preview the exact operation before committing."
            });
        }
        self.draft = Some(draft);
        if preview && allowed {
            self.start(e);
        }
        if commit && writable && self.current(e) {
            let cached = self.cached.as_ref().unwrap();
            match e.commit_thin_preview(cached.revision, &cached.preview) {
                Ok(()) => self.reset_document(),
                Err(error) => self.fail(error),
            }
        }
    }
}
fn bounded_name(name: &str) -> String {
    let mut chars = name.chars();
    let prefix: String = chars.by_ref().take(32).collect();
    format!("{prefix}{}", if chars.next().is_some() { "…" } else { "" })
}
fn numeric(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .id(egui::Id::new(("thin-number", label)))
            .desired_width(width),
    );
}
fn scope(ui: &mut egui::Ui, e: &Editor, d: &mut Draft) {
    ui.label(RichText::new("SCOPE · stored rows [start, end), zero-based").strong());
    ui.horizontal_wrapped(|ui| {
        ui.label("Pattern");
        egui::ComboBox::from_id_salt("thin-pattern")
            .selected_text(format!("P{}", d.pattern.0))
            .width(120.0)
            .show_ui(ui, |ui| {
                let entries = e.song.patterns.iter().collect::<Vec<_>>();
                egui::ScrollArea::vertical().max_height(190.0).show_rows(
                    ui,
                    22.0,
                    entries.len(),
                    |ui, rows| {
                        for row in rows {
                            let (id, p) = entries[row];
                            if ui
                                .add(
                                    egui::Button::selectable(
                                        d.pattern == *id,
                                        format!(
                                            "P{} · {} · {} rows",
                                            id.0,
                                            bounded_name(&p.name),
                                            p.rows
                                        ),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!(
                                    "P{} · name {} UTF-8 bytes; label may be elided",
                                    id.0,
                                    p.name.len()
                                ))
                                .clicked()
                            {
                                d.pattern = *id;
                            }
                        }
                    },
                );
            });
        ui.label("Track");
        egui::ComboBox::from_id_salt("thin-track")
            .selected_text(format!("T{}", d.track.0))
            .width(120.0)
            .show_ui(ui, |ui| {
                egui::ScrollArea::vertical().max_height(190.0).show_rows(
                    ui,
                    22.0,
                    e.song.track_order.len(),
                    |ui, rows| {
                        for row in rows {
                            let id = e.song.track_order[row];
                            let t = &e.song.tracks[&id];
                            if ui
                                .add(
                                    egui::Button::selectable(
                                        d.track == id,
                                        format!(
                                            "T{} · {} · {} columns",
                                            id.0,
                                            bounded_name(&t.name),
                                            t.note_columns
                                        ),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!(
                                    "T{} · name {} UTF-8 bytes; label may be elided",
                                    id.0,
                                    t.name.len()
                                ))
                                .clicked()
                            {
                                d.track = id;
                            }
                        }
                    },
                );
            });
    });
    ui.horizontal_wrapped(|ui| {
        numeric(ui, "Row start", &mut d.start, 70.0);
        numeric(ui, "End exclusive", &mut d.end, 70.0);
        if let Some(p) = e.song.patterns.get(&d.pattern) {
            if ui.small_button("Whole pattern").clicked() {
                d.start = "0".into();
                d.end = p.rows.to_string();
            }
            ui.label(format!("/ {} rows", p.rows));
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Note columns");
        if let Some(track) = e.song.tracks.get(&d.track) {
            if ui.small_button("All columns").clicked() {
                d.columns = (0..track.note_columns).collect();
            }
            if ui.small_button("No columns").clicked() {
                d.columns.clear();
            }
            for n in 0..track.note_columns {
                let mut chosen = d.columns.contains(&n);
                if ui.checkbox(&mut chosen, format!("N{}", n + 1)).changed() {
                    if chosen {
                        d.columns.push(n);
                        d.columns.sort_unstable();
                    } else {
                        d.columns.retain(|&v| v != n);
                    }
                }
            }
        }
    });
    ui.label(
        RichText::new(format!(
            "{} selected column(s); invalid/out-of-range scope is refused, never clamped",
            d.columns.len()
        ))
        .small()
        .color(MUTED),
    )
    .on_hover_ui(|ui| column_list(ui, &d.columns));
}
fn settings(ui: &mut egui::Ui, d: &mut Draft) {
    ui.label(RichText::new("SEEDED THIN · shared pattrns probability gate").strong());
    ui.horizontal_wrapped(|ui| {
        numeric(ui, "Seed", &mut d.seed, 170.0);
        numeric(ui, "Removal %", &mut d.percent, 50.0);
        numeric(ui, "Removal cap", &mut d.cap, 65.0);
    });
    ui.label(RichText::new("Integer percent 0..100; seed u64; hard removal cap 0..4096. Exceeding the cap or failing retained-event proof rejects the WHOLE preview, not a first-N subset.").small().color(MUTED));
    ui.label("Protect current reviewed roles:");
    ui.horizontal_wrapped(|ui| {
        for role in ROLES {
            let mut checked = d.protected.contains(&role);
            if ui.checkbox(&mut checked, role_name(role)).changed() {
                if checked {
                    d.protected.push(role);
                } else {
                    d.protected.retain(|v| *v != role);
                }
                d.protected
                    .sort_by_key(|role| ROLES.iter().position(|r| r == role).unwrap());
            }
        }
    });
    ui.label(RichText::new("Unknown / Mixed / stale / unreviewed are ALWAYS preserved. These are human-reviewed binding labels, not heuristic suggestions or a fresh audio check. Ghost is manual articulation, not a separate protection switch.").small().color(WARN));
}
fn role_name(role: PercussionRole) -> &'static str {
    match role {
        PercussionRole::Kick => "Kick",
        PercussionRole::Snare => "Snare",
        PercussionRole::ClosedHat => "Closed hat",
        PercussionRole::OpenHatRide => "Open hat / ride",
        PercussionRole::Crash => "Crash",
        PercussionRole::Mixed => "Mixed",
        PercussionRole::Unknown => "Unknown",
    }
}
fn reason_name(reason: ThinReason) -> &'static str {
    match reason {
        ThinReason::NotOn => "Not an ON",
        ThinReason::Phrase => "Phrase trigger preserved",
        ThinReason::Effects => "Effects preserved",
        ThinReason::NoAudibleUse => "No evaluated ON use",
        ThinReason::UnknownSource => "Unknown source preserved",
        ThinReason::Unreviewed => "Unreviewed preserved",
        ThinReason::StaleReview => "Stale review preserved",
        ThinReason::MixedOrUnknown => "Mixed / Unknown preserved",
        ThinReason::ProtectedRole => "Protected reviewed role",
        ThinReason::GateKept => "Gate kept",
        ThinReason::Removed => "REMOVE whole cell",
    }
}
fn column_list(ui: &mut egui::Ui, columns: &[u16]) {
    ui.set_max_width(280.0);
    ui.label("Exact selected columns (display N1 = stored0)");
    egui::ScrollArea::vertical().max_height(170.0).show_rows(
        ui,
        18.0,
        columns.len(),
        |ui, rows| {
            for index in rows {
                ui.monospace(format!(
                    "N{} · stored {}",
                    u32::from(columns[index]) + 1,
                    columns[index]
                ));
            }
        },
    );
}
fn ready_summary(ui: &mut egui::Ui, p: &ThinPreview, current: bool) {
    let c = p.config();
    let s = p.summary();
    ui.label(
        RichText::new(format!(
            "{} · remove {}/{} eligible · {} records · P{}/T{} [{},{}) · {} cols",
            if current { "READY" } else { "PREVIOUS / STALE" },
            s.removed_cells,
            s.eligible_cells,
            s.reviewed_cells,
            c.scope.pattern.0,
            c.scope.track.0,
            c.scope.row_start,
            c.scope.row_end,
            c.scope.columns.len()
        ))
        .small()
        .color(if current { AQUA } else { WARN }),
    )
    .on_hover_ui(|ui| column_list(ui, &c.scope.columns));
    ui.label(
        RichText::new(format!(
            "Seed {} · {}% · cap {} · protect {} · {} ON removed",
            c.seed,
            c.removal_percent,
            c.max_removed,
            if c.protected_roles.is_empty() {
                "no concrete roles".into()
            } else {
                c.protected_roles
                    .iter()
                    .map(|r| role_name(*r))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            s.removed_starts
        ))
        .small()
        .color(MUTED),
    );
}
fn source_label(source: &ThinSource) -> String {
    match source {
        ThinSource::Legacy => "legacy source / no reviewed region".into(),
        ThinSource::Region { region, root_note } => {
            format!("R{} · native root {}", region.0, root_note)
        }
        ThinSource::DrumPad {
            kit,
            pad,
            region,
            layer,
            ..
        } => format!(
            "Kit{} / Pad{} / R{}{}",
            kit.0,
            pad.0,
            region.0,
            layer.map_or(String::new(), |id| format!(" / layer{}", id.0))
        ),
        ThinSource::Multisample {
            map,
            zone,
            region,
            root_note,
        } => format!(
            "M{} / Z{} / R{} · native root {}",
            map.0, zone.0, region.0, root_note
        ),
    }
}
fn decision_line(decision: &ThinDecision) -> String {
    format!(
        "r{:05} N{:03}  {}  {:<4}  {} use(s)",
        decision.row,
        u32::from(decision.column) + 1,
        reason_name(decision.reason),
        note_label(decision.before.note),
        decision.uses
    )
}
fn preview_body(
    ui: &mut egui::Ui,
    p: &ThinPreview,
    selected: &mut Option<usize>,
    selected_use: &mut Option<usize>,
    reveal_use: &mut bool,
    view: &mut View,
) {
    let s = p.summary();
    ui.label(format!(
        "{} stored selected · {} empty · {} eligible · {} removed · ON starts {} -> {}",
        s.selected_stored_cells,
        s.empty_cells,
        s.eligible_cells,
        s.removed_cells,
        s.base_starts,
        s.candidate_starts
    ));
    ui.horizontal_wrapped(|ui| {
        for (reason, count) in &s.reason_counts {
            ui.label(
                RichText::new(format!("{}: {}", reason_name(*reason), count))
                    .small()
                    .color(if *reason == ThinReason::Removed {
                        AQUA
                    } else {
                        MUTED
                    }),
            );
        }
    });
    egui::CollapsingHeader::new("Affected contexts / proof limits / full warnings").show(
        ui,
        |ui| {
            ui.label(format!(
                "{} occurrence(s), {} ordered block(s), {} alias target(s)",
                s.affected_occurrences, s.affected_blocks, s.affected_aliases
            ));
            ui.label(format!("Non-lifetime controls {} -> {}; removed {}. Retained controls and ON starts are proved; release tails and audio are not invariant.", s.base_controls, s.candidate_controls, s.removed_controls));
            egui::ScrollArea::vertical()
                .id_salt("thin-contexts")
                .max_height(110.0)
                .show_rows(ui, 19.0, p.contexts().len(), |ui, rows| {
                    for index in rows {
                        let c = &p.contexts()[index];
                        ui.add(egui::Label::new(RichText::new(format!(
                            "POS {:02} · P{} / T{} · muted {}",
                            c.occurrence, c.pattern.0, c.track.0, c.muted
                        )).monospace()).truncate());
                    }
                });
            for warning in &s.warnings {
                ui.label(RichText::new(warning).small().color(WARN));
            }
        },
    );
    ui.horizontal(|ui| {
        ui.selectable_value(view, View::Decisions, "Decisions");
        ui.selectable_value(view, View::Changes, "Removed cells");
        ui.selectable_value(view, View::Uses, "Resolved uses");
    });
    match view {
        View::Decisions => {
            egui::ScrollArea::vertical().id_salt("thin-decisions").max_height(160.0).show_rows(ui,22.0,p.decisions().len(),|ui,rows|for index in rows{let d=&p.decisions()[index];if ui.add(egui::Button::selectable(*selected==Some(index),RichText::new(decision_line(d)).monospace().color(if d.reason==ThinReason::Removed{AQUA}else{MUTED})).truncate()).on_hover_text(format!("Exact source row {} / stored column {}\nReason {:?}; gate index {:?}\nBefore {:#?}",d.row,d.column,d.reason,d.gate_index,d.before)).clicked(){*selected=Some(index);*selected_use=None;}});
        }
        View::Changes => {
            egui::ScrollArea::vertical()
                .id_salt("thin-changes")
                .max_height(160.0)
                .show_rows(ui, 22.0, p.changes().len(), |ui, rows| {
                    for index in rows {
                        let c = &p.changes()[index];
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "r{:05} N{:03} · {} -> ABSENT WHOLE CELL",
                                    c.row,
                                    u32::from(c.column) + 1,
                                    note_label(c.before.note)
                                ))
                                .monospace(),
                            )
                            .truncate(),
                        )
                        .on_hover_text(format!("Before {:#?}\nAfter {:?}", c.before, c.after));
                    }
                });
            if p.changes().is_empty() {
                ui.label("No cells removed; no-op commit is disabled.");
            }
        }
        View::Uses => {
            egui::ScrollArea::vertical()
                .id_salt("thin-uses")
                .max_height(180.0)
                .show_rows(ui, 22.0, p.uses().len(), |ui, rows| {
                    for index in rows {
                        let u = &p.uses()[index];
                        if ui
                            .add(
                                egui::Button::selectable(
                                    *selected_use == Some(index),
                                    RichText::new(format!(
                                        "#{index} POS{:02} T{} r{} N{} · I{} · {}",
                                        u.occurrence,
                                        u.track.0,
                                        u.row,
                                        u32::from(u.column) + 1,
                                        u.instrument.0,
                                        source_label(&u.source)
                                    ))
                                    .monospace(),
                                )
                                .truncate(),
                            )
                            .on_hover_text("Select this realized use for its complete, scrollable exact record below.")
                            .clicked()
                        {
                            *selected = Some(u.decision);
                            *selected_use = Some(index);
                            *reveal_use = true;
                        }
                    }
                });
        }
    }
    if let Some(index) = selected.and_then(|index| p.decisions().get(index).map(|_| index)) {
        let d = &p.decisions()[index];
        ui.separator();
        ui.monospace(format!(
            "SELECTED SOURCE CELL · P{} / T{} / row {} / stored column {}",
            p.config().scope.pattern.0,
            p.config().scope.track.0,
            d.row,
            d.column
        ));
        ui.label(format!(
            "{} · {} realized ON use(s) · gate index {:?}",
            reason_name(d.reason),
            d.uses,
            d.gate_index
        ));
        ui.monospace(format!(
            "Stored note {} · instrument {:?} · velocity {:?} · pan {:?} · delay {:?}",
            note_label(d.before.note),
            d.before.instrument,
            d.before.velocity,
            d.before.pan,
            d.before.delay
        ));
        ui.label(RichText::new("None means stored unspecified/inherited, not an evaluated fixed value. Deletion is before -> absent whole cell.").small().color(MUTED));
        let uses = p
            .uses()
            .iter()
            .enumerate()
            .filter(|(_, u)| u.decision == index)
            .collect::<Vec<_>>();
        egui::ScrollArea::vertical()
            .id_salt("thin-selected-uses")
            .max_height(130.0)
            .show_rows(ui, 22.0, uses.len(), |ui, rows| {
                for index in rows {
                    let (use_index, u) = uses[index];
                    if ui.add(
                        egui::Button::selectable(
                            *selected_use == Some(use_index),
                            RichText::new(format!(
                                "POS{:02} / T{} @frame{} · {} · {}",
                                u.occurrence,
                                u.track.0,
                                u.frame,
                                source_label(&u.source),
                                u.reviewed_role
                                    .as_ref()
                                    .map_or("unreviewed".into(), |r| format!(
                                        "{} / {:?} · binding {}",
                                        role_name(r.role),
                                        r.articulation,
                                        if u.review_current_match {
                                            "matches"
                                        } else {
                                            "STALE"
                                        }
                                    ))
                            ))
                            .monospace(),
                        )
                        .truncate(),
                    )
                    .on_hover_text("Select this realized use for its complete, scrollable exact record below.")
                    .clicked() {
                        *selected_use = Some(use_index);
                        *reveal_use = true;
                    }
                }
            });
        egui::CollapsingHeader::new("Exact selected cell").show(ui, |ui| {
            ui.monospace(format!("{d:#?}"));
            ui.label("Select a realized-use row above to inspect every source, role binding and control field in a scrollable read-only record.");
        });
    }
    if let Some((index, u)) = selected_use.and_then(|index| p.uses().get(index).map(|u| (index, u)))
    {
        ui.separator();
        let header = ui.label(
            RichText::new(format!("EXACT SELECTED USE #{index}"))
                .strong()
                .color(AQUA),
        );
        ui.label(RichText::new("Read-only complete record. Select/copy text; Ctrl+Home/End or PageUp/Down, or wheel scroll. Stored review is metadata, not fresh audio verification.").small().color(MUTED));
        // A read-only TextBuffer retains native keyboard selection/navigation and
        // scroll-to-caret, without allowing text input to mutate this exact record.
        // It lives in the existing bounded outer body, never an unscrollable tooltip.
        let exact = format!("{u:#?}");
        let mut text = exact.as_str();
        let id = ui.make_persistent_id(("thin-exact-use", p.id(), index));
        if *reveal_use {
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(0),
                )));
            state.store(ui.ctx(), id);
        }
        let response = ui.add(
            egui::TextEdit::multiline(&mut text)
                .id(id)
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width())
                .cursor_at_end(false)
                .frame(false),
        );
        if *reveal_use {
            header.scroll_to_me_animation(
                Some(egui::Align::Min),
                egui::style::ScrollAnimation::none(),
            );
            response.request_focus();
            // The selected record may be newly appended below the prior viewport.
            // Finish the one selection-triggered reveal only once it is measured visible;
            // no continuing scroll policy remains after that point.
            *reveal_use = !ui.clip_rect().contains_rect(header.rect);
            if *reveal_use {
                ui.ctx().request_repaint();
            }
        }
    }
    egui::CollapsingHeader::new("Transient recipe / exact provenance").show(ui, |ui| {
        ui.label(RichText::new("This reversible record is transient review evidence, not a persisted or live regeneration instruction. Committed ordinary cells remain authoritative.").small().color(MUTED));
        ui.label("Plan ID"); ui.monospace(p.id());
        ui.label(format!("{} · gate {} · pattrns {}",s.algorithm,s.gate_algorithm,s.pattrns_revision));
        ui.label(format!("Effective probability {:?} · gate calls {} · random draws {}",s.effective_probability,s.gate_calls,s.random_draws));
        let r=p.record();
        for (label,hash) in [("Base SHA256",&r.base_sha256),("Candidate SHA256",&r.candidate_sha256),("Decisions SHA256",&r.decisions_sha256),("Realized uses SHA256",&r.uses_sha256),("Changes SHA256",&r.changes_sha256),("Retained ON/control proof SHA256",&r.retained_proof_sha256)] { ui.label(RichText::new(label).small().color(MUTED));ui.monospace(hash); }
        ui.label(format!("Record v{} · {} whole-cell changes · config is the exact READY/PREVIOUS scope and settings above",r.version,r.change_count));
    });
}

#[cfg(test)]
pub(crate) fn test_song() -> crate::song::Song {
    use crate::{
        slice_features::{Articulation, RoleLabel},
        song::*,
    };
    let mut song = example_song();
    song.title = "THIN authored review fixture (not classifier evidence)".into();
    song.order.truncate(1);
    song.patterns.retain(|id, _| *id == PatternId(1));
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 16;
    pattern.tracks.clear();
    pattern.track_aliases.clear();
    song.register_sample(
        SampleId(1),
        SampleAsset {
            path: "/missing/thin-pure-preview.wav".into(),
            sha256: "a".repeat(64),
            sample_rate: 48000,
            channels: 1,
            frames: 24000,
        },
    )
    .unwrap();
    for id in 1..=6 {
        song.upsert_region(
            RegionId(id),
            SampleRegion {
                name: format!("Authored R{id}"),
                sample: SampleId(1),
                start_frame: 0,
                end_frame: 6000,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
        song.instruments.insert(
            InstrumentId(id),
            Instrument {
                name: format!("Source {id}"),
                sample: None,
                region: Some(RegionId(id)),
                kind: InstrumentKind::Pitched,
            },
        );
    }
    for (id, role) in [
        (1, PercussionRole::ClosedHat),
        (2, PercussionRole::Kick),
        (3, PercussionRole::Mixed),
        (4, PercussionRole::Snare),
        (6, PercussionRole::ClosedHat),
    ] {
        song.set_region_role(
            RegionId(id),
            Some(RoleLabel {
                role,
                articulation: Articulation::Normal,
                note: "Explicit synthetic author choice, not detector ground truth".into(),
            }),
        )
        .unwrap();
    }
    song.regions.get_mut(&RegionId(6)).unwrap().start_frame = 1;
    for row in 0..16 {
        song.patterns
            .get_mut(&PatternId(1))
            .unwrap()
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(row)
            .or_default()
            .insert(
                0,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(u64::from(row % 6 + 1))),
                    velocity: Some(if row == 12 { 0.0 } else { 0.5 }),
                    pan: Some(0.0),
                    delay: Some(0.01),
                    ..Default::default()
                },
            );
    }
    song.validate().unwrap();
    song
}
#[cfg(test)]
mod tests {
    use super::*;
    fn editor() -> Editor {
        let mut e = Editor::default();
        e.replace_document(test_song()).unwrap();
        e.edit_mode = true;
        e
    }
    #[test]
    fn exact_integer_ranges_defaults_and_independent_protections() {
        let e = editor();
        let mut ui = ThinUi::default();
        ui.open(&e);
        let d = ui.draft.as_mut().unwrap();
        assert_eq!(
            d.protected,
            vec![PercussionRole::Kick, PercussionRole::Snare]
        );
        d.seed = u64::MAX.to_string();
        d.percent = "100".into();
        d.cap = "0".into();
        let c = d.config().unwrap();
        assert_eq!(c.seed, u64::MAX);
        assert_eq!(c.max_removed, 0);
        for input in ["-1", "101", "0.1", "NaN", "18446744073709551616"] {
            d.percent = input.into();
            assert!(d.config().is_err());
        }
        d.percent = "0".into();
        for input in ["-1", "4097", "0.1"] {
            d.cap = input.into();
            assert!(d.config().is_err());
        }
        d.cap = "4096".into();
        d.seed = "18446744073709551616".into();
        assert!(d.config().is_err());
        d.seed = "0".into();
        d.start = "-1".into();
        assert!(d.config().is_err());
        assert!(bounded_name(&"界".repeat(100000)).len() < 110);
    }
    #[test]
    fn previous_review_is_retained_but_stale_and_cancel_tracks_old_worker() {
        let e = editor();
        let mut ui = ThinUi::default();
        ui.open(&e);
        ui.draft.as_mut().unwrap().end = "16".into();
        ui.draft.as_mut().unwrap().percent = "100".into();
        let p = mutations::preview(&e.song, ui.draft.as_ref().unwrap().config().unwrap()).unwrap();
        let id = p.id().to_owned();
        ui.cached = Some(Cached {
            revision: e.revision(),
            preview: p.clone(),
            reveal: false,
        });
        assert!(ui.current(&e));
        ui.draft.as_mut().unwrap().cap = "0".into();
        ui.invalidate("changed");
        assert!(!ui.current(&e));
        ui.fail("whole-preview cap rejection".into());
        assert_eq!(ui.cached.as_ref().unwrap().preview.id(), id);
        let (sender, receiver) = mpsc::sync_channel(1);
        ui.pending = Some(Pending {
            epoch: ui.epoch,
            revision: e.revision(),
            receiver,
        });
        ui.reset_document();
        ui.open(&e);
        ui.start(&e);
        assert!(ui.busy());
        sender.send(Ok(p)).unwrap();
        ui.poll();
        assert!(!ui.busy());
        assert!(ui.cached.is_none());
    }
    #[test]
    fn document_revision_fullbase_and_selection_guard_previous_review() {
        let mut e = editor();
        let mut ui = ThinUi::default();
        ui.open(&e);
        let p = mutations::preview(&e.song, ui.draft.as_ref().unwrap().config().unwrap()).unwrap();
        ui.cached = Some(Cached {
            revision: e.revision(),
            preview: p,
            reveal: false,
        });
        assert!(ui.current(&e));
        e.song.title.push('x');
        assert!(!ui.current(&e));
        e.song.title.pop();
        assert!(ui.current(&e));
        e.row = 1;
        assert!(!ui.current(&e));
        e.row = 0;
        e.set_timing(121.0, 4).unwrap();
        e.undo();
        assert!(!ui.current(&e));
    }
    #[test]
    fn maximum_columns_long_names_and_previous_error_keep_footer_inside_both_viewports() {
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let mut e = editor();
            e.song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
            e.song.tracks.get_mut(&TrackId(1)).unwrap().name = "界Ｗ".repeat(10000);
            e.song.patterns.get_mut(&PatternId(1)).unwrap().name = "界Ｗ".repeat(10000);
            for row in 0..16 {
                for column in 0..256 {
                    e.song
                        .patterns
                        .get_mut(&PatternId(1))
                        .unwrap()
                        .tracks
                        .entry(TrackId(1))
                        .or_default()
                        .entry(row)
                        .or_default()
                        .insert(
                            column,
                            crate::song::Cell {
                                instrument: Some(crate::song::InstrumentId(1)),
                                ..Default::default()
                            },
                        );
                }
            }
            e.song
                .patterns
                .get_mut(&PatternId(1))
                .unwrap()
                .tracks
                .get_mut(&TrackId(1))
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .insert(
                    0,
                    crate::song::Cell {
                        note: crate::song::Note::Note(60),
                        instrument: Some(crate::song::InstrumentId(2)),
                        ..Default::default()
                    },
                );
            let pattern = e.song.patterns.remove(&PatternId(1)).unwrap();
            let mut pattern = pattern;
            let cells = pattern.tracks.remove(&TrackId(1)).unwrap();
            pattern.tracks.insert(TrackId(u64::MAX), cells);
            e.song.patterns.insert(PatternId(u64::MAX), pattern);
            e.song.order[0].pattern = PatternId(u64::MAX);
            let track = e.song.tracks.remove(&TrackId(1)).unwrap();
            e.song.tracks.insert(TrackId(u64::MAX), track);
            for id in &mut e.song.track_order {
                if *id == TrackId(1) {
                    *id = TrackId(u64::MAX);
                }
            }
            e.track = TrackId(u64::MAX);
            e.edit_mode = false;
            let mut ui = ThinUi::default();
            ui.open(&e);
            let d = ui.draft.as_mut().unwrap();
            d.columns = (0..256).collect();
            d.end = "16".into();
            d.percent = "100".into();
            d.seed = u64::MAX.to_string();
            d.protected = ROLES.to_vec();
            let p = mutations::preview(&e.song, d.config().unwrap()).unwrap();
            assert_eq!(p.decisions().len(), 4096);
            ui.cached = Some(Cached {
                revision: e.revision(),
                preview: p,
                reveal: false,
            });
            ui.draft.as_mut().unwrap().cap = "0".into();
            ui.fail("Explicit cap refusal with a long retained explanation. ".repeat(20));
            let ctx = egui::Context::default();
            let mut output = None;
            for i in 0..15 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(f64::from(i) / 60.0),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |panel| ui.show(panel, &mut e));
                    },
                ));
            }
            let output = output.unwrap();
            for label in ["Preview THIN", "Commit THIN", "Cancel / close"] {
                assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.job.text==label && s.clip_rect.contains_rect(egui::Rect::from_min_size(t.pos,t.galley.size())) && t.pos.y+t.galley.size().y<size.y)),"Footer {label} must be visible at {size:?}");
            }
            let shaped = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) => Some(t.galley.job.text.len()),
                    _ => None,
                })
                .sum::<usize>();
            assert!(
                shaped < 20000,
                "Never shape huge names or full column vectors into fixed footer"
            );
        }
    }
    fn use_fixture(keyzone: bool) -> crate::song::Song {
        use crate::song::*;
        let mut song = test_song();
        song.set_region_role(
            RegionId(1),
            Some(crate::slice_features::RoleLabel {
                role: PercussionRole::ClosedHat,
                articulation: crate::slice_features::Articulation::Ghost,
                note: "界\n".repeat(64), // exactly 256 UTF-8 bytes, including legal line breaks
            }),
        )
        .unwrap();
        if keyzone {
            song.multisamples.insert(
                MultisampleId(u64::MAX),
                MultisampleInstrument {
                    name: "Exact source".into(),
                    zones: std::collections::BTreeMap::from([(
                        KeyZoneId(u64::MAX),
                        KeyZone {
                            name: "Native root override".into(),
                            note_min: 0,
                            note_max: 127,
                            velocity_min: 0,
                            velocity_max: 127,
                            region: RegionId(1),
                            root_note: 67,
                        },
                    )]),
                },
            );
            song.instruments.get_mut(&InstrumentId(1)).unwrap().kind =
                InstrumentKind::Multisample {
                    map: MultisampleId(u64::MAX),
                };
        } else {
            song.drum_kits.insert(
                DrumKitId(u64::MAX),
                DrumKitInstrument {
                    name: "Exact source".into(),
                    pads: std::collections::BTreeMap::from([(
                        PadId(u64::MAX),
                        DrumPad {
                            slot: 0,
                            name: "Layered source".into(),
                            note: 60,
                            region: None,
                            velocity_zones: vec![VelocityZone {
                                min_velocity: 0,
                                max_velocity: 127,
                                alternatives: vec![
                                    PadLayer {
                                        id: PadLayerId(u64::MAX - 1),
                                        region: RegionId(1),
                                    },
                                    PadLayer {
                                        id: PadLayerId(u64::MAX),
                                        region: RegionId(1),
                                    },
                                ],
                            }],
                            root_note: 60,
                            tune_semitones: 0,
                            gain: 1.0,
                            pan: 0.0,
                            mode: PadMode::OneShot,
                            new_note: PadNewNote::Overlap,
                            choke_group: None,
                        },
                    )]),
                },
            );
            song.instruments.get_mut(&InstrumentId(1)).unwrap().kind = InstrumentKind::DrumKit {
                kit: DrumKitId(u64::MAX),
            };
        }
        song.instruments.get_mut(&InstrumentId(1)).unwrap().region = None;
        song.order.push(Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        });
        song.validate().unwrap();
        song
    }
    fn use_frame(
        ui: &mut ThinUi,
        e: &mut Editor,
        ctx: &egui::Context,
        size: egui::Vec2,
        frame: &mut u64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        *frame += 1;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(*frame as f64 / 60.0),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |panel| ui.show(panel, e));
            },
        )
    }
    fn use_point(output: &egui::FullOutput, prefix: &str) -> Option<egui::Pos2> {
        output.shapes.iter().rev().find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text.starts_with(prefix) => {
                let rect = t.visual_bounding_rect();
                (rect.top() >= s.clip_rect.top() && rect.bottom() <= s.clip_rect.bottom())
                    .then(|| rect.intersect(s.clip_rect).center())
            }
            _ => None,
        })
    }
    fn use_key(key: egui::Key, pressed: bool, command: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: command,
                command,
                ..Default::default()
            },
        }
    }
    #[test]
    fn exact_use_selection_resets_only_when_review_is_replaced_or_cleared() {
        let e = editor();
        let mut ui = ThinUi::default();
        ui.open(&e);
        let p = mutations::preview(&e.song, ui.draft.as_ref().unwrap().config().unwrap()).unwrap();
        ui.cached = Some(Cached {
            revision: e.revision(),
            preview: p.clone(),
            reveal: false,
        });
        ui.selected = Some(0);
        ui.selected_use = Some(0);
        ui.invalidate("draft changed");
        ui.fail("new preview refused".into());
        assert_eq!(ui.selected_use, Some(0));
        assert_eq!(ui.cached.as_ref().unwrap().preview.id(), p.id());
        let (sender, receiver) = mpsc::sync_channel(1);
        ui.pending = Some(Pending {
            epoch: ui.epoch,
            revision: e.revision(),
            receiver,
        });
        sender.send(Ok(p)).unwrap();
        ui.poll();
        assert_eq!(ui.selected_use, None);
        assert_eq!(ui.selected, None);
        ui.selected_use = Some(0);
        ui.reset_document();
        assert_eq!(ui.selected_use, None);
    }
    #[test]
    fn exact_use_both_routes_support_keyboard_wheel_and_full_long_record_without_mutation() {
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            for keyzone in [false, true] {
                for list in [false, true] {
                    let mut e = editor();
                    e.replace_document(use_fixture(keyzone)).unwrap();
                    e.edit_mode = true;
                    let before = e.song.clone();
                    let revision = e.revision();
                    let mut ui = ThinUi::default();
                    ui.open(&e);
                    ui.draft.as_mut().unwrap().end = "16".into();
                    ui.draft.as_mut().unwrap().percent = "100".into();
                    let p =
                        mutations::preview(&e.song, ui.draft.as_ref().unwrap().config().unwrap())
                            .unwrap();
                    let plan_id = p.id().to_owned();
                    let expected_index = if list { 0 } else { 16 };
                    let expected_use = p.uses()[expected_index].clone();
                    assert_eq!(expected_use.occurrence, usize::from(!list));
                    ui.cached = Some(Cached {
                        revision,
                        preview: p,
                        reveal: true,
                    });
                    if list {
                        ui.view = View::Uses;
                    } else {
                        ui.selected = Some(0);
                    }
                    let ctx = egui::Context::default();
                    let mut frame = 0;
                    let mut output = use_frame(&mut ui, &mut e, &ctx, size, &mut frame, vec![]);
                    let prefix = if list {
                        "#0 POS00"
                    } else {
                        "POS01 / T1 @frame"
                    };
                    let mut found = None;
                    for _ in 0..180 {
                        output = use_frame(&mut ui, &mut e, &ctx, size, &mut frame, vec![]);
                        if let Some(point) = use_point(&output, prefix) {
                            found = Some(point);
                            break;
                        }
                        let rect = ui.body_layout.unwrap().0;
                        output = use_frame(
                            &mut ui,
                            &mut e,
                            &ctx,
                            size,
                            &mut frame,
                            vec![
                                egui::Event::PointerMoved(egui::pos2(
                                    rect.right() - 3.0,
                                    rect.center().y,
                                )),
                                egui::Event::MouseWheel {
                                    unit: egui::MouseWheelUnit::Point,
                                    delta: egui::vec2(0.0, -20.0),
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        );
                    }
                    let point = found.expect("Exact use row reachable through actual outer scroll");
                    for pressed in [true, false] {
                        output = use_frame(
                            &mut ui,
                            &mut e,
                            &ctx,
                            size,
                            &mut frame,
                            vec![
                                egui::Event::PointerMoved(point),
                                egui::Event::PointerButton {
                                    pos: point,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        );
                    }
                    for _ in 0..30 {
                        output = use_frame(&mut ui, &mut e, &ctx, size, &mut frame, vec![]);
                    }
                    assert_eq!(
                        ui.selected_use,
                        Some(expected_index),
                        "exact index, not just decision"
                    );
                    let exact = format!("{expected_use:#?}");
                    for pressed in [true, false] {
                        output = use_frame(
                            &mut ui,
                            &mut e,
                            &ctx,
                            size,
                            &mut frame,
                            vec![use_key(egui::Key::End, pressed, true)],
                        );
                    }
                    for _ in 0..30 {
                        output = use_frame(&mut ui, &mut e, &ctx, size, &mut frame, vec![]);
                    }
                    let field_visible = |output: &egui::FullOutput, needle: &str| {
                        output.shapes.iter().any(|s| match &s.shape {
                            egui::epaint::Shape::Text(t) if t.galley.job.text == exact => {
                                t.galley.rows.iter().any(|r| {
                                    r.glyphs
                                        .iter()
                                        .map(|g| g.chr)
                                        .collect::<String>()
                                        .contains(needle)
                                        && s.clip_rect
                                            .contains_rect(r.rect().translate(t.pos.to_vec2()))
                                })
                            }
                            _ => false,
                        })
                    };
                    assert!(
                        field_visible(&output, "review_current_match:"),
                        "binding field keyboard-reachable at {size:?}; focus {:?}; body {:?}",
                        ctx.memory(|m| m.focused()),
                        ui.body_layout
                    );
                    assert!(
                        field_visible(&output, "retention:"),
                        "final retention keyboard-reachable at {size:?}"
                    );
                    for label in ["Commit THIN", "Cancel / close"] {
                        assert!(
                            use_point(&output, label).is_some(),
                            "fixed footer remains visible"
                        );
                    }
                    for pressed in [true, false] {
                        let _ = use_frame(
                            &mut ui,
                            &mut e,
                            &ctx,
                            size,
                            &mut frame,
                            vec![use_key(egui::Key::A, pressed, true)],
                        );
                    }
                    let copied = use_frame(
                        &mut ui,
                        &mut e,
                        &ctx,
                        size,
                        &mut frame,
                        vec![egui::Event::Copy],
                    );
                    assert!(copied.platform_output.commands.iter().any(|command|matches!(command,egui::OutputCommand::CopyText(text) if text==&exact)),"copy preserves complete exact metadata");
                    // A read-only buffer consumes text navigation/typing without altering either
                    // the exact displayed metadata, the opaque review or the ordinary Song.
                    output = use_frame(
                        &mut ui,
                        &mut e,
                        &ctx,
                        size,
                        &mut frame,
                        vec![
                            egui::Event::Text("SHOULD NOT EDIT".into()),
                            use_key(egui::Key::Backspace, true, false),
                        ],
                    );
                    assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.job.text==exact)));
                    for pressed in [true, false] {
                        let _ = use_frame(
                            &mut ui,
                            &mut e,
                            &ctx,
                            size,
                            &mut frame,
                            vec![use_key(egui::Key::Home, pressed, true)],
                        );
                    }
                    for _ in 0..30 {
                        let _ = use_frame(&mut ui, &mut e, &ctx, size, &mut frame, vec![]);
                    }
                    let top = ui.body_layout.unwrap().1;
                    let mut wheel_reached = false;
                    for _ in 0..100 {
                        let rect = ui.body_layout.unwrap().0;
                        output = use_frame(
                            &mut ui,
                            &mut e,
                            &ctx,
                            size,
                            &mut frame,
                            vec![
                                egui::Event::PointerMoved(egui::pos2(
                                    rect.right() - 3.0,
                                    rect.center().y,
                                )),
                                egui::Event::MouseWheel {
                                    unit: egui::MouseWheelUnit::Point,
                                    delta: egui::vec2(0.0, -22.0),
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        );
                        if field_visible(&output, "retention:") {
                            wheel_reached = true;
                            break;
                        }
                    }
                    assert!(
                        wheel_reached && ui.body_layout.unwrap().1 > top,
                        "trailing exact field wheel-reachable"
                    );
                    assert_eq!(e.song, before);
                    assert_eq!(e.revision(), revision);
                    assert_eq!(ui.cached.as_ref().unwrap().preview.id(), plan_id);
                }
            }
        }
    }
}
