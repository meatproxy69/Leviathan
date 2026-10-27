//! Explicit whole-cell rectangles. The private clipboard belongs to one Editor
//! document; all copy, replacement, validation and history authority is shared.
use crate::{
    cell_blocks::{PasteDestination, RectScope},
    editor::{DocumentCellPreview, DocumentClipboard, Editor},
    song::{PatternId, TrackId},
    tracker_ui::{grid_id, AQUA},
};
use eframe::egui::{self, Color32, RichText};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
const WARN: Color32 = Color32::from_rgb(229, 185, 112);
const PAGE: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    occurrence: usize,
    pattern: PatternId,
    track: TrackId,
    row: u32,
    column: u16,
}
impl Context {
    fn of(e: &Editor) -> Option<Self> {
        e.has_grid().then(|| Self {
            occurrence: e.occurrence,
            pattern: e.pattern_id(),
            track: e.track,
            row: e.row,
            column: e.column,
        })
    }
    fn destination(&self) -> PasteDestination {
        PasteDestination {
            pattern: self.pattern,
            track: self.track,
            row_start: self.row,
            column_start: self.column,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Draft {
    pattern: PatternId,
    track: TrackId,
    row_start: String,
    row_end: String,
    column_start: String,
    column_end: String,
}
fn integer(text: &str, max: u64, label: &str) -> Result<u64, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "{label} must be an unsigned integer; no rounding or clamping"
        ));
    }
    let value = text
        .parse::<u64>()
        .map_err(|_| format!("{label} exceeds the integer range"))?;
    if value > max {
        return Err(format!("{label} must be in 0..{max}; no clamping"));
    }
    Ok(value)
}
impl Draft {
    fn at(e: &Editor) -> Self {
        Self {
            pattern: e.pattern_id(),
            track: e.track,
            row_start: e.row.to_string(),
            row_end: (e.row + 1).to_string(),
            column_start: e.column.to_string(),
            column_end: (e.column + 1).to_string(),
        }
    }
    fn scope(&self) -> Result<RectScope, String> {
        Ok(RectScope {
            pattern: self.pattern,
            track: self.track,
            row_start: integer(&self.row_start, u64::from(u32::MAX), "Row start")? as u32,
            row_end: integer(&self.row_end, u64::from(u32::MAX), "Row end (exclusive)")? as u32,
            column_start: integer(&self.column_start, u64::from(u16::MAX), "Column start")? as u16,
            column_end: integer(
                &self.column_end,
                u64::from(u16::MAX),
                "Column end (exclusive)",
            )? as u16,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Copy,
    Paste,
    Clear,
}
impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Paste => "Paste Replace",
            Self::Clear => "Clear",
        }
    }
}
enum Completed {
    Copy(Box<DocumentClipboard>),
    Preview(Box<DocumentCellPreview>),
}
struct Pending {
    epoch: u64,
    revision: u64,
    context: Context,
    draft: Draft,
    action: Action,
    receiver: Receiver<Result<Completed, String>>,
}
struct Ready {
    epoch: u64,
    context: Context,
    draft: Draft,
    action: Action,
    plan: DocumentCellPreview,
}
#[derive(Default)]
pub(crate) struct CellBlocksUi {
    open: bool,
    draft: Option<Draft>,
    clipboard: Option<DocumentClipboard>,
    pending: Option<Pending>,
    completed: Option<Result<Completed, String>>,
    ready: Option<Ready>,
    epoch: u64,
    revision: Option<u64>,
    error: Option<String>,
    notice: Option<String>,
    page: usize,
    selected: Option<usize>,
    detail: String,
    reveal_detail: bool,
    reveal_error: bool,
    #[cfg(any(test, feature = "ui-capture"))]
    body_layout: Option<(egui::Rect, f32)>,
}
impl CellBlocksUi {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn reset_document(&mut self) {
        self.close();
        self.draft = None;
        self.clipboard = None;
        self.revision = None;
    }
    pub(crate) fn close(&mut self) {
        self.open = false;
        self.epoch = self.epoch.wrapping_add(1);
        self.ready = None;
        self.completed = None;
        self.error = None;
        self.notice = None;
        self.selected = None;
        self.detail.clear();
        // Retain the single canceled worker until its actual terminal result.
    }
    pub(crate) fn open(&mut self, e: &Editor) {
        if !e.has_grid() {
            return;
        }
        self.sync(e);
        if self.draft.is_none() {
            self.draft = Some(Draft::at(e));
        }
        self.open = true;
    }
    fn invalidate(&mut self, message: &str) {
        self.epoch = self.epoch.wrapping_add(1);
        self.notice = Some(message.into());
        self.error = None;
        // A previous exact review remains visibly stale, never silently refreshed.
    }
    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.notice = None;
        self.reveal_error = true;
    }
    fn sync(&mut self, e: &Editor) {
        if self
            .clipboard
            .as_ref()
            .is_some_and(|c| !e.cell_clipboard_is_current_document(c))
            || self
                .ready
                .as_ref()
                .is_some_and(|r| !e.cell_preview_is_current_document(&r.plan))
        {
            self.reset_document();
        }
        if !e.has_grid() {
            self.close();
            self.draft = None;
            return;
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|d| d.pattern != e.pattern_id() || d.track != e.track)
        {
            self.invalidate("Pattern or track changed; review this destination explicitly.");
            self.draft = Some(Draft::at(e));
        }
        if self.revision.is_some_and(|r| r != e.revision()) {
            self.invalidate("Document changed; clipboard retained, previous preview is stale.");
        }
        self.revision = Some(e.revision());
    }
    pub(crate) fn poll(&mut self) {
        if self.completed.is_some() {
            return;
        }
        let Some(p) = &self.pending else {
            return;
        };
        match p.receiver.try_recv() {
            Ok(result) => self.completed = Some(result),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.completed = Some(Err(
                    "Cell rectangle worker disconnected; document unchanged".into(),
                ))
            }
        }
        if self.completed.is_some() && self.pending.as_ref().is_some_and(|p| p.epoch != self.epoch)
        {
            self.completed = None;
            self.pending = None;
        }
        // Completion is applied only after current-frame draft/context guards.
    }
    fn finish(&mut self, e: &Editor) {
        let Some(result) = self.completed.take() else {
            return;
        };
        let Some(p) = self.pending.take() else {
            return;
        };
        if !self.open
            || p.epoch != self.epoch
            || p.revision != e.revision()
            || Context::of(e).as_ref() != Some(&p.context)
            || self.draft.as_ref() != Some(&p.draft)
        {
            return;
        }
        match result {
            Ok(Completed::Copy(clipboard)) => match e.validate_cell_copy(&clipboard) {
                Ok(()) => {
                    self.clipboard = Some(*clipboard);
                    self.ready = None;
                    self.selected = None;
                    self.detail.clear();
                    self.page = 0;
                    self.error = None;
                    self.notice=Some("Copied whole cells to this document's private clipboard. No Song or OS clipboard change.".into());
                }
                Err(error) => self.fail(error),
            },
            Ok(Completed::Preview(plan)) => match e.validate_cell_preview(&plan) {
                Ok(()) => {
                    self.ready = Some(Ready {
                        epoch: p.epoch,
                        context: p.context,
                        draft: p.draft,
                        action: p.action,
                        plan: *plan,
                    });
                    self.error = None;
                    self.notice = None;
                    self.page = 0;
                    self.selected = None;
                    self.detail.clear();
                }
                Err(error) => self.fail(error),
            },
            Err(error) => self.fail(error),
        }
    }
    fn start(&mut self, e: &Editor, action: Action) {
        if self.busy() {
            return;
        }
        self.sync(e);
        let Some(context) = Context::of(e) else {
            return;
        };
        let Some(draft) = self.draft.clone() else {
            return;
        };
        let scope = match draft.scope() {
            Ok(v) => v,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let clipboard = if action == Action::Paste {
            match self.clipboard.clone() {
                Some(c) if e.cell_clipboard_is_current_document(&c) => Some(c),
                _ => {
                    self.fail("Copy a rectangle in this document first.".into());
                    return;
                }
            }
        } else {
            None
        };
        let capture = match e.capture_cell_document() {
            Ok(c) => c,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let destination = context.destination();
        let revision = capture.revision();
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("leviathan-cell-rectangle".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match action {
                        Action::Copy => capture
                            .copy_cells(&scope)
                            .map(|value| Completed::Copy(Box::new(value))),
                        Action::Paste => capture
                            .paste_cells_preview(clipboard.as_ref().unwrap(), &destination)
                            .map(|value| Completed::Preview(Box::new(value))),
                        Action::Clear => capture
                            .clear_cells_preview(&scope)
                            .map(|value| Completed::Preview(Box::new(value))),
                    }))
                    .unwrap_or_else(|_| {
                        Err("Cell rectangle worker panicked; document unchanged".into())
                    });
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    epoch: self.epoch,
                    revision,
                    context,
                    draft,
                    action,
                    receiver,
                });
                self.error = None;
                self.notice = None;
            }
            Err(error) => self.fail(format!("Could not start cell rectangle worker: {error}")),
        }
    }
    fn current(&self, e: &Editor) -> bool {
        self.ready.as_ref().is_some_and(|r| {
            r.epoch == self.epoch
                && r.plan.revision() == e.revision()
                && e.cell_preview_is_current_document(&r.plan)
                && Context::of(e).as_ref() == Some(&r.context)
                && self.draft.as_ref() == Some(&r.draft)
        })
    }
    pub(crate) fn highlight(&self, e: &Editor) -> Option<RectScope> {
        if self.current(e) {
            return self
                .ready
                .as_ref()
                .map(|r| r.plan.preview().summary().destination.clone());
        }
        let scope = self.draft.as_ref()?.scope().ok()?;
        let rows = e.song.patterns.get(&scope.pattern)?.rows;
        let columns = e.song.tracks.get(&scope.track)?.note_columns;
        (scope.pattern == e.pattern_id()
            && scope.track == e.track
            && scope.row_start < scope.row_end
            && scope.row_end <= rows
            && scope.column_start < scope.column_end
            && scope.column_end <= columns
            && u64::from(scope.row_end - scope.row_start)
                * u64::from(scope.column_end - scope.column_start)
                <= 4096)
            .then_some(scope)
    }
    pub(crate) fn shortcuts(&mut self, e: &Editor, ctx: &egui::Context) -> bool {
        if !ctx.memory(|m| m.has_focus(grid_id())) {
            return false;
        }
        let action = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Copy => Some(Action::Copy),
                egui::Event::Paste(_) => Some(Action::Paste),
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if modifiers.command || modifiers.ctrl => match key {
                    egui::Key::C => Some(Action::Copy),
                    egui::Key::V => Some(Action::Paste),
                    _ => None,
                },
                _ => None,
            })
        });
        if let Some(action) = action {
            self.open(e);
            self.start(e, action);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ReviewTab {
    #[default]
    Changes,
    Clipboard,
    Contexts,
    Aliases,
}
fn numeric(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(90.0)
            .id(egui::Id::new(("cell-rectangle", label))),
    );
}
fn cell_label(cell: &Option<crate::song::Cell>) -> String {
    match cell {
        None => "HOLE (no stored cell)".into(),
        Some(cell) if *cell == crate::song::Cell::default() => "STORED DEFAULT cell".into(),
        Some(cell) => format!(
            "{} / I{} / V{} / P{} / D{}{}",
            crate::editor::note_label(cell.note),
            cell.instrument.map_or("-".into(), |v| v.0.to_string()),
            cell.velocity.map_or("-".into(), |v| v.to_string()),
            cell.pan.map_or("-".into(), |v| v.to_string()),
            cell.delay.map_or("-".into(), |v| v.to_string()),
            if cell.phrase.is_some() || !cell.effects.is_empty() {
                " / FX or phrase"
            } else {
                ""
            }
        ),
    }
}
fn scope_text(s: &RectScope) -> String {
    format!(
        "P{} / T{} · rows [{},{}) · columns [{},{})",
        s.pattern.0, s.track.0, s.row_start, s.row_end, s.column_start, s.column_end
    )
}
fn pages(ui: &mut egui::Ui, page: &mut usize, total: usize) {
    let count = total.div_ceil(PAGE).max(1);
    *page = (*page).min(count - 1);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(*page > 0, egui::Button::new("Previous page"))
            .clicked()
        {
            *page -= 1;
        }
        ui.monospace(format!("Page {} / {count} · {total} records", *page + 1));
        if ui
            .add_enabled(*page + 1 < count, egui::Button::new("Next page"))
            .clicked()
        {
            *page += 1;
        }
        if ui
            .add_enabled(*page + 1 < count, egui::Button::new("Last page"))
            .clicked()
        {
            *page = count - 1;
        }
        if ui
            .add_enabled(*page > 0, egui::Button::new("First page"))
            .clicked()
        {
            *page = 0;
        }
    });
}
impl CellBlocksUi {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, e: &mut Editor) {
        self.sync(e);
        if !self.open {
            self.finish(e);
            return;
        }
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        let original = draft.clone();
        let context = Context::of(e);
        let allowed = ui.is_enabled();
        let lost_focus = ui.ctx().input(|i| {
            !i.focused
                || i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        });
        if !allowed || lost_focus {
            self.invalidate("Interaction interrupted; prepare a new preview before committing.");
        }
        let current = self.ready.as_ref().is_some_and(|r| {
            r.epoch == self.epoch
                && r.plan.revision() == e.revision()
                && e.cell_preview_is_current_document(&r.plan)
                && context.as_ref() == Some(&r.context)
                && r.draft == draft
        });
        let mut open = true;
        let mut action = None;
        let mut commit = false;
        let mut close = false;
        let mut clear_clipboard = false;
        let mut tab = ui.ctx().data_mut(|d| {
            d.get_temp::<ReviewTab>(egui::Id::new("cell-rectangle-review-tab"))
                .unwrap_or_default()
        });
        let previous_tab = tab;
        egui::Window::new("Cell rectangle")
            .id(egui::Id::new("cell-rectangle-window")).open(&mut open)
            .default_pos(egui::pos2(205.0,76.0)).default_width(650.0).min_width(500.0)
            .resizable(false).collapsible(false).show(ui.ctx(),|ui| {
                ui.add_enabled_ui(allowed,|ui| {
                    ui.label(RichText::new("WHOLE CELLS · private document clipboard").strong().color(AQUA));
                    ui.label("Replace includes holes: a copied hole removes the destination cell. No merge, field mask, growth, cropping or routing rewrite.");
                    let _body=egui::ScrollArea::vertical().id_salt("cell-rectangle-body")
                        .max_height((ui.ctx().content_rect().height()-310.0-20.0*(usize::from(self.busy())+usize::from(!e.edit_mode)+usize::from(self.error.is_some())) as f32).max(120.0)).auto_shrink([false,false])
                        .show(ui,|ui| {
                            ui.monospace(format!("Selection P{} / T{} · zero-based, half-open bounds",draft.pattern.0,draft.track.0));
                            ui.weak(if current {"Grid highlight: current reviewed destination footprint. Selection inputs below are for Copy / Clear."} else {"Grid highlight: explicit Copy / Clear selection (invalid ranges are not highlighted)."});
                            ui.horizontal_wrapped(|ui|{numeric(ui,"Row start",&mut draft.row_start);numeric(ui,"Row end",&mut draft.row_end);});
                            ui.horizontal_wrapped(|ui|{numeric(ui,"Column start",&mut draft.column_start);numeric(ui,"Column end",&mut draft.column_end);});
                            if let Ok(s)=draft.scope() {
                                if s.row_start<s.row_end&&s.column_start<s.column_end {
                                    ui.label(format!("Inclusive display: rows {}..{} · N{}..N{} · {} positions",s.row_start,s.row_end-1,u32::from(s.column_start)+1,s.column_end,u64::from(s.row_end-s.row_start)*u64::from(s.column_end-s.column_start)));
                                }
                            }
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Selection = cursor cell").clicked(){draft=Draft::at(e);}
                                if ui.button("Whole current track").clicked(){draft.row_start="0".into();draft.row_end=e.pattern().rows.to_string();draft.column_start="0".into();draft.column_end=e.song.tracks[&e.track].note_columns.to_string();}
                                ui.weak("Maximum 4,096 positions, including holes; larger scopes are refused.");
                            });
                            if e.pattern().track_aliases.contains_key(&e.track) {
                                ui.colored_label(WARN,"ALIAS: Copy reads the resolved source. Paste/Clear refuse this local destination; use Edit source or Make unique explicitly.");
                            }
                            ui.separator();
                            ui.monospace(format!("Paste destination: P{} / T{} · row {} / N{} (cursor)",e.pattern_id().0,e.track.0,e.row,e.column+1));
                            ui.label("Paste keeps the copied TrackId and current referenced instrument/phrase IDs. These are stored events, not frozen sound or a renderability proof.");
                            if let Some(c)=&self.clipboard {
                                let s=c.snapshot().summary();
                                ui.monospace(format!("COPIED {} × {} · {} stored / {} holes / {} explicit defaults",s.rows,s.columns,s.stored_cells,s.holes,s.explicit_default_cells));
                                ui.label(scope_text(&s.source_scope));
                                egui::CollapsingHeader::new("Clipboard identity and resolved source chain").show(ui,|ui| {
                                    ui.label(format!("Snapshot {}",c.snapshot().id()));
                                    ui.label(format!("Capture revision {} · {} encoded bytes",c.revision(),s.serialized_bytes));
                                    ui.label(format!("Historical source document SHA-256 {}",c.snapshot().source_document_sha256()));
                                    egui::ScrollArea::vertical().id_salt("cell-copy-chain").max_height(130.0).show_rows(ui,20.0,s.source_chain.len(),|ui,rows|for index in rows {let v=&s.source_chain[index];ui.monospace(format!("{index}: P{} / T{}",v.pattern.0,v.track.0));});
                                    for warning in &s.warnings {ui.colored_label(WARN,warning);}
                                });
                            } else {ui.weak("No internal clipboard. Copy is available in Read Mode; no OS clipboard service is used.");}
                            ui.separator();
                            ui.horizontal_wrapped(|ui| {
                                ui.selectable_value(&mut tab,ReviewTab::Changes,"Changes");
                                ui.selectable_value(&mut tab,ReviewTab::Clipboard,"Clipboard cells");
                                ui.selectable_value(&mut tab,ReviewTab::Contexts,"Repeated uses");
                                ui.selectable_value(&mut tab,ReviewTab::Aliases,"Aliases");
                            });
                            if tab!=previous_tab {self.page=0;self.selected=None;self.detail.clear();}
                            let mut chosen:Option<(usize,String)>=None;
                            match tab {
                                ReviewTab::Clipboard=>if let Some(c)=&self.clipboard {
                                    let s=c.snapshot();pages(ui,&mut self.page,s.cells().len());
                                    if let Ok(page)=s.page(self.page*PAGE,PAGE) {for (offset,v) in page.cells.iter().enumerate(){let index=page.start+offset;let label=format!("#{index} · +row {} / +column {} · {}",v.row_offset,v.column_offset,cell_label(&v.cell));
                                        if ui.add(egui::Button::selectable(self.selected==Some(index),label).truncate()).clicked(){chosen=Some((index,serde_json::to_string_pretty(v).unwrap_or_default()));}
                                    }}
                                },
                                ReviewTab::Changes=>if let Some(r)=&self.ready {
                                    let changes=r.plan.preview().changes();pages(ui,&mut self.page,changes.len());
                                    for (index,v) in changes.iter().enumerate().skip(self.page*PAGE).take(PAGE) {
                                        if ui.add(egui::Button::selectable(self.selected==Some(index),format!("#{index} · row {} / N{} · {} -> {}",v.row,v.column+1,cell_label(&v.before),cell_label(&v.after))).truncate()).clicked(){chosen=Some((index,serde_json::to_string_pretty(v).unwrap_or_default()));}
                                    }
                                },
                                ReviewTab::Contexts=>if let Some(r)=&self.ready {
                                    let values=r.plan.preview().contexts();pages(ui,&mut self.page,values.len());
                                    for (index,v) in values.iter().enumerate().skip(self.page*PAGE).take(PAGE) {if ui.selectable_label(self.selected==Some(index),format!("#{index} · POS {} · P{} / T{} · muted {}",v.occurrence,v.pattern.0,v.track.0,v.muted)).clicked(){chosen=Some((index,serde_json::to_string_pretty(v).unwrap_or_default()));}}
                                },
                                ReviewTab::Aliases=>if let Some(r)=&self.ready {
                                    let values=r.plan.preview().aliases();pages(ui,&mut self.page,values.len());
                                    for (index,v) in values.iter().enumerate().skip(self.page*PAGE).take(PAGE) {if ui.selectable_label(self.selected==Some(index),format!("#{index} · P{} / T{}",v.pattern.0,v.track.0)).clicked(){chosen=Some((index,serde_json::to_string_pretty(v).unwrap_or_default()));}}
                                },
                            }
                            if let Some((index,text))=chosen {self.selected=Some(index);self.detail=text;self.reveal_detail=true;}
                            if !self.detail.is_empty() {
                                let header=ui.label(RichText::new("Exact selected record · read only").strong().color(AQUA));
                                ui.weak("Complete stored record. Read-only text supports Ctrl+Home/End, PageUp/Down and wheel navigation.");
                                let binding=if tab==ReviewTab::Clipboard {self.clipboard.as_ref().map(|c|c.snapshot().id())} else {self.ready.as_ref().map(|r|r.plan.preview().id())};
                                let id=ui.make_persistent_id(("cell-rectangle-record",binding,tab as u8,self.selected));
                                if self.reveal_detail {let mut state=egui::text_edit::TextEditState::load(ui.ctx(),id).unwrap_or_default();state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(0))));state.store(ui.ctx(),id);}
                                let mut text=self.detail.as_str();
                                let response=ui.add(egui::TextEdit::multiline(&mut text).id(id).font(egui::TextStyle::Monospace).desired_width(ui.available_width()).cursor_at_end(false).frame(false));
                                if self.reveal_detail {header.scroll_to_me_animation(Some(egui::Align::Min),egui::style::ScrollAnimation::none());response.request_focus();self.reveal_detail = !ui.clip_rect().contains_rect(header.rect);if self.reveal_detail {ui.ctx().request_repaint();}}
                            }
                            if let Some(r)=&self.ready {
                                let p=r.plan.preview();let s=p.summary();
                                egui::CollapsingHeader::new("Exact plan identity, work limits and caveats").show(ui,|ui| {
                                    ui.label(format!("{} · {}",s.algorithm,p.id()));
                                    ui.label(format!("Base SHA-256 {}",p.base_sha256()));
                                    ui.label(format!("Candidate SHA-256 {}",p.candidate_sha256()));
                                    ui.monospace(format!("Diff {} bytes · aliases {} · ordered contexts {}",s.diff_bytes,s.affected_aliases,s.affected_contexts));
                                    ui.label(format!("Base work: {:?}",s.base_work));
                                    ui.label(format!("Projected work: {:?}",s.projected_work));
                                    for warning in &s.warnings {ui.colored_label(WARN,warning);}
                                });
                            }
                            if let Some(error)=&self.error {let response=ui.colored_label(Color32::LIGHT_RED,error);if self.reveal_error {response.scroll_to_me(Some(egui::Align::BOTTOM));self.reveal_error=false;}}
                            if let Some(notice)=&self.notice {ui.label(RichText::new(notice).small().color(AQUA));}
                        });
                    #[cfg(any(test,feature="ui-capture"))] {self.body_layout=Some((_body.inner_rect,_body.state.offset.y));}
                    ui.separator();
                    if let Some(r)=&self.ready {
                        let s=r.plan.preview().summary();
                        ui.monospace(format!("{} {} · {} changed (+{} / -{} / {} replaced)",if current{"READY"}else{"PREVIOUS / STALE"},r.action.label(),s.changed_cells,s.inserted_cells,s.removed_cells,s.replaced_cells));
                        ui.label(RichText::new(scope_text(&s.destination)).small());
                    } else {ui.weak("Copy or preview a whole-cell operation. Nothing changes until Commit.");}
                    if self.busy(){ui.colored_label(WARN,"Preparing bounded metadata; no Song mutation on completion.");}
                    if !e.edit_mode {ui.weak("READ MODE · Copy/review only; Commit requires Edit mode.");}
                    if self.error.is_some(){ui.colored_label(Color32::LIGHT_RED,"Request refused — exact error is in the scrollable body.");}
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!self.busy(),egui::Button::new("Copy")).clicked(){action=Some(Action::Copy);}
                        if ui.add_enabled(!self.busy()&&self.clipboard.is_some(),egui::Button::new("Preview Paste Replace")).clicked(){action=Some(Action::Paste);}
                        if ui.add_enabled(!self.busy(),egui::Button::new("Preview Clear")).clicked(){action=Some(Action::Clear);}
                        if ui.add_enabled(!self.busy()&&current&&e.edit_mode,egui::Button::new("Commit")).clicked(){commit=true;}
                        if ui.button("Close").clicked(){close=true;}
                    });
                    ui.horizontal(|ui| {if ui.add_enabled(!self.busy()&&self.clipboard.is_some(),egui::Button::new("Forget internal copy")).clicked(){clear_clipboard=true;}ui.weak("Changed commit: one Undo · no cursor or sound action");});
                });
            });
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("cell-rectangle-review-tab"), tab));
        self.draft = Some(draft);
        if self.draft.as_ref() != Some(&original) {
            self.invalidate("Selection changed; previous preview is stale. Prepare again.");
        }
        if !open || close {
            self.close();
            return;
        }
        if clear_clipboard {
            self.clipboard = None;
            self.ready = None;
            self.selected = None;
            self.detail.clear();
            self.invalidate("Internal copy forgotten; Song unchanged.");
        }
        if !allowed || lost_focus {
            self.finish(e);
            return;
        }
        self.finish(e);
        if let Some(action) = action {
            self.start(e, action);
        }
        if commit && self.current(e) && !self.busy() {
            let r = self.ready.as_ref().unwrap();
            let before_revision = e.revision();
            match e.commit_cell_block_preview(&r.plan) {
                Ok(()) => {
                    self.ready = None;
                    self.selected = None;
                    self.detail.clear();
                    self.error = None;
                    self.notice=Some(if e.revision()==before_revision {"No stored-cell change; no history entry. Clipboard retained."} else {"Whole-cell operation committed once. Clipboard retained; no cursor or audio action."}.into());
                    self.revision = Some(e.revision());
                }
                Err(error) => self.fail(error),
            }
        }
        ui.ctx().memory_mut(|m| m.surrender_focus(grid_id()));
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(crate) fn capture_state(&self, e: &Editor) -> serde_json::Value {
        serde_json::json!({"open":self.open,"busy":self.busy(),"completion_waiting":self.completed.is_some(),"current":self.current(e),"epoch":self.epoch,"error":self.error,"notice":self.notice,
            "draft":self.draft.as_ref().map(|d|serde_json::json!({"pattern":d.pattern,"track":d.track,"row_start":d.row_start,"row_end":d.row_end,"column_start":d.column_start,"column_end":d.column_end,"parsed_scope":d.scope().ok(),"parse_error":d.scope().err()})),
            "clipboard":self.clipboard.as_ref().map(|c|serde_json::json!({"id":c.snapshot().id(),"revision":c.revision(),"summary":c.snapshot().summary(),"cells":c.snapshot().cells(),"cells_json":serde_json::to_string(c.snapshot().cells()).ok(),"source_document_sha256":c.snapshot().source_document_sha256()})),
            "preview":self.ready.as_ref().map(|r|{let p=r.plan.preview();serde_json::json!({"id":p.id(),"revision":r.plan.revision(),"request":p.request(),"summary":p.summary(),"changes":p.changes(),"changes_json":serde_json::to_string(p.changes()).ok(),"base_json":serde_json::to_string(p.base()).ok(),"candidate_json":serde_json::to_string(p.candidate()).ok(),"contexts":p.contexts(),"aliases":p.aliases(),"candidate":p.candidate(),"base_sha256":p.base_sha256(),"candidate_sha256":p.candidate_sha256()})}),
            "page":self.page,"selected":self.selected,"detail":self.detail,
            "body":self.body_layout.map(|(r,y)|serde_json::json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y],"offset":y}))})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{Cell, InstrumentId, Note};
    fn editor() -> Editor {
        let mut e = Editor {
            edit_mode: true,
            ..Default::default()
        };
        let id = e.pattern_id();
        let rows = e
            .song
            .patterns
            .get_mut(&id)
            .unwrap()
            .tracks
            .entry(e.track)
            .or_default();
        rows.clear();
        rows.entry(0).or_default().insert(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(InstrumentId(1)),
                velocity: Some(0.3),
                delay: Some(-0.0),
                ..Default::default()
            },
        );
        rows.entry(0).or_default().insert(1, Cell::default());
        rows.entry(1).or_default().insert(
            1,
            Cell {
                note: Note::Off,
                ..Default::default()
            },
        );
        rows.entry(5).or_default().insert(
            0,
            Cell {
                note: Note::Note(66),
                instrument: Some(InstrumentId(1)),
                ..Default::default()
            },
        );
        e
    }
    fn wait(ui: &mut CellBlocksUi, e: &Editor) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while ui.busy() {
            ui.poll();
            ui.finish(e);
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn rectangle(ui: &mut CellBlocksUi, e: &Editor) {
        ui.open(e);
        let d = ui.draft.as_mut().unwrap();
        d.row_start = "0".into();
        d.row_end = "2".into();
        d.column_start = "0".into();
        d.column_end = "2".into();
    }
    #[test]
    fn cell_rectangle_numeric_text_is_exact_and_never_clamped() {
        let e = editor();
        let mut d = Draft::at(&e);
        for invalid in [
            "-1",
            "1.0",
            "1e2",
            "NaN",
            "18446744073709551616",
            " 1",
            "+1",
            "",
        ] {
            d.row_end = invalid.into();
            assert!(d.scope().is_err(), "{invalid}");
            assert_eq!(d.row_end, invalid);
        }
        d.row_end = u32::MAX.to_string();
        d.column_end = u16::MAX.to_string();
        let s = d.scope().unwrap();
        assert_eq!((s.row_end, s.column_end), (u32::MAX, u16::MAX));
        let mut ui = CellBlocksUi::default();
        ui.open(&e);
        ui.draft = Some(d);
        assert!(ui.highlight(&e).is_none());
    }
    #[test]
    fn cell_rectangle_opaque_copy_preserves_holes_defaults_and_exact_history() {
        let mut e = editor();
        let before = serde_json::to_vec(&e.song).unwrap();
        let cursor = (e.occurrence, e.track, e.row, e.column, e.field);
        let revision = e.revision();
        let mut ui = CellBlocksUi::default();
        rectangle(&mut ui, &e);
        e.edit_mode = false;
        ui.start(&e, Action::Copy);
        wait(&mut ui, &e);
        let c = ui.clipboard.as_ref().unwrap().snapshot();
        assert_eq!(
            (
                c.summary().stored_cells,
                c.summary().holes,
                c.summary().explicit_default_cells
            ),
            (3, 1, 1)
        );
        assert_eq!(
            c.cells()[0].as_ref().unwrap().delay.unwrap().to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        assert_eq!(e.revision(), revision);
        e.row = 4;
        ui.start(&e, Action::Paste);
        wait(&mut ui, &e);
        let ready = ui.ready.as_ref().unwrap();
        assert!(e.commit_cell_block_preview(&ready.plan).is_err());
        e.edit_mode = true;
        e.commit_cell_block_preview(&ready.plan).unwrap();
        let expected = serde_json::to_vec(ready.plan.preview().candidate()).unwrap();
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), expected);
        assert_eq!(
            (e.occurrence, e.track, e.column, e.field),
            (cursor.0, cursor.1, cursor.3, cursor.4)
        );
        assert_eq!(e.row, 4);
        assert!(
            !e.pattern().tracks[&e.track][&5].contains_key(&0),
            "Hole removes destination cell, keeping its row container"
        );
        assert!(
            e.pattern().tracks[&e.track][&4].contains_key(&1),
            "Explicit default stays stored"
        );
        e.undo();
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        ui.sync(&e);
        assert!(ui.clipboard.is_some());
        e.redo();
        assert_eq!(serde_json::to_vec(&e.song).unwrap(), expected);
        ui.sync(&e);
        assert!(ui.clipboard.is_some());
    }
    #[test]
    fn cell_rectangle_failed_replacement_retains_review_but_copy_and_document_owner_revoke_it() {
        let mut e = editor();
        let mut ui = CellBlocksUi::default();
        rectangle(&mut ui, &e);
        ui.start(&e, Action::Copy);
        wait(&mut ui, &e);
        e.row = 4;
        ui.start(&e, Action::Paste);
        wait(&mut ui, &e);
        let id = ui.ready.as_ref().unwrap().plan.preview().id().to_owned();
        ui.draft.as_mut().unwrap().row_end = "65536".into();
        ui.invalidate("draft changed");
        for _ in 0..2 {
            ui.start(&e, Action::Clear);
            wait(&mut ui, &e);
            assert!(ui.error.is_some());
            assert_eq!(ui.ready.as_ref().unwrap().plan.preview().id(), id);
            assert!(!ui.current(&e));
        }
        rectangle(&mut ui, &e);
        ui.start(&e, Action::Copy);
        wait(&mut ui, &e);
        assert!(ui.ready.is_none());
        let same = e.song.clone();
        e.replace_document(same).unwrap();
        ui.sync(&e);
        assert!(ui.clipboard.is_none());
        assert!(!ui.is_open());
    }
    #[test]
    fn cell_rectangle_late_copy_and_preview_are_inert_after_close_context_or_revision() {
        for change in 0..3 {
            let mut e = editor();
            let before = serde_json::to_vec(&e.song).unwrap();
            let mut ui = CellBlocksUi::default();
            rectangle(&mut ui, &e);
            ui.start(&e, Action::Copy);
            match change {
                0 => ui.close(),
                1 => e.row = 3,
                _ => {
                    let same = e.song.clone();
                    e.replace_document(same).unwrap();
                }
            }
            wait(&mut ui, &e);
            assert!(ui.clipboard.is_none());
            assert!(ui.ready.is_none());
            assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
        }
    }
    fn visible(out: &egui::FullOutput, label: &str, size: egui::Vec2) -> bool {
        fn walk(shape: &egui::Shape, clip: egui::Rect, label: &str, viewport: egui::Rect) -> bool {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    let r = egui::Rect::from_min_size(t.pos, t.galley.size());
                    clip.contains_rect(r) && viewport.contains_rect(r)
                }
                egui::Shape::Vec(v) => v.iter().any(|s| walk(s, clip, label, viewport)),
                _ => false,
            }
        }
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        out.shapes
            .iter()
            .any(|s| walk(&s.shape, s.clip_rect, label, viewport))
    }
    #[test]
    fn cell_rectangle_maximum_scope_pages_keep_fixed_footer_reachable_at_both_sizes() {
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let mut e = editor();
            let old_track = e.track;
            let high_track = TrackId(u64::MAX);
            let old_pattern = e.pattern_id();
            let high_pattern = PatternId(u64::MAX);
            let track = e.song.tracks.remove(&old_track).unwrap();
            e.song.tracks.insert(high_track, track);
            for id in &mut e.song.track_order {
                if *id == old_track {
                    *id = high_track;
                }
            }
            for p in e.song.patterns.values_mut() {
                if let Some(rows) = p.tracks.remove(&old_track) {
                    p.tracks.insert(high_track, rows);
                }
            }
            let pattern = e.song.patterns.remove(&old_pattern).unwrap();
            e.song.patterns.insert(high_pattern, pattern);
            for occurrence in &mut e.song.order {
                if occurrence.pattern == old_pattern {
                    occurrence.pattern = high_pattern;
                }
            }
            e.track = high_track;
            e.song.tracks.get_mut(&e.track).unwrap().note_columns = 256;
            let pattern = e.pattern_id();
            for row in 0..16 {
                for column in 0..256 {
                    e.song
                        .patterns
                        .get_mut(&pattern)
                        .unwrap()
                        .tracks
                        .entry(e.track)
                        .or_default()
                        .entry(row)
                        .or_default()
                        .entry(column)
                        .or_default();
                }
            }
            let mut ui = CellBlocksUi::default();
            rectangle(&mut ui, &e);
            let d = ui.draft.as_mut().unwrap();
            d.row_end = "16".into();
            d.column_end = "256".into();
            ui.start(&e, Action::Copy);
            wait(&mut ui, &e);
            assert_eq!(
                ui.clipboard.as_ref().unwrap().snapshot().cells().len(),
                4096
            );
            ui.start(&e, Action::Clear);
            wait(&mut ui, &e);
            assert_eq!(
                ui.ready.as_ref().unwrap().plan.preview().changes().len(),
                4096
            );
            ui.invalidate("Previous review retained while another bounded worker is pending.");
            let (_held_sender, receiver) = mpsc::sync_channel(1);
            ui.pending = Some(Pending {
                epoch: ui.epoch,
                revision: e.revision(),
                context: Context::of(&e).unwrap(),
                draft: ui.draft.clone().unwrap(),
                action: Action::Copy,
                receiver,
            });
            ui.error = Some("Explicit refusal with all details retained. ".repeat(100));
            e.edit_mode = false;
            let ctx = egui::Context::default();
            for i in 0..8 {
                let out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |body| ui.show(body, &mut e));
                    },
                );
                if i > 3 {
                    for label in [
                        "Copy",
                        "Preview Paste Replace",
                        "Preview Clear",
                        "Commit",
                        "Close",
                        "Forget internal copy",
                    ] {
                        assert!(visible(&out, label, size), "{label} at {size:?}");
                    }
                }
            }
            assert_eq!(
                ui.clipboard
                    .as_ref()
                    .unwrap()
                    .snapshot()
                    .page(4095, 1)
                    .unwrap()
                    .cells
                    .len(),
                1
            );
        }
    }
    #[test]
    fn cell_rectangle_shortcuts_require_grid_focus_and_do_not_publish_os_clipboard() {
        let e = editor();
        let mut ui = CellBlocksUi::default();
        let ctx = egui::Context::default();
        let out = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Copy],
                focused: true,
                ..Default::default()
            },
            |ctx| {
                assert!(!ui.shortcuts(&e, ctx));
            },
        );
        assert!(out.platform_output.commands.is_empty());
        assert!(ui.clipboard.is_none());
    }
    #[test]
    fn cell_rectangle_exact_record_is_readonly_and_every_line_is_keyboard_or_wheel_reachable() {
        use crate::song::{Direction, Glide, Retrigger, StepEffects};
        for size in [egui::vec2(900.0, 620.0), egui::vec2(1180.0, 780.0)] {
            let mut e = editor();
            let id = e.pattern_id();
            e.song
                .patterns
                .get_mut(&id)
                .unwrap()
                .tracks
                .get_mut(&e.track)
                .unwrap()
                .get_mut(&0)
                .unwrap()
                .insert(
                    0,
                    Cell {
                        note: Note::Note(60),
                        instrument: Some(InstrumentId(1)),
                        velocity: Some(-0.0),
                        pan: Some(-0.0),
                        delay: Some(f64::from_bits(1)),
                        effects: StepEffects {
                            retrigger: Some(Retrigger {
                                repeats: 16,
                                spacing: 0.01,
                            }),
                            release: Some(0.99),
                            glide: Some(Glide {
                                target: 127,
                                semitones_per_second: 0.01,
                            }),
                            cut: None,
                            offset: Some(u32::MAX as u64),
                            direction: Some(Direction::Reverse),
                        },
                        phrase: None,
                    },
                );
            let before = serde_json::to_vec(&e.song).unwrap();
            let revision = e.revision();
            let mut ui = CellBlocksUi::default();
            rectangle(&mut ui, &e);
            ui.start(&e, Action::Clear);
            wait(&mut ui, &e);
            let record = &ui.ready.as_ref().unwrap().plan.preview().changes()[0];
            ui.selected = Some(0);
            ui.detail = serde_json::to_string_pretty(record).unwrap();
            ui.reveal_detail = true;
            let exact = ui.detail.clone();
            let ctx = egui::Context::default();
            let mut time = 0.0;
            let frame = |ui: &mut CellBlocksUi,
                         e: &mut Editor,
                         time: &mut f64,
                         events: Vec<egui::Event>| {
                *time += 1.0 / 60.0;
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        focused: true,
                        time: Some(*time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |body| ui.show(body, e));
                    },
                )
            };
            for _ in 0..12 {
                frame(&mut ui, &mut e, &mut time, vec![]);
            }
            let key_event = |key, pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
            };
            for pressed in [true, false] {
                frame(
                    &mut ui,
                    &mut e,
                    &mut time,
                    vec![key_event(egui::Key::End, pressed)],
                );
            }
            let mut out = frame(&mut ui, &mut e, &mut time, vec![]);
            let row_visible = |out: &egui::FullOutput, needle: &str| {
                out.shapes.iter().any(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == exact => {
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
            for _ in 0..15 {
                out = frame(&mut ui, &mut e, &mut time, vec![]);
            }
            assert!(
                row_visible(&out, "\"after\": null"),
                "Last full record field must be keyboard visible at {size:?}"
            );
            for pressed in [true, false] {
                frame(
                    &mut ui,
                    &mut e,
                    &mut time,
                    vec![key_event(egui::Key::Home, pressed)],
                );
            }
            for _ in 0..15 {
                out = frame(&mut ui, &mut e, &mut time, vec![]);
            }
            assert!(
                row_visible(&out, "\"row\": 0"),
                "First record field must be keyboard visible"
            );
            let mut visited = std::collections::BTreeSet::new();
            let mut total = 0;
            for _ in 0..100 {
                for s in &out.shapes {
                    if let egui::Shape::Text(t) = &s.shape {
                        if t.galley.text() == exact {
                            total = t.galley.rows.len();
                            for (index, r) in t.galley.rows.iter().enumerate() {
                                if s.clip_rect
                                    .contains_rect(r.rect().translate(t.pos.to_vec2()))
                                {
                                    visited.insert(index);
                                }
                            }
                        }
                    }
                }
                if total > 0 && visited.len() == total {
                    break;
                }
                let body = ui.body_layout.unwrap().0;
                out = frame(
                    &mut ui,
                    &mut e,
                    &mut time,
                    vec![
                        egui::Event::PointerMoved(body.center()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -35.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                visited.len(),
                total,
                "Every shaped exact-record row is reachable at {size:?}"
            );
            assert!(total > 20);
            frame(
                &mut ui,
                &mut e,
                &mut time,
                vec![egui::Event::Text("malicious edit".into())],
            );
            assert_eq!(ui.detail, exact);
            assert_eq!(serde_json::to_vec(&e.song).unwrap(), before);
            assert_eq!(e.revision(), revision);
            assert!(visible(&out, "Commit", size));
            assert!(visible(&out, "Close", size));
        }
    }
}
