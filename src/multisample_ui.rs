//! Native melodic key-zone authoring. The ordinary Song remains the only event
//! authority; this inspector has no keyboard audition or audio ownership.
use crate::{
    editor::{Editor, note_label},
    song::{
        InstrumentId, InstrumentKind, KeyZone, KeyZoneId, MultisampleId, MultisampleInstrument,
        RegionId, Song,
    },
    tracker_ui::{AQUA, grid_id},
};
use eframe::egui::{self, Color32, RichText};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const WARNING: Color32 = Color32::from_rgb(229, 185, 112);

#[derive(Clone, Debug, PartialEq)]
struct ZoneDraft {
    name: String,
    note_min: String,
    note_max: String,
    velocity_min: String,
    velocity_max: String,
    region: Option<RegionId>,
    root: String,
}
impl ZoneDraft {
    fn from_zone(zone: &KeyZone) -> Self {
        Self {
            name: zone.name.clone(),
            note_min: zone.note_min.to_string(),
            note_max: zone.note_max.to_string(),
            velocity_min: zone.velocity_min.to_string(),
            velocity_max: zone.velocity_max.to_string(),
            region: Some(zone.region),
            root: zone.root_note.to_string(),
        }
    }
    fn parse(&self) -> Result<KeyZone, String> {
        Ok(KeyZone {
            name: self.name.clone(),
            note_min: midi(&self.note_min)?,
            note_max: midi(&self.note_max)?,
            velocity_min: midi(&self.velocity_min)?,
            velocity_max: midi(&self.velocity_max)?,
            region: self.region.ok_or("Choose a region for this zone")?,
            root_note: midi(&self.root)?,
        })
    }
}
#[derive(Clone)]
struct Draft {
    revision: u64,
    base: Song,
    id: MultisampleId,
    name: String,
    zones: BTreeMap<KeyZoneId, ZoneDraft>,
    selected: Option<KeyZoneId>,
    next_zone: Option<u64>,
}
impl Draft {
    fn new(editor: &Editor, id: MultisampleId) -> Result<Self, String> {
        let map = editor
            .song
            .multisamples
            .get(&id)
            .ok_or("Map no longer exists")?;
        Ok(Self {
            revision: editor.revision(),
            base: editor.song.clone(),
            id,
            name: map.name.clone(),
            zones: map
                .zones
                .iter()
                .map(|(id, zone)| (*id, ZoneDraft::from_zone(zone)))
                .collect(),
            selected: map.zones.keys().next().copied(),
            next_zone: map
                .zones
                .keys()
                .map(|id| id.0)
                .max()
                .unwrap_or(0)
                .checked_add(1),
        })
    }
    fn current(&self, editor: &Editor) -> bool {
        self.revision == editor.revision()
            && self.base == editor.song
            && editor.song.multisamples.contains_key(&self.id)
    }
    fn changed(&self, editor: &Editor) -> bool {
        self.parse().ok().as_ref() != editor.song.multisamples.get(&self.id)
    }
    fn parse(&self) -> Result<MultisampleInstrument, String> {
        let zones = self
            .zones
            .iter()
            .map(|(id, zone)| {
                zone.parse()
                    .map(|zone| (*id, zone))
                    .map_err(|error| format!("Zone {}: {error}", id.0))
            })
            .collect::<Result<_, _>>()?;
        Ok(MultisampleInstrument {
            name: self.name.clone(),
            zones,
        })
    }
    fn add_zone(&mut self) -> Result<(), String> {
        if self.zones.len() >= 128 {
            return Err("At most 128 zones per map".into());
        }
        let id = KeyZoneId(self.next_zone.ok_or("Stable zone ID namespace exhausted")?);
        self.next_zone = id.0.checked_add(1);
        self.zones.insert(id, ZoneDraft {
            name: format!("Zone {}", id.0),
            note_min: "0".into(),
            note_max: "127".into(),
            velocity_min: "0".into(),
            velocity_max: "127".into(),
            region: None,
            root: "60".into(),
        });
        self.selected = Some(id);
        Ok(())
    }
}
fn midi(text: &str) -> Result<u8, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Enter an integer 0..127; no clamping".into());
    }
    text.parse::<u8>()
        .ok()
        .filter(|value| *value <= 127)
        .ok_or_else(|| "Value must be 0..127; no clamping".into())
}
fn next_map(song: &Song) -> Result<MultisampleId, String> {
    song.multisamples
        .keys()
        .map(|id| id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .map(MultisampleId)
        .ok_or_else(|| "Map ID namespace exhausted".into())
}
fn number(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .id(egui::Id::new(("keyzone-number", label)))
            .desired_width(43.0),
    );
}

pub(crate) struct MultisampleUi {
    pub open: bool,
    instrument: Option<InstrumentId>,
    map: Option<MultisampleId>,
    new_name: String,
    draft: Option<Draft>,
    inspect: Option<(RegionId, InstrumentId)>,
    error: Option<String>,
    notice: String,
}
impl Default for MultisampleUi {
    fn default() -> Self {
        Self {
            open: false,
            instrument: None,
            map: None,
            new_name: "New melodic map".into(),
            draft: None,
            inspect: None,
            error: None,
            notice: String::new(),
        }
    }
}
impl MultisampleUi {
    pub(crate) fn agent_state(&self, editor: &Editor) -> Value {
        let draft=self.draft.as_ref().map(|d|json!({"revision":d.revision,"map_id":d.id.0,"name":d.name,"selected_zone":d.selected.map(|v|v.0),"zones":d.zones.iter().map(|(id,z)|json!({"id":id.0,"name":z.name,"note_min":z.note_min,"note_max":z.note_max,"velocity_min":z.velocity_min,"velocity_max":z.velocity_max,"region":z.region.map(|r|r.0),"root_midi":z.root})).collect::<Vec<_>>()}));
        json!({"component":"multisample","open":self.open,"edit_mode":editor.edit_mode,"instrument":self.instrument.map(|v|v.0),"map":self.map.map(|v|v.0),"new_name":self.new_name,"draft":draft,"error":self.error,"notice":self.notice,
            "instruments":editor.song.instruments.iter().map(|(id,i)|json!({"id":id.0,"name":i.name,"assigned_map":match i.kind{InstrumentKind::Multisample{map}=>Some(map.0),_=>None}})).collect::<Vec<_>>(),
            "maps":editor.song.multisamples.iter().map(|(id,m)|json!({"id":id.0,"name":m.name,"zones":m.zones.iter().map(|(zid,z)|json!({"id":zid.0,"name":z.name,"note_min":z.note_min,"note_max":z.note_max,"velocity_min":z.velocity_min,"velocity_max":z.velocity_max,"region":z.region.0,"root_midi":z.root_note})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "regions":editor.song.regions.iter().map(|(id,r)|json!({"id":id.0,"name":r.name,"root_midi":r.root_note,"start_frame":r.start_frame,"end_frame":r.end_frame})).collect::<Vec<_>>(),
            "actions":{"select_instrument":{"instrument_id":"integer|null"},"select_map":{"map_id":"integer|null"},"open_for":{"instrument_id":"integer"},"set_new_name":{"name":"string"},"create":{},"assign":{},"detach":{},"remove_map":{},"reload":{},"select_zone":{"zone_id":"integer|null"},"add_zone":{},"remove_zone":{"zone_id":"integer"},"patch_map":{"name":"string"},"patch_zone":{"zone_id":"integer","fields":["name","note_min","note_max","velocity_min","velocity_max","region","root"]},"apply":{},"fresh_copy":{},"inspect_region":{}},
            "policy":{"requires_edit_mode_for_mutation":true,"apply_is_atomic_single_edit":true,"zones_use_stable_ids_sorted_by_id":true}})
    }
    pub(crate) fn agent_action(
        &mut self,
        editor: &mut Editor,
        _base: Option<&Path>,
        action: Value,
    ) -> Result<Value, String> {
        let o = action.as_object().ok_or("action must be an object")?;
        let name = o
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action name required")?;
        let fields: &[&str] = match name {
            "select_instrument" | "open_for" => &["action", "instrument_id"],
            "select_map" => &["action", "map_id"],
            "set_new_name" | "patch_map" => &["action", "name"],
            "select_zone" => &["action", "zone_id"],
            "remove_zone" => &["action", "zone_id"],
            "patch_zone" => &["action", "zone_id", "fields"],
            "create" | "assign" | "detach" | "remove_map" | "reload" | "add_zone" | "apply"
            | "fresh_copy" | "inspect_region" => &["action"],
            _ => return Err(format!("unknown multisample action: {name}")),
        };
        if let Some(k) = o.keys().find(|k| !fields.contains(&k.as_str())) {
            return Err(format!("field {k} is not accepted for {name}"));
        }
        let allowed = [
            "action",
            "instrument_id",
            "map_id",
            "name",
            "zone_id",
            "fields",
        ];
        if let Some(k) = o.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(format!("unknown action field: {k}"));
        }
        let integer = |k: &str| {
            o.get(k)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{k} must be unsigned integer"))
        };
        let nullable = |k: &str| -> Result<Option<u64>, String> {
            match o.get(k) {
                Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_u64()
                    .map(Some)
                    .ok_or_else(|| format!("{k} must be integer|null")),
                None => Err(format!("{k} required")),
            }
        };
        match name {
            "select_instrument" => {
                let id = nullable("instrument_id")?.map(InstrumentId);
                if id.is_some_and(|id| !editor.song.instruments.contains_key(&id)) {
                    return Err("unknown instrument".into());
                }
                self.instrument = id;
                self.map = assigned_map(editor, id);
                self.reload(editor);
                self.error = None;
            }
            "select_map" => {
                let id = nullable("map_id")?.map(MultisampleId);
                if id.is_some_and(|id| !editor.song.multisamples.contains_key(&id)) {
                    return Err("unknown map".into());
                }
                self.map = id;
                self.reload(editor);
                self.error = None;
            }
            "open_for" => self.open_for(editor, InstrumentId(integer("instrument_id")?)),
            "set_new_name" => {
                self.new_name = o
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("name must be string")?
                    .into()
            }
            "create" => self.create(editor)?,
            "assign" => self.assign(editor, false)?,
            "detach" => self.assign(editor, true)?,
            "remove_map" => self.remove(editor)?,
            "reload" => {
                self.reload(editor);
                self.error = None;
                self.notice.clear()
            }
            "select_zone" => {
                let id = nullable("zone_id")?.map(KeyZoneId);
                let d = self.draft.as_mut().ok_or("choose a map first")?;
                if id.is_some_and(|id| !d.zones.contains_key(&id)) {
                    return Err("unknown zone".into());
                }
                d.selected = id;
            }
            "add_zone" => self
                .draft
                .as_mut()
                .ok_or("choose a map first")?
                .add_zone()?,
            "remove_zone" => {
                let id = KeyZoneId(integer("zone_id")?);
                let d = self.draft.as_mut().ok_or("choose a map first")?;
                if !d.zones.contains_key(&id) {
                    return Err("unknown zone".into());
                }
                d.zones.remove(&id);
                if d.selected == Some(id) {
                    d.selected = d.zones.keys().next().copied();
                }
            }
            "patch_map" => {
                let d = self.draft.as_mut().ok_or("choose a map first")?;
                d.name = o
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("name must be string")?
                    .into()
            }
            "patch_zone" => {
                let id = KeyZoneId(integer("zone_id")?);
                let f = o
                    .get("fields")
                    .and_then(Value::as_object)
                    .ok_or("fields object required")?;
                let fields = [
                    "name",
                    "note_min",
                    "note_max",
                    "velocity_min",
                    "velocity_max",
                    "region",
                    "root",
                ];
                if let Some(k) = f.keys().find(|k| !fields.contains(&k.as_str())) {
                    return Err(format!("unknown zone field: {k}"));
                }
                let mut d = self.draft.as_ref().cloned().ok_or("choose a map first")?;
                let z = d.zones.get_mut(&id).ok_or("unknown zone")?;
                for (k, v) in f {
                    match k.as_str() {
                        "name" => z.name = v.as_str().ok_or("name must be text")?.into(),
                        "note_min" => {
                            z.note_min = v.as_str().ok_or("note_min must be text")?.into()
                        }
                        "note_max" => {
                            z.note_max = v.as_str().ok_or("note_max must be text")?.into()
                        }
                        "velocity_min" => {
                            z.velocity_min = v.as_str().ok_or("velocity_min must be text")?.into()
                        }
                        "velocity_max" => {
                            z.velocity_max = v.as_str().ok_or("velocity_max must be text")?.into()
                        }
                        "root" => z.root = v.as_str().ok_or("root must be text")?.into(),
                        "region" => {
                            let region = if v.is_null() {
                                None
                            } else {
                                Some(RegionId(v.as_u64().ok_or("region must be integer|null")?))
                            };
                            if region.is_some_and(|id| !editor.song.regions.contains_key(&id)) {
                                return Err("unknown region".into());
                            }
                            z.region = region;
                            if let Some(r) = region.and_then(|id| editor.song.regions.get(&id)) {
                                z.root = r.root_note.to_string();
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                self.draft = Some(d);
            }
            "apply" => self.apply(editor, false)?,
            "fresh_copy" => self.apply(editor, true)?,
            "inspect_region" => {
                let id = self
                    .draft
                    .as_ref()
                    .and_then(|d| d.selected)
                    .and_then(|id| self.draft.as_ref()?.zones.get(&id))
                    .and_then(|z| z.region)
                    .ok_or("selected zone has no region")?;
                let inst = self.instrument.ok_or("choose instrument")?;
                self.inspect = Some((id, inst));
                self.open = false;
            }
            _ => return Err(format!("unknown multisample action: {name}")),
        }
        Ok(self.agent_state(editor))
    }
    pub fn reset_document(&mut self) {
        *self = Self::default();
    }
    pub fn open_for(&mut self, editor: &Editor, instrument: InstrumentId) {
        self.open = true;
        if self.instrument != Some(instrument) {
            self.instrument = Some(instrument);
            self.map = assigned_map(editor, Some(instrument));
            self.draft = None;
            self.error = None;
        }
        self.sync(editor);
    }
    pub fn selected_instrument(&self) -> Option<InstrumentId> {
        self.instrument
    }
    pub fn take_region_request(&mut self) -> Option<(RegionId, InstrumentId)> {
        self.inspect.take()
    }
    fn reload(&mut self, editor: &Editor) {
        self.notice.clear();
        let selected = self
            .draft
            .as_ref()
            .filter(|draft| Some(draft.id) == self.map)
            .and_then(|draft| draft.selected);
        self.draft = self.map.and_then(|id| Draft::new(editor, id).ok());
        if let Some(draft) = &mut self.draft {
            if selected.is_some_and(|id| draft.zones.contains_key(&id)) {
                draft.selected = selected;
            }
        }
    }
    fn sync(&mut self, editor: &Editor) {
        if self
            .instrument
            .is_some_and(|id| !editor.song.instruments.contains_key(&id))
        {
            self.instrument = None;
            self.draft = None;
        }
        if self
            .map
            .is_some_and(|id| !editor.song.multisamples.contains_key(&id))
        {
            self.map = None;
            self.draft = None;
            self.error = Some(
                "Selected map was removed; choose a current map. Nothing was recreated.".into(),
            );
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.current(editor))
        {
            self.reload(editor);
            self.error = Some(
                "Document changed; mapping draft reloaded. Review the current map before applying."
                    .into(),
            );
        } else if self.draft.is_none() {
            self.reload(editor);
        }
    }
    fn create(&mut self, editor: &mut Editor) -> Result<(), String> {
        let (instrument, map) = editor.create_multisample_instrument(self.new_name.clone())?;
        self.instrument = Some(instrument);
        self.map = Some(map);
        self.reload(editor);
        self.notice = format!(
            "Created I{} + map M{} without changing existing notes or mappings. Add regions before playback.",
            instrument.0, map.0
        );
        Ok(())
    }
    fn apply(&mut self, editor: &mut Editor, copy: bool) -> Result<(), String> {
        let draft = self.draft.as_ref().ok_or("Choose a map first")?;
        if !draft.current(editor) {
            return Err("Mapping draft is stale; reopen the current map".into());
        }
        let value = draft.parse()?;
        let id = if copy {
            next_map(&editor.song)?
        } else {
            draft.id
        };
        let mut candidate = editor.song.clone();
        candidate.upsert_multisample(id, value)?;
        editor.apply_document_edit_if_current(draft.revision, &draft.base, candidate)?;
        self.map = Some(id);
        self.reload(editor);
        self.notice = if copy {
            format!(
                "Independent map M{} created from this draft; instrument assignment is unchanged. Regions remain shared.",
                id.0
            )
        } else {
            format!(
                "Map M{} applied atomically to all assigned instruments; ordinary note cells unchanged.",
                id.0
            )
        };
        Ok(())
    }
    fn assign(&mut self, editor: &mut Editor, detach: bool) -> Result<(), String> {
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.current(editor) || draft.changed(editor))
        {
            return Err(
                "Apply or Reload the mapping draft before changing an instrument assignment".into(),
            );
        }
        let instrument = self.instrument.ok_or("Choose a target instrument")?;
        let map = if detach {
            None
        } else {
            Some(self.map.ok_or("Choose a map")?)
        };
        editor.assign_multisample(instrument, map)?;
        self.reload(editor);
        self.notice = if detach {
            "Detached melodic map only; no other instrument kind or map definition was removed."
                .into()
        } else {
            "Map assigned explicitly; previous legacy sample/region or kit assignment replaced (Undo restores it).".into()
        };
        Ok(())
    }
    fn remove(&mut self, editor: &mut Editor) -> Result<(), String> {
        let id = self.map.ok_or("Choose a map")?;
        editor.remove_multisample(id)?;
        self.map = None;
        self.draft = None;
        self.notice = "Map definition removed; regions and source files retained.".into();
        Ok(())
    }
    fn result(&mut self, result: Result<(), String>) {
        self.error = result.err();
        if self.error.is_some() {
            self.notice.clear();
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if !self.open {
            return;
        }
        self.sync(editor);
        ui.ctx()
            .memory_mut(|memory| memory.surrender_focus(grid_id()));
        let enabled = ui.is_enabled();
        let writable = enabled && editor.edit_mode;
        let mut open = true;
        let mut close = false;
        egui::Window::new("Melodic key zones")
            .id(egui::Id::new("melodic-keyzones"))
            .default_pos(egui::pos2(180.0, 100.0)).default_width(680.0).min_width(560.0)
            .resizable(false).collapsible(false).open(&mut open)
            .show(ui.ctx(), |ui| {
                if !enabled { ui.disable(); }
                ui.label(RichText::new("MELODIC MAP · ONE SOURCE PER NOTE · NO AUDITION").small().color(AQUA));
                egui::ScrollArea::vertical().id_salt("keyzone-body").auto_shrink([false,false])
                    .max_height((ui.ctx().content_rect().height() - 340.0).max(170.0)).show(ui, |ui| {
                    self.controls(ui, editor, writable);
                    if let Some(draft) = &mut self.draft { zone_editor(ui, editor, draft, writable); }
                    ui.separator();
                    ui.label(RichText::new("Inclusive note × velocity bins 0..127. A hit selects one region; overlapping rectangles reject. Gaps/empty maps may be saved, but an uncovered played note fails evaluation/export—no fallback or silence substitute.").small().color(WARNING));
                    ui.collapsing("Playback / shared mapping details", |ui| {
                        ui.label("MIDI notes are audible pitch, not drum selectors. Velocity bin is round(127 × effective ON velocity), before later volume changes. Region root is copied only when explicitly selected; the stored zone root overrides the region reference.");
                        ui.label("Native source rate, region envelope and loop remain authoritative. Later volume or glide does not choose a different zone; phrase transposition occurs before zone selection. No stacking, random choice or round robin here.");
                        ui.label("Apply edits this shared map for every assigned instrument. Fresh copy creates independent mapping metadata using the same shared regions. Mapping validation does not decode source audio or guarantee whole-song renderability.");
                    });
                });
                ui.separator();
                if let Some(draft) = &self.draft {
                    let users = editor.song.instruments.iter().filter(|(_, instrument)| matches!(instrument.kind, InstrumentKind::Multisample { map } if map == draft.id)).map(|(id, _)| id.0).collect::<Vec<_>>();
                    ui.label(RichText::new(format!("DRAFT M{} · {} zones · {} assigned instruments · revision {}", draft.id.0, draft.zones.len(), users.len(), draft.revision)).small().color(AQUA)).on_hover_ui(|ui| assigned_ids(ui, &users));
                }
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
                if !editor.edit_mode { ui.label(RichText::new("READ MODE · inspect only; enable Edit mode to change mappings").small().color(WARNING)); }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(writable && self.draft.is_some(), egui::Button::new("Apply map")).clicked() { let result = self.apply(editor, false); self.result(result); }
                    if ui.add_enabled(writable && self.draft.is_some(), egui::Button::new("Fresh copy")).on_hover_text("Create a new independent map from this entire draft. Keep the original map and current instrument assignment.").clicked() { let result = self.apply(editor, true); self.result(result); }
                    if ui.add_enabled(writable && self.map.is_some(), egui::Button::new("Remove map")).on_hover_text("Refuses while assigned; detach its instruments first. Regions are never deleted.").clicked() { let result = self.remove(editor); self.result(result); }
                    if ui.button("Close").clicked() { close = true; }
                });
                if !self.notice.is_empty() { ui.add(egui::Label::new(RichText::new(&self.notice).small().color(MUTED)).truncate()).on_hover_text(&self.notice); }
            });
        self.open = self.open && open && !close;
    }
    fn controls(&mut self, ui: &mut egui::Ui, editor: &mut Editor, writable: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Instrument");
            let before = self.instrument;
            egui::ComboBox::from_id_salt("keyzone-instrument")
                .width(210.0)
                .selected_text(
                    self.instrument
                        .map_or("Choose target".into(), |id| format!("I{}", id.0)),
                )
                .show_ui(ui, |ui| {
                    for (id, instrument) in &editor.song.instruments {
                        ui.selectable_value(
                            &mut self.instrument,
                            Some(*id),
                            format!("I{} · {}", id.0, instrument.name),
                        );
                    }
                });
            if before != self.instrument {
                self.map = assigned_map(editor, self.instrument);
                self.reload(editor);
                self.error = None;
                self.notice.clear();
            }
            ui.label("Map");
            let before = self.map;
            egui::ComboBox::from_id_salt("keyzone-map")
                .width(210.0)
                .selected_text(
                    self.map
                        .map_or("Choose map".into(), |id| format!("M{}", id.0)),
                )
                .show_ui(ui, |ui| {
                    for (id, map) in &editor.song.multisamples {
                        ui.selectable_value(
                            &mut self.map,
                            Some(*id),
                            format!("M{} · {}", id.0, map.name),
                        );
                    }
                });
            if before != self.map {
                self.reload(editor);
                self.error = None;
                self.notice.clear();
            }
        });
        let assignment = self
            .instrument
            .and_then(|id| editor.song.instruments.get(&id))
            .map_or(
                "No target instrument".into(),
                |instrument| match instrument.kind {
                    InstrumentKind::Pitched => "Current kind: pitched sample/region".into(),
                    InstrumentKind::DrumKit { kit } => format!("Current kind: drum kit {}", kit.0),
                    InstrumentKind::Multisample { map } => {
                        format!("Assigned melodic map M{}", map.0)
                    }
                },
            );
        ui.label(RichText::new(assignment).small().color(MUTED));
        let clean = self
            .draft
            .as_ref()
            .is_none_or(|draft| !draft.changed(editor));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    writable && clean && self.instrument.is_some() && self.map.is_some(),
                    egui::Button::new("Assign map"),
                )
                .on_disabled_hover_text(
                    "Enable Edit mode and Apply or Reload pending mapping changes first",
                )
                .clicked()
            {
                let result = self.assign(editor, false);
                self.result(result);
            }
            if ui
                .add_enabled(
                    writable && clean && assigned_map(editor, self.instrument).is_some(),
                    egui::Button::new("Detach map"),
                )
                .clicked()
            {
                let result = self.assign(editor, true);
                self.result(result);
            }
            if ui
                .add_enabled(self.draft.is_some(), egui::Button::new("Reload saved map"))
                .clicked()
            {
                self.reload(editor);
                self.error = None;
                self.notice.clear();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("New name"); ui.add(egui::TextEdit::singleline(&mut self.new_name).id(egui::Id::new("keyzone-new-name")).desired_width(205.0));
            if ui.add_enabled(writable, egui::Button::new("New map + instrument")).on_hover_text("Allocate fresh IDs atomically; preserve all existing notes, mappings, sources and cursor.").clicked() { let result = self.create(editor); self.result(result); }
        });
        if let Some(draft) = &self.draft {
            let selected = draft
                .selected
                .and_then(|id| draft.zones.get(&id))
                .and_then(|zone| zone.region);
            if ui.add_enabled(selected.is_some() && self.instrument.is_some(), egui::Button::new("Inspect selected region")).on_hover_text("Open the existing Samples inspector with Map on Apply OFF; does not assign this instrument or save the mapping draft.").clicked() {
                self.inspect = selected.zip(self.instrument);
                self.open = false;
            }
        } else {
            ui.label(RichText::new("Choose a map or create a fresh map + instrument. Existing occupied instruments are not repurposed automatically.").small().color(MUTED));
        }
    }
}
fn assigned_ids(ui: &mut egui::Ui, users: &[u64]) {
    ui.set_max_width(245.0);
    ui.label(format!("{} assigned instrument IDs", users.len()));
    let height = ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt("keyzone-assigned-ids")
        .max_height(170.0)
        .show_rows(ui, height, users.len(), |ui, range| {
            for index in range {
                ui.label(RichText::new(format!("I{}", users[index])).monospace());
            }
        });
}

fn assigned_map(editor: &Editor, instrument: Option<InstrumentId>) -> Option<MultisampleId> {
    match &editor.song.instruments.get(&instrument?)?.kind {
        InstrumentKind::Multisample { map } => Some(*map),
        _ => None,
    }
}
fn zone_editor(ui: &mut egui::Ui, editor: &Editor, draft: &mut Draft, writable: bool) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Map name");
        ui.add_enabled(
            writable,
            egui::TextEdit::singleline(&mut draft.name)
                .id(egui::Id::new("keyzone-map-name"))
                .desired_width(280.0),
        );
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                writable && draft.zones.len() < 128 && draft.next_zone.is_some(),
                egui::Button::new("Add zone"),
            )
            .clicked()
        {
            let _ = draft.add_zone();
        }
        if ui
            .add_enabled(
                writable && draft.selected.is_some(),
                egui::Button::new("Remove draft zone"),
            )
            .clicked()
        {
            if let Some(id) = draft.selected.take() {
                draft.zones.remove(&id);
                draft.selected = draft.zones.keys().next().copied();
            }
        }
        ui.label(
            RichText::new("Draft only · Apply saves all boundaries together")
                .small()
                .color(MUTED),
        );
    });
    egui::ScrollArea::vertical()
        .id_salt("keyzone-list")
        .max_height(118.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (id, zone) in &draft.zones {
                let text = format!(
                    "Z{:03}  N {}..{}  V {}..{}  {}",
                    id.0,
                    zone.note_min,
                    zone.note_max,
                    zone.velocity_min,
                    zone.velocity_max,
                    zone.region
                        .map_or("NO REGION".into(), |id| format!("R{}", id.0))
                );
                ui.selectable_value(
                    &mut draft.selected,
                    Some(*id),
                    RichText::new(text).monospace(),
                );
            }
        });
    let Some(id) = draft.selected else {
        return;
    };
    let Some(zone) = draft.zones.get_mut(&id) else {
        return;
    };
    ui.label(
        RichText::new(format!("Zone Z{} · stable ID", id.0))
            .small()
            .color(AQUA),
    );
    ui.add_enabled_ui(writable, |ui| {
        ui.horizontal(|ui| { ui.label("Zone name"); ui.add(egui::TextEdit::singleline(&mut zone.name).id(egui::Id::new("keyzone-zone-name")).desired_width(250.0)); });
        ui.horizontal_wrapped(|ui| { number(ui,"Note min",&mut zone.note_min); number(ui,"Note max",&mut zone.note_max); number(ui,"Velocity min",&mut zone.velocity_min); number(ui,"Velocity max",&mut zone.velocity_max); });
        ui.horizontal_wrapped(|ui| {
            ui.label("Region"); let before = zone.region;
            egui::ComboBox::from_id_salt("keyzone-region").width(260.0).selected_text(zone.region.map_or("Choose region".into(), |id| format!("R{}", id.0))).show_ui(ui, |ui| {
                for (id, region) in &editor.song.regions { ui.selectable_value(&mut zone.region, Some(*id), format!("R{} · {} · source[{}, {})", id.0, region.name, region.start_frame, region.end_frame)); }
            });
            if before != zone.region { if let Some(region) = zone.region.and_then(|id| editor.song.regions.get(&id)) { zone.root = region.root_note.to_string(); } }
            number(ui,"Native root MIDI",&mut zone.root);
            if let Ok(root) = midi(&zone.root) { ui.label(RichText::new(note_label(crate::song::Note::Note(root))).small().color(MUTED)); }
        });
        ui.label(RichText::new("Native root is the reference pitch at original sample speed, not a selector. Region changes explicitly copy its current root; edit this override as needed.").small().color(MUTED));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_api_exposes_zone_draft_and_rejects_unknown_action_atomically() {
        let (mut editor, ui) = fixture();
        let mut ui = ui;
        let before = editor.song.clone();
        let state = ui.agent_state(&editor);
        assert!(state["actions"]["patch_zone"].is_object());
        assert!(
            ui.agent_action(&mut editor, None, json!({"action":"not_an_action"}))
                .is_err()
        );
        assert_eq!(editor.song, before);
    }
    fn selection(e: &Editor) -> (usize, crate::song::TrackId, u32, u16, crate::editor::Field) {
        (e.occurrence, e.track, e.row, e.column, e.field)
    }
    use crate::song::{SampleAsset, SampleId, SampleRegion};

    fn fixture() -> (Editor, MultisampleUi) {
        let mut editor = Editor {
            edit_mode: true,
            ..Editor::default()
        };
        let mut song = editor.song.clone();
        song.register_sample(SampleId(1), SampleAsset {
            path: "unit-keyzones-no-io.wav".into(),
            sha256: "0".repeat(64),
            sample_rate: 48_000,
            channels: 1,
            frames: 9600,
        })
        .unwrap();
        for (id, start, root) in [(1, 0, 60), (2, 4800, 72)] {
            song.upsert_region(RegionId(id), SampleRegion {
                name: format!("Region {id}"),
                sample: SampleId(1),
                start_frame: start,
                end_frame: start + 4800,
                root_note: root,
                amp_envelope: None,
                forward_loop: None,
            })
            .unwrap();
        }
        editor.apply_document_edit(song).unwrap();
        let (instrument, map) = editor
            .create_multisample_instrument("Melodic map".into())
            .unwrap();
        editor
            .set_keyzones(
                map,
                BTreeMap::from([
                    (KeyZoneId(7), KeyZone {
                        name: "Low".into(),
                        note_min: 0,
                        note_max: 63,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(1),
                        root_note: 60,
                    }),
                    (KeyZoneId(19), KeyZone {
                        name: "High".into(),
                        note_min: 64,
                        note_max: 127,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(2),
                        root_note: 72,
                    }),
                ]),
            )
            .unwrap();
        let mut ui = MultisampleUi::default();
        ui.open_for(&editor, instrument);
        (editor, ui)
    }
    #[test]
    fn exact_keyzone_integers_do_not_clamp_or_accept_fractional_inputs() {
        for input in [
            "-1",
            "128",
            "256",
            "NaN",
            "1.5",
            "1e1",
            "+1",
            "",
            "18446744073709551616",
        ] {
            assert!(midi(input).is_err(), "{input}");
        }
        assert_eq!(midi("0").unwrap(), 0);
        assert_eq!(midi("127").unwrap(), 127);
        assert_eq!(midi(" 060 ").unwrap(), 60);
    }
    #[test]
    fn whole_map_draft_updates_adjacent_boundaries_atomically_and_preserves_failures() {
        let (mut editor, mut ui) = fixture();
        let before = editor.song.clone();
        let cursor = selection(&editor);
        let revision = editor.revision();
        ui.draft
            .as_mut()
            .unwrap()
            .zones
            .get_mut(&KeyZoneId(7))
            .unwrap()
            .note_max = "65".into();
        assert!(
            ui.apply(&mut editor, false)
                .unwrap_err()
                .contains("overlap")
        );
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
        assert_eq!(
            ui.draft.as_ref().unwrap().zones[&KeyZoneId(7)].note_max,
            "65"
        );
        ui.draft
            .as_mut()
            .unwrap()
            .zones
            .get_mut(&KeyZoneId(19))
            .unwrap()
            .note_min = "66".into();
        ui.apply(&mut editor, false).unwrap();
        assert_eq!(
            editor.song.multisamples[&MultisampleId(1)].zones[&KeyZoneId(19)].note_min,
            66
        );
        assert_eq!(selection(&editor), cursor);
        let applied = editor.song.clone();
        let revision = editor.revision();
        ui.apply(&mut editor, false).unwrap();
        assert_eq!(
            editor.revision(),
            revision,
            "Unchanged Apply is exact no-op"
        );
        editor.undo();
        assert_eq!(editor.song, before);
        editor.redo();
        assert_eq!(editor.song, applied);
    }
    #[test]
    fn fresh_copy_does_not_assign_or_modify_original_and_pending_changes_block_assignment() {
        let (mut editor, mut ui) = fixture();
        let before = editor.song.clone();
        let instrument = ui.instrument.unwrap();
        ui.draft.as_mut().unwrap().name = "Independent copy".into();
        assert!(ui.assign(&mut editor, true).is_err());
        assert_eq!(editor.song, before);
        ui.apply(&mut editor, true).unwrap();
        assert_eq!(ui.map, Some(MultisampleId(2)));
        assert_eq!(
            editor.song.multisamples[&MultisampleId(1)],
            before.multisamples[&MultisampleId(1)]
        );
        assert_eq!(editor.song.instruments, before.instruments);
        assert_eq!(
            editor.song.multisamples[&MultisampleId(2)].zones,
            before.multisamples[&MultisampleId(1)].zones
        );
        ui.assign(&mut editor, false).unwrap();
        assert!(matches!(
            editor.song.instruments[&instrument].kind,
            InstrumentKind::Multisample {
                map: MultisampleId(2)
            }
        ));
        editor.undo();
        assert_eq!(editor.song.instruments, before.instruments);
        editor.undo();
        assert_eq!(editor.song, before);
    }
    #[test]
    fn fresh_map_instrument_preserves_occupied_song_and_stale_ids_never_recreate() {
        let mut editor = Editor {
            edit_mode: true,
            ..Editor::default()
        };
        let before = editor.song.clone();
        let cursor = selection(&editor);
        let mut ui = MultisampleUi::default();
        ui.create(&mut editor).unwrap();
        assert_eq!(editor.song.patterns, before.patterns);
        assert_eq!(editor.song.order, before.order);
        assert_eq!(editor.song.samples, before.samples);
        assert_eq!(editor.song.regions, before.regions);
        for (id, value) in &before.instruments {
            assert_eq!(editor.song.instruments[id], *value);
        }
        assert_eq!(selection(&editor), cursor);
        editor.undo();
        assert_eq!(editor.song, before);
        assert!(ui.apply(&mut editor, false).is_err());
        ui.sync(&editor);
        assert!(ui.map.is_none());
        assert!(ui.draft.is_none());
        assert_eq!(editor.song, before);
    }
    #[test]
    fn map_draft_rejects_aba_direct_base_mutations_and_read_mode() {
        let (mut editor, mut ui) = fixture();
        let before = editor.song.clone();
        ui.notice = "Previous action must not describe an undone map".into();
        editor.set_timing(140.0, 4).unwrap();
        editor.undo();
        assert_eq!(editor.song, before);
        assert!(ui.apply(&mut editor, false).is_err());
        ui.sync(&editor);
        assert!(ui.notice.is_empty());
        let revision = editor.revision();
        editor.song.title = "Direct replacement".into();
        assert_eq!(editor.revision(), revision);
        assert!(ui.apply(&mut editor, false).is_err());
        ui.sync(&editor);
        editor.edit_mode = false;
        assert!(ui.apply(&mut editor, false).is_err());
        assert_eq!(editor.song.title, "Direct replacement");
    }
    fn frame(
        ctx: &egui::Context,
        state: &mut MultisampleUi,
        editor: &mut Editor,
        width: f32,
        height: f32,
        enabled: bool,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, height),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| state.show(ui, editor));
                });
            },
        )
    }
    fn label(output: &egui::FullOutput, text: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(t) if t.galley.job.text == text => {
                    let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
                    shape.clip_rect.contains_rect(rect).then_some(rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Fully visible label missing: {text}"))
    }
    fn click(
        ctx: &egui::Context,
        state: &mut MultisampleUi,
        editor: &mut Editor,
        width: f32,
        height: f32,
        enabled: bool,
        point: egui::Pos2,
    ) {
        for pressed in [true, false] {
            frame(ctx, state, editor, width, height, enabled, vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
    }
    #[test]
    fn actual_popup_apply_is_guarded_and_error_footer_is_reachable_at_both_sizes() {
        for (width, height) in [(900.0, 620.0), (1180.0, 780.0)] {
            let (mut editor, mut ui) = fixture();
            let before = editor.song.clone();
            let ctx = egui::Context::default();
            crate::tracker_ui::install_theme(&ctx);
            ui.draft
                .as_mut()
                .unwrap()
                .zones
                .get_mut(&KeyZoneId(19))
                .unwrap()
                .note_min = "63".into();
            let mut output = frame(&ctx, &mut ui, &mut editor, width, height, true, vec![]);
            for _ in 0..5 {
                output = frame(&ctx, &mut ui, &mut editor, width, height, true, vec![]);
            }
            let point = label(&output, "Apply map").center();
            click(&ctx, &mut ui, &mut editor, width, height, false, point);
            assert_eq!(editor.song, before);
            assert!(ui.error.is_none());
            editor.edit_mode = false;
            click(&ctx, &mut ui, &mut editor, width, height, true, point);
            assert_eq!(editor.song, before);
            editor.edit_mode = true;
            for _ in 0..5 {
                output = frame(&ctx, &mut ui, &mut editor, width, height, true, vec![]);
            }
            let point = label(&output, "Apply map").center();
            click(&ctx, &mut ui, &mut editor, width, height, true, point);
            assert_eq!(editor.song, before);
            assert!(ui.error.as_deref().unwrap().contains("overlap"));
            for _ in 0..5 {
                output = frame(&ctx, &mut ui, &mut editor, width, height, true, vec![]);
            }
            for text in [
                "Apply map",
                "Fresh copy",
                "Remove map",
                "Close",
                ui.error.as_deref().unwrap(),
            ] {
                let rect = label(&output, text);
                assert!(
                    rect.top() >= 0.0 && rect.bottom() <= height && rect.right() <= width,
                    "{text}: {rect:?}"
                );
            }
        }
    }
    #[test]
    fn maximum_zone_count_and_large_assignment_list_keep_fixed_actions_bounded() {
        for (width, height) in [(900.0, 620.0), (1180.0, 780.0)] {
            let (mut editor, mut ui) = fixture();
            let mut song = editor.song.clone();
            let zones = (0..128u8)
                .map(|note| {
                    (KeyZoneId(u64::from(note) + 1), KeyZone {
                        name: format!("Zone {note}"),
                        note_min: note,
                        note_max: note,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(1),
                        root_note: 60,
                    })
                })
                .collect();
            song.set_keyzones(MultisampleId(1), zones).unwrap();
            let prototype = song.instruments[&ui.instrument.unwrap()].clone();
            for id in 100..612 {
                song.instruments.insert(InstrumentId(id), prototype.clone());
            }
            editor.apply_document_edit(song).unwrap();
            ui.reload(&editor);
            let ctx = egui::Context::default();
            crate::tracker_ui::install_theme(&ctx);
            let mut output = frame(&ctx, &mut ui, &mut editor, width, height, true, vec![]);
            for _ in 0..5 {
                output = frame(&ctx, &mut ui, &mut editor, width, height, true, vec![]);
            }
            for text in ["Apply map", "Fresh copy", "Remove map", "Close"] {
                let rect = label(&output, text);
                assert!(rect.bottom() <= height && rect.right() <= width);
            }
            let users = (100..612).collect::<Vec<_>>();
            let list = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| assigned_ids(ui, &users));
                },
            );
            let rows=list.shapes.iter().filter(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text.starts_with('I'))).count();
            assert!(
                rows < 20,
                "Exact IDs must be virtualized, not one unbounded string/list: {rows}"
            );
            assert_eq!(editor.song.multisamples[&MultisampleId(1)].zones.len(), 128);
        }
    }
}
