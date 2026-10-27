//! Direct local control of the same editor and transport shown by the desktop.
use super::*;
use crate::agent_bridge::{Bridge, Envelope};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

pub(super) struct Control {
    server: crate::mcp::Server,
    bridge: Option<Bridge>,
    root: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::*;
    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "leviathan-live-agent-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn setup() -> (Workspace, Leviathan, egui::Context, Control) {
        let workspace = Workspace::new();
        let ctx = egui::Context::default();
        let app = Leviathan::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let control = Control::new(&workspace.0).unwrap();
        (workspace, app, ctx, control)
    }
    fn call(
        app: &mut Leviathan,
        control: &mut Control,
        ctx: &egui::Context,
        request: Value,
    ) -> Value {
        app.agent_dispatch(control, ctx, request).unwrap()
    }
    #[test]
    fn live_typed_dispatch_edits_the_visible_editor_and_saves_shared_session() {
        let (workspace, mut app, ctx, mut control) = setup();
        let owner = app.tracker.editor.document_owner();
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_ui","arguments":{"edit_mode":true}}),
        );
        let response = call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"call","name":"leviathan_edit_batch","arguments":{"expected_revision":0,"operations":[{"op":"set_song","title":"Visible agent song","bpm":150.0}]}}),
        );
        assert_eq!(response["ok"], true);
        assert_eq!(app.tracker.editor.song.title, "Visible agent song");
        assert_eq!(app.tracker.editor.song.bpm, 150.0);
        assert!(app.tracker.editor.owns_document(&owner));
        assert!(app.tracker.editor.can_undo());
        let response = call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"call","name":"leviathan_project_save","arguments":{"expected_revision":1,"path":"visible.leviathan.json"}}),
        );
        assert_eq!(response["ok"], true);
        assert!(workspace.0.join("visible.leviathan.json").exists());
        assert_eq!(
            Path::new(&app.project_path).file_name(),
            Some(std::ffi::OsStr::new("visible.leviathan.json"))
        );
        assert!(!app.tracker.editor.dirty);
    }
    #[test]
    fn live_component_discovery_needs_no_pixels_and_reads_every_group() {
        let (_workspace, mut app, ctx, mut control) = setup();
        let discovery = call(&mut app, &mut control, &ctx, json!({"command":"discover"}));
        assert_eq!(discovery["tools"].as_array().unwrap().len(), 64);
        for command in discovery["runtime_commands"].as_array().unwrap() {
            assert!(command["inputSchema"].is_object(), "{command}");
        }
        for component in COMPONENTS {
            let response = call(
                &mut app,
                &mut control,
                &ctx,
                json!({"command":"app_component","arguments":{"component":component}}),
            );
            assert_eq!(response["ok"], true, "{component}: {response}");
            assert!(response["data"]["component_state"].is_object());
        }
        for prohibited in ["ui_screenshot", "ui_input", "ui_inspect"] {
            assert!(app
                .agent_dispatch(&mut control, &ctx, json!({"command":prohibited}))
                .is_err());
        }
    }
    #[test]
    fn live_project_replacement_revokes_visible_drafts_and_modal_controls_are_explicit() {
        let (_workspace, mut app, ctx, mut control) = setup();
        app.tracker.editor.edit_mode = true;
        app.tracker
            .agent_action(json!({"action":"set_draft","value":"72"}))
            .unwrap();
        app.tracker.editor.enter_note(Note::Note(70)).unwrap();
        let response = call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_project","arguments":{"action":"new"}}),
        );
        assert_eq!(response["data"]["confirmation_pending"], true);
        assert!(app
            .agent_dispatch(&mut control, &ctx, json!({"command":"app_play"}))
            .is_err());
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_confirm","arguments":{"action":"cancel"}}),
        );
        assert!(app.session.pending.is_none());
        let revision = control
            .server
            .agent_revision_with_document(&mut app.tracker.editor, &mut app.session);
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"call","name":"leviathan_project_new","arguments":{"expected_revision":revision,"discard_unsaved":true}}),
        );
        assert_eq!(app.tracker.agent_state()["value"], "");
        assert!(!app.tracker.editor.dirty);
        assert!(app.session.path.is_none());
    }
    #[test]
    fn live_edits_and_cancel_preserve_human_file_and_tracker_drafts() {
        let (_workspace, mut app, ctx, mut control) = setup();
        app.tracker.editor.edit_mode = true;
        app.project_path = "human-unsaved-path.json".into();
        let reply = call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"call","name":"leviathan_edit_batch","arguments":{"expected_revision":0,"operations":[{"op":"set_song","title":"Edited"}]}}),
        );
        assert_eq!(reply["ok"], true);
        assert_eq!(app.project_path, "human-unsaved-path.json");
        app.tracker
            .agent_action(json!({"action":"set_draft","value":"72"}))
            .unwrap();
        app.session.notice = "Project opened".into();
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_project","arguments":{"action":"new"}}),
        );
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_confirm","arguments":{"action":"cancel"}}),
        );
        assert_eq!(app.tracker.agent_state()["value"], "72");
        assert_eq!(app.tracker.editor.song.title, "Edited");
        assert!(app.agent_dispatch(&mut control,&ctx,json!({"command":"app_project","arguments":{"action":"set_draft","project_path":"must not stick","asset_path":42}})).is_err());
        assert_eq!(app.project_path, "human-unsaved-path.json");
    }
    #[test]
    fn live_timing_draft_requires_explicit_apply_and_rejects_stale_context() {
        let (_workspace, mut app, ctx, mut control) = setup();
        app.tracker.editor.edit_mode = true;
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_timing","arguments":{"action":"set_draft","bpm":165.0}}),
        );
        assert_eq!(app.tracker.editor.song.bpm, 120.0);
        app.tracker.editor.set_timing(130.0, 4).unwrap();
        assert!(app
            .agent_dispatch(
                &mut control,
                &ctx,
                json!({"command":"app_timing","arguments":{"action":"apply"}})
            )
            .is_err());
        assert_eq!(app.tracker.editor.song.bpm, 130.0);
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_timing","arguments":{"action":"set_draft","bpm":165.0}}),
        );
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_timing","arguments":{"action":"apply"}}),
        );
        assert_eq!(app.tracker.editor.song.bpm, 165.0);
    }
    #[test]
    fn live_transport_calls_existing_driver_and_reports_real_stop_lifecycle() {
        let (_workspace, mut app, ctx, mut control) = setup();
        app.tracker
            .editor
            .replace_document(crate::song::new_song())
            .unwrap();
        app.diagnostic_audio.enabled = true;
        app.diagnostic_audio.held = true;
        call(&mut app, &mut control, &ctx, json!({"command":"app_play"}));
        assert!(app.transport_busy());
        assert_eq!(app.diagnostic_audio.starts, 1);
        assert!(app
            .agent_dispatch(&mut control, &ctx, json!({"command":"app_play"}))
            .is_err());
        assert_eq!(app.diagnostic_audio.starts, 1);
        call(
            &mut app,
            &mut control,
            &ctx,
            json!({"command":"app_seek","arguments":{"frame":48}}),
        );
        call(&mut app, &mut control, &ctx, json!({"command":"app_stop"}));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.transport_busy() {
            app.poll_transport();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(app.diagnostic_audio.error.is_none());
    }
}

impl Control {
    fn new(root: &Path) -> Result<Self, String> {
        Ok(Self {
            server: crate::mcp::Server::new(root)?,
            bridge: None,
            root: root.canonicalize().map_err(|e| e.to_string())?,
        })
    }
    pub(super) fn connect(root: &Path, ctx: &egui::Context) -> Result<Self, String> {
        let mut control = Self::new(root)?;
        control.bridge = Some(Bridge::spawn(ctx.clone())?);
        Ok(control)
    }
}

fn failure(message: impl Into<String>) -> Value {
    json!({"ok":false,"error":{"code":"app_control_error","message":message.into()}})
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn arguments(request: &Value) -> Result<Value, String> {
    match request.get("arguments") {
        None => Ok(json!({})),
        Some(value) if value.is_object() => Ok(value.clone()),
        _ => Err("arguments must be an object".into()),
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UiSettings {
    view: Option<String>,
    edit_mode: Option<bool>,
    follow: Option<bool>,
    looping: Option<bool>,
    edit_step: Option<u32>,
    octave: Option<u8>,
    instrument: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    occurrence: usize,
    track: u64,
    row: u32,
    column: u16,
    #[serde(default)]
    field: Option<String>,
}

const RUNTIME_COMMANDS: &[(&str,&str)] = &[
    ("app_state","Inspect the visible document, cursor, views, confirmations and actual transport state"),
    ("app_ui","Set view, instrument, Edit Mode, Follow, Loop, edit step or octave"),
    ("app_cursor","Select an exact occurrence, track, row, note column and field"),
    ("app_play","Start the current Song snapshot with the existing audio driver"),
    ("app_stop","Request hard Stop; already submitted device buffers cannot be retracted"),
    ("app_seek","Cold-seek the active Song transport by output frame"),
    ("app_audition","Start/stop the currently selected sample-region or pad audition"),
    ("app_mix_apply","Submit current committed mix to the active Song transport"),
    ("app_confirm","Resolve an existing project modal: cancel, discard, save, overwrite"),
    ("app_close","Request normal application close with unsaved-project confirmation"),
    ("app_window","Request window focus or resize"),
    ("app_component","Inspect or operate exact editor drafts, selections, previews and apply/cancel actions"),
    ("app_legacy","Control the optional fixed-motif sample loader and offline export"),
    ("app_project","Control project/save/export path drafts, staged New/Open/Save and direct instrument sample assignment"),
    ("app_timing","Inspect, edit, reset and apply the same Song BPM/LPB draft as the transport menu"),
];

const COMPONENTS: &[&str] = &[
    "tracker",
    "samples",
    "pads",
    "steps",
    "multisample",
    "generation",
    "transform",
    "groove",
    "thin",
    "cells",
    "resample",
    "mix",
];

fn runtime_arguments(command: &str) -> Value {
    let boolean = json!({"type":"boolean"});
    let text = json!({"type":"string"});
    let integer = json!({"type":"integer","minimum":0});
    let (properties, required) = match command {
        "app_ui" => (
            json!({"view":{"enum":["song","samples","pads","steps","mix","legacy"]},"edit_mode":boolean,"follow":boolean,"looping":boolean,"edit_step":{"type":"integer","minimum":0,"maximum":1024},"octave":{"type":"integer","minimum":0,"maximum":9},"instrument":integer}),
            vec![],
        ),
        "app_cursor" => (
            json!({"occurrence":integer,"track":integer,"row":integer,"column":integer,"field":{"enum":["note","instrument","velocity","pan","delay"]}}),
            vec!["occurrence", "track", "row", "column"],
        ),
        "app_seek" => (json!({"frame":integer}), vec!["frame"]),
        "app_audition" => (json!({"action":{"enum":["start","stop"]}}), vec![]),
        "app_confirm" => (
            json!({"action":{"enum":["cancel","discard","save","overwrite"]},"path":text}),
            vec!["action"],
        ),
        "app_component" => (
            json!({"component":{"enum":COMPONENTS},"action":{"type":"object","required":["action"],"properties":{"action":text},"additionalProperties":true,"description":"Read component state first for its action_schema/actions and exact draft values"}}),
            vec!["component"],
        ),
        "app_project" => (
            json!({"action":{"enum":["state","set_draft","new","open","save","export","assign_sample"]},"path":text,"project_path":text,"pending_save_path":text,"song_output":text,"asset_path":text}),
            vec![],
        ),
        "app_timing" => (
            json!({"action":{"enum":["state","set_draft","reset","apply"]},"bpm":{"type":"number"},"lines_per_beat":{"type":"integer","minimum":0,"maximum":65535}}),
            vec![],
        ),
        "app_legacy" => (
            json!({"action":{"enum":["state","set_draft","load","export"]},"sample_path":text,"output_path":text,"bpm":{"type":"number"},"bars":integer}),
            vec![],
        ),
        "app_window" => (
            json!({"focus":boolean,"width":{"type":"number","minimum":900,"maximum":8192},"height":{"type":"number","minimum":620,"maximum":8192},"minimized":boolean,"maximized":boolean,"fullscreen":boolean}),
            vec![],
        ),
        _ => (json!({}), vec![]),
    };
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

fn runtime_catalog() -> Value {
    json!(RUNTIME_COMMANDS.iter().map(|(name,description)|json!({"command":name,"description":description,"inputSchema":{"type":"object","required":["command"],"properties":{"command":{"const":name},"arguments":runtime_arguments(name)},"additionalProperties":false}})).collect::<Vec<_>>())
}

fn validate_runtime_arguments(command: &str, args: &Value) -> Result<(), String> {
    if !RUNTIME_COMMANDS.iter().any(|(name, _)| *name == command) {
        return Ok(());
    }
    let schema = runtime_arguments(command);
    let fields = schema["properties"].as_object().unwrap();
    for key in args
        .as_object()
        .ok_or("arguments must be an object")?
        .keys()
    {
        if !fields.contains_key(key) {
            return Err(format!("Unknown {command} argument: {key}"));
        }
    }
    for key in schema["required"].as_array().unwrap() {
        let key = key.as_str().unwrap();
        if args.get(key).is_none() {
            return Err(format!("{command} requires {key}"));
        }
    }
    Ok(())
}

impl Leviathan {
    fn agent_state(&mut self, control: &mut Control, ctx: &egui::Context) -> Value {
        let revision = control
            .server
            .agent_revision_with_document(&mut self.tracker.editor, &mut self.session);
        let transport=self.transport_shared().map(|shared| json!({
            "running":shared.is_running(),"ready":shared.is_ready(),"ended":shared.has_ended(),
            "position_frame":shared.position(),"looping":shared.looping.load(std::sync::atomic::Ordering::Relaxed),
            "callbacks":shared.callbacks.load(std::sync::atomic::Ordering::Relaxed),
            "underrun_frames":shared.underruns.load(std::sync::atomic::Ordering::Relaxed),
        }));
        let editor = &self.tracker.editor;
        let document = json!({"mode":"live_gui","editor_revision":editor.revision(),"title":editor.song.title,"dirty":editor.dirty,"can_undo":editor.can_undo(),"can_redo":editor.can_redo(),"path":self.session.path,"bpm":editor.song.bpm,"lines_per_beat":editor.song.lines_per_beat});
        let ui = json!({"cursor":{"occurrence":editor.occurrence,"track":editor.track.0,"row":editor.row,"column":editor.column,"field":format!("{:?}",editor.field)},"edit_mode":editor.edit_mode,"edit_step":editor.edit_step,"octave":editor.octave,"view":if self.legacy_tools{"legacy"}else if self.sample_view{"samples"}else if self.tracker.steps.open{"steps"}else if self.tracker.pads.open{"pads"}else{"song"},"instrument":self.instrument_id,"follow":self.follow,"looping":self.loop_song,"window_focused":ctx.input(|i|i.focused)});
        let session = json!({"confirmation_pending":self.session.pending.is_some(),"overwrite_pending":self.session.overwrite.is_some(),"closing":self.session.close_ready,"pending_action":self.session.pending.as_ref().map(|action|match action{crate::session::Action::New=>json!({"action":"new"}),crate::session::Action::Open(path)=>json!({"action":"open","path":path}),crate::session::Action::Close=>json!({"action":"close"})}),"overwrite_path":self.session.overwrite.as_ref().map(|pending|&pending.path)});
        let audio = json!({"audio_available":self.audio_available(),"transport_busy":self.transport_busy(),"audition_owned":self.audition.owned,"agent_audition":self.agent_audition,"transport":transport,"transport_error":self.transport_error(),"mix_delivery":{"message":self.mix_delivery.message,"available":self.mix_delivery.available,"accepted_revision":self.mix_delivery.accepted_revision,"enqueued_revision":self.mix_delivery.enqueued_revision},"playback_evidence":"Runtime counters are diagnostic; not a claim of heard sound"});
        let files = json!({"jobs_busy":self.jobs.busy(),"file_drafts":{"project_path":self.project_path,"pending_save_path":self.pending_save_path,"song_output":self.song_output,"asset_path":self.asset_path},"status":self.status,"error":self.error,"components":COMPONENTS});
        let mut data = serde_json::Map::new();
        for group in [document, ui, session, audio, files] {
            if let Value::Object(group) = group {
                data.extend(group);
            }
        }
        json!({"ok":true,"revision":revision,"data":data})
    }

    fn agent_invalidate_ui(&mut self, replaced: bool) {
        if self.audition.owned {
            self.stop_transport();
            self.audition.last = None;
        }
        self.mix_editor.cancel_authorization();
        self.asset_source = None;
        self.meter_binding = None;
        self.mix_content_cache = None;
        if replaced {
            self.tracker.agent_reset_document();
            self.mix_editor.reset_document();
            self.sample_editor.reset_document();
            self.multisample_editor.reset_document();
            self.resample_editor.reset_document();
            self.tracker.pads.reset_document();
            self.tracker.steps.reset_document();
            self.tracker.thin.reset_document();
            self.tracker.cell_blocks.reset_document();
            self.tracker.transforms.reset_document();
            self.tracker.grooves.reset_document();
            self.tracker.reset_generation_document();
            self.native_picker = None;
            self.revoke_mix_owner();
            self.sample_view = false;
            self.return_to_multisample = false;
        }
        if replaced {
            self.project_path = self
                .session
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
        }
    }

    fn agent_dispatch(
        &mut self,
        control: &mut Control,
        ctx: &egui::Context,
        request: Value,
    ) -> Result<Value, String> {
        let command_name = request
            .get("command")
            .and_then(Value::as_str)
            .ok_or("command must be a string")?
            .to_owned();
        let command = command_name.as_str();
        if matches!(command, "help" | "discover" | "call") {
            let read_only = request
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| {
                    matches!(name, "leviathan_capabilities" | "leviathan_project_read")
                });
            if command == "call"
                && !read_only
                && (self.session.pending.is_some()
                    || self.session.overwrite.is_some()
                    || self.session.close_ready)
            {
                return Err("Resolve the project confirmation with app_confirm first".into());
            }
            let owner = self.tracker.editor.document_owner();
            let before = self.tracker.editor.revision();
            let before_path = self.session.path.clone();
            let mut response = control.server.agent_request_with_document(
                &mut self.tracker.editor,
                &mut self.session,
                request,
            );
            let replaced = !self.tracker.editor.owns_document(&owner);
            if replaced || self.tracker.editor.revision() != before {
                let mix_only = self.tracker.editor.track_mix_change_revision()
                    == Some(self.tracker.editor.revision());
                self.agent_invalidate_ui(replaced);
                if mix_only {
                    self.finish_mix_frame(true);
                }
            }
            if self.session.path != before_path {
                self.project_path = self
                    .session
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
            }
            response["context"] = json!({"mode":"live_gui","same_visible_document":true});
            if command == "help" || command == "discover" {
                response["runtime_commands"] = runtime_catalog();
                response["components"] = json!(COMPONENTS);
            }
            return Ok(response);
        }
        let args = arguments(&request)?;
        validate_runtime_arguments(command, &args)?;
        let component_read = (command == "app_component"
            && args
                .get("action")
                .and_then(|v| v.get("action"))
                .and_then(Value::as_str)
                .is_none_or(|action| matches!(action, "state" | "poll" | "cancel" | "close")))
            || (matches!(command, "app_project" | "app_legacy" | "app_timing")
                && args.get("action").and_then(Value::as_str) == Some("state"));
        if !matches!(command, "app_state" | "app_confirm" | "app_stop")
            && !component_read
            && (self.session.pending.is_some()
                || self.session.overwrite.is_some()
                || self.session.close_ready)
        {
            return Err("Resolve the project confirmation with app_confirm first".into());
        }
        match command {
            "app_state" => {}
            "app_component" => {
                let component = args
                    .get("component")
                    .and_then(Value::as_str)
                    .ok_or("component required; discover lists component names")?;
                let action = args
                    .get("action")
                    .cloned()
                    .unwrap_or_else(|| json!({"action":"state"}));
                let verb = action
                    .get("action")
                    .and_then(Value::as_str)
                    .ok_or("action.action must be a string")?;
                let read = matches!(verb, "state" | "poll" | "cancel" | "close");
                let opening = verb == "open";
                if !read {
                    match component {
                        "samples" | "pads" | "steps" => self.activate_agent_view(component),
                        "mix" => {
                            self.legacy_tools = false;
                        }
                        _ => {
                            self.sample_view = false;
                            self.legacy_tools = false;
                        }
                    }
                }
                let base = self
                    .session
                    .path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map(Path::to_owned)
                    .unwrap_or_else(|| control.root.clone());
                let is_state = verb == "state";
                let state = if is_state {
                    self.agent_component_state(component)?
                } else {
                    match component {
                        "tracker" => self.tracker.agent_action(action)?,
                        "samples" => self.sample_editor.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "pads" => self.tracker.pads.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "steps" => self.tracker.steps.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "multisample" => self.multisample_editor.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "generation" => self.tracker.generation.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "transform" => self.tracker.transforms.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "groove" => self.tracker.grooves.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "thin" => self.tracker.thin.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "cells" => self.tracker.cell_blocks.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "resample" => self.resample_editor.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        "mix" => self.mix_editor.agent_action(
                            &mut self.tracker.editor,
                            Some(&base),
                            action,
                        )?,
                        _ => return Err("Unknown component; use discover".into()),
                    }
                };
                if opening {
                    if component == "mix" {
                        self.open_mix();
                    } else if matches!(
                        component,
                        "generation" | "transform" | "groove" | "thin" | "cells" | "resample"
                    ) {
                        self.tracker.agent_close_other_workflows(component);
                    }
                }
                if !is_state {
                    self.agent_component_handoffs(ctx, &base)?;
                    let applied = self.mix_editor.take_applied();
                    self.finish_mix_frame(applied);
                }
                let mut reply = self.agent_state(control, ctx);
                reply["data"]["component"] = json!(component);
                reply["data"]["operation_result"] = state;
                reply["data"]["component_state"] = self.agent_component_state(component)?;
                let active = if self.sample_view {
                    "samples"
                } else if self.tracker.steps.open {
                    "steps"
                } else if self.tracker.pads.open {
                    "pads"
                } else {
                    "tracker"
                };
                reply["data"]["active_view_component"] = json!(active);
                if active != component {
                    reply["data"]["active_view_state"] = self.agent_component_state(active)?;
                }
                return Ok(reply);
            }
            "app_project" => {
                let path = |text: &str| {
                    let path = PathBuf::from(text);
                    if path.is_relative() {
                        control.root.join(path)
                    } else {
                        path
                    }
                };
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("state");
                if matches!(action, "new" | "open" | "save" | "export") {
                    self.error = None;
                }
                match action{
                    "state"=>{},
                    "set_draft"=>{
                        for name in ["project_path","pending_save_path","song_output","asset_path"] {
                            if args.get(name).is_some_and(|v| !v.is_string()) {
                                return Err("File drafts must be text".into());
                            }
                        }
                        for (name,target) in [("project_path",&mut self.project_path),("pending_save_path",&mut self.pending_save_path),("song_output",&mut self.song_output),("asset_path",&mut self.asset_path)]{
                            if let Some(value)=args.get(name){*target=value.as_str().ok_or("File drafts must be text")?.into();}
                        }
                    },
                    "new"=>self.request_action(crate::session::Action::New),
                    "open"=>{let text=args.get("path").and_then(Value::as_str).unwrap_or(&self.project_path);let path=path(text);self.request_action(crate::session::Action::Open(path));},
                    "save"=>{let text=args.get("path").and_then(Value::as_str).unwrap_or(&self.project_path);let result=self.session.save_request(&mut self.tracker.editor,path(text),false);self.project_result(result.clone());result?;},
                    "export"=>{if let Some(value)=args.get("path"){self.song_output=path(value.as_str().ok_or("path must be text")?).display().to_string();}self.export_song();},
                    "assign_sample"=>{let text=args.get("path").and_then(Value::as_str).unwrap_or(&self.asset_path);let sample=if text.trim().is_empty(){None}else{Some(path(text))};self.tracker.editor.assign_sample(crate::song::InstrumentId(self.instrument_id),sample)?;self.asset_source=None;},
                    _=>return Err("app_project action must be state, set_draft, new, open, save, export or assign_sample".into()),
                }
                if matches!(action, "new" | "open" | "save" | "export") {
                    if let Some(error) = &self.error {
                        return Err(error.clone());
                    }
                }
            }
            "app_timing" => {
                let source = (
                    self.tracker.editor.revision(),
                    self.tracker.editor.song.bpm,
                    self.tracker.editor.song.lines_per_beat,
                    self.tracker.editor.edit_mode,
                );
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("state");
                if action == "apply" && self.timing_source != Some(source) {
                    return Err("Timing draft is stale; inspect or set it again".into());
                }
                if self.timing_source != Some(source) {
                    self.timing_source = Some(source);
                    self.timing_draft = (source.1, source.2);
                }
                match action {
                    "state" => {}
                    "set_draft" => {
                        let bpm = args
                            .get("bpm")
                            .map(|v| v.as_f64().ok_or("bpm must be numeric"))
                            .transpose()?
                            .unwrap_or(self.timing_draft.0);
                        let lpb = args
                            .get("lines_per_beat")
                            .map(|v| {
                                v.as_u64()
                                    .and_then(|n| u16::try_from(n).ok())
                                    .ok_or("lines_per_beat must fit u16")
                            })
                            .transpose()?
                            .unwrap_or(self.timing_draft.1);
                        self.timing_draft = (bpm, lpb);
                    }
                    "reset" => self.timing_draft = (source.1, source.2),
                    "apply" => self
                        .tracker
                        .editor
                        .set_timing(self.timing_draft.0, self.timing_draft.1)?,
                    _ => {
                        return Err(
                            "app_timing action must be state, set_draft, apply or reset".into()
                        )
                    }
                }
                let mut reply = self.agent_state(control, ctx);
                reply["data"]["timing_draft"] = json!({"bpm":self.timing_draft.0,"lines_per_beat":self.timing_draft.1,"source":self.timing_source});
                return Ok(reply);
            }
            "app_legacy" => {
                match args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("state")
                {
                    "state" => {}
                    "set_draft" => {
                        let bpm = args
                            .get("bpm")
                            .map(|v| v.as_f64().ok_or("bpm must be numeric"))
                            .transpose()?
                            .unwrap_or(self.bpm);
                        let bars = args
                            .get("bars")
                            .map(|v| {
                                v.as_u64()
                                    .and_then(|n| u32::try_from(n).ok())
                                    .ok_or("bars must fit u32")
                            })
                            .transpose()?
                            .unwrap_or(self.bars);
                        let manifest = pattern_manifest(bpm, bars).map_err(|e| e.to_string())?;
                        let sample_path = args
                            .get("sample_path")
                            .map(|v| v.as_str().ok_or("sample_path must be text"))
                            .transpose()?;
                        let output_path = args
                            .get("output_path")
                            .map(|v| v.as_str().ok_or("output_path must be text"))
                            .transpose()?;
                        self.bpm = bpm;
                        self.bars = bars;
                        self.manifest = manifest;
                        if let Some(path) = sample_path {
                            self.sample_path = path.into();
                        }
                        if let Some(path) = output_path {
                            self.output_path = path.into();
                        }
                    }
                    "load" => {
                        if self.sample_path.trim().is_empty() {
                            return Err("Set sample_path first".into());
                        }
                        if Path::new(&self.sample_path).is_relative() {
                            self.sample_path =
                                control.root.join(&self.sample_path).display().to_string();
                        }
                        self.load();
                    }
                    "export" => {
                        if self.sample.is_none() {
                            return Err("Load a legacy sample first".into());
                        }
                        if Path::new(&self.output_path).is_relative() {
                            self.output_path =
                                control.root.join(&self.output_path).display().to_string();
                        }
                        self.export();
                    }
                    _ => {
                        return Err(
                            "app_legacy action must be state, set_draft, load or export".into()
                        )
                    }
                }
                let mut reply = self.agent_state(control, ctx);
                reply["data"]["legacy"] = json!({"sample_path":self.sample_path,"output_path":self.output_path,"bpm":self.bpm,"bars":self.bars,"manifest":self.manifest,"sample":self.sample.as_ref().map(|sample|json!({"path":sample.path,"rate":sample.rate,"channels":sample.channels,"frames":sample.frames})),"busy":self.jobs.busy()});
                return Ok(reply);
            }
            "app_ui" => {
                let settings: UiSettings = decode(args)?;
                if settings.octave.is_some_and(|v| v > 9)
                    || settings.edit_step.is_some_and(|v| v > 1024)
                {
                    return Err("octave must be 0..9; edit_step must be 0..1024".into());
                }
                if settings.instrument.is_some_and(|id| {
                    !self
                        .tracker
                        .editor
                        .song
                        .instruments
                        .contains_key(&crate::song::InstrumentId(id))
                }) {
                    return Err("Unknown instrument".into());
                }
                if settings.view.as_deref().is_some_and(|v| {
                    !matches!(v, "song" | "samples" | "pads" | "steps" | "mix" | "legacy")
                }) {
                    return Err("view must be song, samples, pads, steps, mix, or legacy".into());
                }
                if self.audition.owned
                    && self.transport_busy()
                    && (settings.follow.is_some() || settings.looping.is_some())
                {
                    return Err("Stop audition before changing Follow or Loop".into());
                }
                if let Some(value) = settings.edit_mode {
                    self.tracker.editor.edit_mode = value;
                }
                if let Some(value) = settings.edit_step {
                    self.tracker.editor.edit_step = value;
                }
                if let Some(value) = settings.octave {
                    self.tracker.editor.octave = value;
                }
                if let Some(value) = settings.instrument {
                    self.instrument_id = value;
                }
                if let Some(value) = settings.follow {
                    self.follow = value;
                }
                if let Some(value) = settings.looping {
                    self.loop_song = value;
                    if let (Some(shared), Some(song)) =
                        (self.transport_shared(), self.transport_snapshot())
                    {
                        let rows: u64 = song
                            .order
                            .iter()
                            .map(|o| song.patterns[&o.pattern].rows as u64)
                            .sum();
                        let end = (rows as f64 * RATE as f64 * 60.0
                            / (song.bpm * song.lines_per_beat as f64))
                            .round() as u64;
                        shared.set_loop(value, end);
                    }
                }
                if let Some(view) = settings.view {
                    self.activate_agent_view(&view);
                }
            }
            "app_cursor" => {
                let cursor: Cursor = decode(args)?;
                let song = &self.tracker.editor.song;
                let occurrence = song
                    .order
                    .get(cursor.occurrence)
                    .ok_or("Unknown occurrence")?;
                let track = song
                    .tracks
                    .get(&crate::song::TrackId(cursor.track))
                    .ok_or("Unknown track")?;
                if cursor.row >= song.patterns[&occurrence.pattern].rows
                    || cursor.column >= track.note_columns
                {
                    return Err("Cursor is outside pattern/column bounds".into());
                }
                let field = match cursor.field.as_deref().unwrap_or("note") {
                    "note" => crate::editor::Field::Note,
                    "instrument" => crate::editor::Field::Instrument,
                    "velocity" => crate::editor::Field::Velocity,
                    "pan" => crate::editor::Field::Pan,
                    "delay" => crate::editor::Field::Delay,
                    _ => return Err("Unknown tracker field".into()),
                };
                self.tracker
                    .editor
                    .select(cursor.occurrence, crate::song::TrackId(cursor.track))?;
                self.tracker.editor.row = cursor.row;
                self.tracker.editor.column = cursor.column;
                self.tracker.editor.field = field;
            }
            "app_play" => {
                self.error = None;
                self.play_song();
                if let Some(error) = &self.error {
                    return Err(error.clone());
                }
            }
            "app_stop" => self.stop_transport(),
            "app_seek" => {
                let frame = args
                    .get("frame")
                    .and_then(Value::as_u64)
                    .ok_or("frame must be an unsigned integer")?;
                if self.audition.owned {
                    return Err("Song seek is unavailable during audition".into());
                }
                let shared = self
                    .transport_shared()
                    .filter(|s| s.is_running())
                    .ok_or("No active Song transport")?;
                shared.seek(frame);
            }
            "app_audition" => match args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("start")
            {
                "stop" => self.stop_transport(),
                "start" => {
                    let request = self
                        .audition_request()
                        .ok_or("Select a sample-region draft or pad for audition first")?;
                    self.start_agent_audition_request(ctx, request)?;
                }
                _ => return Err("audition action must be start or stop".into()),
            },
            "app_mix_apply" => {
                self.finish_mix_frame(true);
                if self.mix_delivery.requested_revision.is_none() {
                    return Err(self.mix_delivery.message.clone());
                }
            }
            "app_confirm" => {
                if self.session.pending.is_none() && self.session.overwrite.is_none() {
                    return Err("There is no project confirmation to resolve".into());
                }
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .ok_or("confirmation action required")?;
                if action == "cancel" {
                    self.session.cancel();
                    self.error = None;
                    return Ok(self.agent_state(control, ctx));
                }
                let result = match action {
                    "discard" => self.session.discard(&mut self.tracker.editor),
                    "overwrite" => {
                        if self.session.overwrite.is_none() {
                            return Err("There is no overwrite confirmation to resolve".into());
                        }
                        self.session.confirm_overwrite(&mut self.tracker.editor)
                    }
                    "save" => {
                        let path = args
                            .get("path")
                            .and_then(Value::as_str)
                            .ok_or("save requires a workspace-relative path")?;
                        let path = PathBuf::from(path);
                        let path = if path.is_relative() {
                            control.root.join(path)
                        } else {
                            path
                        };
                        self.session
                            .save_request(&mut self.tracker.editor, path, true)
                    }
                    _ => {
                        return Err(
                            "confirmation action must be cancel, discard, save, or overwrite"
                                .into(),
                        )
                    }
                };
                self.project_result(result.clone());
                result?;
            }
            "app_close" => self.request_action(crate::session::Action::Close),
            "app_window" => {
                for name in ["focus", "minimized", "maximized", "fullscreen"] {
                    if args.get(name).is_some_and(|v| !v.is_boolean()) {
                        return Err(format!("{name} must be boolean"));
                    }
                }
                if args.get("width").is_some() || args.get("height").is_some() {
                    let width = args
                        .get("width")
                        .and_then(Value::as_f64)
                        .ok_or("width required")?;
                    let height = args
                        .get("height")
                        .and_then(Value::as_f64)
                        .ok_or("height required")?;
                    if !(900.0..=8192.0).contains(&width) || !(620.0..=8192.0).contains(&height) {
                        return Err("Window size must be 900..8192 by 620..8192 points".into());
                    }
                }
                if args.get("focus").and_then(Value::as_bool) == Some(true) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                if args.get("width").is_some() || args.get("height").is_some() {
                    let width = args
                        .get("width")
                        .and_then(Value::as_f64)
                        .ok_or("width required")?;
                    let height = args
                        .get("height")
                        .and_then(Value::as_f64)
                        .ok_or("height required")?;
                    if !(900.0..=8192.0).contains(&width) || !(620.0..=8192.0).contains(&height) {
                        return Err("Window size must be 900..8192 by 620..8192 points".into());
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        width as f32,
                        height as f32,
                    )));
                }
                if let Some(value) = args.get("minimized") {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(
                        value.as_bool().ok_or("minimized must be boolean")?,
                    ));
                }
                if let Some(value) = args.get("maximized") {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
                        value.as_bool().ok_or("maximized must be boolean")?,
                    ));
                }
                if let Some(value) = args.get("fullscreen") {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                        value.as_bool().ok_or("fullscreen must be boolean")?,
                    ));
                }
            }
            _ => return Err("Unknown live command; use discover".into()),
        }
        Ok(self.agent_state(control, ctx))
    }

    pub(super) fn poll_agent_commands(&mut self, ctx: &egui::Context) {
        let Some(mut control) = self.agent_control.take() else {
            return;
        };
        let message = control
            .bridge
            .as_ref()
            .and_then(|bridge| bridge.try_recv().ok());
        if let Some(Envelope { request, response }) = message {
            let result = self
                .agent_dispatch(&mut control, ctx, request)
                .unwrap_or_else(failure);
            let _ = response.send(result);
        }
        self.agent_control = Some(control);
    }

    fn agent_component_state(&self, component: &str) -> Result<Value, String> {
        let editor = &self.tracker.editor;
        Ok(match component {
            "tracker" => self.tracker.agent_state(),
            "samples" => self.sample_editor.agent_state(editor),
            "pads" => self.tracker.pads.agent_state(editor),
            "steps" => self.tracker.steps.agent_state(editor),
            "multisample" => self.multisample_editor.agent_state(editor),
            "generation" => self.tracker.generation.agent_state(editor),
            "transform" => self.tracker.transforms.agent_state(editor),
            "groove" => self.tracker.grooves.agent_state(editor),
            "thin" => self.tracker.thin.agent_state(editor),
            "cells" => self.tracker.cell_blocks.agent_state(editor),
            "resample" => self.resample_editor.agent_state(editor),
            "mix" => self.mix_editor.agent_state(editor),
            _ => return Err("Unknown component".into()),
        })
    }

    fn activate_agent_view(&mut self, view: &str) {
        if view == "mix" {
            self.legacy_tools = false;
            self.open_mix();
            return;
        }
        self.mix_editor.close();
        self.legacy_tools = view == "legacy";
        if matches!(view, "samples" | "pads" | "steps" | "song") {
            self.multisample_editor.open = false;
            self.resample_editor.hide();
            self.return_to_multisample = false;
            self.sample_view = view == "samples";
            match view {
                "pads" => self.tracker.set_pad_view(true),
                "steps" => self.tracker.set_step_view(true),
                "song" => {
                    self.tracker.set_pad_view(false);
                    let _ = self.tracker.agent_action(json!({"action":"phrase_return"}));
                }
                "samples" => {
                    let _ = self.tracker.agent_action(json!({"action":"phrase_return"}));
                }
                _ => {}
            }
        }
    }

    fn agent_component_handoffs(&mut self, ctx: &egui::Context, base: &Path) -> Result<(), String> {
        if let Some((instrument, pad)) = self.tracker.steps.take_pad_selection() {
            self.tracker
                .pads
                .select_projected_pad(&self.tracker.editor, instrument, pad);
            self.instrument_id = instrument.0;
        }
        if let Some((occurrence, track)) = self.sample_editor.take_generated_position() {
            self.tracker.open_song_position(occurrence, track)?;
            self.sample_view = false;
        }
        if let Some((instrument, pad)) = self.sample_editor.take_marker_bank_kit() {
            self.sample_view = false;
            self.return_to_multisample = false;
            self.tracker.set_pad_view(true);
            self.tracker
                .pads
                .select_projected_pad(&self.tracker.editor, instrument, pad);
            self.instrument_id = instrument.0;
        }
        if let Some((region, instrument)) = self.multisample_editor.take_region_request() {
            self.sample_editor.inspect_region(
                &self.tracker.editor,
                region,
                instrument,
                Some(base),
            )?;
            self.sample_view = true;
            self.return_to_multisample = true;
        }
        if let Some((region, instrument)) = self.tracker.pads.take_region_request() {
            self.sample_editor.inspect_region(
                &self.tracker.editor,
                region,
                instrument,
                Some(base),
            )?;
            self.sample_view = true;
        }
        for action in [
            self.sample_editor.take_audition_action(),
            self.tracker.pads.take_audition_action(),
        ]
        .into_iter()
        .flatten()
        {
            match action {
                crate::audition_ui::Action::Stop => self.stop_transport(),
                crate::audition_ui::Action::Invalid(error) => return Err(error),
                crate::audition_ui::Action::Start(request) => {
                    self.start_agent_audition_request(ctx, request)?
                }
            }
        }
        Ok(())
    }

    pub(super) fn start_audition_request(
        &mut self,
        ctx: &egui::Context,
        request: crate::audition_ui::Request,
    ) -> Result<(), String> {
        self.start_audition_checked(ctx, request, false)
    }

    fn start_agent_audition_request(
        &mut self,
        ctx: &egui::Context,
        request: crate::audition_ui::Request,
    ) -> Result<(), String> {
        self.start_audition_checked(ctx, request, true)
    }

    fn start_audition_checked(
        &mut self,
        ctx: &egui::Context,
        request: crate::audition_ui::Request,
        agent: bool,
    ) -> Result<(), String> {
        let previous = self.agent_audition;
        self.agent_audition = agent;
        let result = (|| -> Result<(), String> {
            if !self.audio_available() {
                return Err("Realtime audio is unavailable in this build".into());
            }
            if self.audition_blocked(ctx) {
                return Err("Audition is blocked by focus, project confirmation or closing".into());
            }
            if self.transport_busy() {
                return Err("Transport is active or still stopping; no mixing or takeover".into());
            }
            if self.audition_request().as_ref() != Some(&request) {
                return Err("Audition request changed before Start; select it again".into());
            }
            let prepared = crate::audition_ui::AuditionUi::prepare(&self.tracker.editor, request)?;
            self.start_transport(prepared.preview.song().clone(), None, false)?;
            self.audition.install(prepared);
            Ok(())
        })();
        if result.is_err() {
            self.agent_audition = previous;
        }
        result
    }
}
