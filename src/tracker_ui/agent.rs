//! Semantic tracker actions shared with the native editor, without input simulation.
use super::*;
use serde_json::{json, Value};

fn field(value: &str) -> Result<Field, String> {
    match value.to_ascii_lowercase().as_str() {
        "note" => Ok(Field::Note),
        "instrument" => Ok(Field::Instrument),
        "velocity" => Ok(Field::Velocity),
        "pan" => Ok(Field::Pan),
        "delay" => Ok(Field::Delay),
        _ => Err("Unknown field".into()),
    }
}
fn number(value: &Value, name: &str) -> Result<u64, String> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{name} must be an unsigned integer"))
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}
fn selection(value: Option<Selection>) -> Value {
    value.map_or(Value::Null,|s|json!({"row":s.row,"track":s.track.0,"column":s.column,"field":format!("{:?}",s.field)}))
}

impl TrackerUi {
    pub(crate) fn agent_close_other_workflows(&mut self, keep: &str) {
        if keep != "generation" {
            self.reset_generation_document();
        }
        if keep != "transform" {
            self.transforms.reset_document();
        }
        if keep != "groove" {
            self.grooves.reset_document();
        }
        if keep != "thin" {
            self.thin.reset_document();
        }
        if keep != "cells" {
            self.cell_blocks.close();
        }
    }
    pub(crate) fn agent_reset_document(&mut self) {
        self.return_to_song();
        self.value.clear();
        self.row_input.clear();
        self.matrix_draft = None;
        self.effects_draft = EffectsDraft::default();
        self.trigger_source = None;
        self.trigger_draft = None;
        self.phrase_browser_open = false;
        self.phrase_owner = 0;
        self.phrase_selected = None;
        self.phrase_settings = None;
        self.phrase_cursor = None;
        self.error = None;
    }
    pub(crate) fn agent_state(&self) -> Value {
        let actions = json!([
            "state",
            "set_draft",
            "apply_value",
            "set_cell",
            "clear_field",
            "clear_cell",
            "off",
            "resize_pattern",
            "effects_draft",
            "effects_apply",
            "effects_reset",
            "effects_clear",
            "trigger_draft",
            "trigger_apply",
            "trigger_reset",
            "trigger_clear",
            "phrase_select",
            "phrase_enter",
            "phrase_return",
            "phrase_create",
            "phrase_draft",
            "phrase_apply",
            "phrase_remove",
            "phrase_cursor",
            "phrase_cell",
            "matrix_open",
            "matrix_source",
            "matrix_apply",
            "matrix_edit_source",
            "matrix_close",
            "order_append",
            "order_move",
            "order_remove",
            "undo",
            "redo"
        ]);
        json!({
            "actions":actions,
            "draft_fields":{"set_draft":["value","row_input","phrase_browser_open"],"effects_draft":["effects","retrigger_spacing"],"trigger_draft":["trigger"],"phrase_draft":["id","definition"],"matrix_source":["source"]},
            "value":self.value,"row_input":self.row_input,"error":self.error,
            "cursor":selection(Some(Selection::from_editor(&self.editor))),
            "cell":self.editor.has_grid().then(||self.editor.cell()),
            "effects_draft":{"source":self.effects_draft.source,"effects":self.effects_draft.value,"retrigger_spacing":self.effects_draft.retrigger_spacing,"effective":self.effects_draft.effective().ok(),"validation_error":self.effects_draft.effective().err()},
            "trigger_draft":self.trigger_draft,"phrase_browser_open":self.phrase_browser_open,"phrase_owner":self.phrase_owner,"phrase_selected":self.phrase_selected,"phrase_target":self.phrase_target,"phrase_cursor":selection(self.phrase_cursor),
            "phrase_settings":self.phrase_settings.as_ref().map(|(id,original,draft)|json!({"id":id,"original":original,"draft":draft})),
            "matrix":self.matrix_draft.as_ref().map(|draft|json!({"revision":draft.revision,"occurrence":draft.occurrence,"pattern":draft.pattern,"track":draft.track,"source":draft.source})),
            "persistent_structure":"The shared operation catalog also exposes tracks, instruments, patterns, order, phrase definitions, aliases and occurrence mutes with full schemas."
        })
    }

    fn agent_editor_action(
        &mut self,
        action: impl FnOnce(&mut Editor) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(id) = self.phrase_target {
            let phrase = self
                .editor
                .song
                .phrases
                .get(&id)
                .cloned()
                .ok_or("Selected phrase no longer exists")?;
            let mut projection = phrase_projection(&self.editor, &phrase, self.phrase_cursor)?;
            action(&mut projection)?;
            self.phrase_cursor = Some(Selection::from_editor(&projection));
            let mut candidate = phrase.clone();
            candidate.cells = projection
                .pattern()
                .tracks
                .get(&TrackId(1))
                .cloned()
                .unwrap_or_default();
            if candidate != phrase {
                self.editor.upsert_phrase(id, candidate)?;
            }
        } else {
            action(&mut self.editor)?;
        }
        self.reveal_current_cursor();
        Ok(())
    }

    fn agent_sync_effects(&mut self) -> Result<(), String> {
        if let Some(id) = self.phrase_target {
            let phrase = self
                .editor
                .song
                .phrases
                .get(&id)
                .ok_or("Selected phrase no longer exists")?;
            let projection = phrase_projection(&self.editor, phrase, self.phrase_cursor)?;
            self.effects_draft.sync(&projection);
        } else {
            if !self.editor.has_grid() {
                return Err("The document has no tracker grid".into());
            }
            self.effects_draft.sync(&self.editor);
        }
        Ok(())
    }

    pub(crate) fn agent_action(&mut self, request: Value) -> Result<Value, String> {
        let action = request
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action must be a string")?;
        let result = (|| -> Result<(), String> {
            match action {
                "state" => {
                    if self.editor.has_grid() || self.phrase_target.is_some() {
                        self.agent_sync_effects()?;
                    }
                }
                "set_draft" => {
                    for key in request.as_object().ok_or("Expected action object")?.keys() {
                        if !matches!(
                            key.as_str(),
                            "action" | "value" | "row_input" | "phrase_browser_open"
                        ) {
                            return Err(format!("Unknown tracker draft field: {key}"));
                        }
                    }
                    if let Some(value) = request.get("value") {
                        self.value = value.as_str().ok_or("value must be text")?.into();
                    }
                    if let Some(value) = request.get("row_input") {
                        self.row_input = value.as_str().ok_or("row_input must be text")?.into();
                    }
                    if let Some(value) = request.get("phrase_browser_open") {
                        self.phrase_browser_open = value
                            .as_bool()
                            .ok_or("phrase_browser_open must be boolean")?;
                    }
                }
                "apply_value" => {
                    let text = request
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or(&self.value)
                        .to_owned();
                    self.agent_editor_action(|editor| editor.enter_value(&text))?;
                }
                "set_cell" => {
                    let cell: Cell = decode(request.get("cell").cloned().ok_or("cell required")?)?;
                    self.agent_editor_action(|editor| editor.set_cell(cell))?;
                }
                "clear_field" => self.agent_editor_action(|editor| editor.clear(false))?,
                "clear_cell" => self.agent_editor_action(|editor| editor.clear(true))?,
                "off" => self.agent_editor_action(|editor| editor.enter_note(Note::Off))?,
                "resize_pattern" => {
                    if self.phrase_target.is_some() {
                        return Err(
                            "Use phrase_draft/phrase_apply to change a phrase length".into()
                        );
                    }
                    let rows = if let Some(value) = request.get("rows") {
                        value
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or("rows must fit u32")?
                    } else {
                        self.row_input.parse().map_err(|_| "Enter a row count")?
                    };
                    self.editor.resize(rows)?;
                }
                "effects_draft" => {
                    self.agent_sync_effects()?;
                    if let Some(value) = request.get("effects") {
                        self.effects_draft.set_value(decode(value.clone())?);
                    }
                    if let Some(value) = request.get("retrigger_spacing") {
                        self.effects_draft.retrigger_spacing = value
                            .as_str()
                            .ok_or("retrigger_spacing must be text")?
                            .into();
                    }
                }
                "effects_apply" => {
                    self.agent_sync_effects()?;
                    let value = self.effects_draft.effective()?;
                    self.agent_editor_action(|editor| editor.set_effects(value))?;
                }
                "effects_reset" => {
                    self.effects_draft.source = None;
                    self.agent_sync_effects()?;
                }
                "effects_clear" => {
                    self.agent_editor_action(|editor| editor.set_effects(StepEffects::default()))?
                }
                "trigger_draft" => {
                    if self.phrase_target.is_some() {
                        return Err("Nested phrase triggers are unavailable".into());
                    }
                    self.trigger_draft = decode(
                        request
                            .get("trigger")
                            .cloned()
                            .ok_or("trigger required (or null)")?,
                    )?;
                    let cell = self.editor.cell();
                    self.trigger_source = Some((
                        self.editor.revision(),
                        self.editor.occurrence,
                        self.editor.track,
                        self.editor.column,
                        self.editor.row,
                        self.editor.edit_mode,
                        cell.phrase,
                    ));
                }
                "trigger_apply" => {
                    if self.phrase_target.is_some() {
                        return Err("Nested phrase triggers are unavailable".into());
                    }
                    let source = (
                        self.editor.revision(),
                        self.editor.occurrence,
                        self.editor.track,
                        self.editor.column,
                        self.editor.row,
                        self.editor.edit_mode,
                        self.editor.cell().phrase,
                    );
                    if self.trigger_source.as_ref() != Some(&source) {
                        return Err(
                            "Phrase-trigger draft is stale; select it again for the current cell"
                                .into(),
                        );
                    }
                    self.editor.set_phrase_trigger(self.trigger_draft.clone())?;
                }
                "trigger_reset" => {
                    let phrase = self.editor.cell().phrase;
                    self.trigger_draft = phrase.clone();
                    self.trigger_source = Some((
                        self.editor.revision(),
                        self.editor.occurrence,
                        self.editor.track,
                        self.editor.column,
                        self.editor.row,
                        self.editor.edit_mode,
                        phrase,
                    ));
                }
                "trigger_clear" => self.editor.set_phrase_trigger(None)?,
                "phrase_select" => {
                    let owner = request
                        .get("instrument")
                        .and_then(Value::as_u64)
                        .unwrap_or(self.phrase_owner);
                    if !self
                        .editor
                        .song
                        .instruments
                        .contains_key(&InstrumentId(owner))
                    {
                        return Err("Unknown phrase instrument".into());
                    }
                    let id = request.get("id").and_then(Value::as_u64).map(PhraseId);
                    if id.is_some_and(|id| {
                        self.editor
                            .song
                            .phrases
                            .get(&id)
                            .is_none_or(|phrase| phrase.instrument != InstrumentId(owner))
                    }) {
                        return Err("Phrase does not belong to the selected instrument".into());
                    }
                    self.phrase_owner = owner;
                    self.phrase_selected = id;
                    self.phrase_settings = None;
                    self.phrase_browser_open = true;
                }
                "phrase_enter" => {
                    let id = request
                        .get("id")
                        .and_then(Value::as_u64)
                        .map(PhraseId)
                        .or(self.phrase_selected)
                        .ok_or("Choose a phrase")?;
                    if !self.editor.song.phrases.contains_key(&id) {
                        return Err("Unknown phrase".into());
                    }
                    self.enter_phrase(id);
                }
                "phrase_return" => self.return_to_song(),
                "phrase_create" => {
                    let next = self
                        .editor
                        .song
                        .phrases
                        .keys()
                        .next_back()
                        .map_or(Some(1), |id| id.0.checked_add(1))
                        .ok_or("Phrase ID space exhausted")?;
                    let instrument = request
                        .get("instrument")
                        .and_then(Value::as_u64)
                        .unwrap_or(self.phrase_owner);
                    let phrase = match request.get("definition") {
                        Some(value) => decode(value.clone())?,
                        None => Phrase {
                            name: format!("Phrase {next}"),
                            instrument: InstrumentId(instrument),
                            rows: 16,
                            lines_per_beat: self.editor.song.lines_per_beat,
                            note_columns: 1,
                            root_note: 60,
                            cells: BTreeMap::new(),
                        },
                    };
                    self.editor.upsert_phrase(PhraseId(next), phrase)?;
                    self.enter_phrase(PhraseId(next));
                }
                "phrase_draft" => {
                    let id = request
                        .get("id")
                        .and_then(Value::as_u64)
                        .map(PhraseId)
                        .or(self.phrase_selected)
                        .ok_or("Choose a phrase")?;
                    let original = self
                        .editor
                        .song
                        .phrases
                        .get(&id)
                        .cloned()
                        .ok_or("Unknown phrase")?;
                    let draft = decode(
                        request
                            .get("definition")
                            .cloned()
                            .ok_or("definition required")?,
                    )?;
                    self.phrase_settings = Some((id, original, draft));
                }
                "phrase_apply" => {
                    let (id, original, draft) = self
                        .phrase_settings
                        .as_ref()
                        .ok_or("Set a phrase draft first")?;
                    if self.editor.song.phrases.get(id) != Some(original) {
                        return Err("Phrase draft is stale; reload it before applying".into());
                    }
                    self.editor.upsert_phrase(*id, draft.clone())?;
                }
                "phrase_remove" => {
                    let id = request
                        .get("id")
                        .and_then(Value::as_u64)
                        .map(PhraseId)
                        .or(self.phrase_selected)
                        .ok_or("Choose a phrase")?;
                    self.editor.remove_phrase(id)?;
                }
                "phrase_cursor" => {
                    let id = self.phrase_target.ok_or("Enter a phrase first")?;
                    let phrase = &self.editor.song.phrases[&id];
                    let row =
                        u32::try_from(number(&request, "row")?).map_err(|_| "row exceeds u32")?;
                    let column = u16::try_from(number(&request, "column")?)
                        .map_err(|_| "column exceeds u16")?;
                    if row >= phrase.rows || column >= phrase.note_columns {
                        return Err("Phrase cursor out of bounds".into());
                    }
                    self.phrase_cursor = Some(Selection {
                        row,
                        track: TrackId(1),
                        column,
                        field: field(
                            request
                                .get("field")
                                .and_then(Value::as_str)
                                .unwrap_or("note"),
                        )?,
                    });
                }
                "phrase_cell" => {
                    let id = request
                        .get("id")
                        .and_then(Value::as_u64)
                        .map(PhraseId)
                        .or(self.phrase_target)
                        .ok_or("Choose a phrase")?;
                    let row =
                        u32::try_from(number(&request, "row")?).map_err(|_| "row exceeds u32")?;
                    let column = u16::try_from(number(&request, "column")?)
                        .map_err(|_| "column exceeds u16")?;
                    self.editor.set_phrase_cell(
                        id,
                        row,
                        column,
                        decode(request.get("cell").cloned().ok_or("cell required")?)?,
                    )?;
                }
                "matrix_open" => self.open_matrix_block(),
                "matrix_source" => {
                    let source: TrackAlias =
                        decode(request.get("source").cloned().ok_or("source required")?)?;
                    self.matrix_draft
                        .as_mut()
                        .ok_or("Open matrix draft first")?
                        .source = source;
                }
                "matrix_apply" => {
                    let draft = self
                        .matrix_draft
                        .as_ref()
                        .ok_or("Open matrix draft first")?;
                    if !self.editor.has_grid()
                        || draft.revision != self.editor.revision()
                        || draft.occurrence != self.editor.occurrence
                        || draft.pattern != self.editor.pattern_id()
                        || draft.track != self.editor.track
                    {
                        return Err("Matrix draft is stale".into());
                    }
                    match request
                        .get("operation")
                        .and_then(Value::as_str)
                        .ok_or("operation required")?
                    {
                        "link" => self.editor.set_track_alias(
                            draft.pattern,
                            draft.track,
                            Some(draft.source.clone()),
                        )?,
                        "unlink" => {
                            self.editor
                                .set_track_alias(draft.pattern, draft.track, None)?
                        }
                        "unique" => self.editor.make_track_unique(draft.pattern, draft.track)?,
                        "mute" => self.editor.set_occurrence_track_mute(
                            draft.occurrence,
                            draft.track,
                            request
                                .get("muted")
                                .and_then(Value::as_bool)
                                .ok_or("muted required")?,
                        )?,
                        _ => return Err("operation must be link, unlink, unique or mute".into()),
                    }
                    self.matrix_draft.as_mut().unwrap().revision = self.editor.revision();
                    self.reveal_current_cursor();
                }
                "matrix_edit_source" => {
                    if self.source_destination().is_none() {
                        return Err("Source is not in the arrangement".into());
                    }
                    self.edit_source();
                }
                "matrix_close" => self.matrix_draft = None,
                "order_append" => self.editor.append(
                    request
                        .get("independent")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )?,
                "order_move" => {
                    let delta = request
                        .get("delta")
                        .and_then(Value::as_i64)
                        .and_then(|n| i32::try_from(n).ok())
                        .ok_or("delta must fit i32")?;
                    self.editor.move_occurrence(delta)?;
                }
                "order_remove" => self.editor.remove_occurrence()?,
                "undo" => self.editor.undo(),
                "redo" => self.editor.redo(),
                _ => return Err("Unknown tracker action; inspect actions in state".into()),
            }
            Ok(())
        })();
        self.error = result.as_ref().err().cloned();
        result?;
        Ok(self.agent_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trigger_draft_rejects_cursor_revision_and_edit_mode_changes() {
        let mut ui = TrackerUi::default();
        ui.editor.edit_mode = true;
        ui.agent_action(json!({"action":"trigger_draft","trigger":null}))
            .unwrap();
        ui.editor.row = 1;
        assert!(ui.agent_action(json!({"action":"trigger_apply"})).is_err());
        ui.agent_action(json!({"action":"trigger_reset"})).unwrap();
        ui.editor.enter_note(Note::Note(60)).unwrap();
        assert!(ui.agent_action(json!({"action":"trigger_apply"})).is_err());
        ui.agent_action(json!({"action":"trigger_reset"})).unwrap();
        ui.editor.edit_mode = false;
        assert!(ui.agent_action(json!({"action":"trigger_apply"})).is_err());
    }
    #[test]
    fn direct_tracker_draft_apply_and_undo_use_real_editor() {
        let mut ui = TrackerUi::default();
        ui.editor.edit_mode = true;
        ui.editor.edit_step = 0;
        let before = ui.editor.song.clone();
        ui.agent_action(json!({"action":"set_draft","value":"72"}))
            .unwrap();
        assert_eq!(ui.editor.song, before);
        ui.agent_action(json!({"action":"apply_value"})).unwrap();
        assert_eq!(ui.editor.cell().note, Note::Note(72));
        ui.agent_action(json!({"action":"undo"})).unwrap();
        assert_eq!(ui.editor.song, before);
    }
    #[test]
    fn direct_phrase_authoring_preserves_song_cursor_and_rejects_stale_definition() {
        let mut ui = TrackerUi::default();
        ui.editor.edit_mode = true;
        let cursor = Selection::from_editor(&ui.editor);
        ui.agent_action(json!({"action":"phrase_create","instrument":1}))
            .unwrap();
        let id = ui.phrase_target.unwrap();
        ui.agent_action(json!({"action":"phrase_cursor","row":0,"column":0,"field":"note"}))
            .unwrap();
        ui.agent_action(json!({"action":"apply_value","value":"64"}))
            .unwrap();
        assert_eq!(
            ui.editor.song.phrases[&id].cells[&0][&0].note,
            Note::Note(64)
        );
        assert_eq!(Selection::from_editor(&ui.editor), cursor);
        let phrase = ui.editor.song.phrases[&id].clone();
        ui.agent_action(json!({"action":"phrase_draft","id":id.0,"definition":phrase}))
            .unwrap();
        ui.editor
            .set_phrase_cell(
                id,
                1,
                0,
                Cell {
                    note: Note::Note(67),
                    ..Cell::default()
                },
            )
            .unwrap();
        assert!(ui.agent_action(json!({"action":"phrase_apply"})).is_err());
    }
}
