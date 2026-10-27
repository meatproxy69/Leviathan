//! Optional drum-pad authoring surface. Pads select/insert ordinary tracker
//! notes; explicit audition requests use the shared desktop transport, not a pad sequencer.
use crate::{
    editor::{Editor, note_label},
    song::{
        Cell, DrumKitId, DrumKitInstrument, DrumPad, InstrumentId, InstrumentKind, Note, PadId,
        PadLayer, PadLayerId, PadMode, PadNewNote, RegionId, Song, TrackId, VelocityZone,
    },
    tracker_ui::{AQUA, grid_id},
};
use eframe::egui::{self, Color32, RichText, Stroke};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const MUTED: Color32 = Color32::from_rgb(133, 157, 167);
const WARNING: Color32 = Color32::from_rgb(229, 185, 112);

#[derive(Clone)]
struct MappingDraft {
    revision: u64,
    kit: DrumKitId,
    id: PadId,
    value: DrumPad,
    sources: SourcesDraft,
    sources_tab: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct LayerDraft {
    id: PadLayerId,
    region: Option<RegionId>,
}
#[derive(Clone, Debug, PartialEq)]
struct ZoneDraft {
    min: String,
    max: String,
    alternatives: Vec<LayerDraft>,
}
#[derive(Clone, Debug, PartialEq)]
struct SourcesDraft {
    layered: bool,
    zones: Vec<ZoneDraft>,
    next_layer: Option<u64>,
}
impl SourcesDraft {
    fn from_pad(pad: &DrumPad) -> Self {
        Self {
            layered: !pad.velocity_zones.is_empty(),
            zones: pad
                .velocity_zones
                .iter()
                .map(|zone| ZoneDraft {
                    min: zone.min_velocity.to_string(),
                    max: zone.max_velocity.to_string(),
                    alternatives: zone
                        .alternatives
                        .iter()
                        .map(|layer| LayerDraft {
                            id: layer.id,
                            region: Some(layer.region),
                        })
                        .collect(),
                })
                .collect(),
            next_layer: pad
                .velocity_zones
                .iter()
                .flat_map(|zone| &zone.alternatives)
                .map(|layer| layer.id.0)
                .max()
                .unwrap_or(0)
                .checked_add(1),
        }
    }
    fn allocate(&mut self) -> Result<PadLayerId, String> {
        let id = self.next_layer.ok_or("No free stable layer ID")?;
        self.next_layer = id.checked_add(1);
        Ok(PadLayerId(id))
    }
    fn enable_zones(&mut self, region: Option<RegionId>) -> Result<(), String> {
        if self.zones.is_empty() {
            let id = self.allocate()?;
            self.zones.push(ZoneDraft {
                min: "0".into(),
                max: "127".into(),
                alternatives: vec![LayerDraft { id, region }],
            });
        }
        self.layered = true;
        Ok(())
    }
    fn total(&self) -> usize {
        self.zones.iter().map(|zone| zone.alternatives.len()).sum()
    }
    fn split(&mut self, index: usize) -> Result<(), String> {
        let zone = self.zones.get(index).ok_or("Zone no longer exists")?;
        if self.zones.len() >= 8 || self.total() + zone.alternatives.len() > 32 {
            return Err("At most 8 zones and 32 alternatives per pad".into());
        }
        let min = velocity_bin(&zone.min)?;
        let max = velocity_bin(&zone.max)?;
        if min >= max {
            return Err("Split needs at least two inclusive velocity bins".into());
        }
        let midpoint = min + (max - min) / 2;
        let regions: Vec<_> = zone.alternatives.iter().map(|layer| layer.region).collect();
        // An exhausted ID namespace must not leave half a split in the draft.
        let mut next = self.clone();
        let alternatives = regions
            .into_iter()
            .map(|region| {
                Ok(LayerDraft {
                    id: next.allocate()?,
                    region,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        next.zones[index].max = midpoint.to_string();
        next.zones.insert(index + 1, ZoneDraft {
            min: (midpoint + 1).to_string(),
            max: max.to_string(),
            alternatives,
        });
        *self = next;
        Ok(())
    }
    fn add_alternative(&mut self, index: usize) -> Result<(), String> {
        let zone = self.zones.get(index).ok_or("Zone no longer exists")?;
        if zone.alternatives.len() >= 8 || self.total() >= 32 {
            return Err("At most 8 alternatives per zone and 32 per pad".into());
        }
        let id = self.allocate()?;
        self.zones[index]
            .alternatives
            .push(LayerDraft { id, region: None });
        Ok(())
    }
    fn parse(
        &self,
        single: Option<RegionId>,
    ) -> Result<(Option<RegionId>, Vec<VelocityZone>), String> {
        if !self.layered {
            return Ok((single, vec![]));
        }
        if self.zones.is_empty() {
            return Err("Zones mode needs a complete 0..127 partition; use Single region / No region to unmap explicitly".into());
        }
        let zones = self
            .zones
            .iter()
            .enumerate()
            .map(|(index, zone)| {
                Ok(VelocityZone {
                    min_velocity: velocity_bin(&zone.min)
                        .map_err(|error| format!("Zone {} minimum: {error}", index + 1))?,
                    max_velocity: velocity_bin(&zone.max)
                        .map_err(|error| format!("Zone {} maximum: {error}", index + 1))?,
                    alternatives: zone
                        .alternatives
                        .iter()
                        .map(|layer| {
                            Ok(PadLayer {
                                id: layer.id,
                                region: layer.region.ok_or_else(|| {
                                    format!("Choose a region for layer {}", layer.id.0)
                                })?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((None, zones))
    }
}
fn velocity_bin(text: &str) -> Result<u8, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("enter a whole bin 0..127; no clamping".into());
    }
    text.parse::<u8>()
        .ok()
        .filter(|value| *value <= 127)
        .ok_or_else(|| "bin must be 0..127; no clamping".into())
}
fn source_summary(pad: &DrumPad) -> String {
    if !pad.velocity_zones.is_empty() {
        format!(
            "{} velocity zones · {} RR alternatives",
            pad.velocity_zones.len(),
            pad.velocity_zones
                .iter()
                .map(|zone| zone.alternatives.len())
                .sum::<usize>()
        )
    } else {
        pad.region.map_or("No source · unmapped".into(), |id| {
            format!("Single region {}", id.0)
        })
    }
}

#[derive(Default)]
pub(crate) struct PadUi {
    pub(crate) audition: crate::audition_ui::PanelState,
    audition_action: Option<crate::audition_ui::Action>,
    pub open: bool,
    instrument: Option<InstrumentId>,
    selected: Option<PadId>,
    resolved_kit: Option<DrumKitId>,
    selected_slot: u8,
    insert_mode: bool,
    kit_choice: Option<DrumKitId>,
    kit_name: String,
    draft: Option<MappingDraft>,
    last_cell: Option<(usize, TrackId, u16, u32, Cell)>,
    inspect_region: Option<(RegionId, InstrumentId)>,
    inspect_layer: Option<PadLayerId>,
    inspect_context: Option<(InstrumentId, DrumKitId, PadId)>,
    error: Option<String>,
}

fn next_id(values: impl Iterator<Item = u64>) -> Result<u64, String> {
    values
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("No free stable ID".into())
}

impl PadUi {
    pub(crate) fn agent_state(&self, editor: &Editor) -> Value {
        let kit = self
            .assigned_kit(editor)
            .and_then(|id| editor.song.drum_kits.get(&id));
        let pads = kit.map(|k| k.pads.iter().map(|(id,p)| json!({"id":id.0,"slot":p.slot,"name":p.name,"selector_midi":p.note,"region":p.region.map(|r|r.0),"root_midi":p.root_note,"tune_semitones":p.tune_semitones,"gain":p.gain,"pan":p.pan,"velocity_zones":p.velocity_zones.iter().map(|z|json!({"min":z.min_velocity,"max":z.max_velocity,"alternatives":z.alternatives.iter().map(|a|json!({"id":a.id.0,"region":a.region.0})).collect::<Vec<_>>() })).collect::<Vec<_>>()})).collect::<Vec<_>>());
        let instruments = editor
            .song
            .instruments
            .iter()
            .map(|(id, i)| json!({"id":id.0,"name":i.name,"kind":format!("{:?}",i.kind)}))
            .collect::<Vec<_>>();
        let regions = editor.song.regions.iter().map(|(id,r)|json!({"id":id.0,"name":r.name,"start_frame":r.start_frame,"end_frame":r.end_frame})).collect::<Vec<_>>();
        let actions = json!({"set_open":{"open":"boolean"},"select_instrument":{"instrument_id":"integer|null"},"select_slot":{"slot":"0..15"},"select_pad":{"pad_id":"integer|null"},"set_insert_mode":{"enabled":"boolean"},"set_kit_name":{"name":"string"},"choose_kit":{"kit_id":"integer|null"},"create_kit_instrument":{},"create_kit":{},"assign_kit":{"kit_id":"integer|null"},"unassign_kit":{},"remove_kit":{"kit_id":"integer"},"edit_mapping":{},"patch_mapping":{"fields":["name","slot","selector_midi","region","root_midi","tune_semitones","gain","pan","mode","new_note","choke_group","sources_tab"]},"set_source_mode":{"mode":"single|velocity_zones"},"patch_zone":{"index":"integer","fields":["min","max"]},"source_edit":{"op":"split|remove_zone|add_alternative|move_alternative|remove_alternative|set_layer_region","zone":"integer","index":"integer","from":"integer","to":"integer","region_id":"integer|null"},"apply_mapping":{},"cancel_mapping":{},"remove_pad":{},"insert_note":{},"audition_start":{},"audition_stop":{},"inspect_region":{"region_id":"integer"},"inspect_layer":{"layer_id":"integer"}});
        let draft = self.draft.as_ref().map(|d| json!({
            "kit": d.kit.0, "pad": d.id.0, "revision": d.revision,
            "settings": {"name":d.value.name,"slot":d.value.slot,"selector_midi":d.value.note,
                "region":d.value.region.map(|v|v.0),"root_midi":d.value.root_note,
                "tune_semitones":d.value.tune_semitones,"gain":d.value.gain,"pan":d.value.pan,
                "mode":format!("{:?}",d.value.mode),"new_note":format!("{:?}",d.value.new_note),"choke_group":d.value.choke_group},
            "sources_tab":d.sources_tab,"source_mode":if d.sources.layered {"velocity_zones"} else {"single"},
            "zones":d.sources.zones.iter().map(|z|json!({"min":z.min,"max":z.max,"alternatives":z.alternatives.iter().map(|l|json!({"id":l.id.0,"region":l.region.map(|v|v.0)})).collect::<Vec<_>>() })).collect::<Vec<_>>()
        }));
        json!({"component":"pads","open":self.open,"edit_mode":editor.edit_mode,
            "selected_instrument":self.instrument.map(|v|v.0),"assigned_kit":self.assigned_kit(editor).map(|v|v.0),
            "selected_pad":self.selected.map(|v|v.0),"selected_slot":self.selected_slot,
            "insert_mode":self.insert_mode,"kit_choice":self.kit_choice.map(|v|v.0),"kit_name":self.kit_name,
            "pads":pads,
            "instruments":instruments,
            "regions":regions,
            "draft":draft,"error":self.error,
            "actions":actions,
            "policy":{"requires_edit_mode_for_mutation":true,"apply_is_single_undo":true,"selection_and_preview_do_not_edit_song":true}})
    }
    pub(crate) fn agent_action(
        &mut self,
        editor: &mut Editor,
        _base: Option<&Path>,
        action: Value,
    ) -> Result<Value, String> {
        let obj = action.as_object().ok_or("action must be an object")?;
        let name = obj
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action name required")?;
        let action_fields: &[&str] = match name {
            "set_open" => &["action", "open"],
            "select_instrument" => &["action", "instrument_id"],
            "select_slot" => &["action", "slot"],
            "select_pad" => &["action", "pad_id"],
            "set_insert_mode" => &["action", "enabled"],
            "set_kit_name" => &["action", "name"],
            "choose_kit" | "assign_kit" => &["action", "kit_id"],
            "remove_kit" => &["action", "kit_id"],
            "patch_mapping" => &["action", "fields"],
            "set_source_mode" => &["action", "mode"],
            "patch_zone" => &["action", "index", "fields"],
            "source_edit" => &["action", "op", "zone", "index", "from", "to", "region_id"],
            "inspect_region" => &["action", "region_id"],
            "inspect_layer" => &["action", "layer_id"],
            "create_kit_instrument"
            | "create_kit"
            | "unassign_kit"
            | "edit_mapping"
            | "apply_mapping"
            | "cancel_mapping"
            | "remove_pad"
            | "insert_note"
            | "audition_start"
            | "audition_stop" => &["action"],
            _ => return Err(format!("unknown pads action: {name}")),
        };
        if let Some(k) = obj.keys().find(|k| !action_fields.contains(&k.as_str())) {
            return Err(format!("field {k} is not accepted for {name}"));
        }
        let allowed = [
            "action",
            "open",
            "instrument_id",
            "slot",
            "pad_id",
            "enabled",
            "name",
            "kit_id",
            "fields",
            "mode",
            "index",
            "zone",
            "from",
            "to",
            "op",
            "region_id",
            "layer_id",
        ];
        if let Some(k) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(format!("unknown action field: {k}"));
        }
        let uint = |key: &str| {
            obj.get(key)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{key} must be an unsigned integer"))
        };
        let nullable = |key: &str| -> Result<Option<u64>, String> {
            match obj.get(key) {
                Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_u64()
                    .map(Some)
                    .ok_or_else(|| format!("{key} must be integer or null")),
                None => Err(format!("{key} required")),
            }
        };
        match name {
            "set_open" => {
                self.open = obj
                    .get("open")
                    .and_then(Value::as_bool)
                    .ok_or("open must be boolean")?;
            }
            "select_instrument" => {
                let id = nullable("instrument_id")?.map(InstrumentId);
                if id.is_some_and(|id| !editor.song.instruments.contains_key(&id)) {
                    return Err("unknown instrument".into());
                }
                self.instrument = id;
                self.sync(editor);
                self.selected = None;
                self.draft = None;
                self.insert_mode = false;
            }
            "select_slot" => {
                let slot = uint("slot")?;
                if slot > 15 {
                    return Err("slot must be 0..15".into());
                }
                let kit = self
                    .assigned_kit(editor)
                    .and_then(|id| editor.song.drum_kits.get(&id));
                self.selected_slot = slot as u8;
                self.selected = kit.and_then(|k| {
                    k.pads
                        .iter()
                        .find(|(_, p)| p.slot == slot as u8)
                        .map(|(id, _)| *id)
                });
                self.draft = None;
            }
            "select_pad" => {
                let id = nullable("pad_id")?.map(PadId);
                let kit = self
                    .assigned_kit(editor)
                    .and_then(|id| editor.song.drum_kits.get(&id));
                if let Some(id) = id {
                    let p = kit.and_then(|k| k.pads.get(&id)).ok_or("unknown pad")?;
                    self.selected_slot = p.slot;
                }
                self.selected = id;
                self.draft = None;
            }
            "set_insert_mode" => {
                let enabled = obj
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("enabled must be boolean")?;
                if enabled && !editor.edit_mode {
                    return Err("insert mode requires Edit Mode".into());
                }
                self.insert_mode = enabled;
            }
            "set_kit_name" => {
                self.kit_name = obj
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("name must be string")?
                    .to_owned()
            }
            "choose_kit" => {
                let id = nullable("kit_id")?.map(DrumKitId);
                if id.is_some_and(|id| !editor.song.drum_kits.contains_key(&id)) {
                    return Err("unknown kit".into());
                }
                self.kit_choice = id;
            }
            "create_kit_instrument" => self.create_kit_instrument(editor)?,
            "create_kit" => self.create_kit(editor)?,
            "assign_kit" => {
                let id = nullable("kit_id")?.map(DrumKitId);
                if id.is_some_and(|id| !editor.song.drum_kits.contains_key(&id)) {
                    return Err("unknown kit".into());
                }
                editor.assign_kit(self.instrument.ok_or("choose target instrument")?, id)?;
                self.sync(editor);
            }
            "unassign_kit" => {
                editor.assign_kit(self.instrument.ok_or("choose target instrument")?, None)?;
                self.sync(editor);
            }
            "remove_kit" => editor.remove_drum_kit(DrumKitId(uint("kit_id")?))?,
            "edit_mapping" => self.edit_mapping(editor)?,
            "patch_mapping" => {
                let mut d = self
                    .draft
                    .as_ref()
                    .cloned()
                    .ok_or("open mapping draft first")?;
                let f = obj
                    .get("fields")
                    .and_then(Value::as_object)
                    .ok_or("fields object required")?;
                let allowed = [
                    "name",
                    "slot",
                    "selector_midi",
                    "region",
                    "root_midi",
                    "tune_semitones",
                    "gain",
                    "pan",
                    "mode",
                    "new_note",
                    "choke_group",
                    "sources_tab",
                ];
                if let Some(k) = f.keys().find(|k| !allowed.contains(&k.as_str())) {
                    return Err(format!("unknown mapping field: {k}"));
                }
                for (k, v) in f {
                    match k.as_str() {
                        "name" => d.value.name = v.as_str().ok_or("name must be string")?.into(),
                        "slot" => {
                            let x = v
                                .as_u64()
                                .filter(|x| *x <= 15)
                                .ok_or("slot must be 0..15")?;
                            d.value.slot = x as u8
                        }
                        "selector_midi" | "root_midi" => {
                            let x = v
                                .as_u64()
                                .filter(|x| *x <= 127)
                                .ok_or("MIDI value must be 0..127")?
                                as u8;
                            if k == "selector_midi" {
                                d.value.note = x
                            } else {
                                d.value.root_note = x
                            }
                        }
                        "region" => {
                            d.value.region = if v.is_null() {
                                None
                            } else {
                                Some(RegionId(v.as_u64().ok_or("region must be integer|null")?))
                            }
                        }
                        "tune_semitones" => {
                            d.value.tune_semitones = v
                                .as_i64()
                                .filter(|x| (-48..=48).contains(x))
                                .ok_or("tune must be -48..48")?
                                as i8
                        }
                        "gain" => {
                            d.value.gain = v
                                .as_f64()
                                .filter(|x| (0.0..=1.0).contains(x))
                                .ok_or("gain must be 0..1")?
                                as f32
                        }
                        "pan" => {
                            d.value.pan = v
                                .as_f64()
                                .filter(|x| (-1.0..=1.0).contains(x))
                                .ok_or("pan must be -1..1")?
                                as f32
                        }
                        "mode" => {
                            d.value.mode = match v.as_str() {
                                Some("OneShot") => PadMode::OneShot,
                                Some("Gated") => PadMode::Gated,
                                _ => return Err("mode must be OneShot or Gated".into()),
                            }
                        }
                        "new_note" => {
                            d.value.new_note = match v.as_str() {
                                Some("Overlap") => PadNewNote::Overlap,
                                Some("ReleasePrevious") => PadNewNote::ReleasePrevious,
                                _ => {
                                    return Err(
                                        "new_note must be Overlap or ReleasePrevious".into()
                                    );
                                }
                            }
                        }
                        "choke_group" => {
                            d.value.choke_group = if v.is_null() {
                                None
                            } else {
                                Some(
                                    v.as_u64()
                                        .filter(|x| (1..=16).contains(x))
                                        .ok_or("choke_group must be 1..16 or null")?
                                        as u8,
                                )
                            }
                        }
                        "sources_tab" => {
                            d.sources_tab = v.as_bool().ok_or("sources_tab must be boolean")?
                        }
                        _ => unreachable!(),
                    }
                }
                self.draft = Some(d);
            }
            "set_source_mode" => {
                let d = self.draft.as_mut().ok_or("open mapping draft first")?;
                match obj.get("mode").and_then(Value::as_str) {
                    Some("single") => d.sources.layered = false,
                    Some("velocity_zones") => d.sources.enable_zones(d.value.region)?,
                    _ => return Err("mode must be single or velocity_zones".into()),
                }
            }
            "patch_zone" => {
                let d = self.draft.as_mut().ok_or("open mapping draft first")?;
                let i = uint("index")? as usize;
                let z = d
                    .sources
                    .zones
                    .get_mut(i)
                    .ok_or("zone index out of range")?;
                let f = obj
                    .get("fields")
                    .and_then(Value::as_object)
                    .ok_or("fields object required")?;
                if let Some(k) = f
                    .keys()
                    .find(|k| k.as_str() != "min" && k.as_str() != "max")
                {
                    return Err(format!("unknown zone field: {k}"));
                }
                for (k, v) in f {
                    let text = v
                        .as_str()
                        .ok_or("zone boundary fields are text")?
                        .to_owned();
                    if k == "min" {
                        z.min = text
                    } else {
                        z.max = text
                    }
                }
            }
            "source_edit" => {
                let d = self.draft.as_mut().ok_or("open mapping draft first")?;
                let z = uint("zone")? as usize;
                let i = uint("index").unwrap_or(0) as usize;
                match obj.get("op").and_then(Value::as_str) {
                    Some("split") => d.sources.split(z)?,
                    Some("remove_zone") => {
                        if d.sources.zones.len() <= 1 {
                            return Err("at least one zone required".into());
                        }
                        if z >= d.sources.zones.len() {
                            return Err("zone index out of range".into());
                        }
                        d.sources.zones.remove(z);
                    }
                    Some("add_alternative") => d.sources.add_alternative(z)?,
                    Some("move_alternative") => {
                        let from = uint("from")? as usize;
                        let to = uint("to")? as usize;
                        let a = d
                            .sources
                            .zones
                            .get_mut(z)
                            .ok_or("zone index out of range")?;
                        if from >= a.alternatives.len() || to >= a.alternatives.len() {
                            return Err("alternative index out of range".into());
                        }
                        a.alternatives.swap(from, to)
                    }
                    Some("set_layer_region") => {
                        let index = uint("index")? as usize;
                        let region = match obj.get("region_id") {
                            Some(Value::Null) => None,
                            Some(v) => Some(RegionId(
                                v.as_u64().ok_or("region_id must be integer or null")?,
                            )),
                            None => return Err("region_id required".into()),
                        };
                        if region.is_some_and(|id| !editor.song.regions.contains_key(&id)) {
                            return Err("unknown region".into());
                        }
                        let layer = d
                            .sources
                            .zones
                            .get_mut(z)
                            .and_then(|zone| zone.alternatives.get_mut(index))
                            .ok_or("alternative index out of range")?;
                        layer.region = region;
                    }
                    Some("remove_alternative") => {
                        let a = &mut d
                            .sources
                            .zones
                            .get_mut(z)
                            .ok_or("zone index out of range")?
                            .alternatives;
                        if a.len() <= 1 || i >= a.len() {
                            return Err("cannot remove alternative".into());
                        }
                        a.remove(i);
                    }
                    _ => return Err("unknown source edit op".into()),
                }
            }
            "apply_mapping" => {
                let d = self.draft.take().ok_or("open mapping draft first")?;
                if d.revision != editor.revision() {
                    self.draft = Some(d);
                    return Err("mapping draft stale".into());
                }
                let (region, zones) = d.sources.parse(d.value.region)?;
                let mut value = d.value.clone();
                value.region = region;
                value.velocity_zones = zones;
                match editor.upsert_drum_pad(d.kit, d.id, value) {
                    Ok(()) => {
                        self.selected = Some(d.id);
                        self.selected_slot = d.value.slot;
                    }
                    Err(e) => {
                        self.draft = Some(d);
                        return Err(e);
                    }
                }
            }
            "cancel_mapping" => self.draft = None,
            "remove_pad" => {
                let d = self.draft.as_ref().ok_or("open mapping draft first")?;
                editor.remove_drum_pad(d.kit, d.id)?;
                self.draft = None;
                self.selected = None;
            }
            "insert_note" => {
                if !self.insert(editor) {
                    return Err(self.error.clone().unwrap_or_else(|| "insert failed".into()));
                }
            }
            "audition_start" => {
                let id = self.selected.ok_or("select a pad before audition")?;
                let playable = self
                    .assigned_kit(editor)
                    .and_then(|kit| editor.song.drum_kits.get(&kit))
                    .and_then(|kit| kit.pads.get(&id))
                    .is_some_and(DrumPad::has_source);
                if !playable || self.draft.is_some() {
                    return Err("audition requires a committed pad with a source".into());
                }
                self.audition_action = self
                    .audition_request()
                    .map(crate::audition_ui::Action::Start);
                if self.audition_action.is_none() {
                    return Err("select a committed mapped pad before audition".into());
                }
            }
            "audition_stop" => self.audition_action = Some(crate::audition_ui::Action::Stop),
            "inspect_region" => {
                let id = RegionId(uint("region_id")?);
                if !editor.song.regions.contains_key(&id) {
                    return Err("unknown region".into());
                }
                self.inspect_region = self.instrument.map(|i| (id, i));
            }
            "inspect_layer" => {
                self.inspect_layer = Some(PadLayerId(uint("layer_id")?));
            }
            _ => return Err(format!("unknown pads action: {name}")),
        }
        Ok(self.agent_state(editor))
    }
    pub fn reset_document(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn selected_instrument(&self) -> Option<InstrumentId> {
        self.instrument
    }
    /// Selection from a current, explicit step projection; never infer column memory here.
    pub(crate) fn select_projected_pad(
        &mut self,
        editor: &Editor,
        instrument: InstrumentId,
        id: PadId,
    ) {
        let Some(InstrumentKind::DrumKit { kit }) = editor
            .song
            .instruments
            .get(&instrument)
            .map(|instrument| &instrument.kind)
        else {
            return;
        };
        let Some(pad) = editor
            .song
            .drum_kits
            .get(kit)
            .and_then(|kit| kit.pads.get(&id))
        else {
            return;
        };
        if self.instrument != Some(instrument)
            || self.resolved_kit != Some(*kit)
            || self.selected != Some(id)
        {
            self.draft = None;
        }
        self.instrument = Some(instrument);
        self.resolved_kit = Some(*kit);
        self.selected = Some(id);
        self.selected_slot = pad.slot;
    }
    pub(crate) fn audition_request(&self) -> Option<crate::audition_ui::Request> {
        if !self.open || self.draft.is_some() {
            return None;
        }
        Some(crate::audition_ui::Request::Pad {
            instrument: self.instrument?,
            pad: self.selected?,
        })
    }
    pub(crate) fn take_audition_action(&mut self) -> Option<crate::audition_ui::Action> {
        self.audition_action.take()
    }
    pub fn take_region_request(&mut self) -> Option<(RegionId, InstrumentId)> {
        self.inspect_region.take()
    }
    fn assigned_kit(&self, editor: &Editor) -> Option<DrumKitId> {
        match &editor.song.instruments.get(&self.instrument?)?.kind {
            InstrumentKind::DrumKit { kit } => Some(*kit),
            InstrumentKind::Pitched | InstrumentKind::Multisample { .. } => None,
        }
    }
    fn sync(&mut self, editor: &Editor) {
        let assigned = self.assigned_kit(editor);
        if self.resolved_kit != assigned {
            self.selected = None;
            self.draft = None;
            self.insert_mode = false;
            self.resolved_kit = assigned;
        }
        let cell = (
            editor.occurrence,
            editor.track,
            editor.column,
            editor.row,
            editor.cell(),
        );
        if self.last_cell.as_ref() != Some(&cell) {
            if let (Some(instrument), Note::Note(note)) = (cell.4.instrument, cell.4.note) {
                if let Some(InstrumentKind::DrumKit { kit }) = editor
                    .song
                    .instruments
                    .get(&instrument)
                    .map(|value| &value.kind)
                {
                    if let Some((id, pad)) = editor
                        .song
                        .drum_kits
                        .get(kit)
                        .and_then(|kit| kit.pads.iter().find(|(_, pad)| pad.note == note))
                    {
                        if self.instrument != Some(instrument)
                            || self.resolved_kit != Some(*kit)
                            || self.selected != Some(*id)
                        {
                            self.draft = None;
                        }
                        self.instrument = Some(instrument);
                        self.resolved_kit = Some(*kit);
                        self.selected = Some(*id);
                        self.selected_slot = pad.slot;
                    }
                }
            }
            self.last_cell = Some(cell);
        }
        if self
            .instrument
            .is_some_and(|id| !editor.song.instruments.contains_key(&id))
        {
            self.instrument = None;
            self.selected = None;
            self.draft = None;
        }
        let kit = self
            .assigned_kit(editor)
            .and_then(|id| editor.song.drum_kits.get(&id));
        if self
            .selected
            .is_some_and(|id| kit.is_none_or(|kit| !kit.pads.contains_key(&id)))
        {
            self.selected = None;
        }
        if let Some(pad) = self.selected.and_then(|id| kit?.pads.get(&id)) {
            self.selected_slot = pad.slot;
        }
        if self.draft.as_ref().is_some_and(|draft| {
            draft.revision != editor.revision() || !editor.song.drum_kits.contains_key(&draft.kit)
        }) {
            self.draft = None;
            self.error = Some(
                "Mapping draft closed: document changed. Reopen the current pad before applying."
                    .into(),
            );
        }
        if !editor.edit_mode {
            self.insert_mode = false;
        }
    }
    fn result(&mut self, result: Result<(), String>) -> bool {
        self.error = result.err();
        self.error.is_none()
    }
    fn create_kit_instrument(&mut self, editor: &mut Editor) -> Result<(), String> {
        let name = if self.kit_name.trim().is_empty() {
            "New drum kit"
        } else {
            self.kit_name.trim()
        };
        let (instrument, kit) = editor.create_drum_kit_instrument(name.into())?;
        self.instrument = Some(instrument);
        self.resolved_kit = Some(kit);
        self.kit_choice = Some(kit);
        self.selected = None;
        self.selected_slot = 0;
        self.draft = None;
        self.insert_mode = false;
        Ok(())
    }
    fn create_kit(&mut self, editor: &mut Editor) -> Result<(), String> {
        let instrument = self.instrument.ok_or("Choose a target instrument first")?;
        let id = DrumKitId(next_id(editor.song.drum_kits.keys().map(|id| id.0))?);
        let mut candidate = editor.song.clone();
        candidate.upsert_drum_kit(id, DrumKitInstrument {
            name: if self.kit_name.trim().is_empty() {
                format!("Kit {}", id.0)
            } else {
                self.kit_name.trim().into()
            },
            pads: BTreeMap::new(),
        })?;
        candidate.assign_kit(instrument, Some(id))?;
        editor.apply_document_edit(candidate)?;
        self.kit_choice = Some(id);
        self.selected = None;
        self.selected_slot = 0;
        Ok(())
    }
    fn edit_mapping(&mut self, editor: &Editor) -> Result<(), String> {
        let kit_id = self
            .assigned_kit(editor)
            .ok_or("Assign a kit to the target instrument first")?;
        let kit = editor
            .song
            .drum_kits
            .get(&kit_id)
            .ok_or("Kit no longer exists")?;
        let (id, value) = if let Some((id, pad)) = self
            .selected
            .and_then(|id| kit.pads.get(&id).map(|pad| (id, pad)))
        {
            (id, pad.clone())
        } else {
            let note = (36..=127)
                .chain(0..36)
                .find(|note| kit.pads.values().all(|pad| pad.note != *note))
                .ok_or("No unused selector note")?;
            (PadId(next_id(kit.pads.keys().map(|id| id.0))?), DrumPad {
                slot: self.selected_slot,
                name: format!("Pad {:02}", self.selected_slot + 1),
                note,
                region: None,
                velocity_zones: vec![],
                root_note: 60,
                tune_semitones: 0,
                gain: 1.0,
                pan: 0.0,
                mode: PadMode::OneShot,
                new_note: PadNewNote::Overlap,
                choke_group: None,
            })
        };
        self.draft = Some(MappingDraft {
            revision: editor.revision(),
            kit: kit_id,
            id,
            sources: SourcesDraft::from_pad(&value),
            sources_tab: !value.velocity_zones.is_empty(),
            value,
        });
        Ok(())
    }
    fn insert(&mut self, editor: &mut Editor) -> bool {
        let result = self
            .instrument
            .ok_or("Choose a target instrument".into())
            .and_then(|instrument| {
                let pad = self.selected.ok_or("Select a mapped pad")?;
                editor.insert_pad_note(instrument, pad)
            });
        self.result(result)
    }
    /// Returns true only after successful ordinary-cell insertion.
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> bool {
        self.sync(editor);
        let enabled = ui.is_enabled();
        let writable = enabled && editor.edit_mode;
        let cell_writable = writable && !crate::tracker_ui::cell_is_read_only(editor);
        if !cell_writable {
            self.insert_mode = false;
        }
        let mut inserted = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("PADS").strong().color(AQUA));
            if ui.small_button("< Order").clicked() {
                self.open = false;
                self.insert_mode = false;
            }
        });
        egui::ComboBox::from_id_salt("pad-instrument")
            .width(ui.available_width() - 8.0)
            .selected_text(self.instrument.map_or("Choose instrument".into(), |id| {
                format!("Instrument {}", id.0)
            }))
            .show_ui(ui, |ui| {
                for (id, instrument) in &editor.song.instruments {
                    if ui
                        .selectable_value(
                            &mut self.instrument,
                            Some(*id),
                            format!("{} · {}", id.0, instrument.name),
                        )
                        .clicked()
                    {
                        self.selected = None;
                        self.draft = None;
                        self.insert_mode = false;
                    }
                }
            });
        let kit_id = self.assigned_kit(editor);
        let kit = kit_id
            .and_then(|id| editor.song.drum_kits.get(&id))
            .cloned();
        let popup_enabled = enabled;
        ui.horizontal(|ui| {
        let menu = ui.menu_button("Kit…", |ui| {
            ui.set_max_width(340.0);
            ui.label("Instrument source: kit OR pitched sample/region");
            ui.label(RichText::new("New instrument preserves the song. Assigning to an existing instrument replaces its source mapping; Undo restores it.").small().color(WARNING));
            ui.add_enabled_ui(popup_enabled && editor.edit_mode, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.kit_name).hint_text("New kit name").desired_width(180.0));
                if ui.button("New kit + new instrument").on_hover_text("Allocate both fresh IDs as one undoable edit. Keep all existing notes, mappings and sources.").clicked() { let result = self.create_kit_instrument(editor); self.result(result); }
                ui.add_enabled_ui(self.instrument.is_some(), |ui| {
                if ui.button("New kit + assign").clicked() { let result = self.create_kit(editor); self.result(result); }
                if ui.add_enabled(kit_id.is_some() && !self.kit_name.trim().is_empty(), egui::Button::new("Rename assigned kit")).clicked() {
                    if let Some((id, mut kit)) = kit_id.and_then(|id| editor.song.drum_kits.get(&id).map(|kit| (id, kit.clone()))) {
                        kit.name = self.kit_name.trim().into();
                        let result = editor.upsert_drum_kit(id, kit); self.result(result);
                    }
                }
                egui::ComboBox::from_id_salt("existing-kit").selected_text(self.kit_choice.map_or("Choose existing kit".into(), |id| format!("Kit {}", id.0))).show_ui(ui, |ui| {
                    for (id, kit) in &editor.song.drum_kits { ui.selectable_value(&mut self.kit_choice, Some(*id), format!("{} · {}", id.0, kit.name)); }
                });
                if ui.add_enabled(self.kit_choice.is_some(), egui::Button::new("Assign chosen kit")).clicked() {
                    let result = editor.assign_kit(self.instrument.unwrap(), self.kit_choice); self.result(result);
                }
                if ui.add_enabled(kit_id.is_some(), egui::Button::new("Unassign kit")).clicked() {
                    let result = editor.assign_kit(self.instrument.unwrap(), None); self.result(result);
                }
                if ui.add_enabled(self.kit_choice.is_some(), egui::Button::new("Remove chosen kit")).on_hover_text("Refuses while any instrument uses it; no region or source is deleted.").clicked() {
                    let result = editor.remove_drum_kit(self.kit_choice.unwrap());
                    if self.result(result) { self.kit_choice = None; }
                }
                });
            });
            if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
        });
        if menu.inner.is_some() {
            ui.memory_mut(|memory| memory.surrender_focus(grid_id()));
        }
        ui.add(
            egui::Label::new(
                RichText::new(
                    kit.as_ref()
                        .map_or("No kit assigned", |kit| kit.name.as_str()),
                )
                .strong(),
            )
            .truncate(),
        );
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.insert_mode, false, "Select");
            ui.add_enabled_ui(cell_writable && kit.is_some(), |ui| {
                ui.selectable_value(&mut self.insert_mode, true, "Insert");
            });
        });
        let width = ((ui.available_width() - 9.0) / 4.0).max(30.0);
        egui::Grid::new("drum-pad-bank").spacing([3.0, 3.0]).show(ui, |ui| {
            for slot in 0..16 {
                let mapped = kit.as_ref().and_then(|kit| kit.pads.iter().find(|(_, pad)| pad.slot == slot));
                let selected = self.selected_slot == slot;
                let label = mapped.map_or_else(|| format!("{:02}\n—", slot + 1), |(_, pad)| format!("{:02}\n{}", slot + 1, note_label(Note::Note(pad.note))));
                let mut button = egui::Button::new(RichText::new(label).monospace().small()).min_size(egui::vec2(width, 42.0));
                if selected { button = button.stroke(Stroke::new(1.0_f32, AQUA)).fill(Color32::from_rgb(39, 68, 73)); }
                let playable = mapped.is_some_and(|(_, pad)| pad.has_source());
                let response = ui.add_enabled(kit.is_some() && (!self.insert_mode || playable), button);
                let response = response.on_hover_text(mapped.map_or("Unmapped slot. Select, then Edit mapping.".into(), |(id, pad)| format!("Pad {} · {}\nSelector MIDI {} · {}\n{}", id.0, pad.name, pad.note, source_summary(pad), if pad.has_source() { "Bank click is Select/Insert only; use the separate Audition selected button for a fixed committed-pad snapshot." } else { "No source assigned: insertion unavailable." })));
                if response.clicked() {
                    response.request_focus();
                    if self.selected_slot != slot || self.selected != mapped.map(|(id, _)| *id) {
                        self.draft = None;
                    }
                    self.selected_slot = slot;
                    self.selected = mapped.map(|(id, _)| *id);
                    if self.insert_mode {
                        if mapped.is_some_and(|(_, pad)| pad.has_source()) && cell_writable { inserted |= self.insert(editor); }
                        else { self.error = Some("This slot has no playable region. Edit mapping before insertion.".into()); }
                    }
                }
                if slot % 4 == 3 { ui.end_row(); }
            }
        });
        ui.label(
            RichText::new(if self.insert_mode {
                "Click = insert + advance"
            } else {
                "Click = select · no sound"
            })
            .small()
            .color(if self.insert_mode { WARNING } else { MUTED }),
        );
        let selected = self.selected.and_then(|id| kit.as_ref()?.pads.get(&id));
        ui.label(
            RichText::new(selected.map_or("Unmapped slot", |pad| pad.name.as_str()))
                .small()
                .color(MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(writable && kit.is_some(), egui::Button::new("Edit mapping"))
                .clicked()
            {
                let result = self.edit_mapping(editor);
                self.result(result);
            }
            if ui
                .add_enabled(
                    cell_writable
                        && selected.is_some_and(|pad| pad.has_source())
                        && editor.has_grid(),
                    egui::Button::new("Insert note"),
                )
                .clicked()
            {
                inserted |= self.insert(editor);
            }
        });
        let (start, stop) = self.audition.controls(
            ui,
            "Audition selected",
            selected.is_some_and(|pad| pad.has_source()) && self.draft.is_none(),
        );
        if start {
            self.audition_action = self
                .audition_request()
                .map(crate::audition_ui::Action::Start);
        }
        if stop {
            self.audition_action = Some(crate::audition_ui::Action::Stop);
        }
        if let Some(pad) = selected {
            ui.label(
                RichText::new(format!(
                    "MIDI {} = selector\nTune {:+} st · gain {:.2}",
                    pad.note, pad.tune_semitones, pad.gain
                ))
                .small()
                .color(MUTED),
            );
            let context = self
                .instrument
                .zip(kit_id)
                .zip(self.selected)
                .map(|((instrument, kit), pad)| (instrument, kit, pad));
            if self.inspect_context != context {
                self.inspect_context = context;
                self.inspect_layer = None;
            }
            if pad.velocity_zones.is_empty() {
                if let Some(region) = pad.region {
                    if ui.button("Inspect region").clicked() {
                        self.inspect_region =
                            self.instrument.map(|instrument| (region, instrument));
                    }
                }
            } else {
                ui.label(RichText::new(source_summary(pad)).small().color(AQUA));
                let choices: Vec<_> = pad.velocity_zones.iter().enumerate().flat_map(|(zone_index,zone)| zone.alternatives.iter().enumerate().map(move |(index,layer)| (layer.id,layer.region,format!("Z{} RR{} · R{}",zone_index+1,index+1,layer.region.0),format!("Zone {} bins {}..{} inclusive · RR {} · stable layer {} · region {}",zone_index+1,zone.min_velocity,zone.max_velocity,index+1,layer.id.0,layer.region.0)))).collect();
                if self
                    .inspect_layer
                    .is_some_and(|id| !choices.iter().any(|choice| choice.0 == id))
                {
                    self.inspect_layer = None;
                }
                let chosen = choices
                    .iter()
                    .find(|choice| Some(choice.0) == self.inspect_layer);
                egui::ComboBox::from_id_salt("inspect-pad-layer")
                    .width(ui.available_width() - 8.0)
                    .height(220.0)
                    .selected_text(chosen.map_or("Choose zone / RR", |choice| choice.2.as_str()))
                    .show_ui(ui, |ui| {
                        for choice in &choices {
                            ui.selectable_value(&mut self.inspect_layer, Some(choice.0), &choice.2)
                                .on_hover_text(&choice.3);
                        }
                    });
                let chosen = choices
                    .iter()
                    .find(|choice| Some(choice.0) == self.inspect_layer);
                if ui.add_enabled(chosen.is_some(),egui::Button::new("Inspect chosen region")).on_hover_text(chosen.map_or("Choose an explicit zone / RR alternative; this does not audition or advance round-robin.",|choice|choice.3.as_str())).clicked() {
                    let region=chosen.unwrap().1;
                    self.inspect_region=self.instrument.map(|instrument|(region,instrument));
                }
            }
        }
        ui.label(
            RichText::new("Ordinary cells; shared Undo. Audition is a separate read-only click, not insertion or held-pad performance.")
                .small()
                .color(MUTED),
        );
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        self.mapping_window(ui.ctx(), editor, enabled);
        inserted
    }

    fn mapping_window(&mut self, ctx: &egui::Context, editor: &mut Editor, enabled: bool) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        ctx.memory_mut(|memory| memory.surrender_focus(grid_id()));
        let mut open = true;
        let mut close = false;
        let writable = enabled
            && editor.edit_mode
            && draft.revision == editor.revision()
            && editor.song.drum_kits.contains_key(&draft.kit);
        egui::Window::new("Pad mapping").id(egui::Id::new("pad-mapping-window")).open(&mut open).collapsible(false).resizable(false).default_width(450.0).default_pos(egui::pos2(((ctx.content_rect().width() - 470.0) * 0.5).max(210.0), 108.0)).show(ctx, |ui| {
            ui.label(RichText::new(format!("KIT {} · PAD {} · slot {:02}", draft.kit.0, draft.id.0, draft.value.slot + 1)).strong().color(AQUA));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut draft.sources_tab, false, "Pad settings");
                ui.selectable_value(&mut draft.sources_tab, true, "Sources");
                ui.label(RichText::new(if draft.sources.layered { format!("{} zones / {} RR", draft.sources.zones.len(), draft.sources.total()) } else { "Single region".into() }).small().color(MUTED));
            });
            egui::ScrollArea::vertical().id_salt("pad-mapping-body").max_height((ctx.content_rect().height() - 330.0).max(120.0)).show(ui, |ui| {
                if draft.sources_tab {
                    ui.add_enabled_ui(writable, |ui| { if let Some(error) = source_controls(ui, &mut draft, &editor.song) { self.error = Some(error); } });
                    return;
                }
                ui.add_enabled_ui(writable, |ui| {
                    ui.horizontal(|ui| { ui.label("Name"); ui.add(egui::TextEdit::singleline(&mut draft.value.name).desired_width(220.0)); });
                    ui.horizontal(|ui| {
                        ui.label("Slot (0–15)"); ui.add(egui::DragValue::new(&mut draft.value.slot).range(0..=15));
                        ui.label("Selector MIDI"); ui.add(egui::DragValue::new(&mut draft.value.note).range(0..=127));
                    });
                    if draft.sources.layered {
                        ui.label(RichText::new(format!("Velocity sources: {} zones / {} ordered alternatives. Edit in Sources; metadata changes preserve this mapping.", draft.sources.zones.len(), draft.sources.total())).small().color(AQUA));
                    } else {
                        region_choice(ui, "pad-region", &mut draft.value.region, &editor.song, 320.0, true);
                    }
                    ui.horizontal(|ui| {
                        ui.label("Native reference MIDI"); ui.add(egui::DragValue::new(&mut draft.value.root_note).range(0..=127));
                        ui.label("Tune (semitones)"); ui.add(egui::DragValue::new(&mut draft.value.tune_semitones).range(-48..=48));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Gain ×"); ui.add(egui::DragValue::new(&mut draft.value.gain).range(0.0..=1.0).speed(0.01));
                        ui.label("Pan"); ui.add(egui::DragValue::new(&mut draft.value.pan).range(-1.0..=1.0).speed(0.01));
                    });
                    ui.horizontal(|ui| {
                        ui.label("OFF behavior");
                        ui.selectable_value(&mut draft.value.mode, PadMode::OneShot, "One-shot");
                        ui.selectable_value(&mut draft.value.mode, PadMode::Gated, "Gated");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Same pad retrigger");
                        ui.selectable_value(&mut draft.value.new_note, PadNewNote::Overlap, "Overlap");
                        ui.selectable_value(&mut draft.value.new_note, PadNewNote::ReleasePrevious, "Release previous");
                    });
                    ui.horizontal(|ui| {
                        let mut choke = draft.value.choke_group.is_some();
                        if ui.checkbox(&mut choke, "Choke group").changed() { draft.value.choke_group = choke.then_some(1); }
                        if let Some(group) = &mut draft.value.choke_group { ui.add(egui::DragValue::new(group).range(1..=16)); }
                    });
                });
                ui.separator();
                ui.label(RichText::new("Selector note chooses a pad; it does NOT transpose playback. Tune 0 plays at native speed; tuning shifts native reference by semitones (sum must stay MIDI 0–127). Glide uses an absolute MIDI target. Gain multiplies cell gain; pan adds/clamps.").small().color(MUTED));
                ui.label(RichText::new("One-shot ignores ordinary OFF/phrase-end; timed release/cut still override. Gated OFF releases the active logical voice. Different pads do not automatically stop each other. Same-pad policy and group choke apply within this kit instrument; choke de-clicks older group voices, not a hard cut.").small().color(MUTED));
                ui.label(RichText::new("Single source per hit (single region or velocity/RR selection). Duplicate slots/selector notes reject the whole edit. Original source stays immutable; regions are edited separately. No time-stretch. Audition uses the committed selected pad only, never this uncommitted mapping draft.").small().color(MUTED));
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(writable, egui::Button::new("Apply mapping")).clicked() {
                    let result = draft.sources.parse(draft.value.region).and_then(|(region, velocity_zones)| {
                        let mut value = draft.value.clone();
                        value.region = region; value.velocity_zones = velocity_zones;
                        editor.upsert_drum_pad(draft.kit, draft.id, value)
                    });
                    if self.result(result) { self.selected = Some(draft.id); self.selected_slot = draft.value.slot; close = true; }
                }
                let exists = editor.song.drum_kits.get(&draft.kit).is_some_and(|kit| kit.pads.contains_key(&draft.id));
                if ui.add_enabled(writable && exists, egui::Button::new("Remove pad")).clicked() {
                    let result = editor.remove_drum_pad(draft.kit, draft.id);
                    if self.result(result) { self.selected = None; close = true; }
                }
                if ui.button("Cancel").clicked() { close = true; }
            });
            if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
        });
        if open && !close {
            self.draft = Some(draft);
        }
    }
}

fn region_choice(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    region: &mut Option<RegionId>,
    song: &Song,
    width: f32,
    allow_unmap: bool,
) {
    let empty = if allow_unmap {
        "No region · unmapped"
    } else {
        "Choose region"
    };
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .height(220.0)
        .selected_text(region.map_or(empty.into(), |id| format!("Region {}", id.0)))
        .show_ui(ui, |ui| {
            ui.selectable_value(region, None, empty);
            for (id, value) in &song.regions {
                ui.selectable_value(
                    region,
                    Some(*id),
                    format!(
                        "{} · {} [{}, {})",
                        id.0, value.name, value.start_frame, value.end_frame
                    ),
                );
            }
        });
}
enum SourceAction {
    Split(usize),
    RemoveZone(usize),
    Add(usize),
    Move(usize, usize, usize),
    Remove(usize, usize),
}
fn source_controls(ui: &mut egui::Ui, draft: &mut MappingDraft, song: &Song) -> Option<String> {
    let mut error = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Source mode");
        if ui
            .selectable_label(!draft.sources.layered, "Single region")
            .clicked()
        {
            draft.sources.layered = false;
        }
        if ui
            .selectable_label(draft.sources.layered, "Velocity zones / RR")
            .clicked()
        {
            error = draft.sources.enable_zones(draft.value.region).err();
        }
    });
    if !draft.sources.layered {
        region_choice(ui, "pad-region", &mut draft.value.region, song, 320.0, true);
        ui.label(RichText::new("Apply replaces the source mode explicitly. No region means unmap; existing used notes may prevent it.").small().color(WARNING));
        return error;
    }
    ui.label(RichText::new("One source per hit, not stacked. Inclusive bins cover 0..127 without gaps/overlap. Draft only until Apply.").small().color(WARNING));
    ui.label(RichText::new("Bin = round(127 × ON velocity before pad gain). Ordered RR per instrument/pad/zone; amplitude unchanged.").small().color(MUTED));
    let details = egui::CollapsingHeader::new("Selection rules").id_salt("pad-source-rules").show(ui, |ui| {
        ui.label(RichText::new("Effective ON velocity includes ordinary column/phrase gain memory before pad gain. RR advances in arranged event order, separately for each instrument/pad/zone. Persisted vector order chooses one source per hit; no random or stacked layering. The source picker does not audition alternatives; the separate committed-pad Audition uses fixed bin 127 and first RR. Pad native reference and Tune override region root notes for every choice. Changing either adjacent bin may temporarily create a gap/overlap; correct the whole draft before Apply. Split copies source choices into fresh stable IDs; Remove never renumbers survivors or repairs other ranges.").small().color(MUTED));
    });
    if details.header_response.clicked() {
        details.header_response.request_focus();
    }
    let total = draft.sources.total();
    let zone_count = draft.sources.zones.len();
    let mut action = None;
    for (zone_index, zone) in draft.sources.zones.iter_mut().enumerate() {
        ui.separator();
        ui.push_id((draft.kit.0,draft.id.0,zone_index), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Zone {:02}",zone_index+1)).strong().color(AQUA));
                ui.label("Min bin");
                ui.add(egui::TextEdit::singleline(&mut zone.min).desired_width(38.0).id(egui::Id::new(("pad-zone-min",draft.kit.0,draft.id.0,zone_index))));
                ui.label("Max bin");
                ui.add(egui::TextEdit::singleline(&mut zone.max).desired_width(38.0).id(egui::Id::new(("pad-zone-max",draft.kit.0,draft.id.0,zone_index))));
                if ui.add_enabled(zone_count<8 && total+zone.alternatives.len()<=32,egui::Button::new("Split")).on_hover_text("Split this draft range in half. Copy source choices into new stable IDs for the upper zone; nothing is committed yet.").clicked() { action=Some(SourceAction::Split(zone_index)); }
                if ui.add_enabled(zone_count>1,egui::Button::new("Remove zone")).on_hover_text("Leaves other draft boundaries unchanged; repair coverage before Apply.").clicked() { action=Some(SourceAction::RemoveZone(zone_index)); }
            });
            let count = zone.alternatives.len();
            for (index, layer) in zone.alternatives.iter_mut().enumerate() {
                ui.push_id(layer.id.0, |ui| { ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("RR {:02}",index+1)).monospace().small());
                    ui.add_sized([50.0,18.0],egui::Label::new(format!("#{}",layer.id.0)).truncate()).on_hover_text(format!("Stable PadLayerId {}. Moving/removing alternatives never renumbers surviving IDs.",layer.id.0));
                    region_choice(ui, "region", &mut layer.region, song, 150.0, false);
                    if ui.add_enabled(index>0,egui::Button::new("Up")).on_hover_text("Move earlier in deterministic RR order").clicked() { action=Some(SourceAction::Move(zone_index,index,index-1)); }
                    if ui.add_enabled(index+1<count,egui::Button::new("Dn")).on_hover_text("Move later in deterministic RR order").clicked() { action=Some(SourceAction::Move(zone_index,index,index+1)); }
                    if ui.add_enabled(count>1,egui::Button::new("X")).on_hover_text("Remove this draft alternative only").clicked() { action=Some(SourceAction::Remove(zone_index,index)); }
                }); });
            }
            if ui.add_enabled(count<8 && total<32,egui::Button::new("+ alternate")).clicked() { action=Some(SourceAction::Add(zone_index)); }
        });
    }
    if let Some(action) = action {
        match action {
            SourceAction::Split(index) => error = draft.sources.split(index).err(),
            SourceAction::RemoveZone(index) => {
                draft.sources.zones.remove(index);
            }
            SourceAction::Add(index) => error = draft.sources.add_alternative(index).err(),
            SourceAction::Move(zone, from, to) => {
                draft.sources.zones[zone].alternatives.swap(from, to)
            }
            SourceAction::Remove(zone, index) => {
                draft.sources.zones[zone].alternatives.remove(index);
            }
        }
    }
    ui.label(
        RichText::new(format!(
            "{} / 8 zones · {} / 32 alternatives · 1..8 alternatives per zone",
            draft.sources.zones.len(),
            draft.sources.total()
        ))
        .small()
        .color(MUTED),
    );
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_schema_is_discoverable_and_unknown_action_is_atomic() {
        let mut editor = editor();
        let before = editor.song.clone();
        let mut ui = PadUi::default();
        let state = ui.agent_state(&editor);
        assert!(state["actions"]["patch_mapping"].is_object());
        assert!(
            ui.agent_action(&mut editor, None, json!({"action":"not_an_action"}))
                .is_err()
        );
        assert_eq!(editor.song, before);
        assert_eq!(ui.agent_state(&editor)["selected_pad"], Value::Null);
    }
    fn editor() -> Editor {
        let mut editor = Editor {
            edit_mode: true,
            ..Editor::default()
        };
        let mut song = editor.song.clone();
        for pattern in song.patterns.values_mut() {
            pattern.tracks.clear();
        }
        song.register_sample(crate::song::SampleId(1), crate::song::SampleAsset {
            path: "unit-no-io.wav".into(),
            sha256: "0".repeat(64),
            sample_rate: 48_000,
            channels: 1,
            frames: 4800,
        })
        .unwrap();
        song.upsert_region(RegionId(1), crate::song::SampleRegion {
            name: "Unit region".into(),
            sample: crate::song::SampleId(1),
            start_frame: 0,
            end_frame: 4800,
            root_note: 60,
            amp_envelope: None,
            forward_loop: None,
        })
        .unwrap();
        editor.apply_document_edit(song).unwrap();
        let pad = DrumPad {
            slot: 7,
            name: "Stable identity".into(),
            note: 36,
            region: Some(RegionId(1)),
            velocity_zones: vec![],
            root_note: 60,
            tune_semitones: 0,
            gain: 1.0,
            pan: 0.0,
            mode: PadMode::OneShot,
            new_note: PadNewNote::Overlap,
            choke_group: None,
        };
        editor
            .upsert_drum_kit(DrumKitId(1), DrumKitInstrument {
                name: "Kit".into(),
                pads: BTreeMap::from([(PadId(42), pad)]),
            })
            .unwrap();
        editor
            .assign_kit(InstrumentId(1), Some(DrumKitId(1)))
            .unwrap();
        editor.insert_pad_note(InstrumentId(1), PadId(42)).unwrap();
        editor.row = 0;
        editor
    }
    #[test]
    fn source_draft_split_reorder_and_metadata_apply_keep_stable_ids_and_one_undo() {
        let mut editor = editor();
        let before = editor.song.clone();
        let pad = before.drum_kits[&DrumKitId(1)].pads[&PadId(42)].clone();
        let mut sources = SourcesDraft::from_pad(&pad);
        assert_eq!(sources.parse(pad.region).unwrap(), (pad.region, vec![]));
        sources.enable_zones(pad.region).unwrap();
        sources.split(0).unwrap();
        sources.add_alternative(0).unwrap();
        assert!(
            sources.parse(pad.region).is_err(),
            "No implicit source fallback"
        );
        sources.zones[0].alternatives[1].region = Some(RegionId(1));
        sources.zones[0].alternatives.swap(0, 1);
        let (region, zones) = sources.parse(pad.region).unwrap();
        assert_eq!(region, None);
        assert_eq!(
            zones
                .iter()
                .map(|zone| (zone.min_velocity, zone.max_velocity))
                .collect::<Vec<_>>(),
            vec![(0, 63), (64, 127)]
        );
        assert_eq!(
            zones[0]
                .alternatives
                .iter()
                .map(|layer| layer.id)
                .collect::<Vec<_>>(),
            vec![PadLayerId(3), PadLayerId(1)]
        );
        assert_eq!(zones[1].alternatives[0].id, PadLayerId(2));
        assert_eq!(
            editor.song, before,
            "Draft editing never mutates ordinary cells or history"
        );
        let mut updated = pad.clone();
        updated.name = "Renamed with zones".into();
        updated.region = region;
        updated.velocity_zones = zones;
        editor
            .upsert_drum_pad(DrumKitId(1), PadId(42), updated.clone())
            .unwrap();
        assert!(editor.song.drum_kits[&DrumKitId(1)].pads[&PadId(42)].has_source());
        let reopened = SourcesDraft::from_pad(&updated);
        assert_eq!(
            reopened.parse(None).unwrap(),
            (None, updated.velocity_zones.clone())
        );
        editor.undo();
        assert_eq!(editor.song, before);
        editor.redo();
        assert_eq!(
            editor.song.drum_kits[&DrumKitId(1)].pads[&PadId(42)],
            updated
        );
    }
    #[test]
    fn source_draft_gap_invalid_integers_and_exhausted_ids_never_clamp_or_partially_apply() {
        let mut editor = editor();
        let before = editor.song.clone();
        let revision = editor.revision();
        let mut pad = before.drum_kits[&DrumKitId(1)].pads[&PadId(42)].clone();
        let mut sources = SourcesDraft::from_pad(&pad);
        sources.enable_zones(pad.region).unwrap();
        sources.split(0).unwrap();
        sources.zones[0].max = "62".into();
        let (region, zones) = sources.parse(pad.region).unwrap();
        pad.region = region;
        pad.velocity_zones = zones;
        assert!(
            editor
                .upsert_drum_pad(DrumKitId(1), PadId(42), pad.clone())
                .is_err()
        );
        assert_eq!(editor.song, before);
        assert_eq!(editor.revision(), revision);
        assert_eq!(sources.zones[0].max, "62");
        sources.zones[1].min = "63".into();
        let (region, zones) = sources.parse(None).unwrap();
        pad.region = region;
        pad.velocity_zones = zones;
        editor
            .upsert_drum_pad(DrumKitId(1), PadId(42), pad.clone())
            .unwrap();
        editor.undo();
        assert_eq!(editor.song, before);
        for bad in [
            "-1",
            "128",
            "63.000000001",
            "NaN",
            "1e2",
            "18446744073709551616",
            "",
        ] {
            sources.zones[0].max = bad.into();
            assert!(sources.parse(None).is_err(), "{bad}");
            assert_eq!(sources.zones[0].max, bad);
        }
        pad.velocity_zones = vec![VelocityZone {
            min_velocity: 0,
            max_velocity: 127,
            alternatives: vec![PadLayer {
                id: PadLayerId(u64::MAX),
                region: RegionId(1),
            }],
        }];
        let mut exhausted = SourcesDraft::from_pad(&pad);
        let exact = exhausted.clone();
        assert_eq!(exhausted.parse(None).unwrap(), (None, pad.velocity_zones));
        assert!(exhausted.split(0).is_err());
        assert_eq!(exhausted, exact);
        assert!(exhausted.add_alternative(0).is_err());
        assert_eq!(exhausted, exact);
    }
    #[test]
    fn source_mode_switches_are_explicit_and_do_not_guess_first_alternative() {
        let editor = editor();
        let mut pad = editor.song.drum_kits[&DrumKitId(1)].pads[&PadId(42)].clone();
        pad.region = None;
        pad.velocity_zones = vec![VelocityZone {
            min_velocity: 0,
            max_velocity: 127,
            alternatives: vec![PadLayer {
                id: PadLayerId(19),
                region: RegionId(1),
            }],
        }];
        let mut sources = SourcesDraft::from_pad(&pad);
        sources.layered = false;
        assert_eq!(
            sources.parse(pad.region).unwrap(),
            (None, vec![]),
            "Switching mode never silently selects RR alternative 1"
        );
        assert_eq!(
            sources.parse(Some(RegionId(1))).unwrap(),
            (Some(RegionId(1)), vec![])
        );
        sources.enable_zones(None).unwrap();
        assert_eq!(sources.parse(None).unwrap(), (None, pad.velocity_zones));
    }
    #[test]
    fn cell_selector_follows_stable_pad_identity_across_visual_slot_move() {
        let mut editor = editor();
        let mut ui = PadUi::default();
        ui.sync(&editor);
        assert_eq!(ui.instrument, Some(InstrumentId(1)));
        assert_eq!(ui.selected, Some(PadId(42)));
        assert_eq!(ui.selected_slot, 7);
        let mut moved = editor.song.drum_kits[&DrumKitId(1)].pads[&PadId(42)].clone();
        moved.slot = 2;
        editor
            .upsert_drum_pad(DrumKitId(1), PadId(42), moved.clone())
            .unwrap();
        ui.sync(&editor);
        assert_eq!(ui.selected, Some(PadId(42)));
        assert_eq!(ui.selected_slot, 2);
        editor
            .upsert_drum_kit(DrumKitId(2), DrumKitInstrument {
                name: "Different kit".into(),
                pads: BTreeMap::from([(PadId(42), moved)]),
            })
            .unwrap();
        editor
            .assign_kit(InstrumentId(1), Some(DrumKitId(2)))
            .unwrap();
        ui.sync(&editor);
        assert!(
            ui.selected.is_none(),
            "Same numeric PadId in a new kit must not silently preserve selection"
        );
    }
    #[test]
    fn mapping_conflicts_and_revision_aba_never_commit_stale_drafts() {
        let mut editor = editor();
        let mut ui = PadUi::default();
        ui.sync(&editor);
        ui.edit_mapping(&editor).unwrap();
        let revision = editor.revision();
        editor.set_timing(140.0, 4).unwrap();
        editor.undo();
        assert_ne!(editor.revision(), revision);
        ui.sync(&editor);
        assert!(ui.draft.is_none());
        ui.edit_mapping(&editor).unwrap();
        let before = editor.song.clone();
        let mut duplicate = editor.song.drum_kits[&DrumKitId(1)].pads[&PadId(42)].clone();
        duplicate.name = "Conflicting selector".into();
        duplicate.slot = 3;
        assert!(
            editor
                .upsert_drum_pad(DrumKitId(1), PadId(43), duplicate)
                .is_err()
        );
        assert_eq!(editor.song, before);
    }
}
